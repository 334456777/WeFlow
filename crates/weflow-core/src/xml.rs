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
    find_ci_bytes(hay.as_bytes(), from, needle.as_bytes())
}

/// [`find_ci`] on bytes.
///
/// A needle with two bytes that have no case (`<`, `/`, `>`, digits) is searched for by the first and last of them,
/// a fixed distance apart ([`pair_search`]); any other by its rarest byte ([`rarest_find`]).
fn find_ci_bytes(h: &[u8], from: usize, n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return (from <= h.len()).then_some(from);
    }
    if from > h.len() || h.len() - from < n.len() {
        return None;
    }
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        let caseless = |b: &u8| !b.is_ascii_alphabetic();
        if let (Some(first), Some(last)) =
            (n.iter().position(caseless), n.iter().rposition(caseless))
        {
            let tail = |hay: &[u8], n: &[u8]| rarest_find(hay, 0, n);
            let pair = u8::try_from(first).ok().zip(u8::try_from(last).ok());
            if let Some(found) = pair
                .filter(|(a, b)| a != b)
                .and_then(|pair| pair_search(&h[from..], n, pair, tail))
            {
                return found.map(|p| from + p);
            }
        }
    }
    rarest_find(h, from, n)
}

/// [`find_ci`] by the needle's rarest byte: a search for that byte (both cases) skips to each place the needle could
/// start. `n` is not empty and `from` is at most `h.len()`.
fn rarest_find(h: &[u8], from: usize, n: &[u8]) -> Option<usize> {
    if h.len() - from < n.len() {
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

/// The longest tag [`find_tag`] builds a `<tag>` of on the stack; a longer one is searched for by [`find_tag_each`].
const TAG_MAX: usize = 61;

/// First `<tag>` (or `</tag>` when `closing`) at or after `from`, ASCII case-insensitive: `(start, end)` byte range.
///
/// The `<` and `>` of such a tag are a fixed distance apart and have no case, so a SIMD search for that pair of bytes
/// ([`pair_find`]) skips the text between candidates, rather than stopping at every `<`.
fn find_tag(h: &[u8], from: usize, tag: &[u8], closing: bool) -> Option<(usize, usize)> {
    if tag.len() > TAG_MAX {
        return find_tag_each(h, from, tag, closing);
    }
    let mut buf = [0u8; TAG_MAX + 3];
    let name = 1 + usize::from(closing);
    let n = name + tag.len() + 1;
    buf[0] = b'<';
    buf[1] = b'/';
    buf[name..n - 1].copy_from_slice(tag);
    buf[n - 1] = b'>';
    let p = from + pair_find(h.get(from..)?, &buf[..n])?;
    Some((p, p + n))
}

/// The first `needle` in `hay`, ASCII case-insensitive; `needle` starts with `<` and ends with `>`, bytes without case.
///
/// memchr's packed pair searcher finds where those two bytes are as far apart as in `needle`, a vector of text at a
/// time (AVX2 or else SSE2 on x86-64, NEON on aarch64), and each such place is compared in full. Text too short for
/// the vectors, and other architectures, are searched by [`find_each`], which gives the same answer.
fn pair_find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    if let Some(found) = u8::try_from(needle.len() - 1)
        .ok()
        .and_then(|last| pair_search(hay, needle, (0, last), find_each))
    {
        return found;
    }
    find_each(hay, needle)
}

/// The first `needle` in `hay`, ASCII case-insensitive, by a vector search for its bytes at `indices`, which must
/// have no case; `tail` searches the end of the text the vectors leave. `None` when the text is too short for the
/// vectors or the CPU has none: the caller searches some other way.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn pair_search(
    hay: &[u8],
    needle: &[u8],
    (i, j): (u8, u8),
    tail: fn(&[u8], &[u8]) -> Option<usize>,
) -> Option<Option<usize>> {
    // the vectors need some text past the last byte of the needle; for less, a plain search is as quick
    if hay.len() < needle.len() + 32 {
        return None;
    }
    let pair = memchr::arch::all::packedpair::Pair::with_indices(needle, i, j)?;
    #[cfg(target_arch = "x86_64")]
    {
        use memchr::arch::x86_64::{avx2, sse2};
        if let Some(f) = avx2::packedpair::Finder::with_pair(needle, pair) {
            let prefilter = |h: &[u8]| f.find_prefilter(h);
            return Some(pair_candidates(
                hay,
                needle,
                f.min_haystack_len(),
                prefilter,
                tail,
            ));
        }
        if let Some(f) = sse2::packedpair::Finder::with_pair(needle, pair) {
            let prefilter = |h: &[u8]| f.find_prefilter(h);
            return Some(pair_candidates(
                hay,
                needle,
                f.min_haystack_len(),
                prefilter,
                tail,
            ));
        }
    }
    #[cfg(target_arch = "aarch64")]
    if let Some(f) = memchr::arch::aarch64::neon::packedpair::Finder::with_pair(needle, pair) {
        let prefilter = |h: &[u8]| f.find_prefilter(h);
        return Some(pair_candidates(
            hay,
            needle,
            f.min_haystack_len(),
            prefilter,
            tail,
        ));
    }
    None
}

/// The first `needle` in `hay`, ASCII case-insensitive, from the places `prefilter` gives: the first place in a text
/// of at least `min_len` bytes where the needle could start, or `None` when there is none. `tail` searches the rest.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn pair_candidates(
    hay: &[u8],
    needle: &[u8],
    min_len: usize,
    prefilter: impl Fn(&[u8]) -> Option<usize>,
    tail: fn(&[u8], &[u8]) -> Option<usize>,
) -> Option<usize> {
    let mut at = 0;
    while hay.len() - at >= min_len {
        let c = at + prefilter(&hay[at..])?;
        if hay
            .get(c..c + needle.len())
            .is_some_and(|w| w.eq_ignore_ascii_case(needle))
        {
            return Some(c);
        }
        at = c + 1;
    }
    tail(&hay[at..], needle).map(|p| at + p)
}

/// The first `needle` in `hay`, ASCII case-insensitive, by stopping at each occurrence of its first byte.
fn find_each(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let first = *needle.first()?;
    let mut i = 0;
    loop {
        let p = i + memchr::memchr(first, hay.get(i..)?)?;
        if hay
            .get(p..p + needle.len())
            .is_some_and(|w| w.eq_ignore_ascii_case(needle))
        {
            return Some(p);
        }
        i = p + 1;
    }
}

/// [`find_tag`] by stopping at every `<`: for a tag longer than [`TAG_MAX`], and to check [`find_tag`] against.
fn find_tag_each(h: &[u8], from: usize, tag: &[u8], closing: bool) -> Option<(usize, usize)> {
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
///
/// `<name` is searched for by its rarest byte ([`find_ci`]), which skips the text between the places that byte is,
/// rather than stopping at every `<`.
fn find_open(h: &[u8], from: usize, name: &[u8]) -> Option<usize> {
    if name.len() > TAG_MAX {
        return find_open_each(h, from, name);
    }
    let mut buf = [0u8; TAG_MAX + 1];
    buf[0] = b'<';
    buf[1..=name.len()].copy_from_slice(name);
    find_ci_bytes(h, from, &buf[..=name.len()])
}

/// [`find_open`] by stopping at every `<`: for a name longer than [`TAG_MAX`], and to check [`find_open`] against.
fn find_open_each(h: &[u8], from: usize, name: &[u8]) -> Option<usize> {
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

/// An attribute value in double quotes, as `(?i)<tag\s+[^>]*attr\s*=\s*"([^"]*)"` captures it.
///
/// `tag` and `attr` must be non-empty ASCII. As in the regex, `attr` may end another attribute's name (`md5` in
/// `cdnthumbmd5`), and the last match before the tag's first `>` wins, since `[^>]*` is greedy.
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

/// Where the next element `name` at or after `from` is: `(start, gt, end, close_end)`, the offsets of its `<`, of the
/// `>` that ends its open tag, of its closing tag and just after that. Unlike [`elements`], an element holding others
/// of the same name ends at its own closing tag: each open tag inside goes one deeper (a self-closing one does not),
/// each closing tag one shallower. An element whose closing tags do not balance ends at the first one after it, as
/// in [`elements`].
fn outer_element_at(
    xml: &str,
    mut from: usize,
    tag: &[u8],
) -> Option<(usize, usize, usize, usize)> {
    let h = xml.as_bytes();
    loop {
        let p = find_open(h, from, tag)?;
        let after = p + 1 + tag.len();
        if !word_ends_at(xml, after) {
            from = p + 1;
            continue;
        }
        let gt = after + memchr::memchr(b'>', &h[after..])?;
        let mut first_close = None;
        let mut depth = 1usize;
        let mut i = gt + 1;
        while let Some(r) = memchr::memchr(b'<', &h[i..]) {
            let q = i + r;
            if is_close_at(h, q, tag) {
                let close_end = q + 3 + tag.len();
                first_close.get_or_insert((q, close_end));
                depth -= 1;
                if depth == 0 {
                    return Some((p, gt, q, close_end));
                }
                i = close_end;
            } else if h
                .get(q + 1..q + 1 + tag.len())
                .is_some_and(|w| w.eq_ignore_ascii_case(tag))
                && word_ends_at(xml, q + 1 + tag.len())
            {
                let Some(r) = memchr::memchr(b'>', &h[q..]) else {
                    break;
                };
                let inner_gt = q + r;
                if h[inner_gt - 1] != b'/' {
                    depth += 1;
                }
                i = inner_gt + 1;
            } else {
                i = q + 1;
            }
        }
        let (end, close_end) = first_close?;
        return Some((p, gt, end, close_end));
    }
}

/// The attribute text and the body of each outermost element `name`, like [`elements`] except that an element
/// holding others of the same name (a chat record's `<dataitem>` holding a nested record's, #1174) is one element
/// with them in its body. `name` must be ASCII and end with a word character.
pub fn outer_elements<'a>(
    xml: &'a str,
    name: &'a str,
) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
    let tag = name.as_bytes();
    let mut from = 0;
    std::iter::from_fn(move || {
        let (start, gt, end, close_end) = outer_element_at(xml, from, tag)?;
        from = close_end;
        Some((&xml[start + 1 + tag.len()..gt], &xml[gt + 1..end]))
    })
}

/// `xml` without its outermost elements `name`, the elements nested in them included.
pub fn without_outer_elements<'a>(xml: &'a str, name: &str) -> Cow<'a, str> {
    let tag = name.as_bytes();
    let mut kept = String::new();
    let mut from = 0;
    while let Some((start, _, _, close_end)) = outer_element_at(xml, from, tag) {
        kept.push_str(&xml[from..start]);
        from = close_end;
    }
    if from == 0 {
        return Cow::Borrowed(xml);
    }
    kept.push_str(&xml[from..]);
    Cow::Owned(kept)
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

pub use weflow_native::entities::decode_entities;

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    /// Text of tags, open and closing, of a few names in both cases and lengths, with attributes or without, broken
    /// ones (`<` with no `>`, `<<`, `</>`), runs of hex digits, white space and multi-byte characters: long enough for
    /// the vector search, with candidates (a `<` and a `>` as far apart as in a tag) that are not the tag.
    fn tag_soup() -> impl proptest::strategy::Strategy<Value = String> {
        use proptest::prelude::*;
        const PIECES: &[&str] = &[
            "<",
            ">",
            "/",
            "</",
            "<a>",
            "</a>",
            "<A>",
            "</A>",
            "<ab>",
            "</ab>",
            "<aB>",
            "<b_c>",
            "</b_c>",
            "<ac>",
            "</ac>",
            "<a x=\"1\">",
            "<a/>",
            "<a >",
            "</a >",
            "<<a>",
            "</>",
            "<>",
            "<title>",
            "</TITLE>",
            "<titlf>",
            "a",
            "b",
            "_",
            " ",
            "\n",
            "中",
            "]]>",
            "<![CDATA[",
            "0123456789abcdef",
            "https://x.invalid/?k=v&j=w",
        ];
        prop::collection::vec(prop::sample::select(PIECES), 0..400).prop_map(|p| p.concat())
    }

    proptest::proptest! {
        #![proptest_config(crate::property_cases(512))]

        /// For any text, tag and start, the vector search finds the tag the search stopping at every `<` finds.
        #[test]
        fn any_tag_is_found_where_stopping_at_every_bracket_finds_it(
            xml in tag_soup(),
            tag in proptest::sample::select(&["a", "A", "ab", "AB", "ac", "b_c", "b", "title", "Title", "", "x"][..]),
            closing in proptest::bool::ANY,
            from in 0usize..2000,
        ) {
            let h = xml.as_bytes();
            proptest::prop_assert_eq!(
                find_tag(h, from, tag.as_bytes(), closing),
                find_tag_each(h, from, tag.as_bytes(), closing)
            );
        }

        /// For any text, needle and start, [`find_ci`] finds the first place a byte-by-byte comparison finds: needles
        /// with two bytes that have no case (searched for by that pair), one or none (by the rarest byte).
        #[test]
        fn any_needle_is_found_where_comparing_at_every_offset_finds_it(
            xml in tag_soup(),
            needle in proptest::sample::select(&[
                "</a>", "</A>", "<title>", "</TITLE>", "<ab>", "</b_c>", "<a x=\"1\">", "a>", "<a", "</", "<>",
                "0123", "k=v&", "1\">", "x", "TITLE", "?k=", "/a/", "<<a>",
            ][..]),
            from in 0usize..2000,
        ) {
            let (h, n) = (xml.as_bytes(), needle.as_bytes());
            let each = (from..=h.len().saturating_sub(n.len()))
                .find(|&i| h.get(i..i + n.len()).is_some_and(|w| w.eq_ignore_ascii_case(n)));
            proptest::prop_assert_eq!(find_ci(&xml, from, needle), each);
        }

        /// For any text, name and start, `<name` searched for by its rarest byte is found where stopping at every `<`
        /// finds it.
        #[test]
        fn any_open_tag_is_found_where_stopping_at_every_bracket_finds_it(
            xml in tag_soup(),
            name in proptest::sample::select(&["a", "A", "ab", "AB", "ac", "b_c", "b", "title", "Title", "", "x", "/a"][..]),
            from in 0usize..2000,
        ) {
            let h = xml.as_bytes();
            proptest::prop_assert_eq!(
                find_open(h, from, name.as_bytes()),
                find_open_each(h, from, name.as_bytes())
            );
        }
    }

    /// A tag longer than fits on the stack, and tags at the very end of a long text, where the vectors stop.
    #[test]
    fn long_tags_and_tags_at_the_end_are_found() {
        let long = "t".repeat(TAG_MAX + 1);
        let xml = format!("{}<{long}>x</{long}>", " ".repeat(100));
        assert_eq!(tag_inner(&xml, &long), Some("x"));
        for pad in 0..80 {
            let xml = format!("{}<a>{}</a>", "<b>".repeat(pad), "y".repeat(pad));
            let h = xml.as_bytes();
            for closing in [false, true] {
                assert_eq!(
                    find_tag(h, 0, b"a", closing),
                    find_tag_each(h, 0, b"a", closing),
                    "{pad}"
                );
            }
        }
    }

    /// An element holding others of its name is one element, ending at its own closing tag (#1174); a
    /// self-closing one inside does not go deeper; closing tags that do not balance end it at the first, as
    /// [`elements`] does; and with nothing nested, both read the same.
    #[test]
    fn outer_elements_keep_the_nested_ones_inside() {
        let xml =
            "<l><i a=\"1\">x<r><i a=\"2\">y</i><i a=\"3\"/><I>z</I></r>w</i><i a=\"4\">v</i></l>";
        let found: Vec<_> = outer_elements(xml, "i").collect();
        assert_eq!(
            found,
            [
                (" a=\"1\"", "x<r><i a=\"2\">y</i><i a=\"3\"/><I>z</I></r>w"),
                (" a=\"4\"", "v"),
            ]
        );
        assert_eq!(
            without_outer_elements(xml, "r"),
            "<l><i a=\"1\">xw</i><i a=\"4\">v</i></l>"
        );
        assert_eq!(without_outer_elements("<l>x</l>", "r"), "<l>x</l>");
        let unbalanced = "<i>a<i>b</i>c";
        assert_eq!(
            outer_elements(unbalanced, "i").collect::<Vec<_>>(),
            elements(unbalanced, "i").collect::<Vec<_>>()
        );
        let nested = "x<title>a<title>b</title>c</title>";
        assert_eq!(
            outer_elements(nested, "title").collect::<Vec<_>>(),
            [("", "a<title>b</title>c")]
        );
        for xml in SAMPLES.iter().filter(|x| **x != nested) {
            assert_eq!(
                outer_elements(xml, "title").collect::<Vec<_>>(),
                elements(xml, "title").collect::<Vec<_>>(),
                "{xml}"
            );
        }
        let flat = "<l><item>a</item><items>no</items><item k='v'>b</item></l>";
        assert_eq!(
            outer_elements(flat, "item").collect::<Vec<_>>(),
            elements(flat, "item").collect::<Vec<_>>()
        );
    }

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
}
