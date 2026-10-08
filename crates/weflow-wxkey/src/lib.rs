//! Compatibility with wx_key.dll's C++ bool ABI. The CLI calls the same Rust capture
//! code directly; only the desktop app loads this library. No legacy DLL is loaded.
#![allow(non_snake_case)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct State {
    #[cfg(all(windows, target_arch = "x86_64"))]
    hook: Option<weflow_native::windows_db_key::Hook>,
    statuses: VecDeque<(String, c_int)>,
    error: String,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

impl State {
    fn status(&mut self, text: String, level: c_int) {
        if self.statuses.len() == 100 {
            self.statuses.pop_front();
        }
        self.statuses.push_back((text, level));
    }

    fn fail(&mut self, error: String) -> bool {
        self.error.clone_from(&error);
        self.status(error, 2);
        false
    }
}

fn boundary(call: impl FnOnce(&mut State) -> bool) -> bool {
    let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
    catch_unwind(AssertUnwindSafe(|| call(&mut state)))
        .unwrap_or_else(|_| state.fail("Rust key helper panicked".into()))
}

/// Attach and arm the debugger before returning. `CleanupHook` must precede DLL unload.
#[no_mangle]
pub extern "C" fn InitializeHook(pid: u32) -> bool {
    boundary(|state| {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            if state.hook.is_some() {
                return state.fail("Hook is already initialized".into());
            }
            state.error.clear();
            state.statuses.clear();
            match weflow_native::windows_db_key::Hook::start(pid) {
                Ok(hook) => {
                    state.status(format!("WeChat version {}", hook.version), 0);
                    state.hook = Some(hook);
                    state.status("Hook installed; log in to WeChat now".into(), 1);
                    true
                }
                Err(error) => state.fail(error.to_string()),
            }
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        {
            let _ = pid;
            state.fail("Database-key capture requires Windows x64".into())
        }
    })
}

/// # Safety
/// `buffer` must point to at least `capacity` writable bytes when non-null.
#[no_mangle]
pub unsafe extern "C" fn PollKeyData(buffer: *mut c_char, capacity: c_int) -> bool {
    if buffer.is_null() || capacity < 65 {
        return false;
    }
    *buffer = 0;
    boundary(|state| {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            let Some(hook) = state.hook.as_mut() else {
                return false;
            };
            match hook.poll() {
                Ok(Some(key)) => write_text(buffer, capacity, &key),
                Ok(None) => false,
                Err(error) => state.fail(error.to_string()),
            }
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        {
            let _ = state;
            false
        }
    })
}

#[no_mangle]
pub extern "C" fn CleanupHook() -> bool {
    boundary(|state| {
        #[cfg(all(windows, target_arch = "x86_64"))]
        if let Some(mut hook) = state.hook.take() {
            if let Err(error) = hook.cleanup() {
                return state.fail(error.to_string());
            }
        }
        state.statuses.clear();
        true
    })
}

/// Returns a thread-local UTF-8 snapshot, valid until the next call on the same thread.
#[no_mangle]
pub extern "C" fn GetLastErrorMsg() -> *const c_char {
    thread_local! { static ERROR: RefCell<CString> = RefCell::new(CString::default()); }
    let text = state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .error
        .replace('\0', " ");
    ERROR.with(|error| {
        *error.borrow_mut() = CString::new(text).unwrap_or_default();
        error.borrow().as_ptr()
    })
}

/// # Safety
/// `buffer` must point to `capacity` writable bytes; `level` must be writable.
#[no_mangle]
pub unsafe extern "C" fn GetStatusMessage(
    buffer: *mut c_char,
    capacity: c_int,
    level: *mut c_int,
) -> bool {
    if buffer.is_null() || capacity <= 0 || level.is_null() {
        return false;
    }
    *buffer = 0;
    boundary(|state| {
        let Some((text, severity)) = state.statuses.front() else {
            return false;
        };
        if !write_text(buffer, capacity, text) {
            return false;
        }
        *level = *severity;
        state.statuses.pop_front();
        true
    })
}

/// Compatibility candidate JSON for older desktop callers. Uses #95's Rust parser;
/// candidate codes are not validated image keys. The selected account must verify them.
/// # Safety
/// `buffer` must point to at least `capacity` writable bytes when non-null.
#[no_mangle]
pub unsafe extern "C" fn GetImageKey(buffer: *mut c_char, capacity: c_int) -> bool {
    if buffer.is_null() || capacity <= 0 {
        return false;
    }
    *buffer = 0;
    boundary(|state| {
        let collected =
            weflow_core::image_keys::collect_codes(&weflow_core::image_keys::default_kvcomm_dirs());
        if collected.candidates.is_empty() {
            return state.fail("No image-key candidate codes in kvcomm".into());
        }
        let keys: Vec<_> = collected
            .candidates
            .iter()
            .map(|candidate| serde_json::json!({ "code": candidate.code }))
            .collect();
        let json = serde_json::json!({ "accounts": [{ "keys": keys }] }).to_string();
        if !write_text(buffer, capacity, &json) {
            return state.fail("Image-key result buffer is too small".into());
        }
        true
    })
}

unsafe fn write_text(buffer: *mut c_char, capacity: c_int, text: &str) -> bool {
    if buffer.is_null() || capacity <= 0 || text.len() >= capacity as usize {
        return false;
    }
    std::ptr::copy_nonoverlapping(text.as_ptr(), buffer.cast(), text.len());
    *buffer.add(text.len()) = 0;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_abi_and_buffer_validation() {
        assert_eq!(std::mem::size_of::<bool>(), 1);
        unsafe {
            assert!(!PollKeyData(std::ptr::null_mut(), 65));
            assert!(!GetStatusMessage(
                std::ptr::null_mut(),
                256,
                std::ptr::null_mut()
            ));
            assert!(!GetImageKey(std::ptr::null_mut(), 8192));
            let mut buffer = [0x55u8; 65];
            assert!(!PollKeyData(buffer.as_mut_ptr().cast(), 64));
            assert_eq!(buffer, [0x55u8; 65]);
            assert!(write_text(buffer.as_mut_ptr().cast(), 65, &"a".repeat(64)));
            assert_eq!(buffer[64], 0);
            assert!(!write_text(buffer.as_mut_ptr().cast(), 64, &"a".repeat(64)));
        }
    }

    #[test]
    fn failed_initialization_reports_error_and_cleanup_is_idempotent() {
        assert!(!InitializeHook(0));
        unsafe {
            let error = std::ffi::CStr::from_ptr(GetLastErrorMsg())
                .to_str()
                .unwrap();
            assert!(!error.is_empty());
            let mut buffer = [0u8; 512];
            let mut level = -1;
            assert!(GetStatusMessage(
                buffer.as_mut_ptr().cast(),
                512,
                &mut level
            ));
            assert_eq!(level, 2);
        }
        assert!(CleanupHook());
        assert!(CleanupHook());
    }
}
