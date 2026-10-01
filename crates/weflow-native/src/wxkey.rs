use std::ffi::CStr;
use std::os::raw::{c_char, c_int};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use libloading::Library;

// The DLL exports C++ `bool`s (see `keyService.ts`: `bool InitializeHook(uint32)` …), not `int`s.
type InitializeHookFn = unsafe extern "C" fn(u32) -> bool;
type PollKeyDataFn = unsafe extern "C" fn(*mut c_char, c_int) -> bool;
type CleanupHookFn = unsafe extern "C" fn() -> bool;
type GetLastErrorMsgFn = unsafe extern "C" fn() -> *const c_char;
type GetImageKeyFn = unsafe extern "C" fn(*mut c_char, c_int) -> bool;
type GetStatusMessageFn = unsafe extern "C" fn(*mut c_char, c_int, *mut c_int) -> bool;

pub struct WxKey {
    _lib: Option<Library>,
    initialize_hook: Option<InitializeHookFn>,
    poll_key_data: Option<PollKeyDataFn>,
    cleanup_hook: Option<CleanupHookFn>,
    get_last_error_msg: Option<GetLastErrorMsgFn>,
    get_image_key: Option<GetImageKeyFn>,
    get_status_message: Option<GetStatusMessageFn>,
}

impl WxKey {
    pub fn load(runtime_dir: &Path) -> Result<Self> {
        let lib_path = find_wx_key_library(runtime_dir);
        match lib_path {
            Some(path) => {
                let lib = unsafe { Library::new(&path) }
                    .with_context(|| format!("failed to load {}", path.display()))?;
                Ok(Self {
                    initialize_hook: load_symbol::<InitializeHookFn>(&lib, b"InitializeHook\0"),
                    poll_key_data: load_symbol::<PollKeyDataFn>(&lib, b"PollKeyData\0"),
                    cleanup_hook: load_symbol::<CleanupHookFn>(&lib, b"CleanupHook\0"),
                    get_last_error_msg: load_symbol::<GetLastErrorMsgFn>(
                        &lib,
                        b"GetLastErrorMsg\0",
                    ),
                    get_image_key: load_symbol::<GetImageKeyFn>(&lib, b"GetImageKey\0"),
                    get_status_message: load_symbol::<GetStatusMessageFn>(
                        &lib,
                        b"GetStatusMessage\0",
                    ),
                    _lib: Some(lib),
                })
            }
            None => Ok(Self {
                _lib: None,
                initialize_hook: None,
                poll_key_data: None,
                cleanup_hook: None,
                get_last_error_msg: None,
                get_image_key: None,
                get_status_message: None,
            }),
        }
    }

    pub fn is_available(&self) -> bool {
        self._lib.is_some()
    }

    /// Hooks WeChat (`pid`) and waits up to `timeout` for the 64-hex-digit database key, which
    /// WeChat only produces while it opens its databases (i.e. while the user logs in).
    /// `on_status` receives the DLL's status messages (`level`: 0 info, 1 success, 2 error).
    pub fn get_db_key(
        &self,
        pid: u32,
        timeout: std::time::Duration,
        on_status: &mut dyn FnMut(&str, i32),
    ) -> std::result::Result<String, DbKeyError> {
        self.get_db_key_with_tick(pid, timeout, on_status, &mut |_| {})
    }

    /// Like [`get_db_key`](Self::get_db_key), and calls `on_tick` with the remaining seconds about once per second.
    pub fn get_db_key_with_tick(
        &self,
        pid: u32,
        timeout: std::time::Duration,
        on_status: &mut dyn FnMut(&str, i32),
        on_tick: &mut dyn FnMut(u64),
    ) -> std::result::Result<String, DbKeyError> {
        let (Some(init), Some(poll), Some(cleanup)) =
            (self.initialize_hook, self.poll_key_data, self.cleanup_hook)
        else {
            return Err(DbKeyError::Other("wx_key library not loaded".into()));
        };
        let ok = unsafe { init(pid) };
        if !ok {
            let error = self
                .get_last_error_msg
                .map(|f| unsafe { take_cstr(f()) })
                .unwrap_or_default();
            if !error.is_empty() {
                if error.contains("0xC0000022")
                    || error.contains("ACCESS_DENIED")
                    || error.contains("打开目标进程失败")
                {
                    return Err(DbKeyError::AccessDenied(error));
                }
                return Err(DbKeyError::Other(error));
            }
            let status = self.status_message().map(|(m, _)| m).unwrap_or_default();
            return Err(DbKeyError::Other(if status.is_empty() {
                "initialization failed".into()
            } else {
                status
            }));
        }
        let start = std::time::Instant::now();
        let mut login_hint = false;
        let mut buf = vec![0u8; 128];
        let mut result = None;
        let mut last_tick = u64::MAX;
        while start.elapsed() < timeout {
            let left = timeout.saturating_sub(start.elapsed()).as_secs();
            if left != last_tick {
                last_tick = left;
                on_tick(left);
            }
            buf.iter_mut().for_each(|b| *b = 0);
            if unsafe { poll(buf.as_mut_ptr() as *mut c_char, buf.len() as c_int) } {
                let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                let key = String::from_utf8_lossy(&buf[..len]).trim().to_string();
                if key.len() == 64 {
                    on_status("key obtained", 1);
                    result = Some(key);
                    break;
                }
            }
            for _ in 0..5 {
                let Some((msg, level)) = self.status_message() else {
                    break;
                };
                if !msg.is_empty() {
                    if is_login_related(&msg) {
                        login_hint = true;
                    }
                    on_status(&msg, level);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        unsafe { cleanup() };
        match result {
            Some(k) => Ok(k),
            None if login_hint => Err(DbKeyError::LoginRequired),
            None => Err(DbKeyError::Timeout),
        }
    }

    fn status_message(&self) -> Option<(String, i32)> {
        let f = self.get_status_message?;
        let mut buf = vec![0u8; 256];
        let mut level: c_int = 0;
        if !unsafe {
            f(
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as c_int,
                &mut level,
            )
        } {
            return None;
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Some((
            String::from_utf8_lossy(&buf[..len]).trim().to_string(),
            level,
        ))
    }

    /// Raw JSON of the `kvcomm` cache scan: `{"accounts":[{"wxid":…,"keys":[{"code":…}]}]}`.
    pub fn get_image_key(&self) -> Result<String> {
        let get_image_key = self
            .get_image_key
            .ok_or_else(|| anyhow!("wx_key library not loaded"))?;
        let mut buffer = vec![0u8; 8192];
        let ok =
            unsafe { get_image_key(buffer.as_mut_ptr() as *mut c_char, buffer.len() as c_int) };
        if ok {
            let len = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
            Ok(String::from_utf8_lossy(&buffer[..len]).to_string())
        } else {
            let msg = self
                .get_last_error_msg
                .map(|f| unsafe { take_cstr(f()) })
                .unwrap_or_default();
            Err(anyhow!(if msg.is_empty() {
                "failed to read the image key cache".to_string()
            } else {
                msg
            }))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DbKeyError {
    /// The process could not be opened (needs administrator rights / a security product interferes).
    AccessDenied(String),
    /// WeChat is running but has not logged in, so it never opened its databases.
    LoginRequired,
    Timeout,
    Other(String),
}

impl std::fmt::Display for DbKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AccessDenied(d) => write!(f, "access denied: cannot open the WeChat process ({d})"),
            Self::LoginRequired => write!(f, "WeChat is running but not logged in; log in while the command is waiting"),
            Self::Timeout => write!(f, "timed out waiting for the key; log in to WeChat (or restart it) while the command is running"),
            Self::Other(m) => write!(f, "{m}"),
        }
    }
}

/// `isLoginRelatedText`
pub fn is_login_related(value: &str) -> bool {
    let n: String = value.split_whitespace().collect::<String>().to_lowercase();
    !n.is_empty()
        && [
            "登录",
            "扫码",
            "二维码",
            "请在手机上确认",
            "手机确认",
            "切换账号",
            "wechatlogin",
            "qrcode",
            "scan",
        ]
        .iter()
        .any(|k| n.contains(k))
}

unsafe fn take_cstr(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    CStr::from_ptr(ptr).to_string_lossy().to_string()
}

unsafe fn symbol<T: Copy>(lib: &Library, name: &[u8]) -> Option<T> {
    lib.get::<T>(name).ok().map(|sym| *sym)
}

fn load_symbol<T: Copy>(lib: &Library, name: &[u8]) -> Option<T> {
    unsafe { symbol::<T>(lib, name) }
}

fn find_wx_key_library(runtime_dir: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        let candidates = [
            runtime_dir.join("key/win32/x64/wx_key.dll"),
            runtime_dir.join("key/win32/arm64/wx_key.dll"),
        ];
        candidates.into_iter().find(|p| p.exists())
    } else if cfg!(target_os = "macos") {
        let candidates = [runtime_dir.join("key/macos/universal/libwx_key.dylib")];
        candidates.into_iter().find(|p| p.exists())
    } else {
        None
    }
}

pub fn run_key_helper(runtime_dir: &Path, args: &[&str]) -> Result<String> {
    let helper = find_key_helper(runtime_dir)?;
    let output = Command::new(&helper)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {}", helper.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("{} failed: {stderr}", helper.display()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn run_image_scan_helper(runtime_dir: &Path, args: &[&str]) -> Result<String> {
    let helper = find_image_scan_helper(runtime_dir)?;
    let output = Command::new(&helper)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {}", helper.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("{} failed: {stderr}", helper.display()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn find_key_helper(runtime_dir: &Path) -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        let candidates = [
            runtime_dir.join("key/macos/universal/xkey_helper"),
            runtime_dir.join("key/macos/xkey_helper"),
        ];
        candidates
            .into_iter()
            .find(|p| p.exists())
            .ok_or_else(|| anyhow!("xkey_helper not found"))
    } else if cfg!(target_os = "linux") {
        let candidates = [runtime_dir.join("key/linux/x64/xkey_helper_linux")];
        candidates
            .into_iter()
            .find(|p| p.exists())
            .ok_or_else(|| anyhow!("xkey_helper_linux not found"))
    } else {
        Err(anyhow!("key helper is not available on this platform"))
    }
}

fn find_image_scan_helper(runtime_dir: &Path) -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        let candidates = [
            runtime_dir.join("key/macos/universal/image_scan_helper"),
            runtime_dir.join("key/macos/image_scan_helper"),
        ];
        candidates
            .into_iter()
            .find(|p| p.exists())
            .ok_or_else(|| anyhow!("image_scan_helper not found"))
    } else {
        Err(anyhow!(
            "image scan helper is not available on this platform"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_keywords() {
        assert!(is_login_related("请扫码登录"));
        assert!(is_login_related("Please scan the QR code"));
        assert!(!is_login_related("hook installed"));
    }
}
