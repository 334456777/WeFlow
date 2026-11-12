//! Quoted replies: what they quote, its preview and who sent it.

use super::xml::{
    decode_html_entities, extract_app_message_type, extract_xml_value, looks_like_wxid,
    normalize_app_message_content, sanitize_quoted_content, strip_sender_prefix, MsgXml,
};
use super::{format_plain_export_content, PlainOpts};

#[derive(Clone)]
pub(super) struct QuoteInfo {
    pub content: Option<String>,
    pub sender: Option<String>,
    pub quote_type: Option<String>,
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

/// The `<refermsg>` block of a quote, in content whose entities are decoded.
pub(crate) fn refermsg_xml(normalized: &str) -> Option<String> {
    let start = memchr::memmem::find(normalized.as_bytes(), b"<refermsg>")?;
    // 被引用的是「带引用的图片」等消息时，其 content 里还嵌着一层（已解码的）<refermsg>，
    // 取最后一个结束标签才是外层，否则 type / displayname 会被截掉。
    let end = memchr::memmem::rfind(normalized.as_bytes(), b"</refermsg>")?;
    if end < start {
        return None;
    }
    Some(normalized[start..end + "</refermsg>".len()].to_string())
}

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
            let from = sp + start_char.len().saturating_sub(1);
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

/// The quoted text of a `<refermsg>` block: the part the reply picked when it names one, else all of it.
pub(crate) fn extract_preferred_quoted_text(refer_xml: &str) -> String {
    if refer_xml.is_empty() {
        return String::new();
    }
    let mut sources = vec![decode_html_entities(refer_xml)];
    let raw_msgsource = extract_xml_value(refer_xml, "msgsource");
    if !raw_msgsource.is_empty() {
        let decoded = decode_html_entities(&raw_msgsource);
        if !decoded.is_empty() {
            sources.push(decoded);
        }
    }
    let first = if sources[0].is_empty() {
        refer_xml.to_string()
    } else {
        sources[0].clone()
    };
    let full = sanitize_quoted_content(&extract_xml_value(&first, "content"));
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
            let v = sanitize_quoted_content(&extract_xml_value(source, tag));
            if !v.is_empty() {
                return v;
            }
        }
    }
    full
}

/// The quote a `<refermsg>` block (of the normalized content) holds.
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
        "49" => "[链接]".into(),
        "42" => "[名片]".into(),
        "48" => "[位置]".into(),
        _ => quoted_other_preview(&refer_content),
    };
    // 被引用的文字本身是 XML（如引用了一条夹带图片的消息）时，换成可读预览
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

/// 被引用消息的内容本身是 XML（引用的是另一条引用、文件、图片、笔记……）时的可读预览。
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

/// 微信会把过长的被引用内容截断，XML 可能没有结尾标签，所以只看开头。
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

/// 引用里「被引用内容」的文本；XML 内容改成可读预览，普通文本去掉清洗后原样返回。
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
    let out =
        format_plain_export_content(content, ty, &PlainOpts::default(), None, None, None, None);
    if out.is_empty() {
        "[消息]".into()
    } else {
        out
    }
}

#[derive(Clone, Debug)]
pub struct QuotedDisplay {
    pub reply_text: String,
    pub quoted_sender: Option<String>,
    pub quoted_preview: String,
}

pub(super) fn extract_quoted_reply_display(content: &str) -> Option<QuotedDisplay> {
    MsgXml::new(content).quoted_display().cloned()
}

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

pub fn is_quoted_reply_message(local_type: i64, content: &str) -> bool {
    if local_type == 244813135921 {
        return true;
    }
    let normalized = normalize_app_message_content(content);
    if !(local_type == 49 || normalized.contains("<appmsg") || normalized.contains("<msg>")) {
        return false;
    }
    extract_app_message_type(&normalized) == "57" || normalized.contains("<refermsg>")
}

/// Whether the message is a reply (a text quote): only local type 244813135921 and app message type 57 are.
/// Images, forwarded chat records and others may carry a `<refermsg>` too, and are not text quotes.
pub fn is_text_quote_message_in(local_type: i64, x: &MsgXml) -> bool {
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
        let d = extract_quoted_reply_display(xml).unwrap();
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
}
