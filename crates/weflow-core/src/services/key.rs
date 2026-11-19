//! The keys: the database key read from WeChat's process, which waits for a fresh start of WeChat, and the
//! image keys.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::ServiceHub;
use crate::error::{AppError, AppResult};

impl ServiceHub {
    /// `key:autoGetDbKey`. WeChat only produces the key while it opens its databases, so on Windows and Linux the
    /// command asks the user to quit and reopen WeChat, hooks the new process and waits (default 180 s in total) for
    /// the user to click "Enter WeChat". On macOS it hooks the running WeChat. Everywhere the login prompt
    /// follows the installed hook.
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

    /// Hooks WeChat and waits for the database key; shared by Windows, Linux and macOS.
    fn key_db_hooked(
        &self,
        wxkey: &weflow_native::wxkey::WxKey,
        pid_override: Option<u32>,
        timeout_secs: u64,
    ) -> AppResult<crate::contract::DbKey> {
        use crate::output::{end_status_line, status_line};
        use weflow_native::wxkey::DbKeyError;
        let started = std::time::Instant::now();
        let remaining = || timeout_secs.saturating_sub(started.elapsed().as_secs());
        let timeout_error = || {
            AppError::native(format!(
                "timed out after {timeout_secs} s without getting the key; quit WeChat completely, reopen it and click \"Enter WeChat\" in the login window, then run the command again"
            ))
        };

        let interrupted = || {
            end_status_line();
            AppError::new("user_interrupt", "interrupted by user", 130)
        };
        let pid = match pid_override {
            Some(pid) => pid,
            None if cfg!(any(windows, target_os = "linux")) => {
                let found = wait_for_fresh_wechat(
                    &mut wechat_pids,
                    &mut || std::thread::sleep(std::time::Duration::from_secs(1)),
                    // an interruption ends the wait as its timeout would
                    &mut || {
                        if key_db_interrupted() {
                            u64::MAX
                        } else {
                            started.elapsed().as_secs()
                        }
                    },
                    timeout_secs,
                    &mut |event| report_wait(event),
                );
                end_status_line();
                if key_db_interrupted() {
                    return Err(interrupted());
                }
                found.ok_or_else(timeout_error)?
            }
            None => find_wechat_pid().ok_or_else(|| {
                AppError::runtime(
                    "WeChat process not found; start WeChat first, or pass --pid (`weflow key processes` lists WeChat's processes)",
                )
            })?,
        };
        let login_phase = || {
            phase(
                json!({ "type": "key_phase", "phase": "login", "pid": pid }),
                if crate::locale::current() == crate::locale::Lang::Zh {
                    format!("检测到微信（pid {pid}）请在登录窗口点击「进入微信」。")
                } else {
                    format!("WeChat found (pid {pid}). Click \"Enter WeChat\" in the login window.")
                },
            )
        };
        // the macOS hook is tried again once at most: more tries can set off the system's protection (#806)
        let mut tried_again = false;
        let key = loop {
            if key_db_interrupted() {
                return Err(interrupted());
            }
            if remaining() == 0 {
                end_status_line();
                return Err(timeout_error());
            }
            // wx_key's own progress messages are noise; only its errors are shown, and the login prompt waits
            // until the hook is in place (#872)
            let mut on_status = |msg: &str, level: i32| {
                if msg == "Hook installed; log in to WeChat now" {
                    login_phase();
                }
                if level == 2 {
                    end_status_line();
                    crate::output::event(
                        json!({ "type": "key_status", "level": level, "message": crate::locale::localize(msg.to_string()) }),
                    );
                }
            };
            let mut on_tick = |left: u64| status_line(&wait_text(Wait::Key, left));
            match wxkey.get_db_key_with_tick(
                pid,
                std::time::Duration::from_secs(remaining()),
                &mut on_status,
                &mut on_tick,
            ) {
                Ok(key) => break key,
                Err(DbKeyError::AccessDenied(detail)) => {
                    end_status_line();
                    let hint = if cfg!(target_os = "linux") {
                        format!(
                            "permission denied: cannot attach to WeChat (pid {pid}). Run weflow as root or allow ptrace through /proc/sys/kernel/yama/ptrace_scope. ({detail})"
                        )
                    } else if cfg!(target_os = "macos") {
                        format!(
                            "permission denied: cannot attach to WeChat (pid {pid}). Run weflow with sudo, and check that macOS lets it debug other processes (System Integrity Protection, code signing). ({detail})"
                        )
                    } else {
                        format!(
                            "permission denied: cannot open the WeChat process (pid {pid}). Run the terminal as administrator, close security software that blocks it, and make sure WeChat itself is not running as administrator. ({detail})"
                        )
                    };
                    return Err(AppError::native(hint));
                }
                Err(DbKeyError::Timeout) => {
                    end_status_line();
                    return Err(timeout_error());
                }
                Err(DbKeyError::Interrupted) => return Err(interrupted()),
                // the process may not be ready to be hooked right after it started: try again
                Err(DbKeyError::NotReady(_)) if remaining() > 0 => {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
                Err(DbKeyError::Unsupported(detail)) => {
                    end_status_line();
                    return Err(AppError::native(unsupported_wechat(&detail)));
                }
                Err(DbKeyError::Other(detail)) if cfg!(target_os = "macos") => {
                    match macos_hook_failed(&detail, tried_again || remaining() == 0) {
                        None => {
                            tried_again = true;
                            std::thread::sleep(std::time::Duration::from_secs(1));
                        }
                        Some(message) => {
                            end_status_line();
                            return Err(AppError::native(message));
                        }
                    }
                }
                Err(other) => {
                    end_status_line();
                    return Err(AppError::native(other.to_string()));
                }
            }
        };
        end_status_line();
        Ok(crate::contract::DbKey {
            decrypt_key: Some(key),
            method: "wx_key".into(),
            pid: Some(pid),
            saved: Vec::new(),
        })
    }

    /// Image keys from the `kvcomm` cache, verified against samples of the account; independent of stored image keys.
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

    /// The running WeChat clients, main application first (`key processes`, macOS).
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
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

    /// An image AES key found in WeChat's memory, on Linux and macOS. `pid` defaults to the running WeChat. Without
    /// `ciphertext` the memory is searched with the first blocks of the newest thumbnails of the configured account,
    /// and a key is taken only once the thumbnails confirm it ([`crate::image_keys::check_memory_key`]), which also
    /// gives the XOR key when one of them decodes with the pair. With `ciphertext` the first key that decrypts that
    /// one block into an image header is taken.
    ///
    /// # Errors
    ///
    /// A usage error for a ciphertext that is not 32 hex characters, `image_key_no_template` without thumbnails,
    /// and a native error when the system does not let weflow read WeChat's memory.
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
                    "WeChat process not found; start WeChat and log in, or give its pid",
                )
            })?,
        };
        use crate::image_keys::MemoryKeyCheck;
        let found = match ciphertext {
            Some(text) => {
                let block = weflow_native::image_key_scan::parse_ciphertext(text.trim())
                    .ok_or_else(|| {
                        AppError::usage(
                            "the ciphertext must be 32 hex characters (one 16-byte block)",
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

/// Ends `key db` on Linux before the user is asked to restart WeChat when this process may not attach to it, as
/// Yama's `ptrace_scope` decides (#854).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn linux_ptrace_check() -> AppResult<()> {
    use weflow_native::linux_key_helper::{ptrace_access, PtraceAccess};
    match ptrace_access() {
        PtraceAccess::Allowed => Ok(()),
        PtraceAccess::NeedsCapability { scope } => Err(AppError::native(format!(
            "permission denied: weflow cannot attach to WeChat while /proc/sys/kernel/yama/ptrace_scope is {scope}. Run weflow as root, or set ptrace_scope to 0"
        ))),
        PtraceAccess::Disabled => Err(AppError::native(
            "permission denied: /proc/sys/kernel/yama/ptrace_scope is 3, which stops every process from attaching to WeChat, root included, until the system restarts",
        )),
    }
}

/// The error when the hook finds nowhere to stop in this WeChat (`detail` says what was missing): WeChat had not
/// fully started, or its version is one the hook does not know (#806).
fn unsupported_wechat(detail: &str) -> String {
    format!(
        "cannot find where WeChat takes its database key ({detail}); make sure WeChat has fully started and is in front, otherwise this WeChat version is not supported yet (4.1.7 and 4.1.8.100 are known to work)"
    )
}

/// What a failure of the macOS hook (`detail`) leads to (#806): `None` to try once more, while `last` is false, as a
/// breakpoint or a thread's state may have been refused at a bad moment; else the error to end with. A WeChat that
/// quit is not tried again.
fn macos_hook_failed(detail: &str, last: bool) -> Option<String> {
    if detail == "process_died_while_waiting" {
        return Some(
            "WeChat quit while the command waited for the key; reopen WeChat, then run the command again".into(),
        );
    }
    last.then(|| {
        format!(
            "cannot hook WeChat ({detail}); quit WeChat completely and restart the Mac before trying again, as more tries can set off macOS's protection"
        )
    })
}

/// Whether `key image-mem` can read WeChat's memory here.
const IMAGE_MEM_SUPPORTED: bool = cfg!(any(
    all(target_os = "linux", target_arch = "x86_64"),
    target_os = "macos"
));

/// The first key in the writable memory of WeChat's process `pid` that decrypts one of `ciphertexts` into the start
/// of an image and that `accept` takes; always `None` where [`IMAGE_MEM_SUPPORTED`] is false.
///
/// # Errors
///
/// A native error saying how to get the permission when the system does not let weflow read WeChat's memory.
#[cfg_attr(
    not(any(all(target_os = "linux", target_arch = "x86_64"), target_os = "macos")),
    expect(unused_variables, reason = "no memory scanner on this platform")
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
                    "permission denied: cannot read WeChat's memory (pid {pid}). Run weflow with sudo. ({detail})"
                ))
            }
            MemoryScanError::PermissionDenied(detail) => AppError::native(format!(
                "permission denied: cannot read WeChat's memory (pid {pid}). Run weflow as root or allow ptrace through /proc/sys/kernel/yama/ptrace_scope. ({detail})"
            )),
            MemoryScanError::Failed(detail) => AppError::native(format!(
                "cannot read WeChat's memory (pid {pid}): {detail}"
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

/// The remaining-time line shown while waiting.
fn wait_text(phase: Wait, remaining: u64) -> String {
    let zh = crate::locale::current() == crate::locale::Lang::Zh;
    let what = match (phase, zh) {
        (Wait::Quit, true) => "等待微信退出…",
        (Wait::Quit, false) => "Waiting for WeChat to quit...",
        (Wait::Open, true) => "等待微信启动…",
        (Wait::Open, false) => "Waiting for WeChat to start...",
        (Wait::Key, true) => "等待密钥…",
        (Wait::Key, false) => "Waiting for the key...",
    };
    if zh {
        format!("{what}（剩余 {remaining} 秒自动退出）")
    } else {
        format!("{what} (exits automatically in {remaining} s)")
    }
}

/// One status message: JSON event with `--json`, a plain line otherwise.
fn phase(json_event: Value, text: String) {
    crate::output::end_status_line();
    if crate::output::json_output() {
        crate::output::event(json_event);
    } else {
        eprintln!("{text}");
    }
}

fn report_wait(event: WaitEvent) {
    let zh = crate::locale::current() == crate::locale::Lang::Zh;
    match event {
        WaitEvent::AskQuit(pids) => {
            let list = pids
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            // the tray is at the bottom right on Windows; Linux desktops put it anywhere, or have none (WSLg)
            let how = match (cfg!(windows), zh) {
                (true, true) => "右下角托盘图标 → 右键 → 退出微信",
                (true, false) => "system tray icon -> right click -> Quit WeChat",
                (false, true) => "从托盘图标或微信的菜单中选择退出",
                (false, false) => "choose Quit from WeChat's tray icon or its menu",
            };
            phase(
                json!({ "type": "key_phase", "phase": "quit_wechat", "pids": pids }),
                if zh {
                    format!("检测到微信正在运行（pid {list}）请先完全退出微信：\n{how}")
                } else {
                    format!("WeChat is running (pid {list}). Quit it completely first:\n{how}")
                },
            );
        }
        WaitEvent::AskOpen { was_running } => phase(
            json!({ "type": "key_phase", "phase": "open_wechat", "wasRunning": was_running }),
            match (was_running, zh) {
                (true, true) => "微信已退出，请重新打开微信。".to_string(),
                (true, false) => "WeChat has quit. Open it again.".to_string(),
                (false, true) => "未检测到微信，请打开微信。".to_string(),
                (false, false) => "WeChat is not running. Open it.".to_string(),
            },
        ),
        WaitEvent::Tick { phase, remaining } => {
            crate::output::status_line(&wait_text(phase, remaining))
        }
    }
}

static KEY_DB_INTERRUPTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Ends `key db` as Ctrl+C does, from another thread.
///
/// While it waits for WeChat it stops waiting, and a hook it has set restores WeChat's debug registers and detaches
/// first, so nothing of it is left in WeChat (#287). It ends with `user_interrupt`. A hook may be about to be set when
/// this is called: call it again until the command ends.
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

/// Waits until WeChat has been quit (when it is running now) and started again; returns the new process id, or
/// `None` once `timeout_secs` have passed. Time and process list are passed in so the logic can be tested.
///
/// WeChat has quit once none of the processes running at first is left, whether or not a new one is there by
/// then: quit and opened again between two looks at the process list, it never has none (#127).
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

/// Process ids of the running WeChat (Weixin.exe / WeChat.exe on Windows, `wechat` on Linux), in the order the
/// system lists them.
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

    /// A failure of the macOS hook is tried again once, unless WeChat quit, and its reason is in the error (#806).
    #[test]
    fn the_macos_hook_is_tried_again_once_and_says_why_it_failed() {
        let breakpoint = "patch_breakpoint_failed:ptrace_word:-1";
        assert_eq!(macos_hook_failed(breakpoint, false), None);
        let last = macos_hook_failed(breakpoint, true).unwrap();
        assert!(
            last.contains(breakpoint) && last.contains("restart the Mac"),
            "{last}"
        );
        let quit = macos_hook_failed("process_died_while_waiting", false).unwrap();
        assert!(quit.starts_with("WeChat quit"), "{quit}");
        let scan = unsupported_wechat("Sink pattern not found");
        assert!(scan.contains("(Sink pattern not found)"), "{scan}");
    }

    /// Drives `wait_for_fresh_wechat` with a scripted process list; each `sleep` is one second.
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
        // running at first, gone after the third check, a new process after two more
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
        // the old process gone and a new one there at the same look (#127)
        let (found, events, _) = run_wait(vec![vec![10, 11], vec![10, 11], vec![42]], 180);
        assert_eq!(found, Some(42));
        assert_eq!(events, ["ask_quit", "tick_quit", "ask_open_after_quit"]);
        // a new process before the old one is gone: still waiting for the old one
        let (found, events, _) =
            run_wait(vec![vec![10], vec![10, 42], vec![10, 42], vec![42]], 180);
        assert_eq!(found, Some(42));
        assert_eq!(
            events,
            ["ask_quit", "tick_quit", "tick_quit", "ask_open_after_quit"]
        );
        // a process left of the old WeChat is not taken for the new one
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
        // WeChat never quits
        let (found, events, seconds) = run_wait(vec![vec![10]], 5);
        assert_eq!(found, None);
        assert_eq!(seconds, 5);
        assert_eq!(events.iter().filter(|e| **e == "tick_quit").count(), 5);
    }
}
