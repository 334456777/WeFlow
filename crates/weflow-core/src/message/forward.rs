//! 合并转发的聊天记录：其中的条目和文字。

use super::xml::{
    decode_html_entities_cow, decode_text_entities, extract_app_message_type, extract_xml_value,
    normalize_app_message_content, normalize_app_message_cow, rx, strip_cdata,
};
use super::ForwardRecord;
use std::collections::HashSet;

/// 聊天记录中一个条目的种类，按聊天记录自己的 `datatype` 编号，与消息的 `local_type` 不同：
/// 导出文字、ChatLab 和 HTML 都按这一张表读（#1171）。
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
    /// `datatype` 对应的条目种类。
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

    /// 这种条目没有自己的文字时，导出文字中代替它的标签。
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

/// 聊天记录的一个条目和它的 `dataid`，`dataid` 用来区分文字相同的两个条目。
struct Item {
    id: Option<String>,
    record: ForwardRecord,
}

/// `found` 中 `dataid` 还不在 `seen` 里的条目，保持原来的顺序；没有 `dataid` 的条目都保留，
/// 因为文字相同的两个这样的条目（一分钟内发了两次的同一句话、两张图片）仍是两条（#1171）。
fn new_items(
    found: Vec<Item>,
    seen: &mut HashSet<String>,
) -> impl Iterator<Item = ForwardRecord> + '_ {
    found
        .into_iter()
        .filter(move |item| item.id.as_ref().is_none_or(|id| seen.insert(id.clone())))
        .map(|item| item.record)
}

/// 聊天记录（`<type>` 19 或带 `<recorditem>` 的消息）中的条目：读每个 `<recorditem>`，`dataid` 相同的只算一条；
/// 都没有读到时直接在内容中找 `<dataitem>`。没有条目时为 `None`。
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

/// 聊天记录（或嵌套的聊天记录）的 `<recordinfo>` 中的条目；`<recordinfo>` 可能是原文、转义过一次的，
/// 或在 CDATA 里。
fn parse_forward_records(container: &str) -> Vec<ForwardRecord> {
    let mut seen = HashSet::new();
    new_items(parse_forward_container(container), &mut seen).collect()
}

/// `container` 中的条目：依次试原文、解码一次实体的、CDATA 里的及其解码一次实体的几种形式，取第一种找到条目的。
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
    // 每种形式都是同一批条目再读一遍：只读第一种找到条目的形式，不把几种混在一起。
    // 再多解码一层的形式还会把嵌套记录中转义的条目当成这一层的条目。
    for seg in &segments {
        // 原样放在条目的 `<recordxml>` 里的嵌套记录属于那个条目，其中的条目不是这一层的（#1174）
        let items: Vec<Item> = crate::xml::outer_elements(seg, "dataitem")
            .filter_map(|(attrs, body)| parse_forward_data_item(body, attrs))
            .collect();
        if !items.is_empty() {
            return items;
        }
    }
    parse_forward_data_item(container, "").into_iter().collect()
}

/// `<dataitem>` 中的一个条目（`body` 是元素的内容，`attrs` 是它的属性）；发送者、文字和标题都没有时为 `None`。
fn parse_forward_data_item(body: &str, attrs: &str) -> Option<Item> {
    let nested_xml = crate::xml::outer_elements(body, "recordxml")
        .next()
        .map(|(_, inner)| strip_cdata(inner))
        .unwrap_or_default();
    // 条目自己的字段，不是嵌套在其中的记录的条目的字段（#1174）
    let body = crate::xml::without_outer_elements(body, "recordxml");
    let body = body.as_ref();
    let datatype_raw = rx(r#"(?i)datatype\s*=\s*["']?(\d+)["']?"#)
        .captures(attrs)
        .map(|c| c[1].to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| Some(extract_xml_value(body, "datatype")).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "0".into());
    let datatype = datatype_raw.parse::<i64>().unwrap_or(0);
    // 数字实体（`&#x0A;`）也在同一遍里解码（#1199）
    let text = |s: &str| decode_text_entities(s).into_owned();
    let sourcename = text(&extract_xml_value(body, "sourcename"));
    let sourcetime = extract_xml_value(body, "sourcetime");
    let mut datadesc = extract_xml_value(body, "datadesc");
    if datadesc.is_empty() {
        datadesc = extract_xml_value(body, "content");
    }
    let datadesc = text(&datadesc);
    let datatitle = text(&extract_xml_value(body, "datatitle"));
    let nested_list = if datatype == 17 && !nested_xml.is_empty() {
        Some(parse_forward_records(&nested_xml)).filter(|l| !l.is_empty())
    } else {
        None
    };
    let pick = |a: String, b: &str| if !a.is_empty() { a } else { b.to_string() };
    let chat_record_title = text(&pick(
        if nested_xml.is_empty() {
            String::new()
        } else {
            extract_xml_value(&nested_xml, "title")
        },
        &datatitle,
    ));
    let chat_record_desc = text(&pick(
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

/// 条目在导出文字中的文字（不含发送者）：依次取文字、标题；都没有时嵌套的聊天记录取描述，其余写种类的标签。
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

/// 条目在导出文字中的行 `<发送者>: <文字>`；嵌套的聊天记录先写一行 `<发送者>: [转发的聊天记录]<标题>`，
/// 其中的条目各缩进一级（两个空格，最多 8 级）。
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

/// 聊天记录消息的导出文字：第一行是 `[转发的聊天记录]` 加标题（依次取 `<nickname>`、`<title>`、`<des>`、
/// `<displayname>`），下面每个条目一行。
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
        // 条目的文字中的数字实体也在同一遍里解码（#1199）
        let escaped = xml
            .replace(">Bob<", ">B&amp;b<")
            .replace(">hello<", ">a&#x0A;b &amp;amp;<");
        assert_eq!(
            format_forward_chat_record_content(&escaped),
            "[转发的聊天记录]群聊的聊天记录\nB&b: a\nb &amp;"
        );
    }

    /// 微信保存的嵌套记录：它的 XML 原样放在条目的 `<recordxml>` 里，位于外层记录的 CDATA 中、条目自己的字段之前，
    /// 这里嵌套两层。每个记录只有自己的条目，每个条目只有自己的发送者（#1174）。
    #[test]
    fn a_nested_record_keeps_its_own_items() {
        let info = |title: &str, items: &str| {
            format!("<recordxml><recordinfo><title>{title}</title><datalist>{items}</datalist></recordinfo></recordxml>")
        };
        let said = |id: &str, who: &str, text: &str| {
            format!(
                "<dataitem datatype=\"1\" dataid=\"{id}\"><sourcename>{who}</sourcename><datadesc>{text}</datadesc>\
                 </dataitem>"
            )
        };
        let deepest = info("最里面", &said("d1", "Dan", "三"));
        let inner = info(
            "群聊",
            &format!(
                "{}<dataitem datatype=\"17\" dataid=\"i2\">{deepest}<sourcename>Carol</sourcename>\
                 <datatitle>最里面</datatitle></dataitem>",
                said("i1", "Carol", "一")
            ),
        );
        let xml = record_of(&[
            (
                "datatype=\"1\" dataid=\"o1\"",
                "<sourcename>Bob</sourcename><datadesc>外面</datadesc>",
            ),
            (
                "datatype=\"17\" dataid=\"o2\"",
                &format!("{inner}<sourcename>Bob</sourcename><datatitle>群聊</datatitle>"),
            ),
        ]);
        let items = parse_chat_history(&xml).unwrap();
        assert_eq!(items.len(), 2, "{items:?}");
        assert_eq!(items[1].sourcename, "Bob");
        let nested = items[1].chat_record_list.as_deref().unwrap();
        assert_eq!(nested.len(), 2, "{nested:?}");
        assert_eq!(nested[1].sourcename, "Carol");
        assert_eq!(
            nested[1].chat_record_list.as_deref().map(<[_]>::len),
            Some(1)
        );
        assert_eq!(
            format_forward_chat_record_content(&xml),
            "[转发的聊天记录]聊天记录\nBob: 外面\nBob: [转发的聊天记录]群聊\n  Carol: 一\n  Carol: [转发的聊天记录]最里面\n    Dan: 三"
        );
    }

    /// 由 `items`（`<dataitem>` 的属性和内容）组成的聊天记录，像微信一样放在 CDATA 里。
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

    /// Bob 在一分钟内发的八个条目，其中两条文字相同、两张是图片：
    /// 八个都要读到，各带自己的 `datatype` 的标签（#1171）。
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
        // 没有 `dataid` 时，文字相同的两个条目仍是两条
        let text = format!("{BOB}<datadesc>收到</datadesc>");
        let xml = record_of(&[("datatype=\"1\"", &text), ("datatype=\"1\"", &text)]);
        assert_eq!(parse_chat_history(&xml).map(|l| l.len()), Some(2));
    }

    /// 没有自己的文字的条目，标签按聊天记录的 `datatype` 编号，而不是消息的 `local_type`（#1171）。
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

    /// 记录中只出现一次的条目，即使按容器的几种形式（原文、解码实体后、CDATA 里）读到，也只算一条；
    /// 两个 `<recorditem>` 中 `dataid` 相同的条目同样只算一条（#1171）。
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
