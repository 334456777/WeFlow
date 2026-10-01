//! Fast, allocation-free scanning of the XML snippets WeChat stores in messages.
//!
//! These replace hot `(?is)<tag>(.*?)</tag>`-style regexes with plain ASCII case-insensitive searches that
//! give exactly the same results (the tags are ASCII; a non-ASCII tag falls back to the regex at the call site).
//! Matches always start at an ASCII byte, so every returned offset is a `char` boundary.

use std::borrow::Cow;

/// Byte offset of the first ASCII case-insensitive occurrence of the ASCII `needle` in `hay[from..]`.
pub fn find_ci(hay: &str, from: usize, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.is_empty() {
        return (from <= h.len()).then_some(from);
    }
    if from > h.len() || h.len() - from < n.len() {
        return None;
    }
    let (lo, up) = (n[0].to_ascii_lowercase(), n[0].to_ascii_uppercase());
    let last_start = h.len() - n.len();
    let mut i = from;
    while i <= last_start {
        let window = &h[i..=last_start];
        let rel = if lo == up { memchr::memchr(lo, window) } else { memchr::memchr2(lo, up, window) }?;
        let p = i + rel;
        if h[p..p + n.len()].eq_ignore_ascii_case(n) {
            return Some(p);
        }
        i = p + 1;
    }
    None
}

/// First `<tag>` (or `</tag>` when `closing`) at or after `from`, ASCII case-insensitive: `(start, end)` byte range.
fn find_tag(h: &[u8], from: usize, tag: &[u8], closing: bool) -> Option<(usize, usize)> {
    let mut i = from;
    while i <= h.len() {
        let p = i + memchr::memchr(b'<', &h[i..])?;
        let mut q = p + 1;
        if closing {
            if h.get(q) != Some(&b'/') {
                i = p + 1;
                continue;
            }
            q += 1;
        }
        let end = q + tag.len();
        if end < h.len() && h[q..end].eq_ignore_ascii_case(tag) && h[end] == b'>' {
            return Some((p, end + 1));
        }
        i = p + 1;
    }
    None
}

/// `(?is)<tag>(.*?)</tag>`: the text between the first `<tag>` and the first `</tag>` after it.
/// `tag` must be ASCII.
pub fn tag_inner<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    debug_assert!(tag.is_ascii());
    let h = xml.as_bytes();
    let (_, start) = find_tag(h, 0, tag.as_bytes(), false)?;
    let (end, _) = find_tag(h, start, tag.as_bytes(), true)?;
    Some(&xml[start..end])
}

/// `(?is)<open.*?>(.*?)</close>` where `open` is an ASCII prefix such as `<appmsg`: the text after the first `>`
/// that follows the first `open`, up to the first `close` after it.
pub fn element_inner<'a>(xml: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let p = find_ci(xml, 0, open)?;
    let gt = p + open.len() + memchr::memchr(b'>', &xml.as_bytes()[p + open.len()..])?;
    let end = find_ci(xml, gt + 1, close)?;
    Some(&xml[gt + 1..end])
}

/// `Regex::new("(?is)<open.*?</close>").replace_all(s, "")` for ASCII `open` / `close`.
pub fn remove_blocks<'a>(s: &'a str, open: &str, close: &str) -> Cow<'a, str> {
    let Some(mut p) = find_ci(s, 0, open) else { return Cow::Borrowed(s) };
    let mut out = String::with_capacity(s.len());
    let mut kept = 0;
    while let Some(c) = find_ci(s, p + open.len(), close) {
        out.push_str(&s[kept..p]);
        kept = c + close.len();
        match find_ci(s, kept, open) {
            Some(next) => p = next,
            None => break,
        }
    }
    if kept == 0 {
        return Cow::Borrowed(s);
    }
    out.push_str(&s[kept..]);
    Cow::Owned(out)
}

/// `s.replace(from, to)` that does not copy when `from` does not occur.
pub fn replace_cow<'a>(s: Cow<'a, str>, from: &str, to: &str) -> Cow<'a, str> {
    if s.contains(from) {
        Cow::Owned(s.replace(from, to))
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    const SAMPLES: &[&str] = &[
        "",
        "<msg><appmsg appid=\"\"><title>Hi</title><type>5</type></appmsg></msg>",
        "<TITLE>upper</TITLE><title>second</title>",
        "<title>unclosed",
        "<title></title>",
        "x<title>a<title>b</title>c</title>",
        "<title >spaced</title>",
        "<titles>no</titles><title>yes</title>",
        "</title><title>after stray close</title>",
        "<title>多字节 ✓</title>",
        "<title>\n<![CDATA[ line\nbreak ]]>\n</title>",
        "<appmsg><refermsg><type>1</type></refermsg><type>57</type></appmsg>",
        "<appmsg a='>'><patMsg><type>9</type></patMsg>x<PATMSG>y</patmsg><type>62</type></APPMSG>",
        "<appmsg>no close",
        "<appmsg><refermsg>unclosed <type>3</type></appmsg>",
        "<<title>>double</title>",
        "<</title>",
        "中文<appmsg>中<type>1</type>文</appmsg>尾",
    ];

    #[test]
    fn tag_inner_matches_the_regex() {
        for tag in ["title", "type", "TiTlE", "des", ""] {
            let re = Regex::new(&format!(r"(?is)<{0}>(.*?)</{0}>", regex::escape(tag))).unwrap();
            for s in SAMPLES {
                let want = re.captures(s).map(|c| c.get(1).unwrap().as_str());
                assert_eq!(tag_inner(s, tag), want, "tag {tag:?} in {s:?}");
            }
        }
    }

    #[test]
    fn element_inner_and_remove_blocks_match_the_regexes() {
        let appmsg = Regex::new(r"(?is)<appmsg.*?>(.*?)</appmsg>").unwrap();
        let refer = Regex::new(r"(?is)<refermsg.*?</refermsg>").unwrap();
        let pat = Regex::new(r"(?is)<patMsg.*?</patMsg>").unwrap();
        for s in SAMPLES {
            let want = appmsg.captures(s).map(|c| c.get(1).unwrap().as_str());
            assert_eq!(element_inner(s, "<appmsg", "</appmsg>"), want, "{s:?}");
            assert_eq!(remove_blocks(s, "<refermsg", "</refermsg>"), refer.replace_all(s, ""), "{s:?}");
            assert_eq!(remove_blocks(s, "<patmsg", "</patmsg>"), pat.replace_all(s, ""), "{s:?}");
        }
    }

    #[test]
    fn random_snippets_match_the_regexes() {
        use rand::{rngs::StdRng, Rng, SeedableRng};
        const TOKENS: &[&str] = &[
            "<title>", "</title>", "<TITLE>", "</Title>", "<title", "title>", "<appmsg", "<APPMSG ", "</appmsg>", "</AppMsg>",
            "<refermsg>", "</refermsg>", "<patMsg>", "</PATMSG>", "<type>", "</type>", "<", ">", "/", "x", "中", "\n", " ",
        ];
        let title = Regex::new(r"(?is)<title>(.*?)</title>").unwrap();
        let ty = Regex::new(r"(?is)<type>(.*?)</type>").unwrap();
        let appmsg = Regex::new(r"(?is)<appmsg.*?>(.*?)</appmsg>").unwrap();
        let refer = Regex::new(r"(?is)<refermsg.*?</refermsg>").unwrap();
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..20_000 {
            let s: String = (0..rng.gen_range(0..14)).map(|_| TOKENS[rng.gen_range(0..TOKENS.len())]).collect();
            assert_eq!(tag_inner(&s, "title"), title.captures(&s).map(|c| c.get(1).unwrap().as_str()), "{s:?}");
            assert_eq!(tag_inner(&s, "type"), ty.captures(&s).map(|c| c.get(1).unwrap().as_str()), "{s:?}");
            assert_eq!(element_inner(&s, "<appmsg", "</appmsg>"), appmsg.captures(&s).map(|c| c.get(1).unwrap().as_str()), "{s:?}");
            assert_eq!(remove_blocks(&s, "<refermsg", "</refermsg>"), refer.replace_all(&s, ""), "{s:?}");
        }
    }

    #[test]
    fn find_ci_is_ascii_case_insensitive() {
        assert_eq!(find_ci("abcABC", 1, "abc"), Some(3));
        assert_eq!(find_ci("中<APPMSG", 0, "<appmsg"), Some(3));
        assert_eq!(find_ci("ab", 0, "abc"), None);
        assert_eq!(find_ci("ab", 3, "a"), None);
        assert_eq!(find_ci("ab", 2, ""), Some(2));
    }
}
