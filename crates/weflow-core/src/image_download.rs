//! Port of WeFlow's `imageDownloadService.ts`: the Windows-only `img_helper.dll` hook that
//! makes WeChat download original-size images for a whitelist of conversations.
//!
//! The DLL (embedded from `crates/weflow-assets/assets/image/win32/x64`) is loaded with `InitImgHelper(pid, whitelist)`
//! against the main `Weixin.exe` process, re-checked every 30 seconds so a restarted WeChat is
//! hooked again. On every other platform the service reports `supported: false`.
use std::ffi::{c_char, CStr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libloading::Library;
use serde_json::{json, Value};

use crate::sync::MutexExt;

type InitFn = unsafe extern "C" fn(u32, *const c_char) -> bool;
type UninstallFn = unsafe extern "C" fn();
type ErrorFn = unsafe extern "C" fn() -> *const c_char;

struct Helper {
    _lib: Library,
    init: InitFn,
    uninstall: UninstallFn,
    error: ErrorFn,
}

#[derive(Default)]
struct Inner {
    helper: Option<Helper>,
    pid: Option<u32>,
    hooked: bool,
    whitelist: Vec<String>,
    poll_stop: Option<Arc<AtomicBool>>,
}

#[derive(Clone)]
pub struct ImageAutoDownload {
    inner: Arc<Mutex<Inner>>,
    runtime_dir: PathBuf,
}

pub fn supported() -> bool {
    cfg!(all(target_os = "windows", target_arch = "x86_64"))
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

/// The whitelist as the DLL expects it: UTF-8 names separated by NUL, terminated by two NULs.
fn whitelist_buffer(whitelist: &[String]) -> Option<Vec<u8>> {
    if whitelist.is_empty() {
        return None;
    }
    let mut buf = whitelist.join("\0").into_bytes();
    buf.extend_from_slice(&[0, 0]);
    Some(buf)
}

impl ImageAutoDownload {
    pub fn new(runtime_dir: &Path) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            runtime_dir: runtime_dir.to_path_buf(),
        }
    }

    fn dll_path(&self) -> PathBuf {
        self.runtime_dir.join("image/win32/x64/img_helper.dll")
    }

    fn ensure_loaded(&self, inner: &mut Inner) -> bool {
        if inner.helper.is_some() {
            return true;
        }
        if !supported() {
            return false;
        }
        let path = self.dll_path();
        if !path.exists() {
            return false;
        }
        // SAFETY: loading runs the initializers of WeFlow's own `img_helper.dll` from the runtime directory; its
        // three exports are read with the types the DLL declares and kept in `Helper` next to the library, so none is
        // called after it unloads.
        unsafe {
            let Ok(lib) = Library::new(&path) else {
                return false;
            };
            let (Ok(init), Ok(uninstall), Ok(error)) = (
                lib.get::<InitFn>(b"InitImgHelper\0").map(|s| *s),
                lib.get::<UninstallFn>(b"UninstallImgHelper\0").map(|s| *s),
                lib.get::<ErrorFn>(b"GetImgHelperError\0").map(|s| *s),
            ) else {
                return false;
            };
            inner.helper = Some(Helper {
                _lib: lib,
                init,
                uninstall,
                error,
            });
        }
        true
    }

    fn unhook_locked(inner: &mut Inner) {
        if inner.hooked {
            if let Some(h) = &inner.helper {
                // SAFETY: `uninstall` comes from the library `h` keeps loaded and takes no arguments.
                unsafe { (h.uninstall)() };
            }
        }
        inner.hooked = false;
        inner.pid = None;
    }

    fn check_and_hook(&self, manual_start: bool) -> Value {
        let pid = find_main_wechat_pid();
        let mut inner = self.inner.lock_or_panic();
        let Some(pid) = pid else {
            if inner.hooked {
                Self::unhook_locked(&mut inner);
            }
            return json!({ "success": true, "error": "waiting for WeChat to start" });
        };
        if inner.hooked && inner.pid == Some(pid) {
            return json!({ "success": true });
        }
        if inner.hooked {
            Self::unhook_locked(&mut inner);
        }
        let whitelist = whitelist_buffer(&inner.whitelist);
        let Some(helper) = &inner.helper else {
            return json!({ "success": false, "error": "core component initialization failed" });
        };
        // SAFETY: `init` comes from the library `helper` keeps loaded; the whitelist is null or the NUL-separated,
        // double-NUL-terminated buffer the DLL expects (see `whitelist_buffer`), alive for the call.
        let ok = unsafe {
            (helper.init)(
                pid,
                whitelist
                    .as_ref()
                    .map_or(std::ptr::null(), |b| b.as_ptr() as *const c_char),
            )
        };
        if ok {
            inner.hooked = true;
            inner.pid = Some(pid);
            return json!({ "success": true });
        }
        // SAFETY: `error` comes from the library `helper` keeps loaded and returns null or a NUL-terminated string,
        // copied right away.
        let msg = unsafe {
            let p = (helper.error)();
            if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p).to_string_lossy().to_string()
            }
        };
        if manual_start {
            if let Some(stop) = inner.poll_stop.take() {
                stop.store(true, Ordering::Relaxed);
            }
        }
        json!({ "success": false, "error": if msg.is_empty() { "hook failed".to_string() } else { msg } })
    }

    /// `image:startAutoDownload`
    pub fn start(&self, whitelist: Vec<String>) -> Value {
        {
            let mut inner = self.inner.lock_or_panic();
            if !self.ensure_loaded(&mut inner) {
                return json!({ "success": false, "error": "core component initialization failed" });
            }
            if inner.hooked {
                Self::unhook_locked(&mut inner);
            }
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
        Self::unhook_locked(&mut inner);
        json!({ "success": true })
    }

    /// `image:getAutoDownloadStatus`
    pub fn status(&self) -> Value {
        let inner = self.inner.lock_or_panic();
        json!({ "isHooked": inner.hooked, "pid": inner.pid, "supported": supported() })
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
        let svc = ImageAutoDownload::new(Path::new("/nonexistent"));
        if !supported() {
            assert_eq!(svc.start(vec![])["success"], false);
        }
        assert_eq!(svc.status()["isHooked"], false);
        assert_eq!(svc.stop()["success"], true);
    }
}
