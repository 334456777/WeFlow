use std::ffi::CStr;
use std::os::raw::{c_char, c_int};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use libloading::Library;

type InitializeHookFn = unsafe extern "C" fn(u32) -> c_int;
type PollKeyDataFn = unsafe extern "C" fn(*mut c_char, c_int) -> c_int;
type CleanupHookFn = unsafe extern "C" fn() -> c_int;
type GetLastErrorMsgFn = unsafe extern "C" fn() -> *const c_char;
type GetImageKeyFn = unsafe extern "C" fn(*mut c_char, c_int) -> c_int;

pub struct WxKey {
    _lib: Option<Library>,
    initialize_hook: Option<InitializeHookFn>,
    poll_key_data: Option<PollKeyDataFn>,
    cleanup_hook: Option<CleanupHookFn>,
    get_last_error_msg: Option<GetLastErrorMsgFn>,
    get_image_key: Option<GetImageKeyFn>,
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
                    get_last_error_msg: load_symbol::<GetLastErrorMsgFn>(&lib, b"GetLastErrorMsg\0"),
                    get_image_key: load_symbol::<GetImageKeyFn>(&lib, b"GetImageKey\0"),
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
            }),
        }
    }

    pub fn is_available(&self) -> bool {
        self._lib.is_some()
    }

    pub fn get_db_key(&self, pid: u32) -> Result<String> {
        let init = self.initialize_hook.ok_or_else(|| anyhow!("wx_key library not loaded"))?;
        let poll = self.poll_key_data.ok_or_else(|| anyhow!("wx_key library not loaded"))?;
        let cleanup = self.cleanup_hook.ok_or_else(|| anyhow!("wx_key library not loaded"))?;

        let rc = unsafe { init(pid) };
        if rc != 0 {
            if let Some(get_err) = self.get_last_error_msg {
                let msg = unsafe { take_cstr(get_err()) };
                return Err(anyhow!("InitializeHook failed: {msg}"));
            }
            return Err(anyhow!("InitializeHook failed with code {rc}"));
        }

        let mut buffer = vec![0u8; 256];
        let rc = unsafe { poll(buffer.as_mut_ptr() as *mut c_char, buffer.len() as c_int) };
        let key = if rc != 0 {
            let len = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
            String::from_utf8_lossy(&buffer[..len]).to_string()
        } else {
            String::new()
        };

        unsafe { cleanup() };
        if key.is_empty() {
            Err(anyhow!("failed to extract db key"))
        } else {
            Ok(key)
        }
    }

    pub fn get_image_key(&self) -> Result<String> {
        let get_image_key = self.get_image_key.ok_or_else(|| anyhow!("wx_key library not loaded"))?;
        let mut buffer = vec![0u8; 512];
        let rc = unsafe { get_image_key(buffer.as_mut_ptr() as *mut c_char, buffer.len() as c_int) };
        if rc != 0 {
            let len = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
            Ok(String::from_utf8_lossy(&buffer[..len]).to_string())
        } else {
            Err(anyhow!("failed to extract image key"))
        }
    }
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
        let candidates = [
            runtime_dir.join("key/macos/universal/libwx_key.dylib"),
        ];
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
        candidates.into_iter().find(|p| p.exists()).ok_or_else(|| anyhow!("xkey_helper not found"))
    } else if cfg!(target_os = "linux") {
        let candidates = [
            runtime_dir.join("key/linux/x64/xkey_helper_linux"),
        ];
        candidates.into_iter().find(|p| p.exists()).ok_or_else(|| anyhow!("xkey_helper_linux not found"))
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
        candidates.into_iter().find(|p| p.exists()).ok_or_else(|| anyhow!("image_scan_helper not found"))
    } else {
        Err(anyhow!("image scan helper is not available on this platform"))
    }
}
