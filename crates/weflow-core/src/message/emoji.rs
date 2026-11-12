//! Stickers: their URL, MD5 and caption, and the percent-decoding of their addresses.

use super::xml::{extract_xml_value, MsgXml};

pub(super) fn normalize_md5(value: &str) -> Option<String> {
    let md5 = value.trim().to_lowercase();
    if md5.len() == 32 && md5.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(md5)
    } else {
        None
    }
}

pub(super) fn extract_loose_hex_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    // `(?i)(?:emoji|sticker|md5)[^a-fA-F0-9]{0,32}([a-fA-F0-9]{32})`: the leftmost keyword that has one
    let keyed = ["emoji", "sticker", "md5"]
        .into_iter()
        .filter_map(|key| {
            crate::xml::find_all_ci(content, 0, key)
                .find_map(|p| hex32_after_gap(content, p + key.len()).map(|hex| (p, hex)))
        })
        .min_by_key(|(p, _)| *p);
    if let Some((_, hex)) = keyed {
        return normalize_md5(hex);
    }
    first_hex32(content).and_then(normalize_md5)
}

/// Whether `n` hex digits start at `i`.
fn hex_at(h: &[u8], i: usize, n: usize) -> bool {
    h.get(i..i + n)
        .is_some_and(|w| w.iter().all(u8::is_ascii_hexdigit))
}

/// `[^a-fA-F0-9]{0,32}([a-fA-F0-9]{32})` at `i`.
fn hex32_after_gap(s: &str, i: usize) -> Option<&str> {
    let (_, (offset, _)) = s[i..]
        .char_indices()
        .enumerate()
        .take(33)
        .find(|(_, (_, c))| c.is_ascii_hexdigit())?;
    let start = i + offset;
    hex_at(s.as_bytes(), start, 32).then(|| &s[start..start + 32])
}

/// `([a-fA-F0-9]{32})`: the first 32 hex digits in a row.
fn first_hex32(s: &str) -> Option<&str> {
    let mut run = 0;
    for (i, b) in s.bytes().enumerate() {
        run = if b.is_ascii_hexdigit() { run + 1 } else { 0 };
        if run == 32 {
            return Some(&s[i + 1 - 32..=i]);
        }
    }
    None
}

pub fn extract_emoji_url(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let h = content.as_bytes();
    // `(?i)cdnurl\s*=\s*['"]([^'"]+)['"]`
    let quoted = crate::xml::find_all_ci(content, 0, "cdnurl").find_map(|p| {
        let quote = crate::xml::assignment_end(content, p + "cdnurl".len())?;
        if !matches!(h.get(quote), Some(b'\'' | b'"')) {
            return None;
        }
        let start = quote + 1;
        let end = start + h[start..].iter().position(|b| matches!(b, b'\'' | b'"'))?;
        (end > start).then(|| &content[start..end])
    });
    if let Some(url) = quoted {
        return Some(percent_decode(&url.replace("&amp;", "&")));
    }
    // `(?i)cdnurl[^>]*>([^<]+)`
    crate::xml::find_all_ci(content, 0, "cdnurl").find_map(|p| {
        let start = p + 1 + memchr::memchr(b'>', &h[p..])?;
        let end = memchr::memchr(b'<', &h[start..]).map_or(h.len(), |r| start + r);
        (end > start).then(|| content[start..end].to_string())
    })
}

pub fn extract_emoji_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let h = content.as_bytes();
    let assigned = || {
        crate::xml::find_all_ci(content, 0, "md5")
            .filter_map(|p| crate::xml::assignment_end(content, p + "md5".len()))
    };
    // `(?i)md5\s*=\s*['"]([a-fA-F0-9]{32})['"]`
    let found = assigned()
        .find(|&q| {
            matches!(h.get(q), Some(b'\'' | b'"'))
                && hex_at(h, q + 1, 32)
                && matches!(h.get(q + 33), Some(b'\'' | b'"'))
        })
        .map(|q| &content[q + 1..q + 33])
        // `(?i)md5\s*=\s*([a-fA-F0-9]{32})`
        .or_else(|| {
            assigned()
                .find(|&q| hex_at(h, q, 32))
                .map(|q| &content[q..q + 32])
        })
        // `(?i)<md5>([a-fA-F0-9]{32})</md5>`
        .or_else(|| {
            crate::xml::find_all_ci(content, 0, "<md5>")
                .map(|p| p + "<md5>".len())
                .find(|&q| hex_at(h, q, 32) && crate::xml::is_close_at(h, q + 32, b"md5"))
                .map(|q| &content[q..q + 32])
        });
    found
        .and_then(normalize_md5)
        .or_else(|| extract_loose_hex_md5(content))
}

/// `s` with its `%XX` escapes decoded; `s` itself when they do not spell UTF-8.
pub(crate) fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    String::from_utf8(percent_decode_bytes(s)).unwrap_or_else(|_| s.to_string())
}

/// `%XX` escapes (two hex digits) turned into bytes; anything else is kept as it is.
pub fn percent_decode_bytes(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

pub fn format_emoji_semantic_text(caption: Option<&str>) -> String {
    match caption.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => format!("[表情：{c}]"),
        None => "[表情]".into(),
    }
}

/// The md5 (`<emoticonmd5>`) of a sticker sent as an app message of type 8, which looks up the sticker's caption.
pub(super) fn appmsg_emoticon_md5_in(x: &MsgXml) -> Option<String> {
    // the tag is matched ASCII case-insensitively; see normalized_may_contain for why `content` can rule it out
    crate::xml::find_ci(x.content(), 0, "emoticonmd5")?;
    if x.app_type_twice() != "8" {
        return None;
    }
    let md5 = extract_xml_value(x.once(), "emoticonmd5").to_ascii_lowercase();
    (md5.len() == 32 && md5.chars().all(|c| c.is_ascii_hexdigit())).then_some(md5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_text_carries_the_caption() {
        assert_eq!(format_emoji_semantic_text(None), "[表情]");
        assert_eq!(format_emoji_semantic_text(Some("  ")), "[表情]");
        assert_eq!(format_emoji_semantic_text(Some("打call")), "[表情：打call]");
        let xml = "<msg><appmsg><title /><type>8</type><appattach><emoticonmd5>35A036EDED53A8D41C043215C66FFDA3</emoticonmd5></appattach></appmsg></msg>";
        assert_eq!(
            appmsg_emoticon_md5_in(&MsgXml::new(xml)).as_deref(),
            Some("35a036eded53a8d41c043215c66ffda3")
        );
        assert_eq!(
            appmsg_emoticon_md5_in(&MsgXml::new("<msg><appmsg><type>5</type></appmsg></msg>")),
            None
        );
    }

    #[test]
    fn percent_decoding_never_splits_characters() {
        assert_eq!(percent_decode_bytes("%E4%B8%AD%2F"), "中/".as_bytes());
        assert_eq!(
            percent_decode_bytes("%a中"),
            "%a中".as_bytes(),
            "a multi-byte character after % is not a hex digit"
        );
        assert_eq!(percent_decode_bytes("%中x"), "%中x".as_bytes());
        assert_eq!(
            percent_decode_bytes("%+5"),
            b"%+5",
            "a sign is not a hex digit"
        );
        assert_eq!(percent_decode_bytes("a%4"), b"a%4");
        assert_eq!(
            percent_decode("%ff"),
            "%ff",
            "invalid UTF-8 keeps the input"
        );
    }
}
