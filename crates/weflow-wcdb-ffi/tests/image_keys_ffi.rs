use std::ffi::{c_void, CStr, CString};
use std::path::Path;

use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
use weflow_wcdb::{wcdb_free_string, weflow_get_image_keys, STATUS_BAD_ARGUMENT, STATUS_OK};

fn sample(account: &Path, code: u32, identity: &str, xor_len: usize) {
    let mut plain = Vec::new();
    let pixels: Vec<u8> = (0..32 * 32 * 3).map(|n| (n * 17 % 256) as u8).collect();
    jpeg_encoder::Encoder::new(&mut plain, 90)
        .encode(&pixels, 32, 32, jpeg_encoder::ColorType::Rgb)
        .unwrap();
    let (xor, aes) = weflow_core::keys::derive_image_keys(code.into(), identity);
    let cipher = aes::Aes128::new(GenericArray::from_slice(aes.as_bytes()));
    let mut encrypted = vec![7, 8, 0x56, 0x32, 8, 7];
    encrypted.extend_from_slice(&16i32.to_le_bytes());
    encrypted.extend_from_slice(&(xor_len as i32).to_le_bytes());
    encrypted.push(1);
    let mut first = GenericArray::clone_from_slice(&plain[..16]);
    cipher.encrypt_block(&mut first);
    encrypted.extend_from_slice(&first);
    let mut padding = GenericArray::clone_from_slice(&[16; 16]);
    cipher.encrypt_block(&mut padding);
    encrypted.extend_from_slice(&padding);
    encrypted.extend_from_slice(&plain[16..plain.len() - xor_len]);
    encrypted.extend(plain[plain.len() - xor_len..].iter().map(|b| b ^ xor));
    std::fs::create_dir_all(account).unwrap();
    std::fs::write(account.join("synthetic_t.dat"), encrypted).unwrap();
}

fn call(account: &Path, identity: &str, dirs: &str) -> (i32, String) {
    let account = CString::new(account.to_str().unwrap()).unwrap();
    let identity = CString::new(identity).unwrap();
    let dirs = CString::new(dirs).unwrap();
    let mut result: *mut c_void = std::ptr::null_mut();
    unsafe {
        let code = weflow_get_image_keys(
            account.as_ptr(),
            identity.as_ptr(),
            dirs.as_ptr(),
            &mut result,
        );
        assert!(!result.is_null());
        let text = CStr::from_ptr(result.cast()).to_str().unwrap().to_string();
        wcdb_free_string(result);
        (code, text)
    }
}

#[test]
fn ffi_acquires_only_selected_account_and_reports_verification_strength() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../temp")
        .join(format!("image-key-ffi-{}", std::process::id()));
    let account = root.join("wxid_me_ab12");
    let codes = root.join("kvcomm");
    std::fs::create_dir_all(&codes).unwrap();
    std::fs::write(codes.join("key_123_456.statistic"), []).unwrap();
    let dirs = serde_json::to_string(&[&codes]).unwrap();
    sample(&account, 123, "wxid_me", 64);
    sample(&root.join("wxid_other_ab12"), 456, "wxid_other", 64);
    let (status, text) = call(&account, "wxid_me", &dirs);
    assert_eq!(status, STATUS_OK);
    let output: serde_json::Value = serde_json::from_str(&text).unwrap();
    let (xor, aes) = weflow_core::keys::derive_image_keys(123, "wxid_me");
    assert_eq!(output["image_xor_key"], xor);
    assert_eq!(output["image_aes_key"], aes);
    assert_eq!(output["verified"], true);
    assert_eq!(output["xor_verified"], true);
    let (status, _) = call(&root.join("missing-account"), "wxid_me", &dirs);
    assert_ne!(status, STATUS_OK);
    sample(&account, 123, "wxid_other", 64);
    let (status, _) = call(&account, "wxid_me", &dirs);
    assert_ne!(status, STATUS_OK, "must not try a sibling identity");
    sample(&account, 123, "wxid_me", 0);
    let (status, text) = call(&account, "wxid_me", &dirs);
    assert_eq!(status, STATUS_OK);
    let output: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(output["aes_verified"], true);
    assert_eq!(output["verified"], false);
    assert_eq!(output["xor_verified"], false);
    assert_ne!(call(&account, "wxid_me", "not JSON").0, STATUS_OK);
    assert_ne!(call(Path::new(""), "", &dirs).0, STATUS_OK);
    assert_eq!(
        unsafe {
            weflow_get_image_keys(
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        },
        STATUS_BAD_ARGUMENT
    );
    std::fs::remove_dir_all(root).unwrap();
}
