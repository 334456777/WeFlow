//! 转账：文字和收付双方。

use super::xml::{extract_xml_value, normalize_app_message_content, xml_text};

/// 两个标识是否为同一个人，按只读数据库层的规则（[`weflow_native::db::same_identity`]）。
///
/// 不区分大小写，裸 wxid 与它的账号目录名（`wxid_abc` 与 `wxid_abc_1a2b`）算同一个，
/// 所以导出与数据库层对谁发了哪条消息的判断一致。
pub fn is_same_wxid(lhs: &str, rhs: &str) -> bool {
    weflow_native::db::same_identity(lhs, rhs)
}

/// 转账文字开头的方向：`[转账]` 或 `[转账收款]`。取自 `paysubtype`（1 为转账，3 为收款）；
/// 没有时按发送者是收款方还是付款方判断，再按本账号；都判断不出时为 `[转账]`。
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

/// 转账的文字，导出与聊天窗口相同（#1185）：方向（[`get_transfer_prefix`]），
/// 然后是金额（`<feedesc>`）和备注（`<pay_memo>`）；没有金额时只写方向。
pub(crate) fn transfer_text(
    content: &str,
    my_wxid: Option<&str>,
    sender_wxid: Option<&str>,
) -> String {
    let normalized = normalize_app_message_content(content);
    let feedesc = xml_text(&normalized, "feedesc");
    let memo = xml_text(&normalized, "pay_memo");
    let prefix = get_transfer_prefix(&normalized, my_wxid, sender_wxid);
    match (feedesc.is_empty(), memo.is_empty()) {
        (true, _) => prefix.to_string(),
        (false, true) => format!("{prefix} {feedesc}"),
        (false, false) => format!("{prefix} {feedesc} {memo}"),
    }
}

/// 导出的消息文字是否为转账的：以 `[转账]` 或 `[转账收款]` 开头。
pub fn is_transfer_export_content(content: &str) -> bool {
    content.starts_with("[转账]") || content.starts_with("[转账收款]")
}

/// 在转账文字的 `[转账]` 或 `[转账收款]` 后面加上括在括号里的 `desc`。
pub fn append_transfer_desc(content: &str, desc: &str) -> String {
    let prefix = if content.starts_with("[转账收款]") {
        "[转账收款]"
    } else {
        "[转账]"
    };
    content.replacen(prefix, &format!("{prefix} ({desc})"), 1)
}

/// 转账消息的付款方和收款方的用户名；XML 中缺少其中一个，或 `<type>` 不是 2000 时为 `None`。
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
