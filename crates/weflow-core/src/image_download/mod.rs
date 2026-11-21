//! The Windows-only image auto-download hook: makes WeChat download original-size images for a whitelist of
//! conversations.
//!
//! The hook is installed in the main `Weixin.exe` process and re-checked every 30 seconds, so a restarted WeChat is
//! hooked again. A process that could not be hooked is tried again after a delay that doubles with each failure, up
//! to 32 minutes, as each try reads all of `Weixin.dll`; a new process is tried at once. On every other platform the
//! service reports `supported: false`.
//!
//! This is the Rust replacement for `img_helper.dll`; it is compiled into `weflow.exe` and needs no Visual C++
//! runtime.
mod hook;
#[cfg(all(windows, target_arch = "x86_64"))]
mod shellcode;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::sync::MutexExt;

pub use hook::supported;

/// How often the poll thread checks that WeChat is hooked.
const POLL_INTERVAL: Duration = Duration::from_secs(30);
/// The longest wait before the poll thread tries again a process it could not hook.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(32 * 60);

#[derive(Default)]
struct Inner {
    hook: Option<hook::Hook>,
    pid: Option<u32>,
    whitelist: Vec<String>,
    poll_stop: Option<Arc<AtomicBool>>,
    failure: Option<Failure>,
}

/// The last failed hook of a WeChat process, and when the poll thread may try that process again.
#[derive(Debug)]
struct Failure {
    pid: u32,
    error: String,
    attempts: u32,
    retry_at: Instant,
}

impl Failure {
    /// A failure to hook `pid` at `now`, counting the failures of the same process before it.
    fn after(previous: Option<&Failure>, pid: u32, error: String, now: Instant) -> Self {
        let attempts = previous
            .filter(|f| f.pid == pid)
            .map_or(0, |f| f.attempts)
            .saturating_add(1);
        Self {
            pid,
            error,
            attempts,
            retry_at: now + retry_delay(attempts),
        }
    }

    /// Whether the poll thread leaves `pid` alone at `now`.
    fn holds(&self, pid: u32, now: Instant) -> bool {
        self.pid == pid && now < self.retry_at
    }
}

/// The wait after the `attempts`-th failure on one process: [`POLL_INTERVAL`], doubled for each failure before it,
/// up to [`MAX_RETRY_DELAY`].
fn retry_delay(attempts: u32) -> Duration {
    let doublings = attempts.saturating_sub(1);
    POLL_INTERVAL
        .saturating_mul(1u32.checked_shl(doublings).unwrap_or(u32::MAX))
        .min(MAX_RETRY_DELAY)
}

/// The image auto-download hook and its state, shared by every clone.
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
    // a process whose command line cannot be read (one this user may not open) is left out
    let lines: Vec<(u32, String)> = crate::process::processes()
        .iter()
        .filter(|p| p.image.eq_ignore_ascii_case("Weixin.exe"))
        .filter_map(|p| Some((p.pid, crate::process::command_line(p.pid)?)))
        .collect();
    let main = pick_main_pid(&lines);
    tracing::debug!(?main, "main WeChat process");
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
    /// No hook yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Hooks WeChat when it runs and is not hooked yet; nothing to do while it is not running.
    fn check_and_hook(&self, manual_start: bool) -> Result<(), String> {
        let pid = find_main_wechat_pid();
        let mut inner = self.inner.lock_or_panic();
        let Some(pid) = pid else {
            if let Some(hook) = inner.hook.take() {
                hook.uninstall();
            }
            inner.pid = None;
            inner.failure = None;
            return Ok(());
        };
        if inner.hook.is_some() && inner.pid == Some(pid) {
            return Ok(());
        }
        if !manual_start {
            if let Some(failure) = inner
                .failure
                .as_ref()
                .filter(|f| f.holds(pid, Instant::now()))
            {
                return Err(failure.error.clone());
            }
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
                inner.failure = None;
                Ok(())
            }
            Err(err) => {
                let error = if err.is_empty() {
                    "hook failed".to_string()
                } else {
                    err
                };
                let failure = Failure::after(inner.failure.as_ref(), pid, error, Instant::now());
                tracing::warn!(
                    pid,
                    attempts = failure.attempts,
                    error = %failure.error,
                    retry_in = ?retry_delay(failure.attempts),
                    "could not hook WeChat for image auto-download"
                );
                let answer = Err(failure.error.clone());
                inner.failure = Some(failure);
                if manual_start {
                    if let Some(stop) = inner.poll_stop.take() {
                        stop.store(true, Ordering::Relaxed);
                    }
                }
                answer
            }
        }
    }

    /// Hooks WeChat now when it runs, and from then on whenever it starts.
    ///
    /// # Errors
    /// Why WeChat could not be hooked, or that this computer cannot download its images.
    pub fn start(&self, whitelist: Vec<String>) -> Result<(), String> {
        if !supported() {
            return Err("image auto download is only supported on Windows x64".into());
        }
        {
            let mut inner = self.inner.lock_or_panic();
            if let Some(hook) = inner.hook.take() {
                hook.uninstall();
            }
            inner.pid = None;
            inner.whitelist = whitelist;
            inner.failure = None;
            if inner.poll_stop.is_none() {
                let stop = Arc::new(AtomicBool::new(false));
                inner.poll_stop = Some(stop.clone());
                let me = self.clone();
                std::thread::spawn(move || loop {
                    for _ in 0..POLL_INTERVAL.as_millis() / 100 {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    // a failure is logged and kept, and tried again after its delay
                    let _ = me.check_and_hook(false);
                });
            }
        }
        self.check_and_hook(true)
    }

    /// Removes the hook and stops looking for WeChat.
    pub fn stop(&self) {
        let mut inner = self.inner.lock_or_panic();
        if let Some(stop) = inner.poll_stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(hook) = inner.hook.take() {
            hook.uninstall();
        }
        inner.pid = None;
        inner.failure = None;
    }

    /// Whether the hook is in WeChat, in which process, and whether this computer can run it.
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
    fn a_process_that_could_not_be_hooked_waits_longer_after_each_failure() {
        assert_eq!(retry_delay(1), POLL_INTERVAL);
        assert_eq!(retry_delay(2), POLL_INTERVAL * 2);
        assert_eq!(retry_delay(3), POLL_INTERVAL * 4);
        assert_eq!(retry_delay(7), MAX_RETRY_DELAY);
        assert_eq!(retry_delay(u32::MAX), MAX_RETRY_DELAY);
        let now = Instant::now();
        let first = Failure::after(None, 7, "Target function not found".into(), now);
        assert_eq!(first.attempts, 1);
        assert!(first.holds(7, now));
        assert!(!first.holds(8, now), "a new process is tried at once");
        assert!(!first.holds(7, now + POLL_INTERVAL));
        let second = Failure::after(Some(&first), 7, "again".into(), now + POLL_INTERVAL);
        assert_eq!(second.attempts, 2);
        assert!(second.holds(7, now + POLL_INTERVAL * 2));
        assert!(!second.holds(7, now + POLL_INTERVAL * 3));
        let other = Failure::after(Some(&second), 9, "other".into(), now);
        assert_eq!(other.attempts, 1, "failures are counted per process");
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
            assert!(svc.start(vec![]).is_err());
        }
        assert_eq!(svc.status()["isHooked"], false);
        svc.stop();
        assert_eq!(svc.status()["isHooked"], false);
    }
}
