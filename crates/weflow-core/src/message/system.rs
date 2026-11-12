//! System messages: revokes, templates with their names filled in, and calls (VoIP).

use super::xml::{
    extract_xml_value, normalize_app_message_content, normalize_app_message_cow, rx,
    sender_prefix_len, strip_sender_prefix,
};

pub(super) struct RevokeInfo {
    pub is_revoke: bool,
    pub is_self_revoke: bool,
    pub revoker_wxid: Option<String>,
}

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

pub(super) fn is_readable_system_message(local_type: i64, content: &str) -> bool {
    local_type == 10000 || has_sysmsg(&normalize_app_message_cow(content))
}

/// Whether normalized content opens a `<sysmsg>` after its sender prefix (see [`is_readable_system_message`]).
pub(super) fn has_sysmsg(normalized: &str) -> bool {
    crate::xml::has_open(&normalized[sender_prefix_len(normalized)..], "sysmsg")
}

pub fn clean_system_message(content: &str) -> String {
    if content.is_empty() {
        return "[系统消息]".into();
    }
    let mut content = content.to_string();
    if let Some(c) = rx(r"(?is)<sysmsg[^>]*>(.*?)</sysmsg>").captures(&content.clone()) {
        content = c[1].to_string();
    }
    // a revoke as the database keeps it: `<revokemsg><content>…</content><revoketime>…</revoketime></revokemsg>`,
    // at times with more elements after it. Only the content is the notice
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
        let replaced = rx(r"\$\{([^}]+)\}").replace_all(&template, |caps: &regex::Captures| {
            let name = regex::escape(&caps[1]);
            let pat = format!(r"(?i)<{0}><!\[CDATA\[([^\]]*)\]\]></{0}>", name);
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

/// The template of a `<sysmsgtemplate>` notice (`"$username$"邀请"$names$"加入了群聊`) with each `$name$` filled from
/// its `<link name="name">` in `source`: the nicknames of the link's members, joined by its separator. A link without
/// members is something to click in WeChat (`$revoke$`), and leaves nothing.
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

    #[test]
    fn system_messages() {
        let xml = "<sysmsg type=\"revokemsg\"><revokemsg><replacemsg><![CDATA[\"Bob\" 撤回了一条消息]]></replacemsg></revokemsg></sysmsg>";
        assert_eq!(clean_system_message(xml), "\"Bob\" 撤回了一条消息");
        // as the database keeps a revoke: the time and what follows it are not part of the notice
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
        // a notice written as a template names its people in the links after it
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
