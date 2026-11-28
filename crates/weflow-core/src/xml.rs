//! 快速且尽量不分配内存地扫描微信保存在消息中的 XML 片段。
//!
//! 这些函数用普通的字符串搜索替代消息解码中的高频正则（`(?is)<tag>(.*?)</tag>`、属性值、`<tag\b`），结果与正则相同；
//! 每个函数都写明它替代的正则。标签和属性名都是 ASCII（非 ASCII 的在调用处退回正则），按 ASCII 大小写不敏感比较；
//! 与正则的 `(?i)` 唯一的差别是 `ſ`（U+017F）和开尔文符号（U+212A）不匹配 `s` 和 `k`。
//! 空白（`\s`）和单词字符（`\b`）与正则一样按 Unicode 判断。匹配总是从 ASCII 字节开始，所以返回的偏移都在 `char`
//! 的边界上。

use std::borrow::Cow;

/// 每个字节在消息 XML 中大致的出现频率：数值越大越常见，没有列出的字节为 0。字母的两种大小写算同一个，
/// [`find_ci`] 同时搜索两种大小写。
const FREQUENCY: [u8; 256] = {
    // 最常见的在前：与微信保存的 XML 一致，标记符号和最常见的英文字母排在最前
    const COMMON: &[u8] = b" <>=\"/etaoinsrhldcum0123456789wfgypbvk.:_-jxqz";
    let mut table = [0u8; 256];
    // 从最不常见的一端往前数，名次随下标一起递增
    let (mut i, mut rank) = (COMMON.len(), 0u8);
    while i > 0 {
        i -= 1;
        rank += 1;
        table[COMMON[i] as usize] = rank;
        table[COMMON[i].to_ascii_uppercase() as usize] = rank;
    }
    table
};

/// ASCII 的 `needle` 在 `hay[from..]` 中第一次出现（ASCII 大小写不敏感）的字节偏移；`from` 超出 `hay` 时为 `None`。
pub fn find_ci(hay: &str, from: usize, needle: &str) -> Option<usize> {
    find_ci_bytes(hay.as_bytes(), from, needle.as_bytes())
}

/// 按字节做 [`find_ci`]。
///
/// 有两个没有大小写的字节（`<`、`/`、`>`、数字等）的搜索串，按其中第一个和最后一个搜索，
/// 这两个字节相距固定（[`pair_search`]）；其他搜索串按其中最少见的字节搜索（[`rarest_find`]）。
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

/// 按搜索串中最少见的字节做 [`find_ci`]：搜索这个字节（两种大小写），直接跳到搜索串可能开始的每个位置。
/// `n` 不为空，`from` 不超过 `h.len()`。
fn rarest_find(h: &[u8], from: usize, n: &[u8]) -> Option<usize> {
    if h.len() - from < n.len() {
        return None;
    }
    // 找搜索串中最少见的字节；每找到一处，它前面 `k` 个字节就是一个可能的开始位置
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

/// [`find_tag`] 在栈上拼出 `<tag>` 时 tag 的最大长度；更长的由 [`find_tag_each`] 搜索。
const TAG_MAX: usize = 61;

/// `from` 及以后第一个 `<tag>`（`closing` 时为 `</tag>`），ASCII 大小写不敏感，返回字节范围 `(start, end)`。
///
/// 这种标签的 `<` 和 `>` 相距固定且没有大小写，所以用 SIMD 搜索这一对字节（[`pair_find`]），跳过候选之间的文字，
/// 不在每个 `<` 处停下。
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

/// `hay` 中第一个 `needle`，ASCII 大小写不敏感；`needle` 以 `<` 开头、以 `>` 结尾，这两个字节没有大小写。
///
/// memchr 的成对字节搜索一次处理一个向量的文字（x86-64 上用 AVX2，没有时用 SSE2；aarch64 上用 NEON），
/// 找出这两个字节的距离与 `needle` 中相同的位置，再逐个完整比较。文字不够一个向量时和其他架构上，
/// 用结果相同的 [`find_each`] 搜索。
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

/// `hay` 中第一个 `needle`（ASCII 大小写不敏感）：用向量搜索它在下标 `i`、`j` 上的两个字节，
/// 这两个字节必须没有大小写；向量处理不到的文字末尾由 `tail` 搜索。
/// 文字不够长或 CPU 没有可用的向量指令时为 `None`，由调用方换一种方法搜索。
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn pair_search(
    hay: &[u8],
    needle: &[u8],
    (i, j): (u8, u8),
    tail: fn(&[u8], &[u8]) -> Option<usize>,
) -> Option<Option<usize>> {
    // 向量要求搜索串最后一个字节之后还有一些文字；文字更短时，普通搜索一样快
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

/// 从 `prefilter` 给出的位置找 `hay` 中第一个 `needle`（ASCII 大小写不敏感）：`prefilter` 返回至少 `min_len`
/// 字节的文字中搜索串可能开始的第一个位置，没有时为 `None`。剩下的部分由 `tail` 搜索。
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

/// 在 `needle` 第一个字节出现的每个位置停下比较，找出 `hay` 中第一个 `needle`（ASCII 大小写不敏感）。
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

/// 在每个 `<` 处停下比较的 [`find_tag`]：用于比 [`TAG_MAX`] 更长的标签，也是测试中 [`find_tag`] 的对照。
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

/// `(?is)<tag>(.*?)</tag>`：第一个 `<tag>` 与其后第一个 `</tag>` 之间的文字。`tag` 必须是 ASCII。
///
/// # Panics
///
/// 调试构建中 `tag` 不是 ASCII 时。
pub fn tag_inner<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    debug_assert!(tag.is_ascii());
    let h = xml.as_bytes();
    let (_, start) = find_tag(h, 0, tag.as_bytes(), false)?;
    let (end, _) = find_tag(h, start, tag.as_bytes(), true)?;
    Some(&xml[start..end])
}

/// `(?is)<tag>.*?</tag>`：第一个 `tag` 元素，包括它的开始和结束标签。`tag` 必须是 ASCII。
pub fn tag_element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let h = xml.as_bytes();
    let (start, inner) = find_tag(h, 0, tag.as_bytes(), false)?;
    let (_, end) = find_tag(h, inner, tag.as_bytes(), true)?;
    Some(&xml[start..end])
}

/// 第一个 `open` 之后的第一个 `>` 之后的文字，到其后第一个 `close` 为止；`open` 是 `<appmsg` 这样的 ASCII 前缀，
/// `close` 是 `</appmsg>` 这样的结束标签，此例相当于 `(?is)<appmsg.*?>(.*?)</appmsg>`。
pub fn element_inner<'a>(xml: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let p = find_ci(xml, 0, open)?;
    let gt = p + open.len() + memchr::memchr(b'>', &xml.as_bytes()[p + open.len()..])?;
    let end = find_ci(xml, gt + 1, close)?;
    Some(&xml[gt + 1..end])
}

/// 去掉 `s` 中从 `open` 到其后第一个 `close` 的每一段，`open`、`close` 为 ASCII（如 `<refermsg`、`</refermsg>`，
/// 此例相当于 `Regex::new("(?is)<refermsg.*?</refermsg>").replace_all(s, "")`）；没有可去掉的时不复制。
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

/// 从 `i` 开始的一段空白（正则的 `\s*`，即 Unicode 的 `White_Space`）之后的偏移。
///
/// # Panics
///
/// `i` 大于 `s` 的长度或不在字符边界上时。
pub fn skip_ws(s: &str, i: usize) -> usize {
    s.len() - s[i..].trim_start().len()
}

/// 从 `i` 开始匹配 `\s*=\s*`，返回其后的偏移；不匹配时为 `None`。
///
/// # Panics
///
/// `i` 大于 `s` 的长度或不在字符边界上时。
pub fn assignment_end(s: &str, i: usize) -> Option<usize> {
    let eq = skip_ws(s, i);
    (s.as_bytes().get(eq) == Some(&b'=')).then(|| skip_ws(s, eq + 1))
}

/// ASCII 的 `needle` 在 `hay[from..]` 中的每一次出现（ASCII 大小写不敏感），按顺序，包括互相重叠的。
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

/// `from` 及以后下一个后面跟着 `name`（ASCII 大小写不敏感）的 `<` 的字节偏移。
///
/// `<name` 交给 [`find_ci`]，按其中最少见的字节搜索，跳过这个字节出现的位置之间的文字，不在每个 `<` 处停下。
fn find_open(h: &[u8], from: usize, name: &[u8]) -> Option<usize> {
    if name.len() > TAG_MAX {
        return find_open_each(h, from, name);
    }
    let mut buf = [0u8; TAG_MAX + 1];
    buf[0] = b'<';
    buf[1..=name.len()].copy_from_slice(name);
    find_ci_bytes(h, from, &buf[..=name.len()])
}

/// 在每个 `<` 处停下比较的 [`find_open`]：用于比 [`TAG_MAX`] 更长的名字，也是测试中 [`find_open`] 的对照。
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

/// `h[i..]` 是否以 `</name>` 开头，ASCII 大小写不敏感。
pub(crate) fn is_close_at(h: &[u8], i: usize, name: &[u8]) -> bool {
    h.get(i..i + 2) == Some(b"</")
        && h.get(i + 2..i + 2 + name.len())
            .is_some_and(|w| w.eq_ignore_ascii_case(name))
        && h.get(i + 2 + name.len()) == Some(&b'>')
}

/// 在 `i` 处结束的单词字符之后，正则的 `\b` 是否成立：后面没有 Unicode 单词字符。
fn word_ends_at(s: &str, i: usize) -> bool {
    !s[i..]
        .chars()
        .next()
        .is_some_and(regex_syntax::is_word_character)
}

/// 双引号里的属性值，与 `(?i)<tag\s+[^>]*attr\s*=\s*"([^"]*)"` 捕获的相同。
///
/// `tag` 和 `attr` 必须是非空的 ASCII。与正则一样，`attr` 可以是另一个属性名的结尾（`cdnthumbmd5` 中的 `md5`），
/// 而且标签的第一个 `>` 之前的最后一个匹配胜出，因为 `[^>]*` 是贪婪的。
///
/// # Panics
///
/// 调试构建中 `tag` 或 `attr` 为空或不是 ASCII 时。
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

/// 十六进制的属性值：`open` 是 `<img` 这样的 ASCII 前缀，`attr` 是 ASCII，`attr` 前面必须是空白；
/// `open` 为 `<img`、`attr` 为 `md5` 时相当于 `(?i)<img[^>]*\smd5\s*=\s*['"]([a-fA-F0-9]+)['"]`。
/// 与 [`attribute`] 一样，第一个 `>` 之前的最后一个匹配胜出。
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

/// `(?i)<tag>([^<]+)</tag>`：第一个只含文字的 `<tag>` 的文字。`tag` 必须是 ASCII。
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

/// 像 `captures_iter` 一样反复匹配 `(?is)<tag>(.*?)</tag>`：每个元素的文字。`tag` 必须是 ASCII。
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

/// 像 `captures_iter` 一样反复匹配 `(?is)<name\b([^>]*)>(.*?)</name>`：每个元素的属性文字和内容。
/// `name` 必须是 ASCII，并以单词字符结尾。
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
        // 这个开始标签之后没有 `>` 或结束标签时，后面的开始标签也不会有
        let gt = after + memchr::memchr(b'>', &h[after..])?;
        let (end, close_end) = find_tag(h, gt + 1, tag, true)?;
        from = close_end;
        return Some((&xml[after..gt], &xml[gt + 1..end]));
    })
}

/// `from` 及以后下一个 `name` 元素的位置 `(start, gt, end, close_end)`：它的 `<`、结束开始标签的 `>`、
/// 它的结束标签，以及结束标签之后的偏移。与 [`elements`] 不同，含有同名元素的元素到它自己的结束标签为止：
/// 里面的每个开始标签深一层（自闭合的不算），每个结束标签浅一层。结束标签不配对的元素与 [`elements`] 一样，
/// 到其后第一个结束标签为止。
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

/// 每个最外层 `name` 元素的属性文字和内容。与 [`elements`] 的区别是，含有同名元素的元素（聊天记录的 `<dataitem>`
/// 里有嵌套记录的 `<dataitem>`，#1174）算一个元素，同名元素在它的内容里。`name` 必须是 ASCII，并以单词字符结尾。
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

/// 去掉最外层的 `name` 元素（连同嵌在其中的元素）后的 `xml`。
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

/// `(?i)<name\b`：是否在任何位置有 `name` 元素的开始标签。`name` 必须是 ASCII，并以单词字符结尾。
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

/// 像 `captures_iter` 一样反复匹配 `(?s)open(.*?)close`，区分大小写：每一对之间的文字。
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

/// `from` 不出现时不复制的 `s.replace(from, to)`。
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

    /// 由几种名字、大小写和长度的开始与结束标签（带或不带属性）、残缺的标签（没有 `>` 的 `<`、`<<`、`</>`）、
    /// 成段的十六进制数字、空白和多字节字符组成的文字：长度够向量搜索用，
    /// 其中有不是这个标签的候选位置（`<` 和 `>` 的距离与标签中相同）。
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

        /// 对任意文字、标签和起点，向量搜索找到的标签与在每个 `<` 处停下的搜索相同。
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

        /// 对任意文字、搜索串和起点，[`find_ci`] 找到的第一个位置与逐个字节比较的相同：
        /// 有两个没有大小写的字节的搜索串（按这一对搜索），以及只有一个或没有的（按最少见的字节搜索）。
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

        /// 对任意文字、名字和起点，按最少见的字节搜索 `<name` 找到的位置与在每个 `<` 处停下的搜索相同。
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

    /// 栈上放不下的长标签，以及长文字最末尾、向量搜索处理不到的标签。
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

    /// 含有同名元素的元素算一个元素，到它自己的结束标签为止（#1174）；里面自闭合的同名元素不加深一层；
    /// 结束标签不配对时与 [`elements`] 一样到第一个为止；没有嵌套时两者读到的相同。
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
