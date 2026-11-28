//! 引用消息（`local_type` 为 [`QUOTE`]）：引用之前一条消息的回复，以及取出回复的文字。

use std::borrow::Cow;

use crate::entities::decode_entities;

/// 引用消息的 `local_type`：子类型为 57 的 app 消息，它的 `<appmsg>` XML 中 `<title>` 是回复的文字，
/// `<refermsg>` 是被引用的消息。
pub const QUOTE: i64 = 244_813_135_921;

/// 从引用消息的 `content` 取出回复的文字：XML 里第一个 `<title>`（不区分 ASCII 大小写）的内容，去掉 CDATA 标记、
/// 解码实体、去掉制表符和换行符以外的控制字符，再去掉首尾空白；没有 `<title>` 时为空。只解码标题的内容，
/// 所以回复里转义过的 `</title>` 属于回复的文字；整段 XML 都被转义时（解码前没有 `<title>`）先解码整段。
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

/// 第一个 `<title>` 与其后第一个 `</title>` 之间的文字，标签不区分 ASCII 大小写。
fn title(xml: &str) -> Option<&str> {
    const OPEN: &str = "<title>";
    const CLOSE: &str = "</title>";
    let start = find_tag(xml, 0, OPEN)? + OPEN.len();
    let end = find_tag(xml, start, CLOSE)?;
    Some(&xml[start..end])
}

/// `xml` 中从 `from` 起第一个 `tag`（以 `<` 开头，不区分 ASCII 大小写）的字节偏移。
/// 匹配从 `<` 开始，所以偏移落在 `char` 的边界上。
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

/// 去掉所有 `<![CDATA[` 和 `]]>` 标记的 `text`，不论标记在哪里。没有标记时不复制。
fn without_cdata(text: &str) -> Cow<'_, str> {
    if text.contains("<![CDATA[") || text.contains("]]>") {
        Cow::Owned(text.replace("<![CDATA[", "").replace("]]>", ""))
    } else {
        Cow::Borrowed(text)
    }
}

/// 去掉 C0 和 C1 控制字符的 `text`，保留制表符、换行符和回车符。
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
            // 回复里写出的标签是回复的文字，不是标题的结尾
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
        // 第一个标题是回复，不是被引用的消息的标题
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
