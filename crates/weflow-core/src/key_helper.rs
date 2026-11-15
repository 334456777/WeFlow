//! JSON printed by the platform key helpers.
//!
//! Linux `xkey_helper_linux` answers `db_scan` with a decimal `target_addr` and `db_hook` with a
//! 64-character hex `key`. macOS `xkey_helper` answers with that same `key` object. A failure is
//! `success: false` and a `result` or `error` string, including when the process exits 0.

use serde_json::Value;

/// One JSON object from a key helper's stdout.
#[derive(Debug, PartialEq, Eq)]
pub enum HelperReply {
    /// 32-byte database key, lowercase hex.
    Key(String),
    /// Virtual address `db_scan` printed, decimal.
    Address(u64),
    /// `success` was false. The string is the helper's own `result` or `error`.
    Failed(String),
}

/// Parse one helper reply. `Err` means stdout was not that JSON, not that the helper missed the key.
pub fn parse_helper_reply(stdout: &str) -> Result<HelperReply, String> {
    let text = stdout.trim();
    if text.is_empty() {
        return Err("key helper printed nothing".into());
    }
    let value: Value = serde_json::from_str(text)
        .map_err(|_| format!("key helper did not return JSON: {text}"))?;
    if value.get("success") != Some(&Value::Bool(true)) {
        let detail = value
            .get("result")
            .or_else(|| value.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("key helper failed");
        return Ok(HelperReply::Failed(detail.to_string()));
    }
    if let Some(key) = value.get("key").and_then(Value::as_str) {
        return hex_key(key).map(HelperReply::Key);
    }
    if let Some(address) = json_u64(&value, "target_addr") {
        return Ok(HelperReply::Address(address));
    }
    Err("key helper succeeded without a key or target_addr".into())
}

/// The ciphertext `image_scan_helper` accepts: 16 bytes written as 32 hex characters.
pub fn require_ciphertext(text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("image ciphertext must be 32 hex characters".into())
    }
}

fn hex_key(key: &str) -> Result<String, String> {
    if key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(key.to_ascii_lowercase())
    } else {
        Err("key helper returned a key that is not 64 hex characters".into())
    }
}

fn json_u64(value: &Value, key: &str) -> Option<u64> {
    match value.get(key)? {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.as_u64(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_scan_address_is_decimal_text() {
        let reply = parse_helper_reply("{\"success\":true,\"target_addr\":\"12345\"}\n").unwrap();
        assert_eq!(reply, HelperReply::Address(12345));
    }

    #[test]
    fn db_hook_key_is_lowercased() {
        let key = "ab".repeat(32);
        let reply = parse_helper_reply(&format!(
            "{{\"success\":true,\"key\":\"{}\"}}\n",
            key.to_uppercase()
        ))
        .unwrap();
        assert_eq!(reply, HelperReply::Key(key));
    }

    #[test]
    fn failure_keeps_the_helper_text() {
        let reply =
            parse_helper_reply("{\"success\":false,\"result\":\"ERROR:UNKNOWN_MODE\"}\n").unwrap();
        assert_eq!(reply, HelperReply::Failed("ERROR:UNKNOWN_MODE".into()));
    }

    #[test]
    fn image_scan_error_field() {
        let reply =
            parse_helper_reply("{\"success\":false,\"error\":\"invalid arguments\"}\n").unwrap();
        assert_eq!(reply, HelperReply::Failed("invalid arguments".into()));
    }

    #[test]
    fn ciphertext_is_32_hex_characters() {
        assert!(require_ciphertext(&"ab".repeat(16)).is_ok());
        assert!(require_ciphertext("abc").is_err());
        assert!(require_ciphertext(&"zz".repeat(16)).is_err());
    }
}
