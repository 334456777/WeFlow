//! Small helpers the services share: JavaScript-style number parsing, file names that are safe on every platform,
//! and which media a message export copies.

use crate::message::rx;

/// JS `parseInt(value, 10)`, NaN → `None`.
pub fn js_parse_int(s: &str) -> Option<i64> {
    rx(r"^\s*[+-]?\d+")
        .find(s)
        .and_then(|m| m.as_str().trim().parse().ok())
}

/// The media a message export copies next to the messages.
#[derive(Clone, Copy, Debug, Default)]
pub struct ApiMediaOptions {
    pub enabled: bool,
    pub images: bool,
    pub voices: bool,
    pub videos: bool,
    pub emojis: bool,
}

/// `value` with the characters no file name may hold replaced by `_` and trailing dots dropped; `fallback` when
/// nothing is left.
pub fn sanitize_file_name(value: &str, fallback: &str) -> String {
    let replaced: String = value
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
                || (c as u32) < 0x20
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    let safe = replaced.trim_end_matches('.').to_string();
    if safe.is_empty() {
        fallback.to_string()
    } else {
        safe
    }
}

/// An unsigned id in decimal without leading zeros (a float is cut to its whole part); empty when it is none.
pub fn normalize_unsigned_token(value: &str) -> String {
    let text = value.trim();
    if text.is_empty() {
        return String::new();
    }
    if text.chars().all(|c| c.is_ascii_digit()) {
        let stripped = text.trim_start_matches('0');
        return if stripped.is_empty() {
            "0".into()
        } else {
            stripped.to_string()
        };
    }
    match text.parse::<f64>() {
        Ok(f) if f.is_finite() && f > 0.0 => format!("{}", crate::num::saturating_i64(f.floor())),
        _ => String::new(),
    }
}

/// A media file a message export copied.
#[derive(Clone, Debug)]
pub struct ApiExportedMedia {
    pub kind: &'static str,
    pub file_name: String,
    pub full_path: String,
    pub relative_path: String,
    /// image only: the exported file is the thumbnail (the original is not on disk)
    pub thumbnail: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_names() {
        assert_eq!(js_parse_int(" 42abc"), Some(42));
        assert_eq!(js_parse_int("-7"), Some(-7));
        assert_eq!(js_parse_int("abc"), None);
        assert_eq!(sanitize_file_name(" a/b:c. ", "x"), "a_b_c");
        assert_eq!(sanitize_file_name("...", "x"), "x");
        assert_eq!(normalize_unsigned_token("000123"), "123");
        assert_eq!(normalize_unsigned_token("000"), "0");
        assert_eq!(normalize_unsigned_token("12.9"), "12");
        assert_eq!(normalize_unsigned_token("abc"), "");
    }
}
