//! The keys: the database key read from WeChat's process, which waits for a fresh start of WeChat, and the
//! image keys.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::ServiceHub;
use crate::error::{AppError, AppResult};

impl ServiceHub {
    /// `key:autoGetDbKey`. On Windows WeChat only produces the key while it opens its databases, so the command
    /// asks the user to quit and reopen WeChat, hooks the new process and waits (default 180 s in total) for the
    /// user to click "Enter WeChat".
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
        let wxkey = weflow_native::wxkey::WxKey::load(&self.ctx.runtime_dir)
            .map_err(|err| AppError::native(err.to_string()))?;
        if wxkey.is_available() {
            return self.key_db_hooked(&wxkey, pid_override, timeout_secs);
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let result = weflow_native::wxkey::run_key_helper(&self.ctx.runtime_dir, &["--db-key"])
                .map_err(|err| AppError::native(err.to_string()))?;
            let key = result.trim().to_string();
            Ok(crate::contract::DbKey {
                decrypt_key: Some(key),
                method: "key_helper".into(),
                pid: None,
                saved: Vec::new(),
            })
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err(AppError::native(
                "wx_key library not found; key extraction requires the platform-specific native library"
            ))
        }
    }

    /// Hooks WeChat and waits for the database key; shared by Windows and macOS.
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
            None if cfg!(windows) => {
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
                AppError::runtime("WeChat process not found (looked for Weixin.exe and WeChat.exe); start WeChat first or pass --pid")
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
        if !cfg!(windows) {
            login_phase();
        }

        let key = loop {
            if key_db_interrupted() {
                return Err(interrupted());
            }
            if remaining() == 0 {
                end_status_line();
                return Err(timeout_error());
            }
            // wx_key's own progress messages are noise; only its errors are shown
            let mut on_status = |msg: &str, level: i32| {
                if cfg!(windows) && msg == "Hook installed; log in to WeChat now" {
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
                    return Err(AppError::native(format!(
                        "permission denied: cannot open the WeChat process (pid {pid}). Run the terminal as administrator, close security software that blocks it, and make sure WeChat itself is not running as administrator. ({detail})"
                    )));
                }
                Err(DbKeyError::Timeout | DbKeyError::LoginRequired) => {
                    end_status_line();
                    return Err(timeout_error());
                }
                Err(DbKeyError::Interrupted) => return Err(interrupted()),
                // the process may not be ready to be hooked right after it started: try again
                Err(DbKeyError::NotReady(_)) if remaining() > 0 => {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
                Err(DbKeyError::Other(_)) if !cfg!(windows) && remaining() > 0 => {
                    std::thread::sleep(std::time::Duration::from_secs(1));
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

    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    pub fn key_scan_image(&self, user_dir: &str) -> AppResult<crate::contract::ScannedImageKeys> {
        #[cfg(target_os = "macos")]
        let expanded = crate::config::expand_home(user_dir);
        #[cfg(target_os = "macos")]
        {
            let result = weflow_native::wxkey::run_image_scan_helper(
                &self.ctx.runtime_dir,
                &[expanded.to_string_lossy().as_ref()],
            )
            .map_err(|err| AppError::native(err.to_string()))?;
            let parsed: Value = serde_json::from_str(&result).unwrap_or(json!({ "raw": result }));
            Ok(crate::contract::ScannedImageKeys {
                result: parsed,
                method: "image_scan_helper".into(),
                user_dir: user_dir.to_string(),
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(AppError::native(
                "`key scan-image` is only available on macOS",
            ))
        }
    }
}

fn find_wechat_pid() -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("pgrep")
            .arg("-x")
            .arg("WeChat")
            .output()
            .ok()?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            return stdout
                .lines()
                .next()
                .and_then(|line| line.trim().parse().ok());
        }
        let output2 = std::process::Command::new("pgrep")
            .arg("-x")
            .arg("微信")
            .output()
            .ok()?;
        if output2.status.success() {
            let stdout = String::from_utf8_lossy(&output2.stdout);
            return stdout
                .lines()
                .next()
                .and_then(|line| line.trim().parse().ok());
        }
        None
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
            phase(
                json!({ "type": "key_phase", "phase": "quit_wechat", "pids": pids }),
                if zh {
                    format!("检测到微信正在运行（pid {list}）请先完全退出微信：\n右下角托盘图标 → 右键 → 退出微信")
                } else {
                    format!("WeChat is running (pid {list}). Quit it completely first:\nsystem tray icon -> right click -> Quit WeChat")
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

/// Ends `key db` as Ctrl+C does, from another thread: while it waits for WeChat it stops waiting, and a hook it has
/// set restores WeChat's debug registers and detaches first, so nothing of it is left in WeChat (#287). It ends
/// with `user_interrupt`. A hook may be about to be set when this is called: call it again until the command ends.
pub fn interrupt_key_db() {
    KEY_DB_INTERRUPTED.store(true, std::sync::atomic::Ordering::Release);
    #[cfg(all(windows, target_arch = "x86_64"))]
    weflow_native::windows_db_key::interrupt();
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

/// Process ids of the running WeChat (Weixin.exe / WeChat.exe on Windows), in the order the system lists them.
fn wechat_pids() -> Vec<u32> {
    #[cfg(target_os = "windows")]
    {
        crate::process::pids_by_image(&["Weixin.exe", "WeChat.exe"])
    }
    #[cfg(not(target_os = "windows"))]
    {
        find_wechat_pid().into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
