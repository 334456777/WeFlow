//! Quoted replies: what they quote, its preview and who sent it.

use super::xml::{
    decode_html_entities, decode_text_entities, extract_app_message_type, extract_xml_value,
    looks_like_wxid, normalize_app_message_content, sanitize_quoted_content, strip_sender_prefix,
    MsgXml,
};

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
    // when the quoted message carries a quote itself (such as an image with a quote), its content holds another
    // (decoded) <refermsg>: the outer block ends at the last closing tag, or its type and displayname would be cut off
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
            // the end is looked for from the last character of the start on, so the two may share it
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

/// `text` with the numeric references of white space (line breaks, tabs, spaces) as spaces. A quoted text is mostly
/// escaped once, as any XML text; some keep their line breaks and spaces as `&#x0A;`, `&#x20;` escaped once more, which
/// are left after the quote is decoded (#1212). The preview turns white space into single spaces anyway.
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

/// The quoted text of a `<refermsg>` block: the part the reply picked when it names one, else all of it.
pub(crate) fn extract_preferred_quoted_text(refer_xml: &str) -> String {
    if refer_xml.is_empty() {
        return String::new();
    }
    // the quoted text is escaped once more inside the block: numeric entities too, such as the `&#x0A;` WeChat writes
    // for a line break (#1212)
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
        // an app message by its own XML: a quoted reply by its words, a link by its title, a file by its name (#1263)
        "49" => quoted_other_preview(&refer_content),
        "42" => "[名片]".into(),
        "48" => "[位置]".into(),
        _ => quoted_other_preview(&refer_content),
    };
    // quoted text that is XML itself (a quote of a message carrying an image, say) becomes a readable preview
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

/// A readable preview of quoted content that is XML itself (a quote of another quote, a file, an image, a note…).
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

/// Whether `text` is an XML payload, by its start alone: WeChat cuts long quoted content short, so the XML may have
/// no closing tag.
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

/// The text of a quote's quoted content: XML becomes a readable preview, other text comes back sanitized, and
/// content that is empty or names a wxid is `[消息]`.
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
    // the text the exports give a message of that type (#1263)
    let out = super::parse_message_content(content, ty, None, None, None).unwrap_or_default();
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

/// Whether the message is a reply (a text quote): only local type 244813135921 and app message type 57 are.
/// Images, forwarded chat records and others may carry a `<refermsg>` too, and are not text quotes.
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
        // multi-byte start and end characters
        assert_eq!(quote("今天天气很好，去公园吧", "天", "好"), "天天气很好");
        assert_eq!(quote("今天天气很好，去公园吧", "天气", "好"), "天气很好");
        // the start and the end may share a character
        assert_eq!(quote("今天天气很好，去公园吧", "公", "公"), "公");
        assert_eq!(quote("今天天气很好，去公园吧", "很好", "好"), "很好");
        assert_eq!(quote("今天天气很好🙂，去公园吧", "🙂", "园"), "🙂，去公园");
    }
}
