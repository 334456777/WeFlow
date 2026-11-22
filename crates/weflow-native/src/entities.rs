//! The XML entities WeChat writes into the text of its messages and posts, and how they are decoded.

use std::borrow::Cow;

/// The longest entity name [`decode_entities`] knows (`quot`, `apos`).
const LONGEST_ENTITY: usize = 4;

/// The character an entity name (between `&` and `;`) stands for, compared ASCII case-insensitively.
fn entity_char(name: &str) -> Option<char> {
    const ENTITIES: [(&str, char); 6] = [
        ("lt", '<'),
        ("gt", '>'),
        ("amp", '&'),
        ("quot", '"'),
        ("apos", '\''),
        ("#39", '\''),
    ];
    ENTITIES
        .iter()
        .find(|(entity, _)| entity.eq_ignore_ascii_case(name))
        .map(|&(_, c)| c)
}

/// Decodes the XML entities WeChat writes (`&lt;`, `&gt;`, `&amp;`, `&quot;`, `&apos;`, `&#39;`, in any ASCII case)
/// in one pass, so the text an entity decodes to is never decoded again: `&amp;lt;` becomes `&lt;`. Anything else
/// after a `&` is kept as it is. Does not copy when there is no `&`.
pub fn decode_entities(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let entity = after.as_bytes()[..after.len().min(LONGEST_ENTITY + 1)]
            .iter()
            .position(|&b| b == b';')
            .and_then(|end| Some((entity_char(&after[..end])?, end)));
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
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        ] {
            assert_eq!(decode_entities(input), want, "{input:?}");
        }
        assert!(matches!(decode_entities("no entity"), Cow::Borrowed(_)));
    }
}
