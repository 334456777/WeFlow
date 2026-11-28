//! 引用回复：被引用的内容、它的预览和发送者。

use super::xml::{
    decode_html_entities, decode_text_entities, extract_app_message_type, extract_xml_value,
    looks_like_wxid, normalize_app_message_content, sanitize_quoted_content, strip_sender_prefix,
    MsgXml,
};

/// 回复中被引用的消息，各项取自 `<refermsg>`，没有时为 `None`。
#[derive(Clone)]
pub(super) struct QuoteInfo {
    /// 被引用内容的预览。
    pub content: Option<String>,
    /// 被引用消息的发送者的显示名（`<displayname>`）；是 wxid 时为 `None`。
    pub sender: Option<String>,
    /// 被引用消息的类型（`<type>`）。
    pub quote_type: Option<String>,
    /// 被引用消息的服务器 ID（`<svrid>`）。
    pub svrid: Option<String>,
}

impl QuoteInfo {
    pub(super) fn empty() -> Self {
        Self {
            content: None,
            sender: None,
            quote_type: None,
            svrid: None,
        }
    }
}

/// 从已解码实体的内容中取出引用的 `<refermsg>` 块。
pub(crate) fn refermsg_xml(normalized: &str) -> Option<String> {
    let start = memchr::memmem::find(normalized.as_bytes(), b"<refermsg>")?;
    // 被引用的消息本身也带引用时（如带引用的图片），它的内容里还有一个（已解码的）<refermsg>：
    // 外层的块要到最后一个结束标签为止，否则外层的 type 和 displayname 会被截掉
    let end = memchr::memmem::rfind(normalized.as_bytes(), b"</refermsg>")?;
    if end < start {
        return None;
    }
    Some(normalized[start..end + "</refermsg>".len()].to_string())
}

/// 回复选中的部分引用：`full` 中从 `<start>` 到 `<end>` 的一段（含两端），没有时按 `<startindex>`、`<endindex>`
/// 的字符下标截取；都没有时为空字符串。
fn extract_partial_quoted_text(xml: &str, full: &str) -> String {
    if xml.is_empty() || full.is_empty() {
        return String::new();
    }
    let start_char = extract_xml_value(xml, "start");
    let end_char = extract_xml_value(xml, "end");
    let start_index = extract_xml_value(xml, "startindex").parse::<i64>().ok();
    let end_index = extract_xml_value(xml, "endindex").parse::<i64>().ok();
    if !start_char.is_empty() && !end_char.is_empty() {
        if let Some(sp) = full.find(&start_char) {
            // 从开始字符串的最后一个字符起找结束字符串，两者可以共用这个字符
            let last = start_char.char_indices().last().map_or(0, |(at, _)| at);
            let from = sp + last;
            if let Some(rel) = full.get(from..).and_then(|s| s.find(&end_char)) {
                let ep = from + rel;
                if ep >= sp {
                    let sliced = full[sp..ep + end_char.len()].trim().to_string();
                    if !sliced.is_empty() {
                        return sliced;
                    }
                }
            }
        }
    }
    if let (Some(s), Some(e)) = (start_index, end_index) {
        if let (true, Ok(skip), Ok(take)) = (e >= s, usize::try_from(s), usize::try_from(e - s + 1))
        {
            let sliced: String = full.chars().skip(skip).take(take).collect();
            let sliced = sliced.trim().to_string();
            if !sliced.is_empty() {
                return sliced;
            }
        }
    }
    String::new()
}

/// 把 `text` 中表示空白（换行、制表符、空格）的数字实体换成空格。被引用的文字大多像其他 XML 文字一样只转义一次；
/// 有些把换行和空格写成多转义了一层的 `&#x0A;`、`&#x20;`，解码引用之后仍留在文字里（#1212）。
/// 预览本来就把空白并成一个空格。
fn white_space_references_as_spaces(text: &str) -> std::borrow::Cow<'_, str> {
    const REFERENCES: [&str; 8] = [
        "&#x0a;", "&#10;", "&#x0d;", "&#13;", "&#x09;", "&#9;", "&#x20;", "&#32;",
    ];
    if !text.contains("&#") {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("&#") {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        match REFERENCES.iter().find(|r| {
            tail.get(..r.len())
                .is_some_and(|h| h.eq_ignore_ascii_case(r))
        }) {
            Some(r) => {
                out.push(' ');
                rest = &tail[r.len()..];
            }
            None => {
                out.push_str("&#");
                rest = &tail[2..];
            }
        }
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

/// `<refermsg>` 块中被引用的文字：回复选了其中一部分时取那一部分（按 `<start>`、`<end>`，下标，
/// 或 `<selectedcontent>` 等元素），否则取全部。
pub(crate) fn extract_preferred_quoted_text(refer_xml: &str) -> String {
    if refer_xml.is_empty() {
        return String::new();
    }
    // 块里被引用的文字多转义了一层，数字实体也是，如微信表示换行的 `&#x0A;`（#1212）
    let mut sources = vec![decode_text_entities(refer_xml).into_owned()];
    let raw_msgsource = extract_xml_value(refer_xml, "msgsource");
    if !raw_msgsource.is_empty() {
        let decoded = decode_text_entities(&raw_msgsource).into_owned();
        if !decoded.is_empty() {
            sources.push(decoded);
        }
    }
    let first = if sources[0].is_empty() {
        refer_xml.to_string()
    } else {
        sources[0].clone()
    };
    let quoted = |text: &str| sanitize_quoted_content(&white_space_references_as_spaces(text));
    let full = quoted(&extract_xml_value(&first, "content"));
    let partial = extract_partial_quoted_text(&first, &full);
    if !partial.is_empty() {
        return partial;
    }
    let tags = [
        "selectedcontent",
        "selectedtext",
        "selectcontent",
        "selecttext",
        "quotecontent",
        "quotetext",
        "partcontent",
        "parttext",
        "excerpt",
        "summary",
        "preview",
    ];
    for source in &sources {
        for tag in tags {
            let v = quoted(&extract_xml_value(source, tag));
            if !v.is_empty() {
                return v;
            }
        }
    }
    full
}

/// 规范化后的内容中的 `<refermsg>` 块所引用的消息。
pub(super) fn quote_info_of(refer: impl AsRef<str>) -> QuoteInfo {
    let refer = refer.as_ref();
    let mut sender = extract_xml_value(refer, "displayname");
    if !sender.is_empty() && looks_like_wxid(&sender) {
        sender.clear();
    }
    let refer_content = extract_xml_value(refer, "content");
    let refer_type = extract_xml_value(refer, "type");
    let svrid = extract_xml_value(refer, "svrid");
    let display = match refer_type.as_str() {
        "1" => extract_preferred_quoted_text(refer),
        "3" => "[图片]".into(),
        "34" => "[语音消息]".into(),
        "43" => "[视频]".into(),
        "47" => "[表情]".into(),
        // app 消息按它自己的 XML 写：回复写它的文字，链接写标题，文件写文件名（#1263）
        "49" => quoted_other_preview(&refer_content),
        "42" => "[名片]".into(),
        "48" => "[位置]".into(),
        _ => quoted_other_preview(&refer_content),
    };
    // 本身是 XML 的被引用文字（如引用了一条带图片的消息）换成可读的预览
    let display = if looks_like_xml_payload(&display) {
        quoted_xml_preview(&display)
    } else {
        display
    };
    let opt = |s: String| if s.is_empty() { None } else { Some(s) };
    QuoteInfo {
        content: opt(display),
        sender: opt(sender),
        quote_type: opt(refer_type),
        svrid: opt(svrid),
    }
}

/// 本身是 XML 的被引用内容的可读预览（被引用的是另一条回复、文件、图片、笔记等）。
fn quoted_xml_preview(content: &str) -> String {
    let normalized = normalize_app_message_content(content);
    let title = extract_xml_value(&normalized, "title");
    if normalized.contains("<appmsg") {
        let with_title = |label: &str| {
            if title.is_empty() {
                label.to_string()
            } else {
                format!("{label} {title}")
            }
        };
        return match extract_app_message_type(&normalized).as_str() {
            "57" => {
                if title.is_empty() {
                    "[引用消息]".into()
                } else {
                    title
                }
            }
            "8" => "[表情]".into(),
            "6" => with_title("[文件]"),
            "19" => "[聊天记录]".into(),
            "24" => "[笔记]".into(),
            "33" | "36" => "[小程序]".into(),
            _ => {
                if title.is_empty() {
                    "[链接]".into()
                } else {
                    title
                }
            }
        };
    }
    if normalized.contains("<img") {
        "[图片]".into()
    } else if normalized.contains("<emoji") {
        "[表情]".into()
    } else if normalized.contains("<voicemsg") {
        "[语音消息]".into()
    } else if normalized.contains("<videomsg") {
        "[视频]".into()
    } else {
        "[消息]".into()
    }
}

/// `text` 是否为 XML，只看开头：微信会截短较长的被引用内容，XML 可能没有结束标签。
fn looks_like_xml_payload(text: &str) -> bool {
    let head = text.trim_start();
    [
        "<?xml",
        "<msg",
        "<img",
        "<appmsg",
        "<emoji",
        "<voicemsg",
        "<videomsg",
    ]
    .iter()
    .any(|p| head.starts_with(p))
}

/// 被引用内容的文字：XML 换成可读的预览，其他文字清理后返回；为空或含 wxid 的内容为 `[消息]`。
fn quoted_other_preview(refer_content: &str) -> String {
    let decoded = decode_html_entities(refer_content);
    let body = strip_sender_prefix(&decoded);
    if looks_like_xml_payload(&body) {
        return quoted_xml_preview(&body);
    }
    if refer_content.is_empty() || refer_content.contains("wxid_") {
        "[消息]".into()
    } else {
        sanitize_quoted_content(refer_content)
    }
}

/// 按被引用消息的类型 `refer_type` 写它的预览：app 消息（49）写标题、文件名或应用名，都没有时写类型的标签；
/// 其他类型写导出给这种消息的文字；类型不是数字时按 [`quoted_other_preview`]。
pub(super) fn format_quoted_reference_preview(content: &str, refer_type: &str) -> String {
    let t: Option<i64> = refer_type.trim().parse().ok();
    let Some(ty) = t else {
        let s = quoted_other_preview(content);
        return if s.is_empty() { "[消息]".into() } else { s };
    };
    if ty == 49 {
        let normalized = normalize_app_message_content(content);
        let title = ["title", "filename", "appname"]
            .iter()
            .map(|k| extract_xml_value(&normalized, k))
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        if !title.is_empty() {
            return strip_sender_prefix(&title);
        }
        return match extract_app_message_type(&normalized)
            .parse::<i64>()
            .unwrap_or(0)
        {
            6 => "[文件]".into(),
            19 => "[聊天记录]".into(),
            33 | 36 => "[小程序]".into(),
            _ => "[链接]".into(),
        };
    }
    // 导出给这种类型的消息写的文字（#1263）
    let out = super::parse_message_content(content, ty, None, None, None).unwrap_or_default();
    if out.is_empty() {
        "[消息]".into()
    } else {
        out
    }
}

/// 引用回复显示出来的三部分。
#[derive(Clone, Debug)]
pub struct QuotedDisplay {
    /// 回复本身的文字。
    pub reply_text: String,
    /// 被引用消息的发送者。
    pub quoted_sender: Option<String>,
    /// 被引用内容的预览。
    pub quoted_preview: String,
}

/// 引用回复的文字 `<回复>[引用 <发送者>：<预览>]`：没有发送者时不写「<发送者>：」，没有回复时只写方括号的部分。
pub fn build_quoted_reply_text(d: &QuotedDisplay) -> String {
    let label = match &d.quoted_sender {
        Some(s) if !s.is_empty() => format!("{s}：{}", d.quoted_preview),
        _ => d.quoted_preview.clone(),
    };
    if d.reply_text.is_empty() {
        format!("[引用 {label}]")
    } else {
        format!("{}[引用 {label}]", d.reply_text)
    }
}

/// 消息是否为回复（文字引用）：只有 `local_type` 为 244813135921 的和 `<type>` 为 57 的 app 消息是。
/// 图片、转发的聊天记录等也可能带 `<refermsg>`，但不是文字引用。
pub fn is_text_quote_message_in(local_type: i64, x: &MsgXml<'_>) -> bool {
    if local_type == 244_813_135_921 {
        return true;
    }
    if matches!(local_type & 0xFFFF_FFFF, 3 | 34 | 43 | 47) {
        return false;
    }
    x.app_type_twice() == "57"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::parse_message_content;

    #[test]
    fn quoted_reply_text() {
        let xml = "<msg><appmsg><title>ok</title><type>57</type><refermsg><type>1</type><displayname>Bob</displayname><content>hi there</content><svrid>123</svrid></refermsg></appmsg></msg>";
        let d = MsgXml::new(xml).quoted_display().cloned().unwrap();
        assert_eq!(build_quoted_reply_text(&d), "ok[引用 Bob：hi there]");
        assert_eq!(
            MsgXml::new(xml).reply_to_message_id().as_deref(),
            Some("123")
        );
        assert_eq!(
            parse_message_content(xml, 49, None, None, None).unwrap(),
            "ok[引用 Bob：hi there]"
        );
    }

    #[test]
    fn partial_quote_uses_indexes() {
        let xml = "<refermsg><type>1</type><content>abcdefg</content><startindex>1</startindex><endindex>3</endindex></refermsg>";
        assert_eq!(extract_preferred_quoted_text(xml), "bcd");
    }

    #[test]
    fn partial_quote_between_start_and_end_characters() {
        let quote = |content: &str, start: &str, end: &str| {
            extract_preferred_quoted_text(&format!(
                "<refermsg><type>1</type><content>{content}</content><start>{start}</start><end>{end}</end></refermsg>"
            ))
        };
        assert_eq!(quote("abcdefg", "b", "d"), "bcd");
        // 多字节的开始和结束字符
        assert_eq!(quote("今天天气很好，去公园吧", "天", "好"), "天天气很好");
        assert_eq!(quote("今天天气很好，去公园吧", "天气", "好"), "天气很好");
        // 开始和结束可以共用一个字符
        assert_eq!(quote("今天天气很好，去公园吧", "公", "公"), "公");
        assert_eq!(quote("今天天气很好，去公园吧", "很好", "好"), "很好");
        assert_eq!(quote("今天天气很好🙂，去公园吧", "🙂", "园"), "🙂，去公园");
    }
}
