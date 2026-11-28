use std::collections::{BTreeMap, BTreeSet};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context as _, Result};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::Diagnostics::Debug::*;
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::LibraryLoader::*;
use windows_sys::Win32::System::Threading::*;

use super::locator::{self, Section};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: 这个守卫拥有该句柄，上面刚检查过它有效，并且只关闭它一次。
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// `operation` 失败的错误，附上本线程最后的系统错误。拒绝访问（错误码 5）时加上前缀 `ACCESS_DENIED: `，
/// `wxkey.rs` 据此报告 `DbKeyError::AccessDenied`。
fn win_error(operation: &str) -> anyhow::Error {
    let error = std::io::Error::last_os_error();
    let kind = if error.raw_os_error() == Some(5) {
        "ACCESS_DENIED: "
    } else {
        ""
    };
    anyhow!("{kind}{operation}: {error}")
}

fn read(process: HANDLE, address: usize, bytes: &mut [u8]) -> Result<()> {
    let mut count = 0;
    // SAFETY: `bytes` 是存活、可写的缓冲区，长 `bytes.len()` 字节，`count` 是存活的局部变量；远程地址只由内核读取，
    // 地址无效时内核返回失败。
    if unsafe {
        ReadProcessMemory(
            process,
            address as *const c_void,
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            &raw mut count,
        )
    } == 0
        || count != bytes.len()
    {
        return Err(win_error("ReadProcessMemory"));
    }
    Ok(())
}

/// 进程 `pid` 中名为 `name`（不区分大小写）的模块。
///
/// 模块还没加载时报 `<name> is not loaded yet`，`wxkey.rs` 据此判断微信还没准备好（`DbKeyError::NotReady`）。
fn module(pid: u32, name: &str) -> Result<MODULEENTRY32W> {
    let snapshot =
        // SAFETY: 参数都是普通的整数；返回的句柄在下面检查，由 `Handle` 持有。
        Handle(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) });
    if snapshot.0 == INVALID_HANDLE_VALUE {
        return Err(win_error("enumerate WeChat modules"));
    }
    #[expect(clippy::cast_possible_truncation, reason = "这个结构体只有 1 KiB 左右")]
    let mut entry = MODULEENTRY32W {
        dwSize: size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    let mut found = None;
    // SAFETY: `entry` 是设好了 `dwSize` 的 `MODULEENTRY32W`，快照句柄处于打开状态。
    let mut available = unsafe { Module32FirstW(snapshot.0, &raw mut entry) } != 0;
    while available {
        let length = entry
            .szModule
            .iter()
            .position(|b| *b == 0)
            .unwrap_or(entry.szModule.len());
        if String::from_utf16_lossy(&entry.szModule[..length]).eq_ignore_ascii_case(name) {
            found = Some(entry);
            break;
        }
        // SAFETY: 与上面的 `Module32FirstW` 相同：`entry` 的 `dwSize` 保持不变，快照仍然打开。
        available = unsafe { Module32NextW(snapshot.0, &raw mut entry) } != 0;
    }
    found.with_context(|| format!("{name} is not loaded yet"))
}

/// 微信进程 `pid` 中 ntdll 的导出函数 `name`（以 NUL 结尾）的地址：两个进程加载的是同一个 `ntdll.dll`，
/// 所以取它在本进程的 ntdll 中的偏移，加上微信进程中 ntdll 的基址。
fn remote_ntdll_symbol(pid: u32, name: &[u8]) -> Result<usize> {
    let local_name: Vec<_> = "ntdll.dll".encode_utf16().chain(Some(0)).collect();
    // SAFETY: `local_name` 是以 NUL 结尾的 UTF-16 字符串，存活到调用结束之后。
    let local = unsafe { GetModuleHandleW(local_name.as_ptr()) };
    let symbol =
        // SAFETY: `local` 是 ntdll 的模块句柄，ntdll 在进程的整个生命周期内都不会卸载；
        // `name` 以 NUL 结尾（两个调用方传入的都是以 `\0` 结尾的字节串字面量）。
        unsafe { GetProcAddress(local, name.as_ptr()) }.context("ntdll debug symbol missing")?;
    let offset = (symbol as *const () as usize)
        .checked_sub(local as usize)
        .context("invalid ntdll symbol")?;
    Ok(module(pid, "ntdll.dll")?.modBaseAddr as usize + offset)
}

/// 只读取微信进程的内存、不附加调试器得到的捕获位置和 `Weixin.dll` 的版本（[`Hook::inspect`] 的结果）。
#[derive(Debug)]
pub struct KeyLocation {
    /// 微信进程中挂钩停下的地址。
    pub address: usize,
    /// 这个地址在 `Weixin.dll` 中的偏移。
    pub module_offset: usize,
    /// `Weixin.dll` 的版本，形如 `a.b.c.d`。
    pub version: String,
}

fn locate(pid: u32) -> Result<KeyLocation> {
    let module = module(pid, "Weixin.dll")?;
    let version = file_version(&module.szExePath)?;
    let signature = locator::signature(version)?;
    let process =
        // SAFETY: 参数都是普通的整数；返回的句柄在下面检查，由 `Handle` 持有。
        Handle(unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) });
    if process.0.is_null() {
        return Err(win_error("open WeChat process"));
    }
    let mut machine = 0;
    let mut native_machine = 0;
    // SAFETY: 进程句柄已打开，两个输出参数都是存活的局部变量。
    if unsafe { IsWow64Process2(process.0, &raw mut machine, &raw mut native_machine) } == 0 {
        return Err(win_error("IsWow64Process2"));
    }
    // 原生的 AMD64，或者 ARM64 主机上模拟运行的 AMD64。ARM64 和 x86 的进程不能用 x64 的 `CONTEXT` 操作。
    if !(machine == 0x8664 || machine == 0 && native_machine == 0x8664) {
        bail!("unsupported WeChat architecture: x64 is required");
    }
    let base = module.modBaseAddr as usize;
    let image_size = module.modBaseSize as usize;
    base.checked_add(image_size)
        .context("invalid module size")?;
    let mut headers = vec![0; image_size.min(64 * 1024)];
    read(process.0, base, &mut headers)?;
    let layout = locator::pe_layout(&headers, image_size)?;
    let mut matches = BTreeSet::new();
    scan_section(
        process.0,
        base,
        layout.text,
        signature.bytes.len(),
        |bytes, rva, owned| {
            matches.extend(
                locator::matches(bytes, signature)
                    .map(|offset| rva + offset)
                    .filter(|point| owned.contains(point)),
            );
        },
    )?;
    let mut exceptions = vec![0; layout.exceptions.size];
    read(process.0, base + layout.exceptions.rva, &mut exceptions)?;
    let functions = locator::function_ranges(&exceptions, layout.text)?;
    let offset = locator::capture_point(
        &matches.into_iter().collect::<Vec<_>>(),
        signature,
        &functions,
    )?;
    Ok(KeyLocation {
        address: base + offset,
        module_offset: offset,
        version: version
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join("."),
    })
}

fn file_version(path: &[u16]) -> Result<[u16; 4]> {
    let mut ignored = 0;
    // SAFETY: `path` 是 Windows 填进 `MODULEENTRY32W` 的模块路径，以 NUL 结尾；`ignored` 是存活的局部变量。
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), &raw mut ignored) };
    if size == 0 {
        return Err(win_error("GetFileVersionInfoSizeW"));
    }
    // 缓冲区用 `Vec<u32>`，返回的 `VS_FIXEDFILEINFO` 因此按 4 字节对齐。
    let mut buffer = vec![0u32; (size as usize).div_ceil(4)];
    // SAFETY: `buffer` 至少有 `size` 个字节（向上取整为整数个 `u32`），`path` 以 NUL 结尾，同上。
    if unsafe { GetFileVersionInfoW(path.as_ptr(), 0, size, buffer.as_mut_ptr().cast()) } == 0 {
        return Err(win_error("GetFileVersionInfoW"));
    }
    let mut value = std::ptr::null_mut();
    let mut length = 0;
    // SAFETY: `buffer` 中是刚读到的版本资源，子块名 `\` 以 NUL 结尾，两个输出参数都是存活的局部变量。
    if unsafe {
        VerQueryValueW(
            buffer.as_ptr().cast(),
            [u16::from(b'\\'), 0].as_ptr(),
            &raw mut value,
            &raw mut length,
        )
    } == 0
        || (length as usize) < size_of::<VS_FIXEDFILEINFO>()
        || value.is_null()
    {
        bail!("invalid WeChat version resource");
    }
    // SAFETY: `value` 指向仍然存活的 `buffer` 内部，那里至少有 `size_of::<VS_FIXEDFILEINFO>()` 个字节（上面检查过）；
    // `read_unaligned` 不要求对齐。
    let info = unsafe { std::ptr::read_unaligned(value.cast::<VS_FIXEDFILEINFO>()) };
    if info.dwSignature != 0xfeef04bd {
        bail!("invalid WeChat version signature");
    }
    Ok([
        (info.dwProductVersionMS >> 16) as u16,
        (info.dwProductVersionMS & 0xFFFF) as u16,
        (info.dwProductVersionLS >> 16) as u16,
        (info.dwProductVersionLS & 0xFFFF) as u16,
    ])
}

/// 把微信进程中 `section` 的内容（`base` 是模块的基址）按 1 MiB 分块读出，对每块调用 `visit`，参数依次是块的字节、
/// 第一个字节的 RVA，以及这一块负责的 RVA 范围（不含前后多读的部分）。
fn scan_section(
    process: HANDLE,
    base: usize,
    section: Section,
    overlap: usize,
    mut visit: impl FnMut(&[u8], usize, std::ops::Range<usize>),
) -> Result<()> {
    const CHUNK: usize = 1024 * 1024;
    let mut buffer = vec![0; CHUNK + overlap + 7];
    for offset in (0..section.size).step_by(CHUNK) {
        // 往后多读 `overlap - 1` 个字节，跨两块的匹配也能读全。
        let start = offset.saturating_sub(7);
        let end = (offset + CHUNK + overlap - 1).min(section.size);
        let bytes = &mut buffer[..end - start];
        read(process, base + section.rva + start, bytes)?;
        visit(
            bytes,
            section.rva + start,
            section.rva + offset..section.rva + (offset + CHUNK).min(section.size),
        );
    }
    Ok(())
}

#[derive(Default)]
struct Output {
    key: Option<String>,
    error: Option<String>,
    diagnostics: CaptureDiagnostics,
}

/// 捕获时的诊断计数：只记次数，不保存也不输出寄存器的值、指针和密钥。
#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
pub struct CaptureDiagnostics {
    /// 挂钩让微信停下的次数。
    pub breakpoint_hits: u64,
    /// 停下时参数没有给出 32 字节密钥（长度不是 32 或地址为 0）的次数。
    pub invalid_arguments: u64,
    /// 停下时读不到参数的次数。
    pub unreadable_arguments: u64,
    /// 停下时读不到密钥的次数。
    pub unreadable_keys: u64,
}

/// 每个挂钩的取消标志。只有挂钩的调试器线程持有标志的强引用，所以这个线程恢复调试寄存器、解除附加之后，
/// 对应的一项就不能再 `upgrade`。
fn interrupts() -> &'static Mutex<Vec<Weak<AtomicBool>>> {
    static ACTIVE: OnceLock<Mutex<Vec<Weak<AtomicBool>>>> = OnceLock::new();
    ACTIVE.get_or_init(Default::default)
}

unsafe extern "system" fn console_interrupt(event: u32) -> i32 {
    match event {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => i32::from(interrupt()),
        // 处理函数一返回，Windows 就结束进程，而且最多只等它约 5 秒：等微信中不再留有断点后才返回（#985）。
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
            let hooks: Vec<_> = interrupts()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            i32::from(interrupt_and_wait(&hooks))
        }
        _ => 0,
    }
}

/// 与 Ctrl+C 一样停下所有正在运行的挂钩；返回是否有要停下的挂钩。
///
/// 每个挂钩先恢复自己设置的调试寄存器并解除附加，再以 `interrupted by user` 失败。用于收不到 Ctrl+C 的进程，
/// 如 `weflow serve` 的作业（#287）。
pub fn interrupt() -> bool {
    cancel(&interrupts().lock().unwrap_or_else(|e| e.into_inner()))
}

/// 请 `hooks` 中仍在运行的每个挂钩停下；返回是否有这样的挂钩。
fn cancel(hooks: &[Weak<AtomicBool>]) -> bool {
    let mut handled = false;
    for cancel in hooks.iter().filter_map(Weak::upgrade) {
        cancel.store(true, Ordering::Release);
        handled = true;
    }
    handled
}

/// 停下 `hooks`，等每个挂钩的调试器线程都结束后才返回，这样微信中不会留下它们的断点；返回其中是否有仍在运行的挂钩。
fn interrupt_and_wait(hooks: &[Weak<AtomicBool>]) -> bool {
    let handled = cancel(hooks);
    for hook in hooks {
        while hook.strong_count() > 0 {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    handled
}

/// 第一个挂钩启动时加上 `console_interrupt`。它一直留到进程结束，没有挂钩在运行时把每个信号都交给下一个处理函数，
/// 因此从不移除：处理函数运行时 Windows 可能锁着处理函数的列表，在持有 [`interrupts`] 的锁的线程里移除它，
/// 可能要等待一个正在等这把锁的处理函数，形成死锁。
fn install_console_handler() -> Result<()> {
    static INSTALLED: Mutex<bool> = Mutex::new(false);
    let mut installed = INSTALLED.lock().unwrap_or_else(|e| e.into_inner());
    if !*installed {
        // SAFETY: `console_interrupt` 在进程的整个生命周期内有效，并且只在这把锁下加一次。
        if unsafe { SetConsoleCtrlHandler(Some(console_interrupt), 1) } == 0 {
            return Err(win_error("SetConsoleCtrlHandler"));
        }
        *installed = true;
    }
    Ok(())
}

/// 挂钩存在期间，把它的取消标志留在 [`interrupts`] 中。
struct InterruptGuard(Weak<AtomicBool>);
impl InterruptGuard {
    /// 登记一个新的取消标志，返回给调试器线程；
    /// 这个线程必须是它唯一的持有者：[`interrupt_and_wait`] 靠强引用的计数判断线程是否已经结束。
    fn new() -> Result<(Self, Arc<AtomicBool>)> {
        install_console_handler()?;
        let cancel = Arc::new(AtomicBool::new(false));
        let entry = Arc::downgrade(&cancel);
        interrupts()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(entry.clone());
        Ok((Self(entry), cancel))
    }
}
impl Drop for InterruptGuard {
    fn drop(&mut self) {
        interrupts()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|entry| !Weak::ptr_eq(entry, &self.0));
    }
}

/// 附加到微信进程的调试器，在接收数据库密钥的位置设了硬件断点。
///
/// 附加的线程在自己的整个生命周期内负责 Windows 的调试循环。丢弃 `Hook` 时会等这个线程结束，因此返回之前，
/// 它占用的那个调试寄存器槽位已经恢复（其他槽位不动），调试器也已解除附加。
pub struct Hook {
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<Result<()>>>,
    output: Arc<Mutex<Output>>,
    _interrupt: InterruptGuard,
    /// 挂钩的 `Weixin.dll` 的版本，形如 `a.b.c.d`。
    pub version: String,
}

/// 只显示版本：密钥会出现在 `output` 中，所以不显示它。
impl std::fmt::Debug for Hook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hook")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl Hook {
    /// 找到微信进程 `pid` 接收数据库密钥的位置，附加一个在那里停下的调试器。
    ///
    /// # Errors
    ///
    /// `pid` 为 0 或本进程、读不了进程或它的 `Weixin.dll`、这个版本没有已知的特征码、调试器 15 秒内没能附加，
    /// 或者运行被中断时返回错误。
    pub fn start(pid: u32) -> Result<Self> {
        let location = Self::inspect(pid)?;
        Self::start_at(pid, location.address, location.version)
    }

    /// 挂钩会在微信进程 `pid` 的什么位置停下：只读取进程的内存得出，不附加调试器。
    ///
    /// # Errors
    ///
    /// `pid` 为 0 或本进程，或者读不了进程或它的 `Weixin.dll`、匹配不到特征码时返回错误。
    pub fn inspect(pid: u32) -> Result<KeyLocation> {
        if pid == 0 || pid == std::process::id() {
            bail!("cannot inspect the current process or PID zero");
        }
        locate(pid)
    }

    fn start_at(pid: u32, target: usize, version: String) -> Result<Self> {
        if pid == 0 || pid == std::process::id() {
            bail!("cannot debug the current process or PID zero");
        }
        let (stop, stop_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let output = Arc::new(Mutex::new(Output::default()));
        let (interrupt, cancel) = InterruptGuard::new()?;
        let worker_output = output.clone();
        // 闭包持有 `cancel`，线程结束时才丢弃它，那时调试器已经解除附加；控制台处理函数靠这一点等待解除附加。
        let worker = std::thread::Builder::new()
            .name("weflow-key-debugger".into())
            .spawn(move || {
                let result = Debugger::attach(pid, target).and_then(|mut debugger| {
                    debugger.run(stop_rx, &ready_tx, &worker_output, &cancel)
                });
                if let Err(error) = &result {
                    worker_output
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .error = Some(error.to_string());
                    let _ = ready_tx.try_send(Err(error.to_string()));
                }
                result
            })?;
        let mut hook = Self {
            stop,
            worker: Some(worker),
            output,
            _interrupt: interrupt,
            version,
        };
        match ready_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(())) => Ok(hook),
            Ok(Err(error)) => {
                hook.cleanup()?;
                bail!(error)
            }
            Err(error) => {
                hook.cleanup()?;
                bail!("debugger initialization: {error}")
            }
        }
    }

    /// 截获密钥之后返回它（64 个小写十六进制字符），只返回一次；在此之前返回 `None`。
    ///
    /// # Errors
    ///
    /// 调试器失败或被中断时返回错误。
    pub fn poll(&mut self) -> Result<Option<String>> {
        let mut output = self.output.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &output.error {
            bail!("{error}");
        }
        Ok(output.key.take())
    }

    /// 挂钩到目前为止的计数。
    pub fn diagnostics(&self) -> CaptureDiagnostics {
        self.output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .diagnostics
    }

    /// 停下调试器，等它恢复微信的调试寄存器并解除附加；丢弃挂钩时也会这样做。
    ///
    /// # Errors
    ///
    /// 调试器线程失败时返回它的错误，线程 panic 时返回 `key debugger thread panicked`。
    pub fn cleanup(&mut self) -> Result<()> {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow!("key debugger thread panicked"))??;
        }
        Ok(())
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

/// 设了断点的一个微信线程。
///
/// 断点占用调试寄存器的第 `slot` 个槽位；`old_address`、`old_control`、`old_status` 是设置之前这个槽位的地址寄存器、
/// DR7 和 DR6 的值，恢复时只写回其中属于这个槽位的部分。
struct WatchedThread {
    handle: Handle,
    slot: usize,
    old_address: u64,
    old_control: u64,
    old_status: u64,
}

// windows-sys 声明的 `CONTEXT` 按 C 的规则对齐（8 字节），但 Windows 要求传给 `GetThreadContext`、`SetThreadContext`
// 的 x64 上下文缓冲区按 16 字节对齐。
#[repr(C, align(16))]
struct AlignedContext(CONTEXT);

// `AlignedContext` 按 16 字节对齐，`CONTEXT` 就在它的开头（#505）
const _: () = {
    assert!(align_of::<AlignedContext>() == 16);
    assert!(std::mem::offset_of!(AlignedContext, 0) == 0);
};
impl std::ops::Deref for AlignedContext {
    type Target = CONTEXT;
    fn deref(&self) -> &CONTEXT {
        &self.0
    }
}
impl std::ops::DerefMut for AlignedContext {
    fn deref_mut(&mut self) -> &mut CONTEXT {
        &mut self.0
    }
}

fn context(thread: HANDLE) -> Result<AlignedContext> {
    let mut ctx = AlignedContext(CONTEXT {
        ContextFlags: CONTEXT_ALL_AMD64,
        ..Default::default()
    });
    // SAFETY: `ctx` 是按 16 字节对齐的 `CONTEXT`（见 `AlignedContext`），已设好 `ContextFlags`；
    // 调用方打开 `thread` 时带了 `THREAD_GET_CONTEXT` 权限。
    if unsafe { GetThreadContext(thread, &raw mut ctx.0) } == 0 {
        return Err(win_error("GetThreadContext"));
    }
    Ok(ctx)
}

fn set_context(thread: HANDLE, ctx: &CONTEXT) -> Result<()> {
    // SAFETY: 每个调用方传入的都是 `context` 返回的上下文，所以按 16 字节对齐；调用方打开 `thread` 时带了
    // `THREAD_SET_CONTEXT` 权限。
    if unsafe { SetThreadContext(thread, ctx) } == 0 {
        return Err(win_error("SetThreadContext"));
    }
    Ok(())
}

fn register(ctx: &mut CONTEXT, slot: usize) -> &mut u64 {
    match slot {
        0 => &mut ctx.Dr0,
        1 => &mut ctx.Dr1,
        2 => &mut ctx.Dr2,
        _ => &mut ctx.Dr3,
    }
}

impl WatchedThread {
    fn install(handle: HANDLE, target: usize) -> Result<Self> {
        // 调试事件中的线程句柄归 Windows 所有。这里另开一个自己的句柄，`ContinueDebugEvent` 之后仍能安全使用，
        // 由 RAII 守卫关闭一次。
        // SAFETY: `handle` 是正在处理的调试事件的线程句柄，在这个事件继续之前有效。
        let tid = unsafe { GetThreadId(handle) };
        // SAFETY: 参数都是普通的整数；返回的句柄在下面检查，由 `Handle` 持有。
        let handle = Handle(unsafe {
            OpenThread(
                THREAD_GET_CONTEXT
                    | THREAD_SET_CONTEXT
                    | THREAD_SUSPEND_RESUME
                    | THREAD_QUERY_LIMITED_INFORMATION,
                0,
                tid,
            )
        });
        if handle.0.is_null() {
            return Err(win_error("OpenThread"));
        }
        let mut ctx = context(handle.0)?;
        let slot = (0..4)
            .find(|i| ctx.Dr7 & (3 << (i * 2)) == 0)
            .context("no free hardware breakpoint slot")?;
        let watched = Self {
            old_address: *register(&mut ctx, slot),
            old_control: ctx.Dr7,
            old_status: ctx.Dr6,
            slot,
            handle,
        };
        *register(&mut ctx, slot) = target as u64;
        // 这个槽位设为执行断点：DR7 中它的 RW、LEN 位清零，本地启用位设为 1；DR6 中它的状态位清零
        ctx.Dr7 = (ctx.Dr7 & !(0xf << (16 + slot * 4))) | (1 << (slot * 2));
        ctx.Dr6 &= !(1 << slot);
        ctx.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64;
        set_context(watched.handle.0, &ctx)?;
        Ok(watched)
    }

    fn restore(&self) -> Result<()> {
        let mut ctx = context(self.handle.0)?;
        // 只写回这个槽位的部分：DR7 中它的启用位和 RW、LEN 位，DR6 中它的状态位，以及它的地址寄存器
        let mask = (3 << (self.slot * 2)) | (0xf << (16 + self.slot * 4));
        ctx.Dr7 = (ctx.Dr7 & !mask) | (self.old_control & mask);
        ctx.Dr6 = (ctx.Dr6 & !(1 << self.slot)) | (self.old_status & (1 << self.slot));
        *register(&mut ctx, self.slot) = self.old_address;
        ctx.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64;
        set_context(self.handle.0, &ctx)
    }
}

/// 附加到微信进程的 Windows 调试器，只在附加它的线程上使用。
struct Debugger {
    pid: u32,
    target: usize,
    process: Option<Handle>,
    threads: BTreeMap<u32, WatchedThread>,
    attached: bool,
    /// 收到后还没继续的调试事件。它挂起期间，微信的所有线程都停着。
    pending: Option<DEBUG_EVENT>,
    /// 继续 `pending` 时传给 `ContinueDebugEvent` 的状态：`DBG_CONTINUE` 或 `DBG_EXCEPTION_NOT_HANDLED`。
    pending_disposition: i32,
    /// 微信进程中 `DbgBreakPoint` 的地址：附加时和 `DebugBreakProcess` 让进程停下时，断点都在这里触发。
    system_breakpoint: usize,
    /// 微信进程中 `DbgUiRemoteBreakin` 的地址：Windows 为让进程停下而创建的线程从这里开始执行。
    system_breakin: usize,
}

impl Debugger {
    fn attach(pid: u32, target: usize) -> Result<Self> {
        let system_breakpoint = remote_ntdll_symbol(pid, b"DbgBreakPoint\0")?;
        let system_breakin = remote_ntdll_symbol(pid, b"DbgUiRemoteBreakin\0")?;
        // SAFETY: 只传入进程 ID；`Debugger` 建好之后由 `Drop` 经 `detach` 解除附加，在此之前没有别的步骤会失败。
        if unsafe { DebugActiveProcess(pid) } == 0 {
            return Err(win_error("DebugActiveProcess"));
        }
        let debugger = Self {
            pid,
            target,
            process: None,
            threads: BTreeMap::new(),
            attached: true,
            pending: None,
            pending_disposition: DBG_CONTINUE,
            system_breakpoint,
            system_breakin,
        };
        // SAFETY: 只传入一个标志，并按 Windows 的要求在附加的线程上调用。
        if unsafe { DebugSetProcessKillOnExit(0) } == 0 {
            return Err(win_error("DebugSetProcessKillOnExit"));
        }
        Ok(debugger)
    }

    /// 调试循环：处理调试事件，直到截获密钥、收到 `stop` 或 `cancelled` 被置位；收到附加时的初始断点后向 `ready` 发送
    /// `Ok(())`。返回前解除附加，截获的密钥在此之后才写进 `output`。
    fn run(
        &mut self,
        stop: mpsc::Receiver<()>,
        ready: &mpsc::SyncSender<std::result::Result<(), String>>,
        output: &Mutex<Output>,
        cancelled: &AtomicBool,
    ) -> Result<()> {
        let mut initialized = false;
        let started = Instant::now();
        let mut captured = None;
        loop {
            if stop.try_recv().is_ok() || cancelled.load(Ordering::Acquire) {
                break;
            }
            if !initialized && started.elapsed() > Duration::from_secs(10) {
                bail!("timed out initializing WeChat debugger");
            }
            let mut event = DEBUG_EVENT::default();
            // SAFETY: `event` 是存活、可写的 `DEBUG_EVENT`，本线程就是附加的线程。
            if unsafe { WaitForDebugEvent(&raw mut event, 50) } == 0 {
                // SAFETY: 读取本线程最后的错误码，没有参数。
                if unsafe { GetLastError() } == ERROR_SEM_TIMEOUT {
                    continue;
                }
                return Err(win_error("WaitForDebugEvent"));
            }
            self.pending = Some(event);
            self.pending_disposition = if event.dwDebugEventCode == EXCEPTION_DEBUG_EVENT {
                DBG_EXCEPTION_NOT_HANDLED
            } else {
                DBG_CONTINUE
            };
            let mut disposition = DBG_CONTINUE;
            // SAFETY: 每个分支只读取 `event.u` 中由 `dwDebugEventCode` 指明的那个联合体字段；
            // 关闭的文件句柄都是调试事件交出来的，各关闭一次；
            // `OpenProcess` 的参数是普通的整数，返回的句柄由 `Handle` 持有。
            unsafe {
                match event.dwDebugEventCode {
                    CREATE_PROCESS_DEBUG_EVENT => {
                        let info = event.u.CreateProcessInfo;
                        if !info.hFile.is_null() {
                            CloseHandle(info.hFile);
                        }
                        let process = Handle(OpenProcess(PROCESS_ALL_ACCESS, 0, self.pid));
                        if process.0.is_null() {
                            return Err(win_error("OpenProcess"));
                        }
                        self.process = Some(process);
                        self.watch(event.dwThreadId, info.hThread)?;
                    }
                    CREATE_THREAD_DEBUG_EVENT => {
                        let info = event.u.CreateThread;
                        // Windows 为让进程停下而临时创建的线程（从 `DbgUiRemoteBreakin` 开始）
                        // 不会执行微信接收密钥的函数，而且清理时可能已经在退出，所以不监视它。
                        if info.lpStartAddress.map(|f| f as *const () as usize)
                            != Some(self.system_breakin)
                        {
                            self.watch(event.dwThreadId, info.hThread)?;
                        }
                    }
                    EXIT_THREAD_DEBUG_EVENT => {
                        self.threads.remove(&event.dwThreadId);
                    }
                    LOAD_DLL_DEBUG_EVENT => {
                        let file = event.u.LoadDll.hFile;
                        if !file.is_null() {
                            CloseHandle(file);
                        }
                    }
                    EXCEPTION_DEBUG_EVENT => {
                        let exception = event.u.Exception.ExceptionRecord;
                        disposition = DBG_EXCEPTION_NOT_HANDLED;
                        if exception.ExceptionCode == EXCEPTION_BREAKPOINT
                            && !initialized
                            && exception.ExceptionAddress as usize == self.system_breakpoint
                        {
                            self.pending_disposition = DBG_CONTINUE;
                            initialized = true;
                            disposition = DBG_CONTINUE;
                            let _ = ready.try_send(Ok(()));
                        } else if exception.ExceptionCode == EXCEPTION_SINGLE_STEP {
                            if let Some(thread) = self.threads.get(&event.dwThreadId) {
                                let mut ctx = context(thread.handle.0)?;
                                if ctx.Rip == self.target as u64
                                    && ctx.Dr6 & (1 << thread.slot) != 0
                                {
                                    self.pending_disposition = DBG_CONTINUE;
                                    captured = usize::try_from(ctx.Rdx)
                                        .ok()
                                        .and_then(|structure| self.read_key(structure, output));
                                    ctx.Dr6 &= !(1 << thread.slot);
                                    // 置位恢复标志（RF），继续运行时跳过这条指令上的执行断点一次。
                                    // 除此之外，整数和 SIMD 寄存器、栈和标志位都保持原样。
                                    ctx.EFlags |= 1 << 16;
                                    ctx.ContextFlags =
                                        CONTEXT_CONTROL_AMD64 | CONTEXT_DEBUG_REGISTERS_AMD64;
                                    set_context(thread.handle.0, &ctx)?;
                                    disposition = DBG_CONTINUE;
                                }
                            }
                        }
                    }
                    EXIT_PROCESS_DEBUG_EVENT => {
                        self.threads.clear();
                        self.continue_event(DBG_CONTINUE)?;
                        self.attached = false;
                        bail!("WeChat exited before a database key was captured");
                    }
                    _ => {}
                }
            }
            self.pending_disposition = disposition;
            if captured.is_some() {
                break;
            }
            self.continue_event(disposition)?;
        }
        // 清理成功之后才公布密钥：调用方报告成功时，微信一定已经解除附加，也没有留下硬件断点。
        self.detach()?;
        if cancelled.load(Ordering::Acquire) {
            bail!("interrupted by user");
        }
        output.lock().unwrap_or_else(|e| e.into_inner()).key = captured;
        Ok(())
    }

    fn watch(&mut self, tid: u32, handle: HANDLE) -> Result<()> {
        self.threads
            .insert(tid, WatchedThread::install(handle, self.target)?);
        Ok(())
    }

    /// 从 RDX 指向的参数结构读出密钥，转成 64 个小写十六进制字符：结构的 +8 是密钥的地址，+16 是字节数。
    /// 读不到参数或密钥、字节数不是 32 或地址为 0 时只计数，返回 `None`；读到的密钥在转换后清零。
    fn read_key(&self, structure: usize, output: &Mutex<Output>) -> Option<String> {
        let mut state = output.lock().unwrap_or_else(|e| e.into_inner());
        state.diagnostics.breakpoint_hits += 1;
        let process = self.process.as_ref()?.0;
        let mut fields = [0; 24];
        if read(process, structure, &mut fields).is_err() {
            state.diagnostics.unreadable_arguments += 1;
            return None;
        }
        let address = usize::try_from(u64::from_le_bytes(fields[8..16].try_into().ok()?)).ok()?;
        let size = u64::from_le_bytes(fields[16..24].try_into().ok()?);
        if size != 32 || address == 0 {
            state.diagnostics.invalid_arguments += 1;
            return None;
        }
        let mut key = [0; 32];
        if read(process, address, &mut key).is_err() {
            state.diagnostics.unreadable_keys += 1;
            key.fill(0);
            return None;
        }
        let hex = key.iter().map(|b| format!("{b:02x}")).collect();
        key.fill(0);
        Some(hex)
    }

    fn continue_event(&mut self, disposition: i32) -> Result<()> {
        if let Some(event) = &self.pending {
            // SAFETY: 这两个 ID 来自本线程收到、还没继续的调试事件。
            if unsafe { ContinueDebugEvent(event.dwProcessId, event.dwThreadId, disposition) } == 0
            {
                return Err(win_error("ContinueDebugEvent"));
            }
            self.pending = None;
        }
        Ok(())
    }

    /// 恢复每个线程的调试寄存器，继续挂起的事件并解除附加；已经退出的线程恢复失败不算错误。已经解除附加时什么也不做。
    fn detach(&mut self) -> Result<()> {
        if !self.attached {
            return Ok(());
        }
        // 总是在一个调试事件之下恢复寄存器，此时 Windows 已让所有线程停下。
        // 逐个调用 `SuspendThread` 会与线程的退出发生竞争，CI 上出现过这种情况。
        if self.pending.is_none() {
            self.pause_for_cleanup()?;
        }
        if !self.attached {
            return Ok(());
        }
        let mut error = None;
        for thread in self.threads.values() {
            if let Err(e) = thread.restore() {
                let mut exit = 0;
                // SAFETY: `thread.handle` 在 `thread` 存活期间一直打开，`exit` 是存活的局部变量。
                if unsafe { GetExitCodeThread(thread.handle.0, &raw mut exit) } == 0
                    || exit == STILL_ACTIVE as u32
                {
                    error.get_or_insert(e);
                }
            }
        }
        if let Err(e) = self.continue_event(self.pending_disposition) {
            error.get_or_insert(e);
        }
        // SAFETY: 只传入本线程所附加的进程的 ID。
        if unsafe { DebugActiveProcessStop(self.pid) } == 0 {
            error.get_or_insert_with(|| win_error("DebugActiveProcessStop"));
        } else {
            self.attached = false;
            self.threads.clear();
        }
        match error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// 用 `DebugBreakProcess` 让微信停在一个调试事件上，供 `detach` 在所有线程都停下时恢复寄存器。
    /// 处理途中收到的其他事件，直到 `system_breakpoint` 处的断点（返回时留在 `pending` 中）或进程退出；最多等 5 秒。
    fn pause_for_cleanup(&mut self) -> Result<()> {
        let process = self
            .process
            .as_ref()
            .context("debugger has no process handle")?
            .0;
        // SAFETY: `process` 是正在调试的进程的句柄，处于打开状态。
        if unsafe { DebugBreakProcess(process) } == 0 {
            return Err(win_error("DebugBreakProcess"));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let mut event = DEBUG_EVENT::default();
            // SAFETY: `event` 是存活、可写的 `DEBUG_EVENT`，本线程就是附加的线程。
            if unsafe { WaitForDebugEvent(&raw mut event, 50) } == 0 {
                // SAFETY: 读取本线程最后的错误码，没有参数。
                if unsafe { GetLastError() } == ERROR_SEM_TIMEOUT {
                    continue;
                }
                return Err(win_error("WaitForDebugEvent during cleanup"));
            }
            self.pending = Some(event);
            self.pending_disposition = DBG_CONTINUE;
            // SAFETY: 每个分支只读取 `event.u` 中由 `dwDebugEventCode` 指明的那个联合体字段；
            // 关闭的文件句柄是调试事件交出来的那一个，只关闭一次。
            unsafe {
                match event.dwDebugEventCode {
                    EXIT_THREAD_DEBUG_EVENT => {
                        self.threads.remove(&event.dwThreadId);
                    }
                    LOAD_DLL_DEBUG_EVENT => {
                        let file = event.u.LoadDll.hFile;
                        if !file.is_null() {
                            CloseHandle(file);
                        }
                    }
                    EXCEPTION_DEBUG_EVENT => {
                        let record = event.u.Exception.ExceptionRecord;
                        if record.ExceptionCode == EXCEPTION_BREAKPOINT
                            && record.ExceptionAddress as usize == self.system_breakpoint
                        {
                            return Ok(());
                        }
                        self.pending_disposition = DBG_EXCEPTION_NOT_HANDLED;
                        if record.ExceptionCode == EXCEPTION_SINGLE_STEP {
                            if let Some(thread) = self.threads.get(&event.dwThreadId) {
                                let mut ctx = context(thread.handle.0)?;
                                if ctx.Rip == self.target as u64
                                    && ctx.Dr6 & (1 << thread.slot) != 0
                                {
                                    ctx.Dr6 &= !(1 << thread.slot);
                                    ctx.EFlags |= 1 << 16;
                                    ctx.ContextFlags =
                                        CONTEXT_CONTROL_AMD64 | CONTEXT_DEBUG_REGISTERS_AMD64;
                                    set_context(thread.handle.0, &ctx)?;
                                    self.pending_disposition = DBG_CONTINUE;
                                }
                            }
                        }
                    }
                    EXIT_PROCESS_DEBUG_EVENT => {
                        self.threads.clear();
                        self.continue_event(DBG_CONTINUE)?;
                        self.attached = false;
                        return Ok(());
                    }
                    _ => {}
                }
            }
            self.continue_event(self.pending_disposition)?;
        }
        bail!("timed out pausing the debugger for cleanup")
    }
}

impl Drop for Debugger {
    fn drop(&mut self) {
        let _ = self.detach();
    }
}

#[cfg(test)]
#[expect(clippy::cast_possible_truncation, reason = "测试中转换的值都放得下")]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, Command, Stdio};

    #[repr(C)]
    struct KeyArgument {
        reserved: u64,
        key: *const u8,
        size: u64,
    }

    // `KeyArgument` 的布局与 `read_key` 读取的位置一致：+8 是密钥的地址，+16 是字节数（#505）
    const _: () = {
        assert!(size_of::<KeyArgument>() == 24);
        assert!(std::mem::offset_of!(KeyArgument, key) == 8);
        assert!(std::mem::offset_of!(KeyArgument, size) == 16);
    };

    /// 目标进程中代替微信接收密钥的函数，断点设在它的入口。按 x64 的调用约定，第二个参数 `key` 在 RDX 中，
    /// 与微信的函数相同。
    #[inline(never)]
    extern "C" fn key_target(first: usize, key: &KeyArgument) -> usize {
        std::hint::black_box(first + key.size as usize)
    }

    /// 目标进程的向量化异常处理函数：继续执行测试抛出的 `0xe0123456` 并打印 `FORWARDED`，其他异常交给下一个处理函数。
    /// 测试据此确认调试器把与密钥无关的异常交还给了目标进程。
    unsafe extern "system" fn unrelated_exception(info: *mut EXCEPTION_POINTERS) -> i32 {
        // SAFETY: Windows 调用向量化异常处理函数时，传入的是正在处理的异常的指针
        let code = unsafe { (*(*info).ExceptionRecord).ExceptionCode };
        if code.cast_unsigned() == 0xe012_3456 {
            println!("FORWARDED");
            EXCEPTION_CONTINUE_EXECUTION
        } else {
            EXCEPTION_CONTINUE_SEARCH
        }
    }

    // 真实、独立的 x64 目标进程，只使用生成的密钥。由下面的测试通过 Rust 的测试框架启动，平常运行测试时不会执行它。
    #[test]
    #[ignore = "synthetic debugger target, launched by capture tests"]
    fn target_process() {
        // SAFETY: 没有参数。
        println!("TARGET {} {}", key_target as *const () as usize, unsafe {
            GetCurrentThreadId()
        });
        std::io::stdout().flush().unwrap();
        for command in std::io::stdin().lock().lines() {
            let command = command.unwrap();
            if command == "exit" {
                return;
            }
            if command == "call" || command == "new-thread" {
                let call = || {
                    // SAFETY: `unrelated_exception` 在整个块中都是有效的处理函数；抛出的正是它会继续执行的那个异常，
                    // 并且块结束前就移除了它。
                    unsafe {
                        let handler = AddVectoredExceptionHandler(1, Some(unrelated_exception));
                        assert!(!handler.is_null());
                        RaiseException(0xe0123456, 0, 0, std::ptr::null());
                        assert_ne!(RemoveVectoredExceptionHandler(handler), 0);
                    }
                    let key = [0x37; 32];
                    // 长度不对的参数和读不了的指针必须被跳过，既不改变目标函数的返回值，也不漏掉之后的密钥调用。
                    for argument in [
                        KeyArgument {
                            reserved: 0,
                            key: key.as_ptr(),
                            size: 16,
                        },
                        KeyArgument {
                            reserved: 0,
                            key: std::ptr::null(),
                            size: 32,
                        },
                        KeyArgument {
                            reserved: 0,
                            key: key.as_ptr(),
                            size: 32,
                        },
                    ] {
                        assert_eq!(
                            key_target(std::hint::black_box(9), std::hint::black_box(&argument)),
                            9 + argument.size as usize
                        );
                    }
                };
                if command == "new-thread" {
                    std::thread::spawn(call).join().unwrap();
                } else {
                    call();
                }
                println!("CALLED");
                std::io::stdout().flush().unwrap();
            }
        }
    }

    struct Target {
        child: Child,
        stdout: BufReader<std::process::ChildStdout>,
        address: usize,
        tid: u32,
    }

    impl Target {
        fn spawn() -> Self {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "windows_db_key::platform::tests::target_process",
                    "--ignored",
                    "--nocapture",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            let mut line = String::new();
            loop {
                assert_ne!(stdout.read_line(&mut line).unwrap(), 0);
                if line.starts_with("TARGET ") {
                    break;
                }
                line.clear();
            }
            let mut fields = line.split_whitespace().skip(1);
            let address = fields.next().unwrap().parse().unwrap();
            let tid = fields.next().unwrap().parse().unwrap();
            Self {
                child,
                stdout,
                address,
                tid,
            }
        }

        fn command(&mut self, command: &str) {
            writeln!(self.child.stdin.as_mut().unwrap(), "{command}").unwrap();
        }

        fn assert_detached_with(&mut self, addresses: [u64; 4], control: u64) {
            let process =
                // SAFETY: 参数都是普通的整数；返回的句柄在下面检查，由 `Handle` 持有。
                Handle(unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, 0, self.child.id()) });
            assert!(!process.0.is_null());
            let mut present = 1;
            assert_ne!(
                // SAFETY: 进程句柄已打开，`present` 是存活的局部变量。
                unsafe { CheckRemoteDebuggerPresent(process.0, &raw mut present) },
                0
            );
            assert_eq!(present, 0);
            // SAFETY: 参数都是普通的整数；返回的句柄由 `Handle` 持有。
            let thread = Handle(unsafe {
                OpenThread(THREAD_GET_CONTEXT | THREAD_SUSPEND_RESUME, 0, self.tid)
            });
            // SAFETY: 线程句柄打开时带了 `THREAD_SUSPEND_RESUME` 权限。
            assert_ne!(unsafe { SuspendThread(thread.0) }, u32::MAX);
            let ctx = context(thread.0).unwrap();
            // SAFETY: 线程句柄打开时带了 `THREAD_SUSPEND_RESUME` 权限，这里恢复的是上面的挂起。
            assert_ne!(unsafe { ResumeThread(thread.0) }, u32::MAX);
            assert_eq!(
                ctx.Dr7 & 0xff,
                control,
                "only the original breakpoint slots remain enabled"
            );
            assert_eq!([ctx.Dr0, ctx.Dr1, ctx.Dr2, ctx.Dr3], addresses);
        }

        fn assert_detached(&mut self) {
            self.assert_detached_with([0; 4], 0);
        }

        fn set_debug_registers(&mut self, addresses: [u64; 4], control: u64) {
            // SAFETY: 参数都是普通的整数；返回的句柄由 `Handle` 持有。
            let thread = Handle(unsafe {
                OpenThread(
                    THREAD_GET_CONTEXT | THREAD_SET_CONTEXT | THREAD_SUSPEND_RESUME,
                    0,
                    self.tid,
                )
            });
            // SAFETY: 线程句柄打开时带了 `THREAD_SUSPEND_RESUME` 权限。
            assert_ne!(unsafe { SuspendThread(thread.0) }, u32::MAX);
            let mut ctx = context(thread.0).unwrap();
            [ctx.Dr0, ctx.Dr1, ctx.Dr2, ctx.Dr3] = addresses;
            ctx.Dr7 = control;
            ctx.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64;
            set_context(thread.0, &ctx).unwrap();
            // SAFETY: 线程句柄打开时带了 `THREAD_SUSPEND_RESUME` 权限，这里恢复的是上面的挂起。
            assert_ne!(unsafe { ResumeThread(thread.0) }, u32::MAX);
        }
    }

    impl Drop for Target {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[test]
    fn captures_key_preserves_arguments_and_detaches_before_publication() {
        for command in ["call", "new-thread"] {
            let mut target = Target::spawn();
            let mut hook =
                Hook::start_at(target.child.id(), target.address, "synthetic".into()).unwrap();
            target.command(command);
            let started = Instant::now();
            let key = loop {
                if let Some(key) = hook.poll().unwrap() {
                    break key;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "key capture timed out"
                );
                std::thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(key, "37".repeat(32));
            hook.cleanup().unwrap();
            assert_eq!(hook.poll().unwrap(), None);
            target.assert_detached();
            let mut line = String::new();
            target.stdout.read_line(&mut line).unwrap();
            assert_eq!(line.trim(), "FORWARDED");
            line.clear();
            target.stdout.read_line(&mut line).unwrap();
            assert_eq!(line.trim(), "CALLED");
        }
    }

    #[test]
    fn cancellation_drop_and_repeated_attach_restore_registers() {
        let mut target = Target::spawn();
        for _ in 0..3 {
            let mut hook =
                Hook::start_at(target.child.id(), target.address, "synthetic".into()).unwrap();
            assert_eq!(hook.poll().unwrap(), None);
            hook.cleanup().unwrap();
            hook.cleanup().unwrap();
            target.assert_detached();
            let hook =
                Hook::start_at(target.child.id(), target.address, "synthetic".into()).unwrap();
            drop(hook);
            target.assert_detached();
        }
    }

    #[test]
    fn user_interrupt_detaches_before_reporting_error() {
        let mut target = Target::spawn();
        let mut hook =
            Hook::start_at(target.child.id(), target.address, "synthetic".into()).unwrap();
        assert!(cancel(&[hook._interrupt.0.clone()]));
        assert!(hook
            .cleanup()
            .unwrap_err()
            .to_string()
            .contains("interrupted"));
        target.assert_detached();
    }

    #[test]
    fn console_close_returns_after_detaching() {
        let mut target = Target::spawn();
        let mut hook =
            Hook::start_at(target.child.id(), target.address, "synthetic".into()).unwrap();
        let hooks = [hook._interrupt.0.clone()];
        assert!(interrupt_and_wait(&hooks));
        // 在等待（`cleanup`）或丢弃挂钩之前，调试器线程自己就已经恢复了寄存器、解除了附加。
        target.assert_detached();
        assert!(!interrupt_and_wait(&hooks), "nothing is left to stop");
        assert!(hook
            .cleanup()
            .unwrap_err()
            .to_string()
            .contains("interrupted"));
    }

    #[test]
    fn preserves_other_slots_and_refuses_exhausted_registers() {
        let mut target = Target::spawn();
        let original = [0, 0, 0x3000, 0];
        target.set_debug_registers(original, 0x10);
        let mut hook =
            Hook::start_at(target.child.id(), target.address, "synthetic".into()).unwrap();
        hook.cleanup().unwrap();
        target.assert_detached_with(original, 0x10);
        let occupied = [0x1000, 0x2000, 0x3000, 0x4000];
        target.set_debug_registers(occupied, 0x55);
        let error = Hook::start_at(target.child.id(), target.address, "synthetic".into())
            .err()
            .unwrap();
        assert!(error
            .to_string()
            .contains("no free hardware breakpoint slot"));
        target.assert_detached_with(occupied, 0x55);
    }

    #[test]
    fn process_exit_and_invalid_pid_are_reported() {
        assert!(Hook::start_at(0, 1, "synthetic".into()).is_err());
        assert!(Hook::start_at(std::process::id(), 1, "synthetic".into()).is_err());
        let mut target = Target::spawn();
        let mut hook =
            Hook::start_at(target.child.id(), target.address, "synthetic".into()).unwrap();
        target.command("exit");
        target.child.wait().unwrap();
        assert!(hook.cleanup().unwrap_err().to_string().contains("exited"));
    }
}
