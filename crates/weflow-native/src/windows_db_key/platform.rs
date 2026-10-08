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
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

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
    if unsafe {
        ReadProcessMemory(
            process,
            address as *const c_void,
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            &mut count,
        )
    } == 0
        || count != bytes.len()
    {
        return Err(win_error("ReadProcessMemory"));
    }
    Ok(())
}

fn module(pid: u32, name: &str) -> Result<MODULEENTRY32W> {
    let snapshot =
        Handle(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) });
    if snapshot.0 == INVALID_HANDLE_VALUE {
        return Err(win_error("enumerate WeChat modules"));
    }
    let mut entry = MODULEENTRY32W {
        dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    let mut found = None;
    let mut available = unsafe { Module32FirstW(snapshot.0, &mut entry) } != 0;
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
        available = unsafe { Module32NextW(snapshot.0, &mut entry) } != 0;
    }
    found.with_context(|| format!("{name} is not loaded yet"))
}

fn remote_ntdll_symbol(pid: u32, name: &[u8]) -> Result<usize> {
    let local_name: Vec<_> = "ntdll.dll".encode_utf16().chain(Some(0)).collect();
    let local = unsafe { GetModuleHandleW(local_name.as_ptr()) };
    let symbol =
        unsafe { GetProcAddress(local, name.as_ptr()) }.context("ntdll debug symbol missing")?;
    let offset = (symbol as *const () as usize)
        .checked_sub(local as usize)
        .context("invalid ntdll symbol")?;
    Ok(module(pid, "ntdll.dll")?.modBaseAddr as usize + offset)
}

/// Read-only inspection of the candidate and client version, without debugger attachment.
pub struct KeyLocation {
    pub address: usize,
    pub module_offset: usize,
    pub version: String,
}

fn locate(pid: u32) -> Result<KeyLocation> {
    let module = module(pid, "Weixin.dll")?;
    let version = file_version(&module.szExePath)?;
    let signature = locator::signature(version)?;
    let process =
        Handle(unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) });
    if process.0.is_null() {
        return Err(win_error("open WeChat process"));
    }
    let mut machine = 0;
    let mut native_machine = 0;
    if unsafe { IsWow64Process2(process.0, &mut machine, &mut native_machine) } == 0 {
        return Err(win_error("IsWow64Process2"));
    }
    // Native AMD64, or AMD64 emulation on an ARM64 host. Never use x64 CONTEXT on ARM64/x86.
    if !(machine == 0x8664 || machine == 0 && native_machine == 0x8664) {
        bail!("unsupported WeChat architecture (requires x64)");
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
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), &mut ignored) };
    if size == 0 {
        return Err(win_error("GetFileVersionInfoSizeW"));
    }
    // Vec<u32> keeps the returned VS_FIXEDFILEINFO aligned.
    let mut buffer = vec![0u32; (size as usize).div_ceil(4)];
    if unsafe { GetFileVersionInfoW(path.as_ptr(), 0, size, buffer.as_mut_ptr().cast()) } == 0 {
        return Err(win_error("GetFileVersionInfoW"));
    }
    let mut value = std::ptr::null_mut();
    let mut length = 0;
    if unsafe {
        VerQueryValueW(
            buffer.as_ptr().cast(),
            [b'\\' as u16, 0].as_ptr(),
            &mut value,
            &mut length,
        )
    } == 0
        || length < std::mem::size_of::<VS_FIXEDFILEINFO>() as u32
        || value.is_null()
    {
        bail!("invalid WeChat version resource");
    }
    let info = unsafe { std::ptr::read_unaligned(value.cast::<VS_FIXEDFILEINFO>()) };
    if info.dwSignature != 0xfeef04bd {
        bail!("invalid WeChat version signature");
    }
    Ok([
        (info.dwProductVersionMS >> 16) as u16,
        info.dwProductVersionMS as u16,
        (info.dwProductVersionLS >> 16) as u16,
        info.dwProductVersionLS as u16,
    ])
}

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
        // Preserve the preceding RCX instruction and matches split across a chunk.
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

/// Counts only; never stores or exposes register values, pointers or key material.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct CaptureDiagnostics {
    pub breakpoint_hits: u64,
    pub invalid_arguments: u64,
    pub unreadable_arguments: u64,
    pub unreadable_keys: u64,
}

fn interrupts() -> &'static Mutex<Vec<Weak<AtomicBool>>> {
    static ACTIVE: OnceLock<Mutex<Vec<Weak<AtomicBool>>>> = OnceLock::new();
    ACTIVE.get_or_init(Default::default)
}

unsafe extern "system" fn console_interrupt(event: u32) -> i32 {
    if event != CTRL_C_EVENT && event != CTRL_BREAK_EVENT {
        return 0;
    }
    let active = interrupts().lock().unwrap_or_else(|e| e.into_inner());
    let mut handled = false;
    for cancel in active.iter().filter_map(Weak::upgrade) {
        cancel.store(true, Ordering::Release);
        handled = true;
    }
    i32::from(handled)
}

struct InterruptGuard(Arc<AtomicBool>);
impl InterruptGuard {
    fn new() -> Result<Self> {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut active = interrupts().lock().unwrap_or_else(|e| e.into_inner());
        if active.is_empty() && unsafe { SetConsoleCtrlHandler(Some(console_interrupt), 1) } == 0 {
            return Err(win_error("SetConsoleCtrlHandler"));
        }
        active.push(Arc::downgrade(&cancel));
        Ok(Self(cancel))
    }
}
impl Drop for InterruptGuard {
    fn drop(&mut self) {
        let mut active = interrupts().lock().unwrap_or_else(|e| e.into_inner());
        active.retain(|entry| !Weak::ptr_eq(entry, &Arc::downgrade(&self.0)));
        if active.is_empty() {
            unsafe {
                SetConsoleCtrlHandler(Some(console_interrupt), 0);
            }
        }
    }
}

/// The attaching thread owns the Windows debug loop for its entire lifetime. Drop joins
/// it, restores only the debug-register slot it owns, and detaches before returning.
pub struct Hook {
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<Result<()>>>,
    output: Arc<Mutex<Output>>,
    _interrupt: InterruptGuard,
    pub version: String,
}

impl Hook {
    pub fn start(pid: u32) -> Result<Self> {
        let location = Self::inspect(pid)?;
        Self::start_at(pid, location.address, location.version)
    }

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
        let interrupt = InterruptGuard::new()?;
        let cancel = interrupt.0.clone();
        let worker_output = output.clone();
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

    pub fn poll(&mut self) -> Result<Option<String>> {
        let mut output = self.output.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &output.error {
            bail!("{error}");
        }
        Ok(output.key.take())
    }

    pub fn diagnostics(&self) -> CaptureDiagnostics {
        self.output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .diagnostics
            .clone()
    }

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

struct WatchedThread {
    handle: Handle,
    slot: usize,
    old_address: u64,
    old_control: u64,
    old_status: u64,
}

// windows-sys declares CONTEXT with C alignment (8), but Windows requires 16-byte
// alignment for the x64 context buffer passed to GetThreadContext/SetThreadContext.
#[repr(C, align(16))]
struct AlignedContext(CONTEXT);
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
    if unsafe { GetThreadContext(thread, &mut ctx.0) } == 0 {
        return Err(win_error("GetThreadContext"));
    }
    Ok(ctx)
}

fn set_context(thread: HANDLE, ctx: &CONTEXT) -> Result<()> {
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
        // Debug-event thread handles belong to Windows. Open our own handle, which can
        // safely outlive ContinueDebugEvent and is closed once by our RAII guard.
        let tid = unsafe { GetThreadId(handle) };
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
        ctx.Dr7 = (ctx.Dr7 & !(0xf << (16 + slot * 4))) | (1 << (slot * 2));
        ctx.Dr6 &= !(1 << slot);
        ctx.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64;
        set_context(watched.handle.0, &ctx)?;
        Ok(watched)
    }

    fn restore(&self) -> Result<()> {
        let mut ctx = context(self.handle.0)?;
        let mask = (3 << (self.slot * 2)) | (0xf << (16 + self.slot * 4));
        ctx.Dr7 = (ctx.Dr7 & !mask) | (self.old_control & mask);
        ctx.Dr6 = (ctx.Dr6 & !(1 << self.slot)) | (self.old_status & (1 << self.slot));
        *register(&mut ctx, self.slot) = self.old_address;
        ctx.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64;
        set_context(self.handle.0, &ctx)
    }
}

struct Debugger {
    pid: u32,
    target: usize,
    process: Option<Handle>,
    threads: BTreeMap<u32, WatchedThread>,
    attached: bool,
    pending: Option<DEBUG_EVENT>,
    pending_disposition: i32,
    system_breakpoint: usize,
    system_breakin: usize,
}

impl Debugger {
    fn attach(pid: u32, target: usize) -> Result<Self> {
        let system_breakpoint = remote_ntdll_symbol(pid, b"DbgBreakPoint\0")?;
        let system_breakin = remote_ntdll_symbol(pid, b"DbgUiRemoteBreakin\0")?;
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
        if unsafe { DebugSetProcessKillOnExit(0) } == 0 {
            return Err(win_error("DebugSetProcessKillOnExit"));
        }
        Ok(debugger)
    }

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
            if unsafe { WaitForDebugEvent(&mut event, 50) } == 0 {
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
                        // Windows' transient debugger break-in thread cannot execute the
                        // WeChat key function, and may already be terminating on cleanup.
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
                                if ctx.Rip as usize == self.target
                                    && ctx.Dr6 & (1 << thread.slot) != 0
                                {
                                    self.pending_disposition = DBG_CONTINUE;
                                    captured = self.read_key(ctx.Rdx as usize, output);
                                    ctx.Dr6 &= !(1 << thread.slot);
                                    // RF skips this instruction's execution breakpoint once. All
                                    // integer/SIMD registers, stack and flags remain otherwise intact.
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
        // Publish a key only after successful cleanup. A caller can never report success
        // while WeChat is still attached or a hardware breakpoint remains installed.
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

    fn read_key(&self, structure: usize, output: &Mutex<Output>) -> Option<String> {
        let mut state = output.lock().unwrap_or_else(|e| e.into_inner());
        state.diagnostics.breakpoint_hits += 1;
        let process = self.process.as_ref()?.0;
        let mut fields = [0; 24];
        if read(process, structure, &mut fields).is_err() {
            state.diagnostics.unreadable_arguments += 1;
            return None;
        }
        let address = u64::from_le_bytes(fields[8..16].try_into().ok()?) as usize;
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
            if unsafe { ContinueDebugEvent(event.dwProcessId, event.dwThreadId, disposition) } == 0
            {
                return Err(win_error("ContinueDebugEvent"));
            }
            self.pending = None;
        }
        Ok(())
    }

    fn detach(&mut self) -> Result<()> {
        if !self.attached {
            return Ok(());
        }
        // Always restore under a debug event, with every thread stopped by Windows.
        // Individual SuspendThread calls race with thread termination, as observed in CI.
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
                if unsafe { GetExitCodeThread(thread.handle.0, &mut exit) } == 0
                    || exit == STILL_ACTIVE as u32
                {
                    error.get_or_insert(e);
                }
            }
        }
        if let Err(e) = self.continue_event(self.pending_disposition) {
            error.get_or_insert(e);
        }
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

    fn pause_for_cleanup(&mut self) -> Result<()> {
        let process = self
            .process
            .as_ref()
            .context("debugger has no process handle")?
            .0;
        if unsafe { DebugBreakProcess(process) } == 0 {
            return Err(win_error("DebugBreakProcess"));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let mut event = DEBUG_EVENT::default();
            if unsafe { WaitForDebugEvent(&mut event, 50) } == 0 {
                if unsafe { GetLastError() } == ERROR_SEM_TIMEOUT {
                    continue;
                }
                return Err(win_error("WaitForDebugEvent during cleanup"));
            }
            self.pending = Some(event);
            self.pending_disposition = DBG_CONTINUE;
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
                                if ctx.Rip as usize == self.target
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

    #[inline(never)]
    extern "C" fn key_target(first: usize, key: &KeyArgument) -> usize {
        std::hint::black_box(first + key.size as usize)
    }

    unsafe extern "system" fn unrelated_exception(info: *mut EXCEPTION_POINTERS) -> i32 {
        if (*(*info).ExceptionRecord).ExceptionCode as u32 == 0xe0123456 {
            println!("FORWARDED");
            EXCEPTION_CONTINUE_EXECUTION
        } else {
            EXCEPTION_CONTINUE_SEARCH
        }
    }

    // A real independent x64 target, with generated keys only. Invoked by the tests
    // below via the Rust test harness, never by the normal suite itself.
    #[test]
    #[ignore = "synthetic debugger target, launched by capture tests"]
    fn target_process() {
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
                    unsafe {
                        let handler = AddVectoredExceptionHandler(1, Some(unrelated_exception));
                        assert!(!handler.is_null());
                        RaiseException(0xe0123456, 0, 0, std::ptr::null());
                        assert_ne!(RemoveVectoredExceptionHandler(handler), 0);
                    }
                    let key = [0x37; 32];
                    // Wrong sizes and unreadable pointers must be skipped without changing
                    // the target function's return value or losing subsequent key calls.
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
                Handle(unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, 0, self.child.id()) });
            assert!(!process.0.is_null());
            let mut present = 1;
            assert_ne!(
                unsafe { CheckRemoteDebuggerPresent(process.0, &mut present) },
                0
            );
            assert_eq!(present, 0);
            let thread = Handle(unsafe {
                OpenThread(THREAD_GET_CONTEXT | THREAD_SUSPEND_RESUME, 0, self.tid)
            });
            assert_ne!(unsafe { SuspendThread(thread.0) }, u32::MAX);
            let ctx = context(thread.0).unwrap();
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
            let thread = Handle(unsafe {
                OpenThread(
                    THREAD_GET_CONTEXT | THREAD_SET_CONTEXT | THREAD_SUSPEND_RESUME,
                    0,
                    self.tid,
                )
            });
            assert_ne!(unsafe { SuspendThread(thread.0) }, u32::MAX);
            let mut ctx = context(thread.0).unwrap();
            [ctx.Dr0, ctx.Dr1, ctx.Dr2, ctx.Dr3] = addresses;
            ctx.Dr7 = control;
            ctx.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64;
            set_context(thread.0, &ctx).unwrap();
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
        hook._interrupt.0.store(true, Ordering::Release);
        assert!(hook
            .cleanup()
            .unwrap_err()
            .to_string()
            .contains("interrupted"));
        target.assert_detached();
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
