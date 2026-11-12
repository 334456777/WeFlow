//! The Windows-only image auto-download hook: makes WeChat download original-size images for a whitelist of
//! conversations.
//!
//! The hook is installed in the main `Weixin.exe` process and re-checked every 30 seconds, so a restarted WeChat is
//! hooked again. On every other platform the service reports `supported: false`.
//!
//! This is the Rust replacement for `img_helper.dll`; it is compiled into `weflow.exe` and needs no Visual C++
//! runtime.
mod hook;
mod shellcode;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::sync::MutexExt;

pub use hook::supported;

#[derive(Default)]
struct Inner {
    hook: Option<hook::Hook>,
    pid: Option<u32>,
    whitelist: Vec<String>,
    poll_stop: Option<Arc<AtomicBool>>,
}

#[derive(Clone, Default)]
pub struct ImageAutoDownload {
    inner: Arc<Mutex<Inner>>,
}

/// The main WeChat process among `(pid, command line)` of the `Weixin.exe` processes: of those whose command line
/// names `weixin.exe` (ignoring case), the one with the shortest; the others carry `--type=…` and such.
#[cfg(any(windows, test))]
fn pick_main_pid(processes: &[(u32, String)]) -> Option<u32> {
    processes
        .iter()
        .filter(|(_, line)| line.to_lowercase().contains("weixin.exe"))
        .min_by_key(|(_, line)| line.len())
        .map(|(pid, _)| *pid)
}

#[cfg(windows)]
fn find_main_wechat_pid() -> Option<u32> {
    let all = crate::process::processes();
    let wechat: Vec<&crate::process::Process> = all
        .iter()
        .filter(|p| p.image.eq_ignore_ascii_case("Weixin.exe"))
        .collect();
    // a process that cannot be opened is left out, as one whose command line WMI did not give
    let lines: Vec<(u32, String)> = wechat
        .iter()
        .filter_map(|p| Some((p.pid, crate::process::command_line(p.pid)?)))
        .collect();
    let main = pick_main_pid(&lines);
    // the other way to tell it, not trusted yet (#101): the one whose parent is no `Weixin.exe`
    let rootless: Vec<u32> = wechat
        .iter()
        .filter(|p| !wechat.iter().any(|q| q.pid == p.parent))
        .map(|p| p.pid)
        .collect();
    tracing::debug!(?main, ?rootless, "main WeChat process");
    main
}

#[cfg(not(windows))]
fn find_main_wechat_pid() -> Option<u32> {
    None
}

/// The whitelist as the shellcode expects it: UTF-8 names separated by NUL, terminated by two NULs.
fn whitelist_buffer(whitelist: &[String]) -> Option<Vec<u8>> {
    if whitelist.is_empty() {
        return None;
    }
    let mut buf = whitelist.join("\0").into_bytes();
    buf.extend_from_slice(&[0, 0]);
    Some(buf)
}

impl ImageAutoDownload {
    pub fn new() -> Self {
        Self::default()
    }

    fn check_and_hook(&self, manual_start: bool) -> Value {
        let pid = find_main_wechat_pid();
        let mut inner = self.inner.lock_or_panic();
        let Some(pid) = pid else {
            if let Some(hook) = inner.hook.take() {
                hook.uninstall();
            }
            inner.pid = None;
            return json!({ "success": true, "error": "waiting for WeChat to start" });
        };
        if inner.hook.is_some() && inner.pid == Some(pid) {
            return json!({ "success": true });
        }
        if let Some(hook) = inner.hook.take() {
            hook.uninstall();
        }
        inner.pid = None;
        let whitelist = whitelist_buffer(&inner.whitelist);
        match hook::Hook::install(pid, whitelist.as_deref().unwrap_or(&[])) {
            Ok(hook) => {
                inner.hook = Some(hook);
                inner.pid = Some(pid);
                json!({ "success": true })
            }
            Err(err) => {
                if manual_start {
                    if let Some(stop) = inner.poll_stop.take() {
                        stop.store(true, Ordering::Relaxed);
                    }
                }
                json!({ "success": false, "error": if err.is_empty() { "hook failed".to_string() } else { err } })
            }
        }
    }

    /// `image:startAutoDownload`
    pub fn start(&self, whitelist: Vec<String>) -> Value {
        if !supported() {
            return json!({ "success": false, "error": "image auto download is only supported on Windows x64" });
        }
        {
            let mut inner = self.inner.lock_or_panic();
            if let Some(hook) = inner.hook.take() {
                hook.uninstall();
            }
            inner.pid = None;
            inner.whitelist = whitelist;
            if inner.poll_stop.is_none() {
                let stop = Arc::new(AtomicBool::new(false));
                inner.poll_stop = Some(stop.clone());
                let me = self.clone();
                std::thread::spawn(move || loop {
                    for _ in 0..300 {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    me.check_and_hook(false);
                });
            }
        }
        self.check_and_hook(true)
    }

    /// `image:stopAutoDownload`
    pub fn stop(&self) -> Value {
        let mut inner = self.inner.lock_or_panic();
        if let Some(stop) = inner.poll_stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(hook) = inner.hook.take() {
            hook.uninstall();
        }
        inner.pid = None;
        json!({ "success": true })
    }

    /// `image:getAutoDownloadStatus`
    pub fn status(&self) -> Value {
        let inner = self.inner.lock_or_panic();
        json!({ "isHooked": inner.hook.is_some(), "pid": inner.pid, "supported": supported() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_process_is_the_shortest_weixin_command_line() {
        let procs = |list: &[(u32, &str)]| -> Vec<(u32, String)> {
            list.iter()
                .map(|(pid, line)| (*pid, line.to_string()))
                .collect()
        };
        let wechat = procs(&[
            (
                11,
                "\"C:\\WeChat\\Weixin.exe\" --type=renderer --lang=en-US",
            ),
            (7, "\"C:\\WeChat\\Weixin.exe\""),
            (9, "\"C:\\WeChat\\Weixin.exe\" --type=gpu-process"),
        ]);
        assert_eq!(pick_main_pid(&wechat), Some(7));
        assert_eq!(pick_main_pid(&procs(&[(3, "weixin.exe")])), Some(3));
        assert_eq!(
            pick_main_pid(&procs(&[(4, "C:\\WECHAT\\WEIXIN.EXE")])),
            Some(4),
            "case"
        );
        assert_eq!(pick_main_pid(&[]), None);
        assert_eq!(pick_main_pid(&procs(&[(1, "other.exe")])), None);
        // of equal lengths, the first listed
        assert_eq!(
            pick_main_pid(&procs(&[(5, "weixin.exe"), (6, "Weixin.exe")])),
            Some(5)
        );
    }

    #[test]
    fn whitelist_is_nul_separated_and_double_nul_terminated() {
        assert_eq!(whitelist_buffer(&[]), None);
        assert_eq!(
            whitelist_buffer(&["a".into(), "b".into()]),
            Some(b"a\0b\0\0".to_vec())
        );
    }

    #[test]
    fn unsupported_platforms_report_so() {
        let svc = ImageAutoDownload::new();
        if !supported() {
            assert_eq!(svc.start(vec![])["success"], false);
        }
        assert_eq!(svc.status()["isHooked"], false);
        assert_eq!(svc.stop()["success"], true);
    }
}
