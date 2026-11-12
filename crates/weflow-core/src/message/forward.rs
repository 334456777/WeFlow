//! Chat histories forwarded as one message: their records and their text.

use super::xml::{
    decode_html_entities, decode_html_entities_cow, extract_app_message_type, extract_xml_value,
    normalize_app_message_content, normalize_app_message_cow, rx,
};
use super::ForwardRecord;

pub(super) fn parse_chat_history(content: &str) -> Option<Vec<ForwardRecord>> {
    let normalized = normalize_app_message_cow(content);
    let app_type = extract_app_message_type(&normalized);
    if app_type != "19" && !normalized.contains("<recorditem") {
        return None;
    }
    let mut items: Vec<ForwardRecord> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let dedupe_key = |r: &ForwardRecord| {
        format!(
            "{}|{}|{}|{}|{}",
            r.datatype,
            r.sourcename,
            r.sourcetime,
            r.datadesc.clone().unwrap_or_default(),
            r.datatitle.clone().unwrap_or_default()
        )
    };
    for record in crate::xml::tag_inners(&normalized, "recorditem") {
        for item in parse_forward_container(record) {
            if seen.insert(dedupe_key(&item)) {
                items.push(item);
            }
        }
    }
    if items.is_empty() && normalized.contains("<dataitem") {
        for item in parse_forward_container(&normalized) {
            if seen.insert(dedupe_key(&item)) {
                items.push(item);
            }
        }
    }
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

fn parse_forward_container(container: &str) -> Vec<ForwardRecord> {
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
    let mut items: Vec<ForwardRecord> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for seg in &segments {
        if seg.is_empty() {
            continue;
        }
        for (attrs, body) in crate::xml::elements(seg, "dataitem") {
            if let Some(parsed) = parse_forward_data_item(body, attrs) {
                let key = format!(
                    "{}|{}|{}|{}|{}",
                    parsed.datatype,
                    parsed.sourcename,
                    parsed.sourcetime,
                    parsed.datadesc.clone().unwrap_or_default(),
                    parsed.datatitle.clone().unwrap_or_default()
                );
                if seen.insert(key) {
                    items.push(parsed);
                }
            }
        }
    }
    if !items.is_empty() {
        return items;
    }
    parse_forward_data_item(container, "").into_iter().collect()
}

fn parse_forward_data_item(body: &str, attrs: &str) -> Option<ForwardRecord> {
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
        Some(parse_forward_container(&nested_xml)).filter(|l| !l.is_empty())
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
    Some(ForwardRecord {
        datatype,
        sourcename,
        sourcetime,
        datadesc: opt(datadesc),
        datatitle: opt(datatitle),
        chat_record_title: opt(chat_record_title),
        chat_record_desc: opt(chat_record_desc),
        chat_record_list: nested_list,
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
    match item.datatype {
        3 => "[图片]".into(),
        34 => "[语音消息]".into(),
        43 => "[视频]".into(),
        47 => "[表情]".into(),
        49 | 8 => "[文件]".into(),
        17 => item
            .chat_record_desc
            .clone()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "[聊天记录]".into()),
        _ => "[消息]".into(),
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
    let text = if record.datatype == 49 || record.datatype == 8 {
        let d = record.datadesc.clone().unwrap_or_default();
        let t = record.datatitle.clone().unwrap_or_default();
        if !d.trim().is_empty() {
            d.trim().to_string()
        } else if !t.trim().is_empty() {
            t.trim().to_string()
        } else {
            "[文件]".into()
        }
    } else {
        format_forward_item_text(record)
    };
    vec![format!("{indent}{prefix}{text}")]
}

pub(super) fn format_forward_chat_record_content(content: &str) -> String {
    let normalized = normalize_app_message_content(content);
    let name = ["nickname", "title", "des", "displayname"]
        .iter()
        .map(|t| extract_xml_value(&normalized, t))
        .find(|v| !v.is_empty())
        .unwrap_or_else(|| "聊天记录".into());
    let header = if name.is_empty() {
        "[转发的聊天记录]".to_string()
    } else {
        format!("[转发的聊天记录]{name}")
    };
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
}
