//! Port of `electron/services/imageDownloadService.ts`: the Windows-only `img_helper.dll` hook that
//! makes WeChat download original-size images for a whitelist of conversations.
//!
//! The DLL (shipped in `resources/image/win32/x64`) is loaded with `InitImgHelper(pid, whitelist)`
//! against the main `Weixin.exe` process, re-checked every 30 seconds so a restarted WeChat is
//! hooked again. On every other platform the service reports `supported: false`.
use std::ffi::{c_char, CStr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libloading::Library;
use serde_json::{json, Value};

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

/// `findMainWeChatPid`: the `Weixin.exe` with the shortest command line is the main process.
pub fn pick_main_pid(processes: &Value) -> Option<u32> {
    let list: Vec<&Value> = match processes {
        Value::Array(a) => a.iter().collect(),
        Value::Object(_) => vec![processes],
        _ => return None,
    };
    list.into_iter()
        .filter_map(|p| {
            let cmd = p.get("CommandLine").and_then(Value::as_str)?;
            if !cmd.to_lowercase().contains("weixin.exe") {
                return None;
            }
            Some((cmd.len(), p.get("ProcessId").and_then(Value::as_u64)? as u32))
        })
        .min_by_key(|(len, _)| *len)
        .map(|(_, pid)| pid)
}

fn find_main_wechat_pid() -> Option<u32> {
    let script = "Get-CimInstance Win32_Process -Filter \"Name = 'Weixin.exe'\" | Select-Object ProcessId, CommandLine | ConvertTo-Json -Compress";
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", script]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    pick_main_pid(&serde_json::from_str::<Value>(text).ok()?)
}

/// The whitelist as the DLL expects it: UTF-8 names separated by NUL, terminated by two NULs.
pub fn whitelist_buffer(whitelist: &[String]) -> Option<Vec<u8>> {
    if whitelist.is_empty() {
        return None;
    }
    let mut buf = whitelist.join("\0").into_bytes();
    buf.extend_from_slice(&[0, 0]);
    Some(buf)
}

impl ImageAutoDownload {
    pub fn new(runtime_dir: &Path) -> Self {
        Self { inner: Arc::new(Mutex::new(Inner::default())), runtime_dir: runtime_dir.to_path_buf() }
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
        unsafe {
            let Ok(lib) = Library::new(&path) else { return false };
            let (Ok(init), Ok(uninstall), Ok(error)) = (lib.get::<InitFn>(b"InitImgHelper\0").map(|s| *s), lib.get::<UninstallFn>(b"UninstallImgHelper\0").map(|s| *s), lib.get::<ErrorFn>(b"GetImgHelperError\0").map(|s| *s)) else { return false };
            inner.helper = Some(Helper { _lib: lib, init, uninstall, error });
        }
        true
    }

    fn unhook_locked(inner: &mut Inner) {
        if inner.hooked {
            if let Some(h) = &inner.helper {
                unsafe { (h.uninstall)() };
            }
        }
        inner.hooked = false;
        inner.pid = None;
    }

    fn check_and_hook(&self, manual_start: bool) -> Value {
        let pid = find_main_wechat_pid();
        let mut inner = self.inner.lock().unwrap();
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
        let Some(helper) = &inner.helper else { return json!({ "success": false, "error": "core component initialization failed" }) };
        let ok = unsafe { (helper.init)(pid, whitelist.as_ref().map_or(std::ptr::null(), |b| b.as_ptr() as *const c_char)) };
        if ok {
            inner.hooked = true;
            inner.pid = Some(pid);
            return json!({ "success": true });
        }
        let msg = unsafe {
            let p = (helper.error)();
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().to_string() }
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
            let mut inner = self.inner.lock().unwrap();
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
        let mut inner = self.inner.lock().unwrap();
        if let Some(stop) = inner.poll_stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        Self::unhook_locked(&mut inner);
        json!({ "success": true })
    }

    /// `image:getAutoDownloadStatus`
    pub fn status(&self) -> Value {
        let inner = self.inner.lock().unwrap();
        json!({ "isHooked": inner.hooked, "pid": inner.pid, "supported": supported() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_process_is_the_shortest_weixin_command_line() {
        let procs = json!([
            { "ProcessId": 11, "CommandLine": "\"C:\\WeChat\\Weixin.exe\" --type=renderer --lang=en-US" },
            { "ProcessId": 7, "CommandLine": "\"C:\\WeChat\\Weixin.exe\"" },
            { "ProcessId": 9, "CommandLine": null }
        ]);
        assert_eq!(pick_main_pid(&procs), Some(7));
        assert_eq!(pick_main_pid(&json!({ "ProcessId": 3, "CommandLine": "weixin.exe" })), Some(3), "a single process is an object, not an array");
        assert_eq!(pick_main_pid(&json!([])), None);
        assert_eq!(pick_main_pid(&json!([{ "ProcessId": 1, "CommandLine": "other.exe" }])), None);
    }

    #[test]
    fn whitelist_is_nul_separated_and_double_nul_terminated() {
        assert_eq!(whitelist_buffer(&[]), None);
        assert_eq!(whitelist_buffer(&["a".into(), "b".into()]), Some(b"a\0b\0\0".to_vec()));
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
