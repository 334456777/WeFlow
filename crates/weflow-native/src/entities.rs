//! The XML entities WeChat writes into the text of its messages and posts, and how they are decoded.

use std::borrow::Cow;

/// The character the entity whose name starts `after` (the text after a `&`) stands for, and how many bytes its name
/// and `;` take; `None` when no entity [`decode_entities`] knows starts there. Names compare ASCII case-insensitively.
fn entity_at(after: &[u8]) -> Option<(char, usize)> {
    // the rest of the name, without case, and the `;`
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

/// Decodes the XML entities WeChat writes (`&lt;`, `&gt;`, `&amp;`, `&quot;`, `&apos;`, `&#39;`, in any ASCII case)
/// in one pass, so the text an entity decodes to is never decoded again: `&amp;lt;` becomes `&lt;`. Anything else
/// after a `&` is kept as it is. Does not copy when there is no `&`.
pub fn decode_entities(s: &str) -> Cow<'_, str> {
    let h = s.as_bytes();
    let Some(mut amp) = memchr::memchr(b'&', h) else {
        return Cow::Borrowed(s);
    };
    let mut out = String::with_capacity(s.len());
    // the text up to `kept` is in `out`; a `&` that starts no entity stays in the text after it
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

    /// The decoding a `&` at a time: the name up to the first `;` within five bytes, looked up in a table.
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

        /// For any text of entities, their pieces, `&` and `;` in every case and order, and multi-byte characters, the
        /// decoding is the one a `&` at a time gives.
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
