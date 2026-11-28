//! 微信写进消息和朋友圈文字里的 XML 实体，以及它们的解码。

use std::borrow::Cow;

/// 以 `after`（`&` 之后的文字）开头的实体代表的字符，以及实体名和 `;` 共占的字节数；`after` 开头不是
/// [`decode_entities`] 认识的实体时为 `None`。实体名不区分 ASCII 大小写。
fn entity_at(after: &[u8]) -> Option<(char, usize)> {
    // 首字母之后是否依次是 `rest`（不区分大小写）和 `;`
    let is = |rest: &[u8]| {
        after
            .get(1..=rest.len())
            .is_some_and(|w| w.eq_ignore_ascii_case(rest))
            && after.get(rest.len() + 1) == Some(&b';')
    };
    let (c, rest): (char, &[u8]) = match after.first()?.to_ascii_lowercase() {
        b'l' => ('<', b"t"),
        b'g' => ('>', b"t"),
        b'a' if is(b"mp") => ('&', b"mp"),
        b'a' => ('\'', b"pos"),
        b'q' => ('"', b"uot"),
        b'#' => ('\'', b"39"),
        _ => return None,
    };
    is(rest).then_some((c, rest.len() + 2))
}

/// 一遍解码微信写的 XML 实体（`&lt;`、`&gt;`、`&amp;`、`&quot;`、`&apos;`、`&#39;`，不区分 ASCII 大小写），
/// 实体解码出的文字不会再次解码：`&amp;lt;` 变成 `&lt;`。`&` 后面不是这些实体时原样保留。没有 `&` 时不复制。
pub fn decode_entities(s: &str) -> Cow<'_, str> {
    let h = s.as_bytes();
    let Some(mut amp) = memchr::memchr(b'&', h) else {
        return Cow::Borrowed(s);
    };
    let mut out = String::with_capacity(s.len());
    // `kept` 之前的文字已写进 `out`；不是实体开头的 `&` 留在 `kept` 之后的文字里，随后一起写入
    let mut kept = 0;
    loop {
        let mut next = amp + 1;
        if let Some((c, len)) = entity_at(&h[amp + 1..]) {
            out.push_str(&s[kept..amp]);
            out.push(c);
            next += len;
            kept = next;
        }
        match memchr::memchr(b'&', &h[next..]) {
            Some(r) => amp = next + r,
            None => break,
        }
    }
    out.push_str(&s[kept..]);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 逐个 `&` 解码的参照实现：取 `&` 之后五个字节内第一个 `;` 之前的名称，在表里查找。
    fn decode_entities_each(s: &str) -> String {
        const ENTITIES: [(&str, char); 6] = [
            ("lt", '<'),
            ("gt", '>'),
            ("amp", '&'),
            ("quot", '"'),
            ("apos", '\''),
            ("#39", '\''),
        ];
        let mut out = String::new();
        let mut rest = s;
        while let Some(amp) = rest.find('&') {
            out.push_str(&rest[..amp]);
            let after = &rest[amp + 1..];
            let entity = after.as_bytes()[..after.len().min(5)]
                .iter()
                .position(|&b| b == b';')
                .and_then(|end| {
                    let name = &after[..end];
                    let c = ENTITIES
                        .iter()
                        .find(|(entity, _)| entity.eq_ignore_ascii_case(name))?
                        .1;
                    Some((c, end))
                });
            rest = match entity {
                Some((c, end)) => {
                    out.push(c);
                    &after[end + 1..]
                }
                None => {
                    out.push('&');
                    after
                }
            };
        }
        out.push_str(rest);
        out
    }

    #[test]
    fn decode_entities_decodes_each_entity_once() {
        for (input, want) in [
            ("&amp;lt;b&amp;gt;", "&lt;b&gt;"),
            ("&lt;b&gt;", "<b>"),
            ("&amp;amp;", "&amp;"),
            ("&quot;a&#39;b&apos;", "\"a'b'"),
            ("&AMP;&Lt;", "&<"),
            ("a & b &; &nbsp; &#40; &", "a & b &; &nbsp; &#40; &"),
            ("&&lt;", "&<"),
            ("中&lt;文&amp;", "中<文&"),
            ("&ap;os; &am;p; &l;t; &lt &a", "&ap;os; &am;p; &l;t; &lt &a"),
        ] {
            assert_eq!(decode_entities(input), want, "{input:?}");
        }
        assert!(matches!(decode_entities("no entity"), Cow::Borrowed(_)));
    }

    proptest::proptest! {
        #![proptest_config(crate::property_cases(1024))]

        /// 由实体、实体的片段、`&`、`;`（各种大小写和顺序）以及多字节字符拼成的任意文字，解码的结果都与逐个 `&`
        /// 解码的结果相同。
        #[test]
        fn any_text_decodes_as_a_and_at_a_time_decodes_it(
            pieces in proptest::collection::vec(
                proptest::sample::select(&[
                    "&", ";", "&lt;", "&GT;", "&Amp;", "&quot;", "&aPOs;", "&#39;", "&#40;", "&nbsp;", "lt", "amp",
                    "pos", "a", "p", "o", "s", "q", "u", "t", "#", "3", "9", "l", "g", "m", "中", " ", "x",
                ][..]),
                0..200,
            ),
        ) {
            let s = pieces.concat();
            proptest::prop_assert_eq!(decode_entities(&s), decode_entities_each(&s));
        }
    }
}
