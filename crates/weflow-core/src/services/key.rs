//! 密钥：从微信进程中读取数据库密钥（要等微信重新启动），以及图片密钥。

use std::path::PathBuf;

use serde_json::{json, Value};

use super::ServiceHub;
use crate::error::{AppError, AppResult};
use crate::locale::tr;

impl ServiceHub {
    /// 从微信取得数据库密钥。
    ///
    /// 微信只在打开数据库时产生密钥，所以在 Windows 和 Linux 上，命令请用户退出并重新打开微信，挂钩新的进程，
    /// 等待用户点击「进入微信」（总共默认 180 秒）。在 macOS 上挂钩正在运行的微信。各平台都在钩子装好之后才提示登录。
    ///
    /// # Errors
    ///
    /// 平台不支持、没有权限挂钩微信、超时、找不到取密钥的位置或挂钩失败时返回 `native_error`；
    /// 被中断时返回 `user_interrupt`；macOS 上找不到微信进程时返回 `runtime_error`。
    pub fn key_db(
        &self,
        pid_override: Option<u32>,
        timeout_secs: u64,
    ) -> AppResult<crate::contract::DbKey> {
        if cfg!(windows) && !cfg!(target_arch = "x86_64") {
            return Err(AppError::native(
                "Windows database-key extraction requires x64",
            ));
        }
        if cfg!(target_os = "linux") && !cfg!(target_arch = "x86_64") {
            return Err(AppError::native(
                "Linux database-key extraction requires x64",
            ));
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        linux_ptrace_check()?;
        let wxkey = weflow_native::wxkey::WxKey::new()
            .map_err(|err| AppError::native(format!("{err:#}")))?;
        if !wxkey.is_available() {
            return Err(AppError::native(
                "database-key extraction is not available on this platform",
            ));
        }
        self.key_db_hooked(&wxkey, pid_override, timeout_secs)
    }

    /// 挂钩微信并等待数据库密钥；Windows、Linux 和 macOS 共用。
    fn key_db_hooked(
        &self,
        wxkey: &weflow_native::wxkey::WxKey,
        pid_override: Option<u32>,
        timeout_secs: u64,
    ) -> AppResult<crate::contract::DbKey> {
        use weflow_native::wxkey::DbKeyError;
        let started = std::time::Instant::now();
        let remaining = || timeout_secs.saturating_sub(started.elapsed().as_secs());
        let timeout_error = || {
            AppError::native(format!(
                "timed out after {timeout_secs} s without getting the key\nQuit WeChat completely, reopen it and click \"Enter WeChat\" in the login window, then run the command again."
            ))
        };
        let interrupted = || AppError::new("user_interrupt", "interrupted by user", 130);
        // 正在进行的等待：显示在一行上，结束时在行尾写明结果
        let mut waiting = WaitLine::default();
        let pid = match pid_override {
            Some(pid) => pid,
            None if cfg!(any(windows, target_os = "linux")) => {
                let found = wait_for_fresh_wechat(
                    &mut wechat_pids,
                    &mut || std::thread::sleep(std::time::Duration::from_secs(1)),
                    // 中断与超时一样结束等待
                    &mut || {
                        if key_db_interrupted() {
                            u64::MAX
                        } else {
                            started.elapsed().as_secs()
                        }
                    },
                    timeout_secs,
                    &mut |event| waiting.report(event),
                );
                if key_db_interrupted() {
                    waiting.end(WaitEnd::Interrupted);
                    return Err(interrupted());
                }
                let Some(pid) = found else {
                    waiting.end(WaitEnd::TimedOut);
                    return Err(timeout_error());
                };
                waiting.end(WaitEnd::Done);
                pid
            }
            None => find_wechat_pid().ok_or_else(|| {
                AppError::runtime(
                    "WeChat process not found\nStart WeChat first, or pass --pid; `weflow key processes` lists WeChat's processes.",
                )
            })?,
        };
        let login_phase = || {
            phase(
                json!({ "type": "key_phase", "phase": "login", "pid": pid }),
                &tr(
                    "WeChat found, pid {pid}. Click \"Enter WeChat\" in the login window.",
                    "检测到微信，pid {pid}。请在登录窗口点击「进入微信」。",
                )
                .replace("{pid}", &pid.to_string()),
            );
        };
        // macOS 的挂钩最多重试一次：重试多了可能触发系统的保护（#806）
        let mut tried_again = false;
        let failed = |waiting: &mut WaitLine, how: WaitEnd, err: AppError| {
            waiting.end(how);
            Err(err)
        };
        let key = loop {
            if key_db_interrupted() {
                return failed(&mut waiting, WaitEnd::Interrupted, interrupted());
            }
            if remaining() == 0 {
                return failed(&mut waiting, WaitEnd::TimedOut, timeout_error());
            }
            // wx_key 自己的进度消息对用户没有用，只显示它的错误；登录提示等钩子装好后才显示（#872）
            let mut on_status = |msg: &str, level: i32| {
                if msg == "Hook installed; log in to WeChat now" {
                    login_phase();
                }
                if level == 2 {
                    crate::output::event(
                        json!({ "type": "key_status", "level": level, "message": crate::locale::localize(msg.to_string()) }),
                    );
                }
            };
            let mut on_tick = |left: u64| waiting.tick(Wait::Key, left);
            match wxkey.get_db_key_with_tick(
                pid,
                std::time::Duration::from_secs(remaining()),
                &mut on_status,
                &mut on_tick,
            ) {
                Ok(key) => break key,
                Err(DbKeyError::AccessDenied(detail)) => {
                    let hint = if cfg!(target_os = "linux") {
                        format!(
                            "permission denied: cannot attach to WeChat, pid {pid}\nRun weflow as root, or allow ptrace through /proc/sys/kernel/yama/ptrace_scope.\nReason: {detail}"
                        )
                    } else if cfg!(target_os = "macos") {
                        format!(
                            "permission denied: cannot attach to WeChat, pid {pid}\nRun weflow with sudo, and check that macOS lets it debug other processes: System Integrity Protection and code signing decide it.\nReason: {detail}"
                        )
                    } else {
                        format!(
                            "permission denied: cannot open the WeChat process, pid {pid}\nRun the terminal as administrator, close security software that blocks it, and make sure WeChat itself is not running as administrator.\nReason: {detail}"
                        )
                    };
                    return failed(&mut waiting, WaitEnd::Failed, AppError::native(hint));
                }
                Err(DbKeyError::Timeout) => {
                    return failed(&mut waiting, WaitEnd::TimedOut, timeout_error());
                }
                Err(DbKeyError::Interrupted) => {
                    return failed(&mut waiting, WaitEnd::Interrupted, interrupted());
                }
                // 进程刚启动时可能还不能挂钩：再试一次
                Err(DbKeyError::NotReady(_)) if remaining() > 0 => {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
                Err(DbKeyError::Unsupported(detail)) => {
                    let err = AppError::native(unsupported_wechat(&detail));
                    return failed(&mut waiting, WaitEnd::Failed, err);
                }
                Err(DbKeyError::Other(detail)) if cfg!(target_os = "macos") => {
                    match macos_hook_failed(&detail, tried_again || remaining() == 0) {
                        None => {
                            tried_again = true;
                            std::thread::sleep(std::time::Duration::from_secs(1));
                        }
                        Some(message) => {
                            return failed(
                                &mut waiting,
                                WaitEnd::Failed,
                                AppError::native(message),
                            );
                        }
                    }
                }
                Err(other) => {
                    let err = AppError::native(other.to_string());
                    return failed(&mut waiting, WaitEnd::Failed, err);
                }
            }
        };
        waiting.end(WaitEnd::Done);
        Ok(crate::contract::DbKey {
            decrypt_key: Some(key),
            method: "wx_key".into(),
            pid: Some(pid),
            saved: Vec::new(),
        })
    }

    /// 从 `kvcomm` 缓存取得图片密钥，并用账号的样本图片校验；与已保存的图片密钥无关。
    ///
    /// # Errors
    ///
    /// 没有给 `user_dir` 而档案中的账号目录找不到时返回配置错误；其余同
    /// [`acquire_image_keys`](crate::image_keys::acquire_image_keys)。
    pub fn key_image(
        &self,
        user_dir: Option<&str>,
        kvcomm_dirs: &[PathBuf],
        scan_budget: usize,
    ) -> AppResult<crate::contract::ImageKeys> {
        let account = match user_dir {
            Some(dir) => crate::config::expand_home(dir),
            None => self.account_dir_only()?,
        };
        let wxid = self.profile().ok().and_then(|p| p.wxid.as_deref());
        crate::image_keys::acquire_image_keys(kvcomm_dirs, &account, wxid, scan_budget)
    }

    /// 正在运行的微信客户端，主程序在前（`key processes`，仅 macOS）。
    ///
    /// # Errors
    ///
    /// 不是 macOS 时返回 `native_error`。
    pub fn key_processes(&self) -> AppResult<crate::contract::WeChatProcesses> {
        #[cfg(target_os = "macos")]
        {
            let processes = weflow_native::macos_key_helper::wechat_processes()
                .into_iter()
                .map(|process| crate::contract::WeChatProcess {
                    pid: process.pid,
                    name: process.name,
                    path: PathBuf::from(process.path),
                    main: process.main,
                })
                .collect();
            Ok(crate::contract::WeChatProcesses { processes })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(AppError::native(
                "`key processes` is only available on macOS",
            ))
        }
    }

    /// 在微信的内存中找到的图片 AES 密钥，支持 Linux 和 macOS。
    ///
    /// `pid` 默认为正在运行的微信。没有 `ciphertext` 时，用配置的账号最新几张缩略图的第一个块搜索内存，
    /// 缩略图确认后才采用找到的密钥（[`crate::image_keys::check_memory_key`]）；其中一张能用这对密钥解码时，
    /// 还会得到 XOR 密钥。有 `ciphertext` 时，采用第一个能把这一块解密成图片文件头的密钥。
    ///
    /// # Errors
    ///
    /// `ciphertext` 不是 32 个十六进制字符时返回用法错误；没有 `ciphertext` 时，账号没有配置好返回配置错误，
    /// 没有缩略图返回 `image_key_no_template`；平台不支持或系统不让 weflow 读微信的内存时返回 `native_error`，
    /// 找不到微信进程时返回 `runtime_error`。
    pub fn key_image_mem(
        &self,
        pid: Option<u32>,
        ciphertext: Option<&str>,
    ) -> AppResult<crate::contract::ImageMemKey> {
        if !IMAGE_MEM_SUPPORTED {
            return Err(AppError::native(
                "`key image-mem` is only available on Linux x64 and macOS",
            ));
        }
        let pid = match pid {
            Some(pid) => pid,
            None => find_wechat_pid().ok_or_else(|| {
                AppError::runtime(
                    "WeChat process not found\nStart WeChat and log in, or pass --pid.",
                )
            })?,
        };
        use crate::image_keys::MemoryKeyCheck;
        let found = match ciphertext {
            Some(text) => {
                let block = weflow_native::image_key_scan::parse_ciphertext(text.trim())
                    .ok_or_else(|| {
                        AppError::usage(
                            "the ciphertext must be 32 hex characters, one 16-byte block",
                        )
                    })?;
                scan_image_memory(pid, &[block], &mut |_| true)?
                    .map(|key| (key, None, "first_block"))
            }
            None => {
                let samples = crate::image_keys::memory_samples(&self.account_dir_only()?)?;
                let ciphertexts = crate::image_keys::memory_ciphertexts(&samples);
                let mut check = None;
                let key = scan_image_memory(pid, &ciphertexts, &mut |key| {
                    check = crate::image_keys::check_memory_key(&samples, key);
                    check.is_some()
                })?;
                key.zip(check).map(|(key, check)| match check {
                    MemoryKeyCheck::DecodedWithXor(xor) => {
                        (key, Some(xor), "jpeg_decode_and_xor_tail")
                    }
                    MemoryKeyCheck::FirstBlocks(_) => (key, None, "sample_first_blocks"),
                })
            }
        };
        let (image_aes_key, image_xor_key, verification) = match found {
            Some((key, xor, verification)) => (
                Some(weflow_native::image_key_scan::hex(&key)),
                xor,
                Some(verification.to_string()),
            ),
            None => (None, None, None),
        };
        Ok(crate::contract::ImageMemKey {
            image_aes_key,
            image_xor_key,
            verification,
            pid,
            saved: Vec::new(),
        })
    }
}

/// 在 Linux 上，Yama 的 `ptrace_scope` 不允许这个进程附加到微信时，在请用户重启微信之前就结束 `key db`（#854）。
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn linux_ptrace_check() -> AppResult<()> {
    use weflow_native::linux_key_helper::{ptrace_access, PtraceAccess};
    match ptrace_access() {
        PtraceAccess::Allowed => Ok(()),
        PtraceAccess::NeedsCapability { scope } => Err(AppError::native(format!(
            "permission denied: weflow cannot attach to WeChat while /proc/sys/kernel/yama/ptrace_scope is {scope}\nRun weflow as root, or set ptrace_scope to 0."
        ))),
        PtraceAccess::Disabled => Err(AppError::native(
            "permission denied: /proc/sys/kernel/yama/ptrace_scope is 3, which stops every process from attaching to WeChat, root included, until the system restarts",
        )),
    }
}

/// 钩子在这个微信里找不到停下的位置时的错误（`detail` 说明缺了什么）：微信还没完全启动，
/// 或者钩子不认识它的版本（#806）。
fn unsupported_wechat(detail: &str) -> String {
    format!(
        "cannot find where WeChat takes its database key\nMake sure WeChat has fully started and is in front; otherwise this WeChat version is not supported yet. WeChat 4.1.7 and 4.1.8.100 are known to work.\nReason: {detail}"
    )
}

/// macOS 的挂钩失败（`detail`）之后怎么办（#806）。
///
/// `last` 为 false 时返回 `None`，再试一次，因为断点或线程状态可能恰好在不合适的时刻被拒绝；否则返回结束时的错误。
/// 微信已经退出时不再重试。
fn macos_hook_failed(detail: &str, last: bool) -> Option<String> {
    if detail == "process_died_while_waiting" {
        return Some(
            "WeChat quit while the command waited for the key\nReopen WeChat, then run the command again.".into(),
        );
    }
    last.then(|| {
        format!(
            "cannot hook WeChat\nQuit WeChat completely and restart the Mac before trying again, as more tries can set off macOS's protection.\nReason: {detail}"
        )
    })
}

/// 在本平台上 `key image-mem` 能否读微信的内存。
const IMAGE_MEM_SUPPORTED: bool = cfg!(any(
    all(target_os = "linux", target_arch = "x86_64"),
    target_os = "macos"
));

/// 微信进程 `pid` 的可写内存中，第一个能把 `ciphertexts` 之一解密成图片开头、且被 `accept` 接受的密钥；
/// [`IMAGE_MEM_SUPPORTED`] 为 false 的平台上总是 `None`。
///
/// # Errors
///
/// 系统不让 weflow 读微信的内存时返回 `native_error`，并说明怎样取得权限；读取失败时同样返回 `native_error`。
#[cfg_attr(
    not(any(all(target_os = "linux", target_arch = "x86_64"), target_os = "macos")),
    expect(unused_variables, reason = "本平台没有内存扫描器")
)]
fn scan_image_memory(
    pid: u32,
    ciphertexts: &[[u8; 16]],
    accept: &mut dyn FnMut(&[u8; 16]) -> bool,
) -> AppResult<Option<[u8; 16]>> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    let found = weflow_native::linux_key_helper::image_mem(pid, ciphertexts, accept);
    #[cfg(target_os = "macos")]
    let found = weflow_native::macos_key_helper::scan_image_key(pid, ciphertexts, accept);
    #[cfg(not(any(all(target_os = "linux", target_arch = "x86_64"), target_os = "macos")))]
    let found: Result<_, weflow_native::image_key_scan::MemoryScanError> = Ok(None);
    found.map_err(|error| {
        use weflow_native::image_key_scan::MemoryScanError;
        match error {
            MemoryScanError::PermissionDenied(detail) if cfg!(target_os = "macos") => {
                AppError::native(format!(
                    "permission denied: cannot read WeChat's memory, pid {pid}\nRun weflow with sudo.\nReason: {detail}"
                ))
            }
            MemoryScanError::PermissionDenied(detail) => AppError::native(format!(
                "permission denied: cannot read WeChat's memory, pid {pid}\nRun weflow as root, or allow ptrace through /proc/sys/kernel/yama/ptrace_scope.\nReason: {detail}"
            )),
            MemoryScanError::Failed(detail) => AppError::native(format!(
                "cannot read WeChat's memory, pid {pid}: {detail}"
            )),
        }
    })
}

fn find_wechat_pid() -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        weflow_native::macos_key_helper::wechat_processes()
            .first()
            .map(|process| process.pid)
    }
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("pgrep")
            .arg("-x")
            .arg("wechat")
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .lines()
            .next()
            .and_then(|line| line.trim().parse().ok())
    }
    #[cfg(target_os = "windows")]
    {
        wechat_pids().first().copied()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wait {
    Quit,
    Open,
    Key,
}

enum WaitEvent {
    AskQuit(Vec<u32>),
    AskOpen { was_running: bool },
    Tick { phase: Wait, remaining: u64 },
}

/// 等待的是什么，写在这一行的开头。
fn wait_head(phase: Wait) -> &'static str {
    match phase {
        Wait::Quit => tr("Waiting for WeChat to quit…", "等待微信退出…"),
        Wait::Open => tr("Waiting for WeChat to start…", "等待微信启动…"),
        Wait::Key => tr("Waiting for the key…", "等待密钥…"),
    }
}

/// 等待怎样结束，写在这一行的末尾。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitEnd {
    Done,
    TimedOut,
    Interrupted,
    Failed,
}

/// 正在进行的等待所在的行：`Waiting for the key…`，后面是灰色的倒计时，结束时倒计时换成结果。
#[derive(Debug, Default)]
struct WaitLine(Option<(Wait, crate::status::Pending)>);

impl WaitLine {
    /// 显示 `phase` 还能等多久；是新的等待时另起一行（前一个等待按完成结束）。
    fn tick(&mut self, phase: Wait, remaining: u64) {
        if self.0.as_ref().map(|(shown, _)| *shown) != Some(phase) {
            self.end(WaitEnd::Done);
            self.0 = Some((phase, crate::status::begin(wait_head(phase))));
        }
        if let Some((_, line)) = &self.0 {
            line.tick(
                &tr("exits in {n} s", "{n} 秒后自动退出").replace("{n}", &remaining.to_string()),
            );
        }
    }

    /// 结束正在进行的等待所在的行（如果有）。
    fn end(&mut self, how: WaitEnd) {
        use crate::colour::Role;
        let Some((_, line)) = self.0.take() else {
            return;
        };
        let (text, role) = match how {
            WaitEnd::Done => (tr("done", "完成"), Role::Success),
            WaitEnd::TimedOut => (tr("timed out", "超时"), Role::Error),
            WaitEnd::Interrupted => (tr("interrupted", "已中断"), Role::Warning),
            WaitEnd::Failed => (tr("failed", "失败"), Role::Error),
        };
        line.finish(text, role);
    }

    fn report(&mut self, event: WaitEvent) {
        match event {
            WaitEvent::AskQuit(pids) => {
                self.end(WaitEnd::Done);
                let list = pids
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                // Windows 的托盘在右下角；Linux 桌面的托盘位置不定，也可能没有（WSLg）
                let how = if cfg!(windows) {
                    tr(
                        "system tray icon → right click → Quit WeChat",
                        "右下角托盘图标 → 右键 → 退出微信",
                    )
                } else {
                    tr(
                        "choose Quit from WeChat's tray icon or its menu",
                        "从托盘图标或微信的菜单中选择退出",
                    )
                };
                phase(
                    json!({ "type": "key_phase", "phase": "quit_wechat", "pids": pids }),
                    &format!(
                        "{}\n{how}",
                        tr(
                            "WeChat is running, pid {list}. Quit it completely first:",
                            "微信正在运行，pid {list}。请先完全退出微信：",
                        )
                        .replace("{list}", &list)
                    ),
                );
            }
            WaitEvent::AskOpen { was_running } => {
                self.end(WaitEnd::Done);
                phase(
                    json!({ "type": "key_phase", "phase": "open_wechat", "wasRunning": was_running }),
                    if was_running {
                        tr("Open WeChat again.", "请重新打开微信。")
                    } else {
                        tr(
                            "WeChat is not running. Open it.",
                            "未检测到微信，请打开微信。",
                        )
                    },
                );
            }
            WaitEvent::Tick { phase, remaining } => self.tick(phase, remaining),
        }
    }
}

/// 一条状态消息：带 `--json` 时是 JSON 事件，否则单独占一行。
fn phase(json_event: Value, text: &str) {
    if crate::output::json_output() {
        crate::output::event(json_event);
    } else {
        crate::status::say(text);
    }
}

static KEY_DB_INTERRUPTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 从另一个线程像 Ctrl+C 那样结束 `key db`。
///
/// 正在等待微信时停止等待；已经装好的钩子先恢复微信的调试寄存器再分离，不在微信里留下任何东西（#287）。
/// 命令以 `user_interrupt` 结束。调用时钩子可能正要装上：命令结束前要反复调用。
pub fn interrupt_key_db() {
    KEY_DB_INTERRUPTED.store(true, std::sync::atomic::Ordering::Release);
    #[cfg(all(windows, target_arch = "x86_64"))]
    weflow_native::windows_db_key::interrupt();
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    weflow_native::linux_key_helper::interrupt();
    #[cfg(target_os = "macos")]
    weflow_native::macos_key_helper::interrupt();
}

fn key_db_interrupted() -> bool {
    KEY_DB_INTERRUPTED.load(std::sync::atomic::Ordering::Acquire)
}

/// 等到微信退出（正在运行时）并重新启动，返回新的进程 ID；超过 `timeout_secs` 时返回 `None`。
/// 时间和进程列表由参数传入，便于测试。
///
/// 最初运行的进程都不在了就算微信已退出，不论这时是否已有新的进程：在两次查看进程列表之间退出又重新打开时，
/// 进程列表从不为空（#127）。
fn wait_for_fresh_wechat(
    list: &mut dyn FnMut() -> Vec<u32>,
    sleep: &mut dyn FnMut(),
    elapsed: &mut dyn FnMut() -> u64,
    timeout_secs: u64,
    report: &mut dyn FnMut(WaitEvent),
) -> Option<u32> {
    let running = list();
    let mut phase = if running.is_empty() {
        report(WaitEvent::AskOpen { was_running: false });
        Wait::Open
    } else {
        report(WaitEvent::AskQuit(running.clone()));
        Wait::Quit
    };
    let fresh = |pids: &[u32]| pids.iter().copied().find(|p| !running.contains(p));
    loop {
        let used = elapsed();
        if used >= timeout_secs {
            return None;
        }
        let pids = list();
        match phase {
            Wait::Quit if !pids.iter().any(|p| running.contains(p)) => {
                phase = Wait::Open;
                report(WaitEvent::AskOpen { was_running: true });
                if let Some(pid) = fresh(&pids) {
                    return Some(pid);
                }
                continue;
            }
            Wait::Open => {
                if let Some(pid) = fresh(&pids) {
                    return Some(pid);
                }
            }
            _ => {}
        }
        report(WaitEvent::Tick {
            phase,
            remaining: timeout_secs - used,
        });
        sleep();
    }
}

/// 正在运行的微信的进程 ID（Windows 上是 `Weixin.exe`、`WeChat.exe`，Linux 上是 `wechat`），按系统列出的顺序。
fn wechat_pids() -> Vec<u32> {
    #[cfg(target_os = "windows")]
    {
        crate::process::pids_by_image(&["Weixin.exe", "WeChat.exe"])
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("pgrep")
            .arg("-x")
            .arg("wechat")
            .output()
            .ok()
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter_map(|line| line.trim().parse().ok())
                    .collect()
            })
            .unwrap_or_default()
    }
    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        find_wechat_pid().into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// macOS 的挂钩失败后重试一次（微信已退出时除外），错误里带着失败的原因（#806）。
    #[test]
    fn the_macos_hook_is_tried_again_once_and_says_why_it_failed() {
        let breakpoint = "patch_breakpoint_failed:ptrace_word:-1";
        assert_eq!(macos_hook_failed(breakpoint, false), None);
        let last = macos_hook_failed(breakpoint, true).unwrap();
        assert!(
            last.contains(&format!("\nReason: {breakpoint}")) && last.contains("restart the Mac"),
            "{last}"
        );
        let quit = macos_hook_failed("process_died_while_waiting", false).unwrap();
        assert!(quit.starts_with("WeChat quit"), "{quit}");
        let scan = unsupported_wechat("Sink pattern not found");
        assert!(scan.ends_with("\nReason: Sink pattern not found"), "{scan}");
    }

    /// 用预先写好的进程列表驱动 `wait_for_fresh_wechat`；每次 `sleep` 算一秒。
    fn run_wait(script: Vec<Vec<u32>>, timeout: u64) -> (Option<u32>, Vec<&'static str>, u64) {
        use std::cell::{Cell, RefCell};
        let calls = Cell::new(0usize);
        let seconds = Cell::new(0u64);
        let events = RefCell::new(Vec::new());
        let found = wait_for_fresh_wechat(
            &mut || {
                let i = calls.get().min(script.len() - 1);
                calls.set(calls.get() + 1);
                script[i].clone()
            },
            &mut || seconds.set(seconds.get() + 1),
            &mut || seconds.get(),
            timeout,
            &mut |event| {
                events.borrow_mut().push(match event {
                    WaitEvent::AskQuit(_) => "ask_quit",
                    WaitEvent::AskOpen { was_running: true } => "ask_open_after_quit",
                    WaitEvent::AskOpen { .. } => "ask_open",
                    WaitEvent::Tick {
                        phase: Wait::Quit, ..
                    } => "tick_quit",
                    WaitEvent::Tick {
                        phase: Wait::Open, ..
                    } => "tick_open",
                    WaitEvent::Tick { .. } => "tick",
                });
            },
        );
        (found, events.into_inner(), seconds.get())
    }

    #[test]
    fn waits_for_quit_then_a_new_start() {
        // 一开始在运行，第三次查看之后退出，再过两次出现新的进程
        let (found, events, _) = run_wait(
            vec![vec![10], vec![10], vec![10], vec![], vec![], vec![77]],
            180,
        );
        assert_eq!(found, Some(77));
        assert_eq!(
            events,
            [
                "ask_quit",
                "tick_quit",
                "tick_quit",
                "ask_open_after_quit",
                "tick_open"
            ]
        );
    }

    #[test]
    fn a_wechat_quit_and_opened_between_two_looks_is_the_new_one() {
        // 同一次查看时旧进程已退出、新进程已出现（#127）
        let (found, events, _) = run_wait(vec![vec![10, 11], vec![10, 11], vec![42]], 180);
        assert_eq!(found, Some(42));
        assert_eq!(events, ["ask_quit", "tick_quit", "ask_open_after_quit"]);
        // 旧进程退出之前就有了新进程：仍然等旧进程退出
        let (found, events, _) =
            run_wait(vec![vec![10], vec![10, 42], vec![10, 42], vec![42]], 180);
        assert_eq!(found, Some(42));
        assert_eq!(
            events,
            ["ask_quit", "tick_quit", "tick_quit", "ask_open_after_quit"]
        );
        // 旧微信留下的进程不会被当成新的
        let (found, _, _) = run_wait(vec![vec![10], vec![], vec![10, 42]], 180);
        assert_eq!(found, Some(42));
    }

    #[test]
    fn skips_the_quit_step_when_wechat_is_not_running() {
        let (found, events, _) = run_wait(vec![vec![], vec![], vec![5]], 180);
        assert_eq!(found, Some(5));
        assert_eq!(events, ["ask_open", "tick_open"]);
    }

    #[test]
    fn gives_up_after_the_timeout() {
        // 微信一直不退出
        let (found, events, seconds) = run_wait(vec![vec![10]], 5);
        assert_eq!(found, None);
        assert_eq!(seconds, 5);
        assert_eq!(events.iter().filter(|e| **e == "tick_quit").count(), 5);
    }
}
