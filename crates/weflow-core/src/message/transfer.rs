//! Transfers: their text and their two sides.

use super::xml::{extract_xml_value, normalize_app_message_content};

/// Whether two identifiers are the same person, by the database layer's rule
/// ([`weflow_native::db::same_identity`]): case-insensitive, and a bare wxid matches its account folder name
/// (`wxid_abc` and `wxid_abc_1a2b`), so the export and the database agree on who sent what.
pub fn is_same_wxid(lhs: &str, rhs: &str) -> bool {
    weflow_native::db::same_identity(lhs, rhs)
}

pub(super) fn get_transfer_prefix(
    content: &str,
    my_wxid: Option<&str>,
    sender_wxid: Option<&str>,
) -> &'static str {
    let normalized = normalize_app_message_content(content);
    if normalized.is_empty() {
        return "[转账]";
    }
    match extract_xml_value(&normalized, "paysubtype").as_str() {
        "3" => return "[转账收款]",
        "1" => return "[转账]",
        _ => {}
    }
    let payer = extract_xml_value(&normalized, "payer_username");
    let receiver = extract_xml_value(&normalized, "receiver_username");
    if let Some(sender) = sender_wxid.filter(|s| !s.is_empty()) {
        let is_payer = is_same_wxid(sender, &payer);
        let is_receiver = is_same_wxid(sender, &receiver);
        if is_receiver && !is_payer {
            return "[转账]";
        }
        if is_payer && !is_receiver {
            return "[转账收款]";
        }
    }
    if let Some(my) = my_wxid.filter(|s| !s.is_empty()) {
        if is_same_wxid(my, &receiver) {
            return "[转账]";
        }
        if is_same_wxid(my, &payer) {
            return "[转账收款]";
        }
    }
    "[转账]"
}

pub fn is_transfer_export_content(content: &str) -> bool {
    content.starts_with("[转账]") || content.starts_with("[转账收款]")
}

pub fn append_transfer_desc(content: &str, desc: &str) -> String {
    let prefix = if content.starts_with("[转账收款]") {
        "[转账收款]"
    } else {
        "[转账]"
    };
    content.replacen(prefix, &format!("{prefix} ({desc})"), 1)
}

/// Payer/receiver usernames of a transfer message, when the XML has both.
pub fn transfer_parties(content: &str) -> Option<(String, String)> {
    let normalized = normalize_app_message_content(content);
    if normalized.is_empty() {
        return None;
    }
    let xml_type = extract_xml_value(&normalized, "type");
    if !xml_type.is_empty() && xml_type != "2000" {
        return None;
    }
    let payer = extract_xml_value(&normalized, "payer_username");
    let receiver = extract_xml_value(&normalized, "receiver_username");
    if payer.is_empty() || receiver.is_empty() {
        return None;
    }
    Some((payer, receiver))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::parse_message_content;

    #[test]
    fn transfer_texts() {
        let xml = "<msg><appmsg><title>转账</title><type>2000</type><wcpayinfo><feedesc>￥1.00</feedesc><paysubtype>1</paysubtype></wcpayinfo></appmsg></msg>";
        assert_eq!(
            parse_message_content(xml, 49, None, None, None).unwrap(),
            "[转账] ￥1.00"
        );
        assert_eq!(
            append_transfer_desc("[转账] ￥1.00", "A 转账给 B"),
            "[转账] (A 转账给 B) ￥1.00"
        );
        let xml2 = "<appmsg><type>2000</type><paysubtype>3</paysubtype></appmsg>";
        assert_eq!(get_transfer_prefix(xml2, None, None), "[转账收款]");
    }
}
