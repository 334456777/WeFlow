//! Quote messages (local type [`QUOTE`]): replies that quote an earlier message, and the text they say.

use std::borrow::Cow;

use crate::entities::decode_entities;

/// WeChat's local type of a quote: an app message of type 57, whose `<appmsg>` XML has the reply as its `<title>`
/// and the quoted message in `<refermsg>`.
pub const QUOTE: i64 = 244_813_135_921;

/// What a quote says, from its `content`: the text of the first `<title>` of its XML (ASCII case-insensitive), the
/// CDATA markers removed, entities decoded, control characters but tab and line breaks removed and trimmed; empty
/// when it has none. Only the title is decoded, so an escaped `</title>` in the reply is part of it; XML escaped as
/// a whole (no `<title>` before decoding) is decoded first.
pub fn reply_text(content: &str) -> String {
    match title(content) {
        Some(raw) => without_controls(&decode_entities(&without_cdata(raw)))
            .trim()
            .to_string(),
        None => {
            let decoded = without_controls(&decode_entities(content));
            title(&decoded)
                .map(|raw| without_cdata(raw).trim().to_string())
                .unwrap_or_default()
        }
    }
}

/// The text between the first `<title>` and the first `</title>` after it, tags compared ASCII case-insensitively.
fn title(xml: &str) -> Option<&str> {
    const OPEN: &str = "<title>";
    const CLOSE: &str = "</title>";
    let start = find_tag(xml, 0, OPEN)? + OPEN.len();
    let end = find_tag(xml, start, CLOSE)?;
    Some(&xml[start..end])
}

/// Byte offset of the first ASCII case-insensitive `tag` (which starts with `<`) in `xml[from..]`. A match starts
/// at a `<`, so the offset is a `char` boundary.
fn find_tag(xml: &str, from: usize, tag: &str) -> Option<usize> {
    let bytes = xml.as_bytes();
    xml[from..]
        .match_indices('<')
        .map(|(i, _)| from + i)
        .find(|&p| {
            bytes
                .get(p..p + tag.len())
                .is_some_and(|w| w.eq_ignore_ascii_case(tag.as_bytes()))
        })
}

/// `text` without the `<![CDATA[` and `]]>` markers, wherever they are. Does not copy when it has none.
fn without_cdata(text: &str) -> Cow<'_, str> {
    if text.contains("<![CDATA[") || text.contains("]]>") {
        Cow::Owned(text.replace("<![CDATA[", "").replace("]]>", ""))
    } else {
        Cow::Borrowed(text)
    }
}

/// `text` without the C0 and C1 control characters, but for tab, line feed and carriage return.
fn without_controls(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(*c as u32, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F..=0x9F))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn quote(title: &str) -> String {
        format!(
            "wxid_bob:\n<msg><appmsg><title>{title}</title><type>57</type>\
             <refermsg><type>1</type><content>hi</content></refermsg></appmsg></msg>"
        )
    }

    #[test]
    fn a_quote_says_its_title() {
        for (title, said) in [
            ("好的 &amp; 收到", "好的 & 收到"),
            ("<![CDATA[好的]]>", "好的"),
            ("  <![CDATA[ 好的 ]]>\n", "好的"),
            // a reply that writes a tag is that text, not the end of the title
            ("写 &lt;/title&gt; 就行", "写 </title> 就行"),
            ("好\u{1}的\t呀", "好的\t呀"),
            ("", ""),
        ] {
            assert_eq!(reply_text(&quote(title)), said, "{title:?}");
        }
        assert_eq!(
            reply_text("<msg><appmsg><TITLE>好的</Title></appmsg></msg>"),
            "好的"
        );
        // the first title is the reply, not the quoted message's
        assert_eq!(
            reply_text(
                "<appmsg><title>回复</title><refermsg><title>原文</title></refermsg></appmsg>"
            ),
            "回复"
        );
    }

    #[test]
    fn a_quote_escaped_as_a_whole_is_decoded_first() {
        assert_eq!(
            reply_text("&lt;msg&gt;&lt;appmsg&gt;&lt;title&gt;好的&lt;/title&gt;&lt;/appmsg&gt;&lt;/msg&gt;"),
            "好的"
        );
    }

    #[test]
    fn a_quote_without_a_title_says_nothing() {
        for content in [
            "<msg><appmsg><type>57</type><refermsg><content>hi</content></refermsg></appmsg></msg>",
            "<msg><appmsg><title>unclosed</appmsg></msg>",
            "",
            "<",
            "中文<titl",
        ] {
            assert_eq!(reply_text(content), "", "{content:?}");
        }
    }

    proptest! {
        #![proptest_config(crate::property_cases(256))]

        #[test]
        fn any_escaped_reply_reads_back(said in "\\PC*") {
            let escaped = said.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
            prop_assert_eq!(reply_text(&quote(&escaped)), said.trim());
        }
    }
}
