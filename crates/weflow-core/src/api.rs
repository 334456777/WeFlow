//! Pure logic of the HTTP API (`electron/services/httpService.ts`): parameter parsing,
//! ChatLab type mapping, quote extraction and the API message shape.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::chat_msg::ChatMessage;
use crate::message::rx;

/// An HTTP error reply (`{ "error": message }` with a status code).
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl ApiError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }
}

// ───────────────────────── ChatLab types ─────────────────────────

pub mod chatlab {
    pub const TEXT: i64 = 0;
    pub const IMAGE: i64 = 1;
    pub const VOICE: i64 = 2;
    pub const VIDEO: i64 = 3;
    pub const FILE: i64 = 4;
    pub const EMOJI: i64 = 5;
    pub const LINK: i64 = 7;
    pub const LOCATION: i64 = 8;
    pub const RED_PACKET: i64 = 20;
    pub const TRANSFER: i64 = 21;
    pub const POKE: i64 = 22;
    pub const CALL: i64 = 23;
    pub const SHARE: i64 = 24;
    pub const REPLY: i64 = 25;
    pub const FORWARD: i64 = 26;
    pub const CONTACT: i64 = 27;
    pub const SYSTEM: i64 = 80;
    pub const RECALL: i64 = 81;
    pub const OTHER: i64 = 99;
}

// ───────────────────────── parameters ─────────────────────────

pub type Params = HashMap<String, String>;

/// JS `parseInt(value, 10)`, NaN → `None`.
pub fn js_parse_int(s: &str) -> Option<i64> {
    rx(r"^\s*[+-]?\d+").find(s).and_then(|m| m.as_str().trim().parse().ok())
}

pub fn parse_int_param(value: Option<&str>, default: i64, min: i64, max: i64) -> i64 {
    match value.and_then(js_parse_int) {
        Some(n) => n.clamp(min, max),
        None => default,
    }
}

pub fn parse_bool_param(params: &Params, keys: &[&str], default: bool) -> bool {
    for key in keys {
        let Some(raw) = params.get(*key) else { continue };
        match raw.trim().to_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => return true,
            "0" | "false" | "no" | "off" => return false,
            _ => {}
        }
    }
    default
}

pub fn parse_string_list_param(value: Option<&str>) -> Option<Vec<String>> {
    let value = value.filter(|v| !v.is_empty())?;
    let mut seen = std::collections::HashSet::new();
    let list: Vec<String> = value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).filter(|s| seen.insert(s.clone())).collect();
    (!list.is_empty()).then_some(list)
}

/// `parseTimeParam`: `YYYYMMDD` (local time; end → 23:59:59) or a unix timestamp in s / ms.
pub fn parse_time_param(param: Option<&str>, is_end: bool) -> i64 {
    use chrono::{Local, NaiveDate, TimeZone};
    let Some(p) = param.filter(|p| !p.is_empty()) else { return 0 };
    if p.len() == 8 && p.chars().all(|c| c.is_ascii_digit()) {
        let (y, m, d): (i32, u32, u32) = (p[..4].parse().unwrap(), p[4..6].parse().unwrap(), p[6..8].parse().unwrap());
        let Some(date) = NaiveDate::from_ymd_opt(y, m, d) else { return 0 };
        let naive = if is_end { date.and_hms_milli_opt(23, 59, 59, 999) } else { date.and_hms_opt(0, 0, 0) };
        return naive.and_then(|n| Local.from_local_datetime(&n).earliest()).map(|dt| dt.timestamp()).unwrap_or(0);
    }
    if p.chars().all(|c| c.is_ascii_digit()) {
        let ts: i64 = p.parse().unwrap_or(0);
        return if ts > 10_000_000_000 { ts / 1000 } else { ts };
    }
    0
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ApiMediaOptions {
    pub enabled: bool,
    pub images: bool,
    pub voices: bool,
    pub videos: bool,
    pub emojis: bool,
}

pub fn parse_media_options(params: &Params) -> ApiMediaOptions {
    if !parse_bool_param(params, &["media", "meiti"], false) {
        return ApiMediaOptions::default();
    }
    ApiMediaOptions {
        enabled: true,
        images: parse_bool_param(params, &["image", "tupian"], true),
        voices: parse_bool_param(params, &["voice", "vioce"], true),
        videos: parse_bool_param(params, &["video"], true),
        emojis: parse_bool_param(params, &["emoji"], true),
    }
}

pub fn api_session_type(username: &str) -> &'static str {
    let n = username.trim();
    let l = n.to_lowercase();
    if n.is_empty() {
        "other"
    } else if l.ends_with("@chatroom") {
        "group"
    } else if l.starts_with("gh_") || l.contains("@openim") || (l.starts_with("weixin") && l != "weixin") {
        "channel"
    } else {
        "private"
    }
}

pub fn sanitize_file_name(value: &str, fallback: &str) -> String {
    let replaced: String = value.trim().chars().map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || (c as u32) < 0x20 { '_' } else { c }).collect();
    let safe = replaced.trim_end_matches('.').to_string();
    if safe.is_empty() {
        fallback.to_string()
    } else {
        safe
    }
}

pub fn normalize_account_id(value: &str) -> String {
    let t = value.trim();
    if t.is_empty() {
        return String::new();
    }
    if t.to_lowercase().starts_with("wxid_") {
        return rx(r"(?i)^(wxid_[^_]+)").captures(t).map(|c| c[1].to_string()).unwrap_or_else(|| t.to_string());
    }
    rx(r"^(.+)_([a-zA-Z0-9]{4})$").captures(t).map(|c| c[1].to_string()).unwrap_or_else(|| t.to_string())
}

// ───────────────────────── XML helpers (http flavour) ─────────────────────────

pub fn decode_entities(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'")
}

fn normalize_app_content(content: &str) -> String {
    decode_entities(content)
}

fn xml_block(xml: &str, tag: &str) -> String {
    if xml.is_empty() || tag.is_empty() {
        return String::new();
    }
    let re = rx(&format!(r"(?is)<{0}\b[^>]*>.*?</{0}>", regex::escape(tag)));
    re.find(xml).map(|m| m.as_str().to_string()).unwrap_or_default()
}

fn xml_value(xml: &str, tag: &str) -> String {
    if xml.is_empty() || tag.is_empty() {
        return String::new();
    }
    let re = rx(&format!(r"(?is)<{0}\b[^>]*>(.*?)</{0}>", regex::escape(tag)));
    match re.captures(xml) {
        Some(c) => decode_entities(&c[1]).replace("<![CDATA[", "").replace("]]>", "").trim().to_string(),
        None => String::new(),
    }
}

fn sanitize_quoted(content: &str) -> String {
    crate::message::sanitize_quoted_content(content)
}

fn looks_like_wxid(value: &str) -> bool {
    let t = value.trim().to_lowercase();
    !t.is_empty() && (t.starts_with("wxid_") || rx(r"^wx[a-z0-9_-]{4,}$").is_match(&t))
}

pub fn app_message_title(content: &str) -> String {
    let normalized = normalize_app_content(content);
    if normalized.is_empty() {
        return String::new();
    }
    let block = xml_block(&normalized, "appmsg");
    sanitize_quoted(&xml_value(if block.is_empty() { &normalized } else { &block }, "title"))
}

pub fn normalize_unsigned_token(value: &str) -> String {
    let text = value.trim();
    if text.is_empty() {
        return String::new();
    }
    if text.chars().all(|c| c.is_ascii_digit()) {
        let stripped = text.trim_start_matches('0');
        return if stripped.is_empty() { "0".into() } else { stripped.to_string() };
    }
    match text.parse::<f64>() {
        Ok(f) if f.is_finite() && f > 0.0 => format!("{}", f.floor() as i64),
        _ => String::new(),
    }
}

/// `extractType49Subtype`
pub fn extract_type49_subtype(raw: &str) -> String {
    let content = normalize_app_content(raw);
    if content.is_empty() {
        return String::new();
    }
    if let Some(c) = rx(r"(?is)<appmsg.*?>(.*?)</appmsg>").captures(&content) {
        let inner = rx(r"(?is)<refermsg.*?</refermsg>").replace_all(&c[1], "").to_string();
        let inner = rx(r"(?is)<patMsg.*?</patMsg>").replace_all(&inner, "").to_string();
        if let Some(t) = rx(r"(?is)<type>(.*?)</type>").captures(&inner) {
            return t[1].replace("<![CDATA[", "").replace("]]>", "").trim().to_string();
        }
    }
    rx(r"(?is)<type>(.*?)</type>").captures(&content).map(|t| t[1].replace("<![CDATA[", "").replace("]]>", "").trim().to_string()).unwrap_or_default()
}

pub fn resolve_type49_subtype(msg: &ChatMessage) -> String {
    let xml_type = msg.xml_type().unwrap_or("").trim().to_string();
    if !xml_type.is_empty() {
        return xml_type;
    }
    let extracted = extract_type49_subtype(&msg.raw_content);
    if !extracted.is_empty() {
        return extracted;
    }
    match msg.app_msg_kind() {
        Some("official-link") | Some("link") => "5".into(),
        Some("file") => "6".into(),
        Some("chat-record") => "19".into(),
        Some("miniapp") => "33".into(),
        Some("quote") => "57".into(),
        Some("transfer") => "2000".into(),
        Some("red-packet") => "2001".into(),
        Some("music") => "3".into(),
        _ => {
            if msg.t49.link_url.is_some() {
                "5".into()
            } else if msg.file_name().is_some() {
                "6".into()
            } else {
                String::new()
            }
        }
    }
}

// ───────────────────────── quotes ─────────────────────────

#[derive(Clone, Debug, Default)]
pub struct ApiQuoteInfo {
    pub reply_text: Option<String>,
    pub reply_to_message_id: Option<String>,
    pub quote: Map<String, Value>,
}

fn message_may_contain_quote(content: &str) -> bool {
    content.contains("<refermsg>") || content.contains("&lt;refermsg&gt;") || content.contains("<type>57</type>") || content.contains("&lt;type&gt;57&lt;/type&gt;")
}

fn extract_reply_to_id(refer_xml: &str) -> Option<String> {
    ["svrid", "msgsvrid", "newmsgid", "msgid"].iter().map(|t| normalize_unsigned_token(&xml_value(refer_xml, t))).find(|n| !n.is_empty() && n != "0")
}

fn preferred_quoted_text(refer_xml: &str) -> String {
    for tag in [
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
        "content",
    ] {
        let v = sanitize_quoted(&xml_value(refer_xml, tag));
        if !v.is_empty() {
            return v;
        }
    }
    String::new()
}

fn resolve_quoted_content(refer_xml: &str, refer_type: &str, refer_content: &str) -> String {
    match refer_type.trim() {
        "1" => preferred_quoted_text(refer_xml),
        "3" => "[图片]".into(),
        "34" => "[语音]".into(),
        "43" => "[视频]".into(),
        "47" => "[动画表情]".into(),
        "42" => "[名片]".into(),
        "48" => "[位置]".into(),
        "49" => {
            let inner = extract_type49_subtype(refer_content);
            match inner.as_str() {
                "57" => {
                    let t = app_message_title(refer_content);
                    if t.is_empty() {
                        "[引用消息]".into()
                    } else {
                        t
                    }
                }
                "6" => "[文件]".into(),
                "19" => "[聊天记录]".into(),
                "33" | "36" => "[小程序]".into(),
                _ => "[链接]".into(),
            }
        }
        _ => {
            if refer_content.is_empty() || refer_content.contains("wxid_") {
                "[消息]".into()
            } else {
                sanitize_quoted(refer_content)
            }
        }
    }
}

fn map_quoted_type49(content: &str) -> i64 {
    match extract_type49_subtype(content).as_str() {
        "57" => chatlab::REPLY,
        "6" => chatlab::FILE,
        "19" => chatlab::FORWARD,
        "33" | "36" => chatlab::SHARE,
        "2000" => chatlab::TRANSFER,
        "2001" => chatlab::RED_PACKET,
        _ => chatlab::LINK,
    }
}

fn map_quoted_message_type(refer_type: &str, refer_content: &str) -> Option<i64> {
    Some(match refer_type.trim() {
        "1" => chatlab::TEXT,
        "3" => chatlab::IMAGE,
        "34" => chatlab::VOICE,
        "43" => chatlab::VIDEO,
        "47" => chatlab::EMOJI,
        "48" => chatlab::LOCATION,
        "42" => chatlab::CONTACT,
        "50" => chatlab::CALL,
        "10000" => chatlab::SYSTEM,
        "49" => map_quoted_type49(refer_content),
        _ => return None,
    })
}

pub fn extract_api_quote_info(msg: &ChatMessage) -> Option<ApiQuoteInfo> {
    let raw = if msg.raw_content.is_empty() { "" } else { &msg.raw_content };
    if raw.is_empty() || !message_may_contain_quote(raw) {
        return None;
    }
    let normalized = normalize_app_content(raw);
    let refer_xml = xml_block(&normalized, "refermsg");
    if refer_xml.is_empty() {
        return None;
    }
    let reply_to = extract_reply_to_id(&refer_xml);
    let refer_type = xml_value(&refer_xml, "type");
    let refer_content = xml_value(&refer_xml, "content");
    let quote_content = resolve_quoted_content(&refer_xml, &refer_type, &refer_content);
    let chatusr = xml_value(&refer_xml, "chatusr");
    let sender = if !chatusr.is_empty() {
        Some(chatusr)
    } else {
        let from = xml_value(&refer_xml, "fromusr");
        (!from.is_empty() && !from.ends_with("@chatroom")).then_some(from)
    };
    let display = xml_value(&refer_xml, "displayname");
    let account_name = (!display.is_empty() && !looks_like_wxid(&display)).then_some(display);
    let quote_type = map_quoted_message_type(&refer_type, &refer_content);

    let mut quote = Map::new();
    if let Some(id) = &reply_to {
        quote.insert("platformMessageId".into(), json!(id));
    }
    if let Some(s) = sender {
        quote.insert("sender".into(), json!(s));
    }
    if let Some(a) = account_name {
        quote.insert("accountName".into(), json!(a));
    }
    if !quote_content.is_empty() {
        quote.insert("content".into(), json!(quote_content));
    }
    if let Some(t) = quote_type {
        quote.insert("type".into(), json!(t));
    }
    let reply_text = app_message_title(&normalized);
    if reply_to.is_none() && quote.is_empty() && reply_text.is_empty() {
        return None;
    }
    Some(ApiQuoteInfo { reply_text: (!reply_text.is_empty()).then_some(reply_text), reply_to_message_id: reply_to, quote })
}

// ───────────────────────── message content / type ─────────────────────────

fn is_reply_message(msg: &ChatMessage, quote: Option<&ApiQuoteInfo>) -> bool {
    let Some(q) = quote else { return false };
    if q.reply_to_message_id.is_none() && q.quote.is_empty() {
        return false;
    }
    msg.local_type == 244813135921 || (msg.local_type == 49 && resolve_type49_subtype(msg) == "57")
}

fn type49_content(msg: &ChatMessage, quote: Option<&ApiQuoteInfo>) -> String {
    let subtype = resolve_type49_subtype(msg);
    let title = msg
        .link_title()
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .or_else(|| msg.file_name().filter(|t| !t.is_empty()).map(str::to_string))
        .unwrap_or_else(|| app_message_title(&msg.raw_content));
    let with = |label: &str| if title.is_empty() { format!("[{label}]") } else { format!("[{label}] {title}") };
    match subtype.as_str() {
        "5" | "49" => with("链接"),
        "6" => with("文件"),
        "19" => with("聊天记录"),
        "33" | "36" => with("小程序"),
        "57" => {
            if !msg.parsed_content.is_empty() {
                msg.parsed_content.clone()
            } else if let Some(t) = quote.and_then(|q| q.reply_text.clone()).filter(|t| !t.is_empty()) {
                t
            } else if !title.is_empty() {
                title
            } else {
                "[引用消息]".into()
            }
        }
        "2000" => with("转账"),
        "2001" => with("红包"),
        "3" => with("音乐"),
        _ => {
            if !msg.parsed_content.is_empty() {
                msg.parsed_content.clone()
            } else if !title.is_empty() {
                title
            } else {
                "[消息]".into()
            }
        }
    }
}

fn normalize_text_content(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    // same prefix strip as the desktop HTTP service: `wxid_x:` (+ newline / <br>)
    let re = rx(r"(?i)^\s*([a-zA-Z0-9_@-]+):(?:\s*(?:\r?\n|<br\s*/?>)\s*|\s*)");
    let text = match re.find(value) {
        Some(m) if !value[m.end()..].starts_with("//") => value[m.end()..].to_string(),
        _ => value.to_string(),
    };
    Some(text.trim().to_string())
}

/// `getMessageContent`
pub fn message_content(msg: &ChatMessage, quote: Option<&ApiQuoteInfo>) -> Option<String> {
    if msg.local_type == 49 {
        return Some(type49_content(msg, quote));
    }
    if is_reply_message(msg, quote) {
        let t = if !msg.parsed_content.is_empty() {
            msg.parsed_content.clone()
        } else if let Some(t) = quote.and_then(|q| q.reply_text.clone()).filter(|t| !t.is_empty()) {
            t
        } else {
            let t = app_message_title(&msg.raw_content);
            if t.is_empty() {
                "[引用消息]".into()
            } else {
                t
            }
        };
        return Some(t);
    }
    if !msg.parsed_content.is_empty() {
        return Some(msg.parsed_content.clone());
    }
    match msg.local_type {
        1 => normalize_text_content(if msg.parsed_content.is_empty() { &msg.raw_content } else { &msg.parsed_content }),
        3 => Some("[图片]".into()),
        34 => Some("[语音]".into()),
        43 => Some("[视频]".into()),
        47 => Some("[表情]".into()),
        42 => Some(msg.card_nickname.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| "[名片]".into())),
        48 => Some("[位置]".into()),
        _ => normalize_text_content(if msg.parsed_content.is_empty() { &msg.raw_content } else { &msg.parsed_content }),
    }
}

fn map_type49(msg: &ChatMessage) -> i64 {
    match resolve_type49_subtype(msg).as_str() {
        "5" | "49" => chatlab::LINK,
        "6" => chatlab::FILE,
        "19" => chatlab::FORWARD,
        "33" | "36" => chatlab::SHARE,
        "57" => chatlab::REPLY,
        "2000" => chatlab::TRANSFER,
        "2001" => chatlab::RED_PACKET,
        _ => chatlab::OTHER,
    }
}

pub fn map_message_type(msg: &ChatMessage) -> i64 {
    match msg.local_type {
        1 => chatlab::TEXT,
        3 => chatlab::IMAGE,
        34 => chatlab::VOICE,
        43 => chatlab::VIDEO,
        47 => chatlab::EMOJI,
        48 => chatlab::LOCATION,
        42 => chatlab::CONTACT,
        50 => chatlab::CALL,
        10000 => chatlab::SYSTEM,
        49 => map_type49(msg),
        244813135921 => chatlab::REPLY,
        266287972401 => chatlab::POKE,
        8594229559345 => chatlab::RED_PACKET,
        8589934592049 => chatlab::TRANSFER,
        _ => chatlab::OTHER,
    }
}

/// `getMessageServerId`
pub fn message_server_id(msg: &ChatMessage) -> String {
    let raw = normalize_unsigned_token(&msg.server_id_raw);
    if !raw.is_empty() && raw != "0" {
        return raw;
    }
    let fallback = normalize_unsigned_token(&msg.server_id.to_string());
    if fallback != "0" {
        fallback
    } else {
        String::new()
    }
}

#[derive(Clone, Debug)]
pub struct ApiExportedMedia {
    pub kind: &'static str,
    pub file_name: String,
    pub full_path: String,
    pub relative_path: String,
}

/// `toApiMessage`
pub fn to_api_message(msg: &ChatMessage, media: Option<&ApiExportedMedia>, base_url: &str) -> Value {
    let server_id = message_server_id(msg);
    let quote = extract_api_quote_info(msg);
    let mut o = Map::new();
    o.insert("localId".into(), json!(msg.local_id));
    o.insert("serverId".into(), json!(if server_id.is_empty() { "0".to_string() } else { server_id }));
    o.insert("localType".into(), json!(msg.local_type));
    o.insert("createTime".into(), json!(msg.create_time));
    o.insert("sortSeq".into(), json!(msg.sort_seq));
    o.insert("isSend".into(), msg.is_send.map(Value::from).unwrap_or(Value::Null));
    o.insert("senderUsername".into(), msg.sender_username.clone().map(Value::from).unwrap_or(Value::Null));
    o.insert("content".into(), message_content(msg, quote.as_ref()).map(Value::from).unwrap_or(Value::Null));
    o.insert("rawContent".into(), json!(msg.raw_content));
    o.insert("parsedContent".into(), json!(msg.parsed_content));
    if let Some(m) = media {
        o.insert("mediaType".into(), json!(m.kind));
        o.insert("mediaFileName".into(), json!(m.file_name));
        o.insert("mediaUrl".into(), json!(format!("{}/api/v1/media/{}", base_url.trim_end_matches('/'), m.relative_path)));
        o.insert("mediaLocalPath".into(), json!(m.full_path));
    }
    if let Some(q) = &quote {
        if let Some(id) = &q.reply_to_message_id {
            o.insert("replyToMessageId".into(), json!(id));
        }
        if !q.quote.is_empty() {
            o.insert("quote".into(), Value::Object(q.quote.clone()));
        }
    }
    Value::Object(o)
}

// ───────────────────────── ChatLab sender resolution ─────────────────────────

pub struct SenderInfo {
    pub sender: String,
    pub account_name: String,
    pub group_nickname: Option<String>,
}

/// `resolveChatLabSenderInfo`
pub fn resolve_chatlab_sender_info(
    msg: &ChatMessage,
    talker_id: &str,
    my_wxid: &str,
    is_group: bool,
    sender_names: &HashMap<String, String>,
    group_nicknames: &HashMap<String, String>,
) -> SenderInfo {
    let mut sender = msg.sender_username.clone().unwrap_or_default().trim().to_string();
    let mut used_unknown = false;
    let same_as_me = !sender.is_empty() && !my_wxid.is_empty() && sender.to_lowercase() == my_wxid.to_lowercase();
    let is_self = msg.is_send == Some(1) || same_as_me;
    if sender.is_empty() && is_self && !my_wxid.is_empty() {
        sender = my_wxid.to_string();
    }
    if sender.is_empty() {
        if msg.local_type == 10000 || msg.local_type == 266287972401 {
            sender = talker_id.to_string();
        } else {
            let id = if msg.local_id != 0 { msg.local_id } else { msg.create_time };
            sender = format!("unknown_sender_{id}");
            used_unknown = true;
        }
    }
    let group_nickname = if is_group { group_nicknames.get(&sender.trim().to_lowercase()).cloned().unwrap_or_default() } else { String::new() };
    let display = sender_names
        .get(&sender)
        .filter(|n| !n.is_empty())
        .cloned()
        .or_else(|| Some(group_nickname.clone()).filter(|n| !n.is_empty()))
        .unwrap_or_else(|| if used_unknown { String::new() } else { sender.clone() });
    let account_name = if is_self {
        "我".to_string()
    } else if display.is_empty() {
        "未知发送者".to_string()
    } else {
        display
    };
    SenderInfo { sender, account_name, group_nickname: (!group_nickname.is_empty()).then_some(group_nickname) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_msg::map_rows;

    #[test]
    fn params() {
        assert_eq!(parse_int_param(Some("50x"), 100, 1, 10000), 50);
        assert_eq!(parse_int_param(Some("99999"), 100, 1, 10000), 10000);
        assert_eq!(parse_int_param(Some("abc"), 100, 1, 10000), 100);
        assert_eq!(parse_int_param(None, 7, 0, 10), 7);
        let mut p = Params::new();
        p.insert("media".into(), "Yes".into());
        p.insert("tupian".into(), "0".into());
        let m = parse_media_options(&p);
        assert!(m.enabled && !m.images && m.voices && m.videos && m.emojis);
        assert_eq!(parse_string_list_param(Some("a, b,a,,c")), Some(vec!["a".into(), "b".into(), "c".into()]));
        assert_eq!(parse_time_param(Some("1700000000000"), false), 1700000000);
        assert_eq!(parse_time_param(Some("1700000000"), true), 1700000000);
        assert_eq!(parse_time_param(Some("garbage"), false), 0);
        let (s, e) = (parse_time_param(Some("20240102"), false), parse_time_param(Some("20240102"), true));
        assert_eq!(e - s, 86399);
    }

    #[test]
    fn session_types() {
        assert_eq!(api_session_type("a@chatroom"), "group");
        assert_eq!(api_session_type("gh_x"), "channel");
        assert_eq!(api_session_type("weixin"), "private");
        assert_eq!(api_session_type("weixinfoo"), "channel");
        assert_eq!(api_session_type("wxid_a"), "private");
        assert_eq!(api_session_type(""), "other");
        assert_eq!(normalize_account_id("wxid_abc_1234"), "wxid_abc");
        assert_eq!(normalize_account_id("foo_ab12"), "foo");
    }

    #[test]
    fn api_message_for_quote_and_link() {
        let quote = "<msg><appmsg><title>ok</title><type>57</type><refermsg><type>1</type><displayname>Bob</displayname><content>hi there</content><svrid>00123</svrid><chatusr>wxid_bob</chatusr></refermsg></appmsg></msg>";
        let link = "<msg><appmsg><title>T</title><type>5</type><url>http://x</url></appmsg></msg>";
        let rows = vec![
            json!({"local_id": "1", "create_time": "100", "local_type": "49", "message_content": quote, "sender_username": "wxid_me", "is_send": "1"}),
            json!({"local_id": "2", "server_id": "77", "create_time": "101", "local_type": "49", "message_content": link, "sender_username": "wxid_bob", "is_send": "0"}),
            json!({"local_id": "3", "create_time": "102", "local_type": "1", "message_content": "wxid_bob:\nhello", "is_send": "0"}),
        ];
        let msgs = map_rows(&rows, "wxid_me");
        let a = to_api_message(&msgs[0], None, "http://h:1");
        assert_eq!(a["content"], "ok");
        assert_eq!(a["replyToMessageId"], "123");
        assert_eq!(a["quote"]["sender"], "wxid_bob");
        assert_eq!(a["quote"]["accountName"], "Bob");
        assert_eq!(a["quote"]["content"], "hi there");
        assert_eq!(a["quote"]["type"], 0);
        assert_eq!(a["serverId"], "0");
        let b = to_api_message(&msgs[1], None, "http://h:1");
        assert_eq!(b["content"], "[链接] T");
        assert_eq!(b["serverId"], "77");
        assert_eq!(map_message_type(&msgs[1]), chatlab::LINK);
        assert_eq!(map_message_type(&msgs[0]), chatlab::REPLY);
        let c = to_api_message(&msgs[2], None, "http://h:1");
        assert_eq!(c["content"], "hello");
        let media = ApiExportedMedia { kind: "image", file_name: "a.jpg".into(), full_path: "/x/a.jpg".into(), relative_path: "s/images/a.jpg".into() };
        let d = to_api_message(&msgs[2], Some(&media), "http://h:1/");
        assert_eq!(d["mediaUrl"], "http://h:1/api/v1/media/s/images/a.jpg");
    }

    #[test]
    fn chatlab_sender() {
        let rows = vec![json!({"local_id": "9", "create_time": "5", "local_type": "1", "message_content": "x", "is_send": "0"})];
        let msgs = map_rows(&rows, "wxid_me");
        let info = resolve_chatlab_sender_info(&msgs[0], "room@chatroom", "wxid_me", true, &HashMap::new(), &HashMap::new());
        assert_eq!(info.sender, "unknown_sender_9");
        assert_eq!(info.account_name, "未知发送者");
        let mut names = HashMap::new();
        names.insert("wxid_bob".to_string(), "Bob".to_string());
        let rows = vec![json!({"local_id": "9", "create_time": "5", "local_type": "1", "message_content": "x", "sender_username": "wxid_bob", "is_send": "0"})];
        let msgs = map_rows(&rows, "wxid_me");
        let mut nicks = HashMap::new();
        nicks.insert("wxid_bob".to_string(), "Bobby".to_string());
        let info = resolve_chatlab_sender_info(&msgs[0], "room@chatroom", "wxid_me", true, &names, &nicks);
        assert_eq!(info.account_name, "Bob");
        assert_eq!(info.group_nickname.as_deref(), Some("Bobby"));
    }
}
