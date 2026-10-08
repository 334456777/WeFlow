#![cfg(all(windows, target_arch = "x86_64"))]

use std::ffi::{c_char, c_int, CStr};

#[test]
fn built_dll_exports_the_cpp_bool_abi() {
    let path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("wx_key.dll");
    unsafe {
        let dll = libloading::Library::new(path).unwrap();
        let init = dll
            .get::<unsafe extern "C" fn(u32) -> bool>(b"InitializeHook\0")
            .unwrap();
        let cleanup = dll
            .get::<unsafe extern "C" fn() -> bool>(b"CleanupHook\0")
            .unwrap();
        let poll = dll
            .get::<unsafe extern "C" fn(*mut c_char, c_int) -> bool>(b"PollKeyData\0")
            .unwrap();
        let status = dll
            .get::<unsafe extern "C" fn(*mut c_char, c_int, *mut c_int) -> bool>(
                b"GetStatusMessage\0",
            )
            .unwrap();
        let error = dll
            .get::<unsafe extern "C" fn() -> *const c_char>(b"GetLastErrorMsg\0")
            .unwrap();
        let image = dll
            .get::<unsafe extern "C" fn(*mut c_char, c_int) -> bool>(b"GetImageKey\0")
            .unwrap();
        assert!(!init(0));
        assert!(!CStr::from_ptr(error()).to_bytes().is_empty());
        assert!(!poll(std::ptr::null_mut(), 65));
        assert!(!image(std::ptr::null_mut(), 8192));
        let mut buffer = [0u8; 512];
        let mut level = 0;
        assert!(status(buffer.as_mut_ptr().cast(), 512, &mut level));
        assert_eq!(level, 2);
        assert!(cleanup());
        assert!(cleanup());
    }
}
