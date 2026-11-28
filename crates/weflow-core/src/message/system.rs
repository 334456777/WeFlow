//! 系统消息：撤回、填上名字的模板、拍一拍和通话（VoIP）。

use super::xml::{
    extract_xml_value, normalize_app_message_content, rx, sender_prefix_len, strip_sender_prefix,
};
use crate::chat_msg::clean_utf16;

/// 系统消息是否为撤回提示，以及撤回者。
pub(super) struct RevokeInfo {
    /// 是否为撤回提示。
    pub is_revoke: bool,
    /// 是否为账号自己撤回的（「你撤回了……」）。
    pub is_self_revoke: bool,
    /// 别人撤回时的撤回者；读不出时为 `None`。
    pub revoker_wxid: Option<String>,
}

/// 读出撤回提示和撤回者：内容含 `revokemsg` 或「撤回」时算撤回提示，含「你撤回」的是账号自己撤回的；
/// 别人撤回时，撤回者取像用户名的 `<session>`，没有时取 `<fromusername>`。
pub(super) fn extract_revoker_info(content: &str) -> RevokeInfo {
    let none = RevokeInfo {
        is_revoke: false,
        is_self_revoke: false,
        revoker_wxid: None,
    };
    if content.is_empty() || (!content.contains("revokemsg") && !content.contains("撤回")) {
        return none;
    }
    if content.contains("你撤回") {
        return RevokeInfo {
            is_revoke: true,
            is_self_revoke: true,
            revoker_wxid: None,
        };
    }
    if let Some(c) = rx(r"(?i)<session>([^<]+)</session>").captures(content) {
        let session = c[1].trim().to_string();
        if session.starts_with("wxid_") || rx(r"^[a-zA-Z][a-zA-Z0-9_-]+$").is_match(&session) {
            return RevokeInfo {
                is_revoke: true,
                is_self_revoke: false,
                revoker_wxid: Some(session),
            };
        }
    }
    if let Some(c) = rx(r"(?i)<fromusername>([^<]+)</fromusername>").captures(content) {
        return RevokeInfo {
            is_revoke: true,
            is_self_revoke: false,
            revoker_wxid: Some(c[1].trim().to_string()),
        };
    }
    RevokeInfo {
        is_revoke: true,
        is_self_revoke: false,
        revoker_wxid: None,
    }
}

/// 测试中与 [`MsgXml::readable_system_once`](super::xml::MsgXml::readable_system_once) 对照的参考实现。
#[cfg(test)]
pub(super) fn is_readable_system_message(local_type: i64, content: &str) -> bool {
    local_type == 10000 || has_sysmsg(&super::xml::normalize_app_message_cow(content))
}

/// 规范化后的内容在发送者前缀之后是否有 `<sysmsg>` 的开始标签。
pub(super) fn has_sysmsg(normalized: &str) -> bool {
    crate::xml::has_open(&normalized[sender_prefix_len(normalized)..], "sysmsg")
}

/// 系统消息给人看的文字，依次取：`<revokemsg>` 的 `<content>`；`<replacemsg>` 的 CDATA；
/// `<template>`，其中的名字按 [`fill_template_links`] 填上；`<title>`；去掉全部标签后的文字，为空时是 `[系统消息]`。
pub fn clean_system_message(content: &str) -> String {
    if content.is_empty() {
        return "[系统消息]".into();
    }
    let mut content = content.to_string();
    if let Some(c) = rx(r"(?is)<sysmsg[^>]*>(.*?)</sysmsg>").captures(&content.clone()) {
        content = c[1].to_string();
    }
    // 数据库中保存的撤回：`<revokemsg><content>…</content><revoketime>…</revoketime></revokemsg>`，
    // 后面有时还有别的元素；只有 `<content>` 是提示的文字
    if let Some((_, revoke)) = crate::xml::elements(&content, "revokemsg").next() {
        let notice = extract_xml_value(revoke, "content");
        if !notice.is_empty() {
            return notice;
        }
    }
    if let Some(c) = rx(r"(?i)<replacemsg><!\[CDATA\[(.*?)\]\]></replacemsg>").captures(&content) {
        return c[1].trim().to_string();
    }
    if let Some(c) = rx(r"(?i)<template><!\[CDATA\[(.*?)\]\]></template>").captures(&content) {
        let template = fill_template_links(&c[1], &content);
        let source = content.clone();
        let replaced = rx(r"\$\{([^}]+)\}").replace_all(&template, |caps: &regex::Captures<'_>| {
            let name = regex::escape(&caps[1]);
            let pat = format!(r"(?i)<{name}><!\[CDATA\[([^\]]*)\]\]></{name}>");
            rx(&pat)
                .captures(&source)
                .map(|m| m[1].to_string())
                .unwrap_or_default()
        });
        return rx(r"<[^>]+>").replace_all(&replaced, "").trim().to_string();
    }
    if let Some(c) = rx(r"(?is)<title>(.*?)</title>").captures(&content) {
        let title = c[1]
            .replace("<![CDATA[", "")
            .replace("]]>", "")
            .trim()
            .to_string();
        if !title.is_empty() {
            return title;
        }
    }
    content = content.replace("<![CDATA[", "").replace("]]>", "");
    let stripped = rx(r"(?i)<img[^>]*>").replace_all(&content, "").to_string();
    let stripped = rx(r"</?[a-zA-Z0-9_:]+[^>]*>")
        .replace_all(&stripped, "")
        .to_string();
    let stripped = rx(r"\s+").replace_all(&stripped, " ").trim().to_string();
    if stripped.is_empty() {
        "[系统消息]".into()
    } else {
        stripped
    }
}

/// 拍一拍（`(62 << 32) | 49`）的文字，聊天窗口与导出相同（#1196）：把保存的整条消息 `content` 的实体解码一次，
/// 再取 `<title>`；没有标题时取「拍了拍」那一句，都没有时为「拍一拍」。
pub(crate) fn pat_text(content: &str) -> String {
    clean_pat_message(&clean_utf16(&crate::xml::decode_entities(content)))
}

/// 对已经解码了实体的内容求 [`pat_text`]。
fn clean_pat_message(content: &str) -> String {
    if content.is_empty() {
        return "拍一拍".into();
    }
    if let Some(c) = rx(r"(?is)<title>(.*?)</title>").captures(content) {
        let title = c[1]
            .replace("<![CDATA[", "")
            .replace("]]>", "")
            .trim()
            .to_string();
        if !title.is_empty() {
            return title;
        }
    }
    if let Some(c) = rx(r"^(.+?拍了拍.+?)(?:[\r\n]|$|ງ|wxid_)").captures(content) {
        return c[1].trim().to_string();
    }
    let mut cleaned = rx(r"wxid_[a-zA-Z0-9_-]+")
        .replace_all(content, "")
        .to_string();
    cleaned = rx(r"[ງ໐໓ຖiht]+").replace_all(&cleaned, " ").to_string();
    cleaned = rx(r"\d{6,}").replace_all(&cleaned, "").to_string();
    cleaned = rx(r"\s+").replace_all(&cleaned, " ").trim().to_string();
    cleaned = clean_utf16(&cleaned);
    if cleaned.chars().count() > 1 && !cleaned.contains("xml") {
        return cleaned;
    }
    "拍一拍".into()
}

/// 把 `<sysmsgtemplate>` 提示的模板（`"$username$"邀请"$names$"加入了群聊`）中的每个 `$名字$` 换成 `source` 中
/// `<link name="名字">` 的成员昵称，用这个 link 的 `<separator>` 连接，没有时用 `、`。
/// 没有成员的 link 是微信里可以点的按钮（如 `$revoke$`），换成空。
fn fill_template_links(template: &str, source: &str) -> String {
    let mut text = template.to_string();
    for (attributes, body) in crate::xml::elements(source, "link") {
        let Some(name) = rx(r#"(?i)\bname\s*=\s*"([^"]+)""#).captures(attributes) else {
            continue;
        };
        let placeholder = format!("${}$", &name[1]);
        if !text.contains(&placeholder) {
            continue;
        }
        let nicknames: Vec<String> = crate::xml::elements(body, "member")
            .map(|(_, member)| extract_xml_value(member, "nickname"))
            .filter(|nickname| !nickname.is_empty())
            .collect();
        let separator = match extract_xml_value(body, "separator") {
            s if s.is_empty() => "、".to_string(),
            s => s,
        };
        text = text.replace(&placeholder, &nicknames.join(&separator));
    }
    text
}

/// 可读的系统消息（带 `<sysmsg>` 的消息）的文字：`<sysmsg>` 中的 `<plain>`，没有时取 `<text>`；去掉发送者前缀，
/// 连续的空白并成一个空格。
pub fn extract_readable_system_message_text(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let normalized = normalize_app_message_content(content);
    let stripped = strip_sender_prefix(&normalized);
    let source = crate::xml::elements(&stripped, "sysmsg")
        .next()
        .map_or_else(|| normalized.clone(), |(_, body)| body.to_string());
    let mut text = extract_xml_value(&source, "plain");
    if text.is_empty() {
        text = extract_xml_value(&source, "text");
    }
    let text = strip_sender_prefix(&text);
    rx(r"\s+").replace_all(&text, " ").trim().to_string()
}

/// 通话消息（`local_type` 50）的文字 `[<通话种类>] <时长或状态>`：`room_type` 为 0 是视频通话，为 1 是语音通话；
/// 接通的写时长，没接通的写状态。
pub fn parse_voip_message(content: &str) -> String {
    if content.is_empty() {
        return "[通话]".into();
    }
    let msg = rx(r"(?i)<msg><!\[CDATA\[(.*?)\]\]></msg>")
        .captures(content)
        .map(|c| c[1].trim().to_string())
        .unwrap_or_default();
    let room_type = rx(r"(?i)<room_type>(\d+)</room_type>")
        .captures(content)
        .and_then(|c| c[1].parse::<i64>().ok())
        .unwrap_or(-1);
    let call_type = match room_type {
        0 => "视频通话",
        1 => "语音通话",
        _ => "通话",
    };
    if msg.contains("通话时长") {
        let duration = rx(r"(?i)通话时长\s*(\d{1,2}:\d{2}(?::\d{2})?)")
            .captures(&msg)
            .map(|c| c[1].to_string())
            .unwrap_or_default();
        return if duration.is_empty() {
            format!("[{call_type}] 已接听")
        } else {
            format!("[{call_type}] {duration}")
        };
    }
    let status = if msg.contains("对方无应答") {
        Some("对方无应答")
    } else if msg.contains("已取消") {
        Some("已取消")
    } else if msg.contains("已在其它设备接听") || msg.contains("已在其他设备接听") {
        Some("已在其他设备接听")
    } else if msg.contains("对方已拒绝") || msg.contains("已拒绝") {
        Some("对方已拒绝")
    } else if msg.contains("忙线未接听") || msg.contains("忙线") {
        Some("忙线未接听")
    } else if msg.contains("未接听") {
        Some("未接听")
    } else {
        None
    };
    if let Some(s) = status {
        return format!("[{call_type}] {s}");
    }
    if !msg.is_empty() {
        return format!("[{call_type}] {msg}");
    }
    format!("[{call_type}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voip_parsing() {
        let xml = "<voipmsg><VoIPBubbleMsg><msg><![CDATA[通话时长 01:23]]></msg><room_type>1</room_type></VoIPBubbleMsg></voipmsg>";
        assert_eq!(parse_voip_message(xml), "[语音通话] 01:23");
        let xml = "<voipmsg><VoIPBubbleMsg><msg><![CDATA[对方无应答]]></msg><room_type>0</room_type></VoIPBubbleMsg></voipmsg>";
        assert_eq!(parse_voip_message(xml), "[视频通话] 对方无应答");
    }

    /// 拍一拍的标题带转义的实体，解码一次后读出（#1196）。
    #[test]
    fn a_pat_is_its_title_decoded_once() {
        let xml = "<msg><appmsg><title>&quot;A&amp;B&quot; 拍了拍 &quot;C&quot;</title><type>62</type></appmsg></msg>";
        assert_eq!(pat_text(xml), "\"A&B\" 拍了拍 \"C\"");
        assert_eq!(pat_text(""), "拍一拍");
    }

    #[test]
    fn system_messages() {
        let xml = "<sysmsg type=\"revokemsg\"><revokemsg><replacemsg><![CDATA[\"Bob\" 撤回了一条消息]]></replacemsg></revokemsg></sysmsg>";
        assert_eq!(clean_system_message(xml), "\"Bob\" 撤回了一条消息");
        // 数据库中保存的撤回：撤回时间和它后面的内容不属于提示
        for more in [
            "",
            "<extcommoninfo><groupinfo><type>1</type><id>0a1b-2c3d</id><count>2</count></groupinfo>\
             <media_expire_at>1700000000</media_expire_at></extcommoninfo>",
        ] {
            let xml = format!(
                "<?xml version=\"1.0\"?><sysmsg type=\"revokemsg\"><revokemsg><content>\"Bob\" 撤回了一条消息</content>\
                 <revoketime>0</revoketime></revokemsg>{more}</sysmsg>"
            );
            assert_eq!(clean_system_message(&xml), "\"Bob\" 撤回了一条消息");
        }
        // 写成模板的提示：其中的人名在后面的 link 里
        let member = |id: &str, name: &str| {
            format!("<member><username><![CDATA[{id}]]></username><nickname><![CDATA[{name}]]></nickname></member>")
        };
        let invite = format!(
            "room@chatroom:\n<sysmsg type=\"sysmsgtemplate\"><sysmsgtemplate><content_template type=\"tmpl_type_profile\">\
             <plain><![CDATA[]]></plain><template><![CDATA[\"$username$\"邀请\"$names$\"加入了群聊  $revoke$]]></template>\
             <link_list><link name=\"username\" type=\"link_profile\"><memberlist>{}</memberlist></link>\
             <link name=\"names\" type=\"link_profile\"><memberlist>{}{}</memberlist>\
             <separator><![CDATA[、]]></separator></link>\
             <link name=\"revoke\" type=\"link_revoke\"><title><![CDATA[撤销]]></title></link></link_list>\
             </content_template></sysmsgtemplate></sysmsg>",
            member("wxid_a", "Alice"),
            member("wxid_b", "Bob"),
            member("wxid_c", "Carol"),
        );
        assert_eq!(
            clean_system_message(&invite),
            "\"Alice\"邀请\"Bob、Carol\"加入了群聊"
        );
        assert!(is_readable_system_message(10000, "x"));
        let rv = extract_revoker_info("你撤回了一条消息");
        assert!(rv.is_revoke && rv.is_self_revoke);
    }
}
