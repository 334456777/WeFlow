//! 表情：地址、MD5 和描述，以及地址的百分号解码。

use super::xml::{extract_xml_value, MsgXml};

/// 去掉首尾空白、转成小写的 32 位十六进制 MD5；不是时为 `None`。
pub(super) fn normalize_md5(value: &str) -> Option<String> {
    let md5 = value.trim().to_lowercase();
    if md5.len() == 32 && md5.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(md5)
    } else {
        None
    }
}

/// 找 MD5 的最后退路：优先取 `emoji`、`sticker`、`md5` 这几个词后面不远处的 32 位十六进制（取最靠前的词），
/// 没有时取全文中第一段连续的 32 位十六进制。
pub(super) fn extract_loose_hex_md5(content: &str) -> Option<String> {
    // 下面两个模式都以连续的 32 位十六进制结尾：没有这样一段时两者都不匹配，用到这条退路的消息多数如此，
    // 扫一遍字节就能确定，不必逐个关键词搜索
    let first = first_hex32(content)?;
    // 相当于 `(?i)(?:emoji|sticker|md5)[^a-fA-F0-9]{0,32}([a-fA-F0-9]{32})`：取后面有 MD5 的最靠前的关键词
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
    normalize_md5(first)
}

/// `h` 从 `i` 开始是否有 `n` 位十六进制数字。
fn hex_at(h: &[u8], i: usize, n: usize) -> bool {
    h.get(i..i + n)
        .is_some_and(|w| w.iter().all(u8::is_ascii_hexdigit))
}

/// 从 `i` 开始匹配 `[^a-fA-F0-9]{0,32}([a-fA-F0-9]{32})`，返回其中的 32 位十六进制。
fn hex32_after_gap(s: &str, i: usize) -> Option<&str> {
    let (_, (offset, _)) = s[i..]
        .char_indices()
        .enumerate()
        .take(33)
        .find(|(_, (_, c))| c.is_ascii_hexdigit())?;
    let start = i + offset;
    hex_at(s.as_bytes(), start, 32).then(|| &s[start..start + 32])
}

/// `([a-fA-F0-9]{32})`：第一段连续的 32 位十六进制数字。
///
/// 连续 32 位的一段必定覆盖两个相邻的 16 的倍数位置，所以只看这些位置上的字节，直到相邻的两个都是十六进制数字，
/// 再量出它们所在的一段有多长。下一对位置从这一段之后开始看，所以第一个够长的一段就是文中的第一段。
fn first_hex32(s: &str) -> Option<&str> {
    const STEP: usize = 16;
    let h = s.as_bytes();
    let hex = |i: usize| h.get(i).is_some_and(u8::is_ascii_hexdigit);
    let mut at = 0;
    while at + STEP < h.len() {
        if !(hex(at) && hex(at + STEP)) {
            at += STEP;
            continue;
        }
        let start = (0..at).rev().take_while(|&i| hex(i)).last().unwrap_or(at);
        let end = (at..h.len()).find(|&i| !hex(i)).unwrap_or(h.len());
        if end - start >= 32 {
            return Some(&s[start..start + 32]);
        }
        // 下一对要看的位置在这一段之后
        at = end.next_multiple_of(STEP);
    }
    None
}

/// 逐个字节查找的 [`first_hex32`]，测试用它对照。
#[cfg(test)]
fn first_hex32_each(s: &str) -> Option<&str> {
    let mut run = 0;
    for (i, b) in s.bytes().enumerate() {
        run = if b.is_ascii_hexdigit() { run + 1 } else { 0 };
        if run == 32 {
            return Some(&s[i + 1 - 32..=i]);
        }
    }
    None
}

/// 表情消息的 `cdnurl`：带引号的属性值，解码 `&amp;` 和百分号转义；没有时取元素的文字；都没有时为 `None`。
pub fn extract_emoji_url(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let h = content.as_bytes();
    // 相当于 `(?i)cdnurl\s*=\s*['"]([^'"]+)['"]`
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
    // 相当于 `(?i)cdnurl[^>]*>([^<]+)`
    crate::xml::find_all_ci(content, 0, "cdnurl").find_map(|p| {
        let start = p + 1 + memchr::memchr(b'>', &h[p..])?;
        let end = memchr::memchr(b'<', &h[start..]).map_or(h.len(), |r| start + r);
        (end > start).then(|| content[start..end].to_string())
    })
}

/// 表情消息的 MD5（小写）：第一个是 32 位十六进制的 `md5` 属性或 `<md5>` 元素；都没有时按
/// `extract_loose_hex_md5` 在全文中找，仍找不到时为 `None`。
pub fn extract_emoji_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let h = content.as_bytes();
    let assigned = || {
        crate::xml::find_all_ci(content, 0, "md5")
            .filter_map(|p| crate::xml::assignment_end(content, p + "md5".len()))
    };
    // 相当于 `(?i)md5\s*=\s*['"]([a-fA-F0-9]{32})['"]`
    let found = assigned()
        .find(|&q| {
            matches!(h.get(q), Some(b'\'' | b'"'))
                && hex_at(h, q + 1, 32)
                && matches!(h.get(q + 33), Some(b'\'' | b'"'))
        })
        .map(|q| &content[q + 1..q + 33])
        // 相当于 `(?i)md5\s*=\s*([a-fA-F0-9]{32})`
        .or_else(|| {
            assigned()
                .find(|&q| hex_at(h, q, 32))
                .map(|q| &content[q..q + 32])
        })
        // 相当于 `(?i)<md5>([a-fA-F0-9]{32})</md5>`
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

/// 解码 `s` 中的 `%XX` 转义；解码结果不是 UTF-8 时原样返回 `s`。
pub(crate) fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    String::from_utf8(percent_decode_bytes(s)).unwrap_or_else(|_| s.to_string())
}

/// 把 `%XX` 转义（两位十六进制数字）换成字节，其他内容原样保留。
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

/// 表情在导出中的文字：`[表情：<描述>]`，没有描述时为 `[表情]`。
pub fn format_emoji_semantic_text(caption: Option<&str>) -> String {
    match caption.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => format!("[表情：{c}]"),
        None => "[表情]".into(),
    }
}

/// 以 `<type>` 为 8 的 app 消息发出的表情的 MD5（`<emoticonmd5>`），用来查表情的描述。
pub(super) fn appmsg_emoticon_md5_in(x: &MsgXml<'_>) -> Option<String> {
    // 标签按 ASCII 大小写不敏感匹配；为什么可以先在 `content` 中排除，见 `normalized_may_contain`
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

    /// 由长度在 32 上下的十六进制段、其他 ASCII 字符和多字节字符组成的文字：抽样查找找到的一段与逐个字节查找的相同。
    #[test]
    fn the_first_32_hex_digits_are_found_however_the_text_is_searched() {
        use rand::{rngs::StdRng, RngExt, SeedableRng};
        const OTHER: &[&str] = &["x", "<", " ", "中", "-", "g", "G", "/"];
        let mut rng = StdRng::seed_from_u64(1098);
        let mut found = 0;
        for _ in 0..20_000 {
            let mut s = String::new();
            for _ in 0..rng.random_range(0..12) {
                if rng.random_range(0..3) == 0 {
                    for _ in 0..rng.random_range(1..40) {
                        s.push(char::from(
                            b"0123456789abcdefABCDEF"[rng.random_range(0..22)],
                        ));
                    }
                } else {
                    for _ in 0..rng.random_range(1..20) {
                        s.push_str(OTHER[rng.random_range(0..OTHER.len())]);
                    }
                }
            }
            let want = first_hex32_each(&s);
            assert_eq!(first_hex32(&s), want, "{s:?}");
            found += usize::from(want.is_some());
        }
        assert!(found >= 2000, "{found}");
    }

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
