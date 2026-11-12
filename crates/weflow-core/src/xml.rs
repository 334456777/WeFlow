//! Fast, allocation-free scanning of the XML snippets WeChat stores in messages.
//!
//! These replace the hot regexes of message decoding (`(?is)<tag>(.*?)</tag>`, attribute values, `<tag\b`) with
//! plain searches that give the same results; each function names the regex it stands for. Tags and attribute
//! names are ASCII (a non-ASCII one falls back to the regex at the call site) and compare ASCII
//! case-insensitively; the one difference from the regex's `(?i)` is that `ſ` (U+017F) and the Kelvin sign
//! (U+212A) do not match `s` and `k`. White space (`\s`) and word characters (`\b`) are Unicode, as in the regex.
//! Matches always start at an ASCII byte, so every returned offset is a `char` boundary.

use std::borrow::Cow;

/// How often each byte turns up in message XML, roughly: higher is more often, 0 for bytes not listed. Letters
/// count both cases, which [`find_ci`] searches for together.
const FREQUENCY: [u8; 256] = {
    // the most common first; markup and the commonest English letters lead, as in the XML WeChat stores
    const COMMON: &[u8] = b" <>=\"/etaoinsrhldcum0123456789wfgypbvk.:_-jxqz";
    let mut table = [0u8; 256];
    // from the least common up, so the rank counts alongside the index
    let (mut i, mut rank) = (COMMON.len(), 0u8);
    while i > 0 {
        i -= 1;
        rank += 1;
        table[COMMON[i] as usize] = rank;
        table[COMMON[i].to_ascii_uppercase() as usize] = rank;
    }
    table
};

/// Byte offset of the first ASCII case-insensitive occurrence of the ASCII `needle` in `hay[from..]`.
pub fn find_ci(hay: &str, from: usize, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.is_empty() {
        return (from <= h.len()).then_some(from);
    }
    if from > h.len() || h.len() - from < n.len() {
        return None;
    }
    // look for the needle's rarest byte; each hit is a candidate start `k` bytes before it
    let k = (0..n.len())
        .min_by_key(|&i| FREQUENCY[usize::from(n[i])])
        .unwrap_or(0);
    let (lo, up) = (n[k].to_ascii_lowercase(), n[k].to_ascii_uppercase());
    let last_start = h.len() - n.len();
    let mut i = from;
    while i <= last_start {
        let window = &h[i + k..=last_start + k];
        let rel = if lo == up {
            memchr::memchr(lo, window)
        } else {
            memchr::memchr2(lo, up, window)
        }?;
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

/// `(?is)<tag>.*?</tag>`: the first element `tag`, tags included. `tag` must be ASCII.
pub fn tag_element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let h = xml.as_bytes();
    let (start, inner) = find_tag(h, 0, tag.as_bytes(), false)?;
    let (_, end) = find_tag(h, inner, tag.as_bytes(), true)?;
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
    let Some(mut p) = find_ci(s, 0, open) else {
        return Cow::Borrowed(s);
    };
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

/// The offset after the run of white space (regex `\s*`, Unicode `White_Space`) starting at `i`.
pub fn skip_ws(s: &str, i: usize) -> usize {
    s.len() - s[i..].trim_start().len()
}

/// `\s*=\s*` at `i`: the offset after it.
pub fn assignment_end(s: &str, i: usize) -> Option<usize> {
    let eq = skip_ws(s, i);
    (s.as_bytes().get(eq) == Some(&b'=')).then(|| skip_ws(s, eq + 1))
}

/// Every ASCII case-insensitive occurrence of the ASCII `needle` in `hay[from..]`, in order (overlapping ones too).
pub fn find_all_ci<'a>(
    hay: &'a str,
    from: usize,
    needle: &'a str,
) -> impl Iterator<Item = usize> + 'a {
    let mut from = from;
    std::iter::from_fn(move || {
        let p = find_ci(hay, from, needle)?;
        from = p + 1;
        Some(p)
    })
}

/// Byte offset of the next `<` followed by `name` (ASCII case-insensitive) at or after `from`.
fn find_open(h: &[u8], from: usize, name: &[u8]) -> Option<usize> {
    let mut i = from;
    loop {
        let p = i + memchr::memchr(b'<', h.get(i..)?)?;
        if h.get(p + 1..p + 1 + name.len())
            .is_some_and(|w| w.eq_ignore_ascii_case(name))
        {
            return Some(p);
        }
        i = p + 1;
    }
}

/// Whether `s[i..]` starts with `</name>`, ASCII case-insensitive.
pub(crate) fn is_close_at(h: &[u8], i: usize, name: &[u8]) -> bool {
    h.get(i..i + 2) == Some(b"</")
        && h.get(i + 2..i + 2 + name.len())
            .is_some_and(|w| w.eq_ignore_ascii_case(name))
        && h.get(i + 2 + name.len()) == Some(&b'>')
}

/// Whether a regex `\b` holds after a word character that ends at `i`: no Unicode word character follows.
fn word_ends_at(s: &str, i: usize) -> bool {
    !s[i..]
        .chars()
        .next()
        .is_some_and(regex_syntax::is_word_character)
}

/// `(?i)<tag\s+[^>]*attr\s*=\s*"([^"]*)"`: an attribute value in double quotes. `tag` and `attr` must be
/// non-empty ASCII. As in the regex, `attr` may end another attribute's name (`md5` in `cdnthumbmd5`), and the
/// last match before the tag's first `>` wins, since `[^>]*` is greedy.
pub fn attribute<'a>(xml: &'a str, tag: &str, attr: &str) -> Option<&'a str> {
    debug_assert!(!tag.is_empty() && tag.is_ascii() && !attr.is_empty() && attr.is_ascii());
    let h = xml.as_bytes();
    let mut from = 0;
    while let Some(p) = find_open(h, from, tag.as_bytes()) {
        from = p + 1;
        let after = p + 1 + tag.len();
        let lo = skip_ws(xml, after);
        if lo == after {
            continue;
        }
        let gt = memchr::memchr(b'>', &h[lo..]).map_or(h.len(), |r| lo + r);
        let value = find_all_ci(&xml[..gt], lo, attr)
            .filter_map(|a| {
                let open = assignment_end(xml, a + attr.len())?;
                if h.get(open) != Some(&b'"') {
                    return None;
                }
                let close = open + 1 + memchr::memchr(b'"', &h[open + 1..])?;
                Some(&xml[open + 1..close])
            })
            .last();
        if value.is_some() {
            return value;
        }
    }
    None
}

/// `(?i)<open[^>]*\sattr\s*=\s*['"]([a-fA-F0-9]+)['"]`, where `open` is an ASCII prefix such as `<img` and
/// `attr` is ASCII: a hex attribute value. The last match before the first `>` wins, as in
/// [`attribute`].
pub fn spaced_hex_attribute<'a>(xml: &'a str, open: &str, attr: &str) -> Option<&'a str> {
    let h = xml.as_bytes();
    let mut from = 0;
    while let Some(p) = find_ci(xml, from, open) {
        from = p + 1;
        let lo = p + open.len();
        let gt = memchr::memchr(b'>', &h[lo..]).map_or(h.len(), |r| lo + r);
        let value = find_all_ci(&xml[..gt], lo, attr)
            .filter(|&a| {
                xml[lo..a]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
            })
            .filter_map(|a| {
                let quote = assignment_end(xml, a + attr.len())?;
                if !matches!(h.get(quote), Some(b'\'' | b'"')) {
                    return None;
                }
                let start = quote + 1;
                let end = start
                    + h[start..]
                        .iter()
                        .take_while(|b| b.is_ascii_hexdigit())
                        .count();
                (end > start && matches!(h.get(end), Some(b'\'' | b'"'))).then(|| &xml[start..end])
            })
            .last();
        if value.is_some() {
            return value;
        }
    }
    None
}

/// `(?i)<open[^>]*\sattr\s*=\s*['"]([^'"]*)['"]`, where `open` is an ASCII prefix such as `<img` and `attr` is ASCII:
/// an attribute value in either kind of quotes, of a whole attribute name (white space before it, so `md5` is not
/// found in `cdnthumbmd5`). The last match before the tag's first `>` wins, as in [`attribute`].
pub fn spaced_attribute<'a>(xml: &'a str, open: &str, attr: &str) -> Option<&'a str> {
    debug_assert!(open.is_ascii() && attr.is_ascii());
    let h = xml.as_bytes();
    let mut from = 0;
    while let Some(p) = find_ci(xml, from, open) {
        from = p + 1;
        let lo = p + open.len();
        let gt = memchr::memchr(b'>', &h[lo..]).map_or(h.len(), |r| lo + r);
        let value = find_all_ci(&xml[..gt], lo, attr)
            .filter(|&a| {
                xml[lo..a]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
            })
            .filter_map(|a| {
                let quote = assignment_end(xml, a + attr.len())?;
                let close = match h.get(quote)? {
                    b'\'' => b'\'',
                    b'"' => b'"',
                    _ => return None,
                };
                let start = quote + 1;
                let end = start + memchr::memchr(close, &h[start..])?;
                (end <= gt).then(|| &xml[start..end])
            })
            .last();
        if value.is_some() {
            return value;
        }
    }
    None
}

/// `(?i)<tag>([^<]+)</tag>`: the text of the first `<tag>` that holds only text. `tag` must be ASCII.
pub fn text_inner<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let (h, name) = (xml.as_bytes(), tag.as_bytes());
    let mut from = 0;
    while let Some((_, start)) = find_tag(h, from, name, false) {
        from = start;
        let end = memchr::memchr(b'<', &h[start..]).map_or(h.len(), |r| start + r);
        if end > start && is_close_at(h, end, name) {
            return Some(&xml[start..end]);
        }
    }
    None
}

/// `(?is)<tag>(.*?)</tag>` matched repeatedly, like `captures_iter`: the text of each element. `tag` must be ASCII.
pub fn tag_inners<'a>(xml: &'a str, tag: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    let (h, name) = (xml.as_bytes(), tag.as_bytes());
    let mut from = 0;
    std::iter::from_fn(move || {
        let (_, start) = find_tag(h, from, name, false)?;
        let (end, after) = find_tag(h, start, name, true)?;
        from = after;
        Some(&xml[start..end])
    })
}

/// `(?is)<name\b([^>]*)>(.*?)</name>` matched repeatedly, like `captures_iter`: the attribute text and the body
/// of each element. `name` must be ASCII and end with a word character.
pub fn elements<'a>(xml: &'a str, name: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
    let (h, tag) = (xml.as_bytes(), name.as_bytes());
    let mut from = 0;
    std::iter::from_fn(move || loop {
        let p = find_open(h, from, tag)?;
        let after = p + 1 + tag.len();
        if !word_ends_at(xml, after) {
            from = p + 1;
            continue;
        }
        // with no `>` or no closing tag after this start, no later start has one either
        let gt = after + memchr::memchr(b'>', &h[after..])?;
        let (end, close_end) = find_tag(h, gt + 1, tag, true)?;
        from = close_end;
        return Some((&xml[after..gt], &xml[gt + 1..end]));
    })
}

/// `(?i)<name\b`: whether an element `name` opens anywhere. `name` must be ASCII and end with a word character.
pub fn has_open(xml: &str, name: &str) -> bool {
    let h = xml.as_bytes();
    let mut from = 0;
    while let Some(p) = find_open(h, from, name.as_bytes()) {
        if word_ends_at(xml, p + 1 + name.len()) {
            return true;
        }
        from = p + 1;
    }
    false
}

/// `(?s)open(.*?)close` matched repeatedly, case-sensitive, like `captures_iter`: the text between each pair.
pub fn between<'a>(
    s: &'a str,
    open: &'a str,
    close: &'a str,
) -> impl Iterator<Item = &'a str> + 'a {
    let mut from = 0;
    std::iter::from_fn(move || {
        let start = from + s[from..].find(open)? + open.len();
        let end = start + s[start..].find(close)?;
        from = end + close.len();
        Some(&s[start..end])
    })
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
    fn find_ci_finds_the_first_match_at_or_after_from() {
        use rand::{rngs::StdRng, RngExt, SeedableRng};
        const PIECES: &[&str] = &[
            "e", "E", "md5", "MD5", "q", "Q", "<", "5", "中", "x", "ty", "TYPE", " ",
        ];
        const NEEDLES: &[&str] = &["", "e", "md5", "emoticonmd5", "<type>", "q5", "xq", "Type"];
        let naive = |h: &str, from: usize, n: &str| {
            (from..=h.len().saturating_sub(n.len()))
                .filter(|_| h.len() >= n.len())
                .find(|&p| h.as_bytes()[p..p + n.len()].eq_ignore_ascii_case(n.as_bytes()))
        };
        let mut rng = StdRng::seed_from_u64(546);
        let mut hits = 0;
        for _ in 0..20_000 {
            let h: String = (0..rng.random_range(0..14))
                .map(|_| PIECES[rng.random_range(0..PIECES.len())])
                .collect();
            let n = NEEDLES[rng.random_range(0..NEEDLES.len())];
            let from = rng.random_range(0..=h.len() + 1);
            let want = if from > h.len() {
                None
            } else {
                naive(&h, from, n)
            };
            assert_eq!(find_ci(&h, from, n), want, "{h:?} from {from} for {n:?}");
            hits += usize::from(want.is_some() && !n.is_empty());
        }
        assert!(hits >= 1000, "{hits}");
    }

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
            assert_eq!(
                remove_blocks(s, "<refermsg", "</refermsg>"),
                refer.replace_all(s, ""),
                "{s:?}"
            );
            assert_eq!(
                remove_blocks(s, "<patmsg", "</patmsg>"),
                pat.replace_all(s, ""),
                "{s:?}"
            );
        }
    }

    #[test]
    fn random_snippets_match_the_regexes() {
        use rand::{rngs::StdRng, RngExt, SeedableRng};
        const TOKENS: &[&str] = &[
            "<title>",
            "</title>",
            "<TITLE>",
            "</Title>",
            "<title",
            "title>",
            "<appmsg",
            "<APPMSG ",
            "</appmsg>",
            "</AppMsg>",
            "<refermsg>",
            "</refermsg>",
            "<patMsg>",
            "</PATMSG>",
            "<type>",
            "</type>",
            "<",
            ">",
            "/",
            "x",
            "中",
            "\n",
            " ",
        ];
        let title = Regex::new(r"(?is)<title>(.*?)</title>").unwrap();
        let ty = Regex::new(r"(?is)<type>(.*?)</type>").unwrap();
        let appmsg = Regex::new(r"(?is)<appmsg.*?>(.*?)</appmsg>").unwrap();
        let refer = Regex::new(r"(?is)<refermsg.*?</refermsg>").unwrap();
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..20_000 {
            let s: String = (0..rng.random_range(0..14))
                .map(|_| TOKENS[rng.random_range(0..TOKENS.len())])
                .collect();
            assert_eq!(
                tag_inner(&s, "title"),
                title.captures(&s).map(|c| c.get(1).unwrap().as_str()),
                "{s:?}"
            );
            assert_eq!(
                tag_inner(&s, "type"),
                ty.captures(&s).map(|c| c.get(1).unwrap().as_str()),
                "{s:?}"
            );
            assert_eq!(
                element_inner(&s, "<appmsg", "</appmsg>"),
                appmsg.captures(&s).map(|c| c.get(1).unwrap().as_str()),
                "{s:?}"
            );
            assert_eq!(
                remove_blocks(&s, "<refermsg", "</refermsg>"),
                refer.replace_all(&s, ""),
                "{s:?}"
            );
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

    #[test]
    fn spaced_attribute_reads_a_whole_attribute() {
        let img = r#"<msg><img aeskey="0123456789abcdef0123456789abcdef" cdnbigimgurl="http://x/y?a=1&amp;b=2" hdlength="2400000" cdnthumbmd5="zz" md5="aabb"/></msg>"#;
        assert_eq!(
            spaced_attribute(img, "<img", "cdnbigimgurl"),
            Some("http://x/y?a=1&amp;b=2")
        );
        assert_eq!(spaced_attribute(img, "<img", "hdlength"), Some("2400000"));
        // `md5` is not the `cdnthumbmd5` before it
        assert_eq!(spaced_attribute(img, "<img", "md5"), Some("aabb"));
        // single quotes, names of any case, and the last match before the tag's `>`
        assert_eq!(
            spaced_attribute("<img MD5='1' md5 = '2' >", "<IMG", "md5"),
            Some("2")
        );
        assert_eq!(spaced_attribute("<img>", "<img", "md5"), None);
        assert_eq!(spaced_attribute("<img md5=x>", "<img", "md5"), None);
        // the value must end before the tag's `>`
        assert_eq!(spaced_attribute("<img md5='1>", "<img", "md5"), None);
    }
}
