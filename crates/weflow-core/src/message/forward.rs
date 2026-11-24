//! Chat histories forwarded as one message: their records and their text.

use super::xml::{
    decode_html_entities, decode_html_entities_cow, extract_app_message_type, extract_xml_value,
    normalize_app_message_content, normalize_app_message_cow, rx,
};
use super::ForwardRecord;
use std::collections::HashSet;

/// What an item of a chat record is, by the record's own `datatype` numbers, which are not a message's
/// `local_type`: the one table the exports' text, ChatLab and HTML read them by (#1171).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecordKind {
    Text,
    Image,
    Voice,
    Video,
    Link,
    File,
    ChatRecord,
    MiniProgram,
    Sticker,
    Other,
}

impl RecordKind {
    /// The kind of an item of `datatype`.
    pub(crate) fn of(datatype: i64) -> Self {
        match datatype {
            1 => Self::Text,
            2 => Self::Image,
            3 => Self::Voice,
            4 => Self::Video,
            5 => Self::Link,
            8 => Self::File,
            17 => Self::ChatRecord,
            19 => Self::MiniProgram,
            37 => Self::Sticker,
            _ => Self::Other,
        }
    }

    /// The text standing in for an item of this kind that has none of its own, as the exports' text writes it.
    pub(crate) fn tag(self) -> &'static str {
        match self {
            Self::Image => "[图片]",
            Self::Voice => "[语音消息]",
            Self::Video => "[视频]",
            Self::Link => "[链接]",
            Self::File => "[文件]",
            Self::ChatRecord => "[聊天记录]",
            Self::MiniProgram => "[小程序]",
            Self::Sticker => "[表情]",
            Self::Text | Self::Other => "[消息]",
        }
    }
}

/// An item of a chat record and its `dataid`, which names it apart from another item with the same words.
struct Item {
    id: Option<String>,
    record: ForwardRecord,
}

/// The items of `found` whose `dataid` is not in `seen` yet, in order; an item without one is always kept, since
/// two such items with the same words (the same text sent twice in a minute, two pictures) are still two (#1171).
fn new_items(
    found: Vec<Item>,
    seen: &mut HashSet<String>,
) -> impl Iterator<Item = ForwardRecord> + '_ {
    found
        .into_iter()
        .filter(move |item| item.id.as_ref().is_none_or(|id| seen.insert(id.clone())))
        .map(|item| item.record)
}

pub(super) fn parse_chat_history(content: &str) -> Option<Vec<ForwardRecord>> {
    let normalized = normalize_app_message_cow(content);
    let app_type = extract_app_message_type(&normalized);
    if app_type != "19" && !normalized.contains("<recorditem") {
        return None;
    }
    let mut items: Vec<ForwardRecord> = Vec::new();
    let mut seen = HashSet::new();
    for record in crate::xml::tag_inners(&normalized, "recorditem") {
        items.extend(new_items(parse_forward_container(record), &mut seen));
    }
    if items.is_empty() && normalized.contains("<dataitem") {
        items.extend(new_items(parse_forward_container(&normalized), &mut seen));
    }
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

/// The records of a chat record's `<recordinfo>` (or of a nested one), which may be kept as text, escaped once or
/// inside a CDATA section.
fn parse_forward_records(container: &str) -> Vec<ForwardRecord> {
    let mut seen = HashSet::new();
    new_items(parse_forward_container(container), &mut seen).collect()
}

fn parse_forward_container(container: &str) -> Vec<Item> {
    if container.is_empty() {
        return Vec::new();
    }
    let mut segments = vec![std::borrow::Cow::Borrowed(container)];
    let decoded = decode_html_entities_cow(container);
    if decoded != container {
        segments.push(decoded);
    }
    for inner in crate::xml::between(container, "<![CDATA[", "]]>") {
        if !inner.is_empty() {
            let decoded_inner = decode_html_entities_cow(inner);
            segments.push(std::borrow::Cow::Borrowed(inner));
            if decoded_inner != inner {
                segments.push(decoded_inner);
            }
        }
    }
    // Each form is the same items read once more: the first form that holds any is the one read, never a mix of
    // them. A form decoded further would also show a nested record's escaped items as items of this one.
    for seg in &segments {
        let items: Vec<Item> = crate::xml::elements(seg, "dataitem")
            .filter_map(|(attrs, body)| parse_forward_data_item(body, attrs))
            .collect();
        if !items.is_empty() {
            return items;
        }
    }
    parse_forward_data_item(container, "").into_iter().collect()
}

fn parse_forward_data_item(body: &str, attrs: &str) -> Option<Item> {
    let datatype_raw = rx(r#"(?i)datatype\s*=\s*["']?(\d+)["']?"#)
        .captures(attrs)
        .map(|c| c[1].to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| Some(extract_xml_value(body, "datatype")).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "0".into());
    let datatype = datatype_raw.parse::<i64>().unwrap_or(0);
    let sourcename = decode_html_entities(&extract_xml_value(body, "sourcename"));
    let sourcetime = extract_xml_value(body, "sourcetime");
    let mut datadesc = extract_xml_value(body, "datadesc");
    if datadesc.is_empty() {
        datadesc = extract_xml_value(body, "content");
    }
    let datadesc = decode_html_entities(&datadesc);
    let datatitle = decode_html_entities(&extract_xml_value(body, "datatitle"));
    let nested_xml = extract_xml_value(body, "recordxml");
    let nested_list = if datatype == 17 && !nested_xml.is_empty() {
        Some(parse_forward_records(&nested_xml)).filter(|l| !l.is_empty())
    } else {
        None
    };
    let pick = |a: String, b: &str| if !a.is_empty() { a } else { b.to_string() };
    let chat_record_title = decode_html_entities(&pick(
        if nested_xml.is_empty() {
            String::new()
        } else {
            extract_xml_value(&nested_xml, "title")
        },
        &datatitle,
    ));
    let chat_record_desc = decode_html_entities(&pick(
        if nested_xml.is_empty() {
            String::new()
        } else {
            extract_xml_value(&nested_xml, "desc")
        },
        &datadesc,
    ));
    if sourcename.is_empty() && datadesc.is_empty() && datatitle.is_empty() {
        return None;
    }
    let opt = |s: String| if s.is_empty() { None } else { Some(s) };
    let id = rx(r#"(?i)\bdataid\s*=\s*["']([^"']+)["']"#)
        .captures(attrs)
        .map(|c| c[1].to_string());
    Some(Item {
        id,
        record: ForwardRecord {
            datatype,
            sourcename,
            sourcetime,
            datadesc: opt(datadesc),
            datatitle: opt(datatitle),
            chat_record_title: opt(chat_record_title),
            chat_record_desc: opt(chat_record_desc),
            chat_record_list: nested_list,
        },
    })
}

fn format_forward_item_text(item: &ForwardRecord) -> String {
    let desc = item.datadesc.clone().unwrap_or_default();
    let desc = desc.trim();
    let title = item.datatitle.clone().unwrap_or_default();
    let title = title.trim().to_string();
    if !desc.is_empty() {
        return desc.to_string();
    }
    if !title.is_empty() {
        return title;
    }
    match RecordKind::of(item.datatype) {
        RecordKind::ChatRecord => item
            .chat_record_desc
            .clone()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| RecordKind::ChatRecord.tag().into()),
        kind => kind.tag().into(),
    }
}

fn build_forward_lines(record: &ForwardRecord, depth: usize) -> Vec<String> {
    let indent = if depth > 0 {
        "  ".repeat(depth.min(8))
    } else {
        String::new()
    };
    let prefix = if record.sourcename.is_empty() {
        String::new()
    } else {
        format!("{}: ", record.sourcename)
    };
    if let Some(list) = record.chat_record_list.as_ref().filter(|l| !l.is_empty()) {
        let title = record
            .chat_record_title
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| record.datatitle.clone().filter(|s| !s.is_empty()))
            .or_else(|| record.chat_record_desc.clone().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "聊天记录".into());
        let mut lines = vec![format!("{indent}{prefix}[转发的聊天记录]{title}")];
        for item in list {
            lines.extend(build_forward_lines(item, depth + 1));
        }
        return lines;
    }
    vec![format!(
        "{indent}{prefix}{}",
        format_forward_item_text(record)
    )]
}

pub(super) fn format_forward_chat_record_content(content: &str) -> String {
    let normalized = normalize_app_message_content(content);
    let name = ["nickname", "title", "des", "displayname"]
        .iter()
        .map(|t| extract_xml_value(&normalized, t))
        .find(|v| !v.is_empty())
        .unwrap_or_else(|| "聊天记录".into());
    let header = format!("[转发的聊天记录]{name}");
    match parse_chat_history(&normalized) {
        Some(records) if !records.is_empty() => {
            let lines: Vec<String> = records
                .iter()
                .flat_map(|r| build_forward_lines(r, 0))
                .collect();
            format!("{header}\n{}", lines.join("\n"))
        }
        _ => header,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::kind::convert_message_type_in;
    use crate::message::xml::MsgXml;

    #[test]
    fn forward_chat_records() {
        let xml = "<msg><appmsg><title>群聊的聊天记录</title><type>19</type><recorditem><![CDATA[<recordinfo><datalist><dataitem datatype=\"1\"><sourcename>Bob</sourcename><sourcetime>2024-01-01 10:00:00</sourcetime><datadesc>hello</datadesc></dataitem></datalist></recordinfo>]]></recorditem></appmsg></msg>";
        let text = format_forward_chat_record_content(xml);
        assert_eq!(text, "[转发的聊天记录]群聊的聊天记录\nBob: hello");
        assert_eq!(convert_message_type_in(49, &MsgXml::new(xml)), 7);
    }

    /// A chat record of `items` (`<dataitem>` attributes and body), inside a CDATA section as WeChat keeps it.
    pub(crate) fn record_of(items: &[(&str, &str)]) -> String {
        let list: String = items
            .iter()
            .map(|(attrs, body)| format!("<dataitem {attrs}>{body}</dataitem>"))
            .collect();
        format!(
            "<msg><appmsg><title>聊天记录</title><type>19</type><recorditem><![CDATA[<recordinfo>\
             <datalist count=\"{}\">{list}</datalist></recordinfo>]]></recorditem></appmsg></msg>",
            items.len()
        )
    }

    const BOB: &str = "<sourcename>Bob</sourcename><sourcetime>2024-01-01 10:00</sourcetime>";

    /// Eight items Bob sent in one minute, two of them the same words and two pictures: all eight are read, each
    /// with the tag of its own `datatype` (#1171).
    pub(crate) fn eight_items() -> String {
        let text = format!("{BOB}<datadesc>收到</datadesc>");
        let bare = BOB.to_string();
        let link = format!("{BOB}<datatitle>一篇文章</datatitle>");
        let file = format!("{BOB}<datatitle>f.pdf</datatitle>");
        record_of(&[
            ("datatype=\"1\" dataid=\"a1\"", &text),
            ("datatype=\"1\" dataid=\"a2\"", &text),
            ("datatype=\"2\" dataid=\"a3\"", &bare),
            ("datatype=\"2\" dataid=\"a4\"", &bare),
            ("datatype=\"3\" dataid=\"a5\"", &bare),
            ("datatype=\"4\" dataid=\"a6\"", &bare),
            ("datatype=\"5\" dataid=\"a7\"", &link),
            ("datatype=\"8\" dataid=\"a8\"", &file),
        ])
    }

    #[test]
    fn every_item_of_a_record_is_read_with_its_own_tag() {
        let xml = eight_items();
        assert_eq!(parse_chat_history(&xml).map(|l| l.len()), Some(8));
        assert_eq!(
            format_forward_chat_record_content(&xml),
            "[转发的聊天记录]聊天记录\nBob: 收到\nBob: 收到\nBob: [图片]\nBob: [图片]\nBob: [语音消息]\nBob: [视频]\n\
             Bob: 一篇文章\nBob: f.pdf"
        );
        // without a `dataid`, two items with the same words are still two
        let text = format!("{BOB}<datadesc>收到</datadesc>");
        let xml = record_of(&[("datatype=\"1\"", &text), ("datatype=\"1\"", &text)]);
        assert_eq!(parse_chat_history(&xml).map(|l| l.len()), Some(2));
    }

    /// The tags of the items with no text of their own, by the record's `datatype` numbers rather than a
    /// message's `local_type` (#1171).
    #[test]
    fn an_item_is_told_by_the_record_s_own_numbers() {
        for (datatype, tag) in [
            (2, "[图片]"),
            (3, "[语音消息]"),
            (4, "[视频]"),
            (5, "[链接]"),
            (8, "[文件]"),
            (17, "[聊天记录]"),
            (19, "[小程序]"),
            (37, "[表情]"),
            (34, "[消息]"),
            (43, "[消息]"),
            (47, "[消息]"),
        ] {
            let record = ForwardRecord {
                datatype,
                sourcename: "Bob".into(),
                ..ForwardRecord::default()
            };
            assert_eq!(format_forward_item_text(&record), tag, "{datatype}");
        }
    }

    /// An item that the record holds once is one item, though it is read in several forms of the container: as
    /// it is, with its entities decoded, inside the CDATA section; and so is an item that two `<recorditem>`s hold
    /// with the same `dataid` (#1171).
    #[test]
    fn an_item_read_in_several_forms_is_one() {
        let item = |id: &str| {
            format!(
                "<dataitem datatype=\"1\" dataid=\"{id}\">{BOB}<datadesc>a &amp; b</datadesc></dataitem>"
            )
        };
        let record = format!(
            "<msg><appmsg><type>19</type><recorditem><![CDATA[<recordinfo><datalist>{}{}</datalist></recordinfo>]]>\
             </recorditem></appmsg></msg>",
            item("x"),
            item("y")
        );
        let items = parse_chat_history(&record).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].datadesc.as_deref(), Some("a & b"));
        let twice = format!(
            "<msg><appmsg><type>19</type><recorditem>{}</recorditem><recorditem>{}</recorditem></appmsg></msg>",
            item("x"),
            item("x")
        );
        assert_eq!(parse_chat_history(&twice).map(|l| l.len()), Some(1));
    }
}
