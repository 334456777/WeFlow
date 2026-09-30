//! Message decoding and formatting, ported from the desktop app's `exportService.ts`.
//!
//! Raw WCDB rows go in, [`ExportMsg`] values and human-readable text come out. The
//! functions mirror their TypeScript counterparts so exports stay comparable with the
//! desktop app. Placeholder strings such as `[图片]` are data and intentionally stay as
//! in the desktop exports.

use std::cell::RefCell;
use std::collections::HashMap;

use regex::Regex;
use serde_json::{json, Map, Value};

// ───────────────────────────── regex + xml helpers ─────────────────────────────

thread_local! {
    static RX_CACHE: RefCell<HashMap<String, Regex>> = RefCell::new(HashMap::new());
}

/// Compile (and cache) a regex. Patterns here are static or escaped, so failure is a bug.
pub fn rx(pattern: &str) -> Regex {
    RX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(found) = cache.get(pattern) {
            return found.clone();
        }
        let compiled = Regex::new(pattern).unwrap_or_else(|e| panic!("bad regex {pattern}: {e}"));
        cache.insert(pattern.to_string(), compiled.clone());
        compiled
    })
}

pub fn extract_xml_value(xml: &str, tag: &str) -> String {
    let re = rx(&format!(r"(?is)<{0}>(.*?)</{0}>", regex::escape(tag)));
    match re.captures(xml) {
        Some(c) => c[1].replace("<![CDATA[", "").replace("]]>", "").trim().to_string(),
        None => String::new(),
    }
}

pub fn extract_xml_attribute(xml: &str, tag: &str, attr: &str) -> String {
    let re = rx(&format!(
        r#"(?i)<{}\s+[^>]*{}\s*=\s*"([^"]*)""#,
        regex::escape(tag),
        regex::escape(attr)
    ));
    re.captures(xml).map(|c| c[1].to_string()).unwrap_or_default()
}

pub fn decode_html_entities(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

pub fn normalize_app_message_content(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    if content.contains("&lt;") && content.contains("&gt;") {
        return content
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#39;", "'");
    }
    content.to_string()
}

/// Strip a leading `wxid_xxx:` sender prefix (but not `http://`).
pub fn strip_sender_prefix(content: &str) -> String {
    let re = rx(r"^\s*([A-Za-z0-9_-]+):");
    if let Some(m) = re.find(content) {
        if !content[m.end()..].starts_with("//") {
            return content[m.end()..].to_string();
        }
    }
    content.to_string()
}

pub fn extract_app_message_type(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let normalized = normalize_app_message_content(content);
    if let Some(c) = rx(r"(?is)<appmsg.*?>(.*?)</appmsg>").captures(&normalized) {
        let inner = rx(r"(?is)<refermsg.*?</refermsg>").replace_all(&c[1], "").to_string();
        let inner = rx(r"(?is)<patMsg.*?</patMsg>").replace_all(&inner, "").to_string();
        if let Some(t) = rx(r"(?is)<type>(.*?)</type>").captures(&inner) {
            return t[1].trim().to_string();
        }
    }
    if !normalized.contains("<appmsg") && !normalized.contains("<msg>") {
        return String::new();
    }
    rx(r"(?i)<type>(\d+)</type>")
        .captures(&normalized)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

fn looks_like_wxid(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return false;
    }
    t.starts_with("wxid_") || rx(r"^wx[a-z0-9_-]{4,}$").is_match(&t)
}

pub fn sanitize_quoted_content(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let mut r = rx(r"wxid_[A-Za-z0-9_-]{3,}").replace_all(content, "").to_string();
    r = rx(r"^[\s:：\-]+").replace(&r, "").to_string();
    r = rx(r"[:：]{2,}").replace_all(&r, ":").to_string();
    r = rx(r"^[\s:：\-]+").replace(&r, "").to_string();
    rx(r"\s+").replace_all(&r, " ").trim().to_string()
}

fn is_hex(c: char) -> bool {
    c.is_ascii_hexdigit()
}

/// `([a-f0-9]{min,max})(?![a-f0-9])` for the first hex run that is at least `min` long:
/// returns the trailing `max`-capped slice of that run (what the lookahead regex matches).
fn last_hex_token(text: &str, min: usize, max: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if is_hex(chars[i]) {
            let start = i;
            while i < chars.len() && is_hex(chars[i]) {
                i += 1;
            }
            let len = i - start;
            if len >= min {
                let take = len.min(max);
                return Some(chars[i - take..i].iter().collect());
            }
        } else {
            i += 1;
        }
    }
    None
}

// ───────────────────────────────── time ─────────────────────────────────

use chrono::{Local, TimeZone};

/// `YYYY-MM-DD HH:MM:SS` in the local time zone (same as the desktop app).
pub fn format_timestamp(ts: i64) -> String {
    match Local.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => String::new(),
    }
}

pub fn format_iso_timestamp(ts: i64) -> String {
    match chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0) {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        None => String::new(),
    }
}

pub fn normalize_timestamp_seconds(value: f64) -> i64 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    let mut n = value.floor() as i64;
    while n > 10_000_000_000 {
        n /= 1000;
    }
    n
}

fn parse_compact_datetime(raw: &str) -> i64 {
    let raw = raw.trim();
    if !rx(r"^\d{8}(?:\d{4}(?:\d{2})?)?$").is_match(raw) {
        return 0;
    }
    let num = |a: usize, b: usize| raw[a..b].parse::<u32>().unwrap_or(0);
    let year = num(0, 4) as i32;
    let month = num(4, 6);
    let day = num(6, 8);
    let hour = if raw.len() >= 12 { num(8, 10) } else { 0 };
    let minute = if raw.len() >= 12 { num(10, 12) } else { 0 };
    let second = if raw.len() >= 14 { num(12, 14) } else { 0 };
    if !(1990..=2200).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return 0;
    }
    match Local.with_ymd_and_hms(year, month, day, hour, minute, second).single() {
        Some(dt) => dt.timestamp().max(0),
        None => 0,
    }
}

fn parse_datetime_text(value: &str) -> i64 {
    let raw = value.trim();
    if raw.is_empty() {
        return 0;
    }
    let compact = parse_compact_datetime(raw);
    if compact > 0 {
        return compact;
    }
    if rx(r"[zZ]|[+-]\d{2}:?\d{2}$").is_match(raw) {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
            return dt.timestamp().max(0);
        }
    }
    let normalized = raw.replace('T', " ");
    let normalized = rx(r"\.\d+$").replace(&normalized, "").replace('/', "-");
    let Some(c) = rx(r"^(\d{4})-(\d{1,2})-(\d{1,2})(?: (\d{1,2}):(\d{1,2})(?::(\d{1,2}))?)?$").captures(&normalized) else {
        return 0;
    };
    let g = |i: usize| c.get(i).map(|m| m.as_str().parse::<u32>().unwrap_or(0)).unwrap_or(0);
    match Local.with_ymd_and_hms(g(1) as i32, g(2), g(3), g(4), g(5), g(6)).single() {
        Some(dt) => dt.timestamp().max(0),
        None => 0,
    }
}

fn normalize_row_timestamp(value: &Value) -> i64 {
    let raw = match value {
        Value::Null => return 0,
        Value::String(s) => s.trim().to_string(),
        other => other.to_string(),
    };
    if raw.is_empty() {
        return 0;
    }
    let compact = parse_compact_datetime(&raw);
    if compact > 0 {
        return compact;
    }
    if let Ok(n) = raw.parse::<f64>() {
        if n.is_finite() && n > 0.0 {
            return normalize_timestamp_seconds(n);
        }
    }
    parse_datetime_text(&raw)
}

// ─────────────────────────────── row accessors ───────────────────────────────

pub fn row_field<'a>(row: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let obj = row.as_object()?;
    for key in keys {
        if let Some(v) = obj.get(*key) {
            match v {
                Value::Null => continue,
                Value::String(s) if s.is_empty() => continue,
                _ => return Some(v),
            }
        }
    }
    None
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub fn row_int(row: &Value, keys: &[&str], fallback: i64) -> i64 {
    let Some(obj) = row.as_object() else { return fallback };
    for key in keys {
        if let Some(v) = obj.get(*key) {
            let text = match v {
                Value::Null => continue,
                Value::String(s) if s.is_empty() => continue,
                other => value_to_string(other),
            };
            let digits: String = text
                .trim()
                .char_indices()
                .take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+')))
                .map(|(_, c)| c)
                .collect();
            if let Ok(n) = digits.parse::<i64>() {
                return n;
            }
        }
    }
    fallback
}

pub fn normalize_unsigned_token(value: &str) -> String {
    let raw = value.trim();
    if raw.is_empty() {
        return "0".into();
    }
    if raw.chars().all(|c| c.is_ascii_digit()) {
        let t = raw.trim_start_matches('0');
        return if t.is_empty() { "0".into() } else { t.to_string() };
    }
    match raw.parse::<f64>() {
        Ok(n) if n.is_finite() && n > 0.0 => format!("{}", n.floor() as i128),
        _ => "0".into(),
    }
}

pub fn get_timestamp_seconds(row: &Value) -> i64 {
    let primary_raw = row_field(
        row,
        &["create_time", "createTime", "createtime", "msg_create_time", "msgCreateTime", "msg_time", "msgTime", "time", "WCDB_CT_create_time"],
    );
    let primary = primary_raw.map(normalize_row_timestamp).unwrap_or(0);
    let sort_seq = row_field(row, &["sort_seq", "sortSeq", "server_seq", "serverSeq"])
        .map(normalize_row_timestamp)
        .unwrap_or(0);
    if primary > 0 && primary < 946_684_800 && sort_seq > 946_684_800 {
        return sort_seq;
    }
    if primary > 0 {
        return primary;
    }
    sort_seq.max(0)
}

// ──────────────────────────── content decoding ────────────────────────────

fn looks_like_hex(s: &str) -> bool {
    s.len() % 2 == 0 && !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn looks_like_base64(s: &str) -> bool {
    s.len() % 4 == 0 && rx(r"^[A-Za-z0-9+/=]+$").is_match(s)
}

fn hex_to_bytes(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

fn base64_to_bytes(s: &str) -> Option<Vec<u8>> {
    let table = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a') as u32 + 26),
            b'0'..=b'9' => Some((c - b'0') as u32 + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let trimmed = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in trimmed.bytes() {
        buf = (buf << 6) | table(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn decode_binary_content(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }
    if data.len() >= 4 && u32::from_le_bytes([data[0], data[1], data[2], data[3]]) == 0xFD2F_B528 {
        if let Ok(out) = zstd::decode_all(data) {
            return String::from_utf8_lossy(&out).to_string();
        }
    }
    let decoded = String::from_utf8_lossy(data).to_string();
    let replacement = decoded.chars().filter(|c| *c == '\u{FFFD}').count();
    if (replacement as f64) < decoded.chars().count() as f64 * 0.2 {
        return decoded.replace('\u{FFFD}', "");
    }
    data.iter().map(|b| *b as char).collect()
}

pub fn decode_maybe_compressed(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    if raw.chars().all(|c| c.is_ascii_digit()) {
        return raw.to_string();
    }
    if raw.len() > 16 && looks_like_hex(raw) {
        if let Some(bytes) = hex_to_bytes(raw) {
            if !bytes.is_empty() {
                return decode_binary_content(&bytes);
            }
        }
    }
    if raw.len() > 16 && looks_like_base64(raw) {
        if let Some(bytes) = base64_to_bytes(raw) {
            return decode_binary_content(&bytes);
        }
    }
    raw.to_string()
}

/// `decodeMessageContent(message_content, compress_content)`, plus the WCDB
/// `WCDB_CT_*` compression flag used by the CLI's raw rows.
pub fn decode_message_content(row: &Value) -> String {
    let as_text = |key: &str| row.get(key).map(value_to_string).unwrap_or_default();
    let compress = as_text("compress_content");
    let mut content = decode_maybe_compressed(&compress);
    if content.is_empty() {
        let raw = as_text("message_content");
        let flagged = as_text("WCDB_CT_message_content") == "4";
        content = if flagged && looks_like_hex(&raw) {
            hex_to_bytes(&raw)
                .and_then(|b| zstd::decode_all(b.as_slice()).ok())
                .map(|b| String::from_utf8_lossy(&b).to_string())
                .unwrap_or_else(|| decode_maybe_compressed(&raw))
        } else {
            decode_maybe_compressed(&raw)
        };
    }
    content
}

// ───────────────────────────── message struct ─────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct ForwardRecord {
    pub datatype: i64,
    pub sourcename: String,
    pub sourcetime: String,
    pub sourceheadurl: Option<String>,
    pub datadesc: Option<String>,
    pub datatitle: Option<String>,
    pub fileext: Option<String>,
    pub datasize: Option<i64>,
    pub chat_record_title: Option<String>,
    pub chat_record_desc: Option<String>,
    pub chat_record_list: Option<Vec<ForwardRecord>>,
}

#[derive(Clone, Debug, Default)]
pub struct ExportMsg {
    pub local_id: i64,
    pub server_id: i64,
    pub server_id_raw: Option<String>,
    pub create_time: i64,
    pub local_type: i64,
    pub content: String,
    pub sender_username: String,
    pub is_send: bool,
    pub image_md5: Option<String>,
    pub image_dat_name: Option<String>,
    pub emoji_cdn_url: Option<String>,
    pub emoji_md5: Option<String>,
    pub emoji_caption: Option<String>,
    pub video_md5: Option<String>,
    pub xml_type: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub file_ext: Option<String>,
    pub file_md5: Option<String>,
    pub location_lat: Option<f64>,
    pub location_lng: Option<f64>,
    pub location_poiname: Option<String>,
    pub location_label: Option<String>,
    pub chat_record_list: Option<Vec<ForwardRecord>>,
}

impl ExportMsg {
    pub fn stable_key(&self) -> String {
        let raw = self.server_id_raw.clone().unwrap_or_else(|| self.server_id.to_string());
        format!(
            "{}:{}:{}",
            normalize_unsigned_token(&self.local_id.to_string()),
            normalize_unsigned_token(&self.create_time.to_string()),
            normalize_unsigned_token(&raw)
        )
    }

    pub fn platform_message_id(&self) -> Option<String> {
        let raw = self.server_id_raw.clone().unwrap_or_else(|| self.server_id.to_string());
        let v = normalize_unsigned_token(&raw);
        if v != "0" {
            Some(v)
        } else {
            None
        }
    }
}

// ────────────────────────── system / revoke / voip ──────────────────────────

pub struct RevokeInfo {
    pub is_revoke: bool,
    pub is_self_revoke: bool,
    pub revoker_wxid: Option<String>,
}

pub fn extract_revoker_info(content: &str) -> RevokeInfo {
    let none = RevokeInfo { is_revoke: false, is_self_revoke: false, revoker_wxid: None };
    if content.is_empty() || (!content.contains("revokemsg") && !content.contains("撤回")) {
        return none;
    }
    if content.contains("你撤回") {
        return RevokeInfo { is_revoke: true, is_self_revoke: true, revoker_wxid: None };
    }
    if let Some(c) = rx(r"(?i)<session>([^<]+)</session>").captures(content) {
        let session = c[1].trim().to_string();
        if session.starts_with("wxid_") || rx(r"^[a-zA-Z][a-zA-Z0-9_-]+$").is_match(&session) {
            return RevokeInfo { is_revoke: true, is_self_revoke: false, revoker_wxid: Some(session) };
        }
    }
    if let Some(c) = rx(r"(?i)<fromusername>([^<]+)</fromusername>").captures(content) {
        return RevokeInfo { is_revoke: true, is_self_revoke: false, revoker_wxid: Some(c[1].trim().to_string()) };
    }
    RevokeInfo { is_revoke: true, is_self_revoke: false, revoker_wxid: None }
}

pub fn is_readable_system_message(local_type: i64, content: &str) -> bool {
    if local_type == 10000 {
        return true;
    }
    let normalized = normalize_app_message_content(content);
    rx(r"(?i)<sysmsg\b").is_match(&strip_sender_prefix(&normalized))
}

pub fn clean_system_message(content: &str) -> String {
    if content.is_empty() {
        return "[系统消息]".into();
    }
    let mut content = content.to_string();
    if let Some(c) = rx(r"(?is)<sysmsg[^>]*>(.*?)</sysmsg>").captures(&content.clone()) {
        content = c[1].to_string();
    }
    if let Some(c) = rx(r"(?i)<replacemsg><!\[CDATA\[(.*?)\]\]></replacemsg>").captures(&content) {
        return c[1].trim().to_string();
    }
    if let Some(c) = rx(r"(?i)<template><!\[CDATA\[(.*?)\]\]></template>").captures(&content) {
        let template = c[1].to_string();
        let source = content.clone();
        let replaced = rx(r"\$\{([^}]+)\}").replace_all(&template, |caps: &regex::Captures| {
            let name = regex::escape(&caps[1]);
            let pat = format!(r"(?i)<{0}><!\[CDATA\[([^\]]*)\]\]></{0}>", name);
            rx(&pat).captures(&source).map(|m| m[1].to_string()).unwrap_or_default()
        });
        return rx(r"<[^>]+>").replace_all(&replaced, "").trim().to_string();
    }
    if let Some(c) = rx(r"(?is)<title>(.*?)</title>").captures(&content) {
        let title = c[1].replace("<![CDATA[", "").replace("]]>", "").trim().to_string();
        if !title.is_empty() {
            return title;
        }
    }
    content = content.replace("<![CDATA[", "").replace("]]>", "");
    let stripped = rx(r"(?i)<img[^>]*>").replace_all(&content, "").to_string();
    let stripped = rx(r"</?[a-zA-Z0-9_:]+[^>]*>").replace_all(&stripped, "").to_string();
    let stripped = rx(r"\s+").replace_all(&stripped, " ").trim().to_string();
    if stripped.is_empty() {
        "[系统消息]".into()
    } else {
        stripped
    }
}

pub fn extract_readable_system_message_text(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let normalized = normalize_app_message_content(content);
    let stripped = strip_sender_prefix(&normalized);
    let source = rx(r"(?is)<sysmsg\b[^>]*>(.*?)</sysmsg>")
        .captures(&stripped)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| normalized.clone());
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
        return if duration.is_empty() { format!("[{call_type}] 已接听") } else { format!("[{call_type}] {duration}") };
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

// ───────────────────────────── type mapping ─────────────────────────────

pub fn convert_message_type(local_type: i64, content: &str) -> i64 {
    let normalized = normalize_app_message_content(content);
    if is_readable_system_message(local_type, &normalized) {
        return 80;
    }
    let xml_type_raw = extract_app_message_type(&normalized);
    let xml_type: Option<i64> = xml_type_raw.parse().ok();
    let looks_app = local_type == 49 || normalized.contains("<appmsg") || normalized.contains("<msg>");
    if looks_app || xml_type.map_or(false, |t| t != 0) {
        let sub = xml_type.unwrap_or(0);
        match sub {
            6 => return 4,
            19 => return 7,
            33 | 36 => return 24,
            57 => return 25,
            2000 => return 99,
            5 | 49 => return 7,
            _ => {
                if xml_type.map_or(false, |t| t != 0) || looks_app {
                    return 7;
                }
            }
        }
    }
    match local_type {
        1 => 0,
        3 => 1,
        34 => 2,
        43 => 3,
        49 | 34359738417 | 103079215153 | 25769803825 => 7,
        47 => 5,
        48 => 8,
        42 => 27,
        50 => 23,
        10000 => 80,
        _ => 99,
    }
}

pub fn message_type_name(local_type: i64, content: Option<&str>) -> &'static str {
    if let Some(content) = content {
        if !content.is_empty() {
            let normalized = normalize_app_message_content(content);
            if is_readable_system_message(local_type, &normalized) {
                return "系统消息";
            }
            match extract_app_message_type(&normalized).as_str() {
                "3" => return "音乐消息",
                "87" => return "群公告",
                "2000" => return "转账消息",
                "5" => return "链接消息",
                "6" => return "文件消息",
                "19" => return "聊天记录",
                "33" | "36" => return "小程序消息",
                "57" => return "引用消息",
                _ => {}
            }
        }
    }
    match local_type {
        1 => "文本消息",
        3 => "图片消息",
        34 => "语音消息",
        42 => "名片消息",
        43 => "视频消息",
        47 => "动画表情",
        48 => "位置消息",
        49 => "链接消息",
        50 => "通话消息",
        10000 => "系统消息",
        244813135921 => "引用消息",
        _ => "其他消息",
    }
}

pub fn weclone_type_name(local_type: i64, content: &str) -> &'static str {
    match local_type {
        1 => return "text",
        3 => return "image",
        47 => return "sticker",
        43 => return "video",
        34 => return "voice",
        48 => return "location",
        _ => {}
    }
    let normalized = normalize_app_message_content(content);
    if local_type == 49 || normalized.contains("<appmsg") || normalized.contains("<msg>") {
        if extract_app_message_type(&normalized) == "6" {
            return "file";
        }
    }
    "text"
}

// ─────────────────────────────── emoji helpers ───────────────────────────────

pub fn normalize_md5(value: &str) -> Option<String> {
    let md5 = value.trim().to_lowercase();
    if md5.len() == 32 && md5.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(md5)
    } else {
        None
    }
}

pub fn extract_loose_hex_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let keyed = rx(r"(?i)(?:emoji|sticker|md5)[^a-fA-F0-9]{0,32}([a-fA-F0-9]{32})").captures(content);
    if let Some(c) = keyed {
        return normalize_md5(&c[1]);
    }
    rx(r"(?i)([a-fA-F0-9]{32})").captures(content).and_then(|c| normalize_md5(&c[1]))
}

pub fn extract_emoji_url(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    if let Some(c) = rx(r#"(?i)cdnurl\s*=\s*['"]([^'"]+)['"]"#).captures(content) {
        let url = c[1].replace("&amp;", "&");
        return Some(percent_decode(&url));
    }
    rx(r"(?i)cdnurl[^>]*>([^<]+)").captures(content).map(|c| c[1].to_string())
}

pub fn extract_emoji_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let found = rx(r#"(?i)md5\s*=\s*['"]([a-fA-F0-9]{32})['"]"#)
        .captures(content)
        .or_else(|| rx(r"(?i)md5\s*=\s*([a-fA-F0-9]{32})").captures(content))
        .or_else(|| rx(r"(?i)<md5>([a-fA-F0-9]{32})</md5>").captures(content));
    found.and_then(|c| normalize_md5(&c[1])).or_else(|| extract_loose_hex_md5(content))
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 0 && i + 2 <= bytes.len() - 1 + 0 {
            let h = (bytes[i + 1] as char).to_digit(16);
            let l = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (h, l) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    match String::from_utf8(out) {
        Ok(s) => s,
        Err(_) => s.to_string(),
    }
}

pub fn format_emoji_semantic_text(caption: Option<&str>) -> String {
    match caption.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => format!("[表情包：{c}]"),
        None => "[表情包]".into(),
    }
}

// ─────────────────────────── image / video / file ───────────────────────────

pub fn normalize_image_dat_name_token(value: &str) -> Option<String> {
    let mut text = value.trim().to_string();
    if text.is_empty() {
        return None;
    }
    text = text.replace("&amp;", "&");
    text = percent_decode(&text);
    if let Some(c) = rx(r"(?i)([0-9a-fA-F]{8,})(?:\.t)?\.dat").captures(&text) {
        return Some(c[1].to_lowercase());
    }
    let no_query = text.split(['?', '#']).next().unwrap_or("").to_string();
    let base = rx(r"^.*[\\/]").replace(&no_query, "").to_string();
    let base = rx(r"(?i)\.(?:t\.)?dat$").replace(&base, "").trim().to_string();
    if base.is_empty() {
        return None;
    }
    let cdn_token = if base.contains('_') { base.split('_').next().unwrap_or("").to_string() } else { base.clone() };
    if rx(r"^[a-fA-F0-9]{16,64}$").is_match(&cdn_token) {
        return Some(cdn_token.to_lowercase());
    }
    if let Some(t) = last_hex_token(&cdn_token, 32, 32) {
        return Some(t.to_lowercase());
    }
    last_hex_token(&cdn_token, 16, 64).map(|t| t.to_lowercase())
}

fn extract_image_dat_name(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let candidates = [
        extract_xml_value(content, "imgname"),
        extract_xml_value(content, "cdnmidimgurl"),
        extract_xml_value(content, "cdnthumburl"),
        extract_xml_attribute(content, "img", "imgname"),
        extract_xml_attribute(content, "img", "cdnmidimgurl"),
        extract_xml_attribute(content, "img", "cdnthumburl"),
    ];
    let pick = candidates.into_iter().find(|c| !c.is_empty()).unwrap_or_default();
    normalize_image_dat_name_token(&pick)
}

pub fn extract_image_dat_name_from_row(row: &Value, content: &str) -> Option<String> {
    let by_column = row_field(row, &["image_path", "imagePath", "image_dat_name", "imageDatName", "img_path", "imgPath", "img_name", "imgName"])
        .map(value_to_string)
        .and_then(|v| normalize_image_dat_name_token(&v));
    if by_column.is_some() {
        return by_column;
    }
    let packed = row_field(
        row,
        &[
            "packed_info_data", "packedInfoData", "packed_info_blob", "packedInfoBlob", "packed_info", "packedInfo",
            "BytesExtra", "bytes_extra", "WCDB_CT_packed_info", "reserved0", "Reserved0", "WCDB_CT_Reserved0",
        ],
    )
    .and_then(|v| decode_packed_info(&value_to_string(v)));
    if let Some(bytes) = packed {
        let text: String = bytes.iter().map(|b| if (0x20..=0x7e).contains(b) { *b as char } else { ' ' }).collect();
        if let Some(c) = rx(r"(?i)([0-9a-fA-F]{8,})(?:\.t)?\.dat").captures(&text) {
            return Some(c[1].to_lowercase());
        }
        if let Some(c) = rx(r"([0-9a-fA-F]{16,})").captures(&text) {
            return Some(c[1].to_lowercase());
        }
    }
    extract_image_dat_name(content)
}

fn decode_packed_info(raw: &str) -> Option<Vec<u8>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let compact: String = trimmed.split_whitespace().collect();
    if looks_like_hex(&compact) {
        if let Some(b) = hex_to_bytes(&compact) {
            return Some(b);
        }
    }
    base64_to_bytes(trimmed).filter(|b| !b.is_empty())
}

pub fn extract_image_md5(content: &str) -> Option<String> {
    let attr = rx(r#"(?i)<img[^>]*\smd5\s*=\s*['"]([a-fA-F0-9]+)['"]"#).captures(content);
    if let Some(c) = attr {
        return Some(c[1].to_lowercase());
    }
    rx(r"(?i)<md5>([^<]+)</md5>").captures(content).map(|c| c[1].to_lowercase())
}

pub fn extract_video_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    if let Some(c) = rx(r#"(?i)<videomsg[^>]*\smd5\s*=\s*['"]([a-fA-F0-9]+)['"]"#).captures(content) {
        return Some(c[1].to_lowercase());
    }
    rx(r"(?i)<md5>([^<]+)</md5>").captures(content).map(|c| c[1].to_lowercase())
}

pub struct FileMeta {
    pub xml_type: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub file_ext: Option<String>,
    pub file_md5: Option<String>,
}

pub fn extract_file_app_message_meta(content: &str) -> Option<FileMeta> {
    let normalized = normalize_app_message_content(content);
    if normalized.is_empty() || (!normalized.contains("<appmsg") && !normalized.contains("<msg>")) {
        return None;
    }
    let xml_type = extract_app_message_type(&normalized);
    if xml_type.is_empty() {
        return None;
    }
    let first = |keys: &[&str]| keys.iter().map(|k| extract_xml_value(&normalized, k)).find(|v| !v.is_empty()).unwrap_or_default();
    let raw_name = first(&["filename", "title"]);
    let raw_ext = extract_xml_value(&normalized, "fileext");
    let raw_size = first(&["totallen", "datasize", "filesize"]);
    let mut raw_md5 = extract_xml_value(&normalized, "md5");
    if raw_md5.is_empty() {
        raw_md5 = extract_xml_attribute(&normalized, "appattach", "md5");
    }
    if raw_md5.is_empty() {
        raw_md5 = extract_loose_hex_md5(&normalized).unwrap_or_default();
    }
    let size = raw_size.trim().parse::<i64>().ok().filter(|n| *n > 0);
    let non_empty = |s: String| {
        let t = decode_html_entities(&s).trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    };
    Some(FileMeta {
        xml_type: Some(xml_type),
        file_name: non_empty(raw_name),
        file_size: size,
        file_ext: non_empty(raw_ext),
        file_md5: if raw_md5.len() == 32 && raw_md5.chars().all(|c| c.is_ascii_hexdigit()) { Some(raw_md5.to_lowercase()) } else { None },
    })
}

pub fn extract_location_meta(content: &str, local_type: i64) -> Option<(Option<f64>, Option<f64>, Option<String>, Option<String>)> {
    if content.is_empty() || local_type != 48 {
        return None;
    }
    let n = normalize_app_message_content(content);
    let pick = |a: String, b: String| if !a.is_empty() { a } else { b };
    let raw_lat = pick(extract_xml_attribute(&n, "location", "x"), extract_xml_attribute(&n, "location", "latitude"));
    let raw_lng = pick(extract_xml_attribute(&n, "location", "y"), extract_xml_attribute(&n, "location", "longitude"));
    let poiname = {
        let a = extract_xml_attribute(&n, "location", "poiname");
        if !a.is_empty() { a } else {
            let b = extract_xml_value(&n, "poiname");
            if !b.is_empty() { b } else { extract_xml_value(&n, "poiName") }
        }
    };
    let label = pick(extract_xml_attribute(&n, "location", "label"), extract_xml_value(&n, "label"));
    let lat = raw_lat.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    let lng = raw_lng.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    let poi = if poiname.is_empty() { None } else { Some(poiname) };
    let lab = if label.is_empty() { None } else { Some(label) };
    if lat.is_none() && lng.is_none() && poi.is_none() && lab.is_none() {
        return None;
    }
    Some((lat, lng, poi, lab))
}

fn location_text(content: &str) -> String {
    let n = normalize_app_message_content(content);
    let first = |vals: Vec<String>| vals.into_iter().find(|v| !v.is_empty()).unwrap_or_default();
    let poiname = first(vec![extract_xml_attribute(&n, "location", "poiname"), extract_xml_value(&n, "poiname"), extract_xml_value(&n, "poiName")]);
    let label = first(vec![extract_xml_attribute(&n, "location", "label"), extract_xml_value(&n, "label")]);
    let lat = first(vec![extract_xml_attribute(&n, "location", "x"), extract_xml_attribute(&n, "location", "latitude")]);
    let lng = first(vec![extract_xml_attribute(&n, "location", "y"), extract_xml_attribute(&n, "location", "longitude")]);
    let mut parts: Vec<String> = Vec::new();
    if !poiname.is_empty() {
        parts.push(poiname.clone());
    }
    if !label.is_empty() && label != poiname {
        parts.push(label);
    }
    if !lat.is_empty() && !lng.is_empty() {
        parts.push(format!("({lat},{lng})"));
    }
    if parts.is_empty() {
        "[位置]".into()
    } else {
        format!("[位置] {}", parts.join(" "))
    }
}

// ─────────────────────────── forward chat records ───────────────────────────

pub fn parse_chat_history(content: &str) -> Option<Vec<ForwardRecord>> {
    let normalized = normalize_app_message_content(content);
    let app_type = extract_app_message_type(&normalized);
    if app_type != "19" && !normalized.contains("<recorditem") {
        return None;
    }
    let mut items: Vec<ForwardRecord> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let dedupe_key = |r: &ForwardRecord| format!("{}|{}|{}|{}|{}", r.datatype, r.sourcename, r.sourcetime, r.datadesc.clone().unwrap_or_default(), r.datatitle.clone().unwrap_or_default());
    for c in rx(r"(?is)<recorditem>(.*?)</recorditem>").captures_iter(&normalized) {
        for item in parse_forward_container(&c[1]) {
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
    if items.is_empty() { None } else { Some(items) }
}

fn parse_forward_container(container: &str) -> Vec<ForwardRecord> {
    if container.is_empty() {
        return Vec::new();
    }
    let mut segments: Vec<String> = vec![container.to_string()];
    let decoded = decode_html_entities(container);
    if decoded != container {
        segments.push(decoded);
    }
    for c in rx(r"(?s)<!\[CDATA\[(.*?)\]\]>").captures_iter(container) {
        let inner = c[1].to_string();
        if !inner.is_empty() {
            let decoded_inner = decode_html_entities(&inner);
            segments.push(inner.clone());
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
        for c in rx(r"(?is)<dataitem\b([^>]*)>(.*?)</dataitem>").captures_iter(seg) {
            if let Some(parsed) = parse_forward_data_item(&c[2], &c[1]) {
                let key = format!("{}|{}|{}|{}|{}", parsed.datatype, parsed.sourcename, parsed.sourcetime, parsed.datadesc.clone().unwrap_or_default(), parsed.datatitle.clone().unwrap_or_default());
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
    let sourceheadurl = extract_xml_value(body, "sourceheadurl");
    let mut datadesc = extract_xml_value(body, "datadesc");
    if datadesc.is_empty() {
        datadesc = extract_xml_value(body, "content");
    }
    let datadesc = decode_html_entities(&datadesc);
    let datatitle = decode_html_entities(&extract_xml_value(body, "datatitle"));
    let fileext = extract_xml_value(body, "fileext");
    let datasize = extract_xml_value(body, "datasize").parse::<i64>().unwrap_or(0);
    let nested_xml = extract_xml_value(body, "recordxml");
    let nested_list = if datatype == 17 && !nested_xml.is_empty() {
        Some(parse_forward_container(&nested_xml)).filter(|l| !l.is_empty())
    } else {
        None
    };
    let pick = |a: String, b: &str| if !a.is_empty() { a } else { b.to_string() };
    let chat_record_title = decode_html_entities(&pick(if nested_xml.is_empty() { String::new() } else { extract_xml_value(&nested_xml, "title") }, &datatitle));
    let chat_record_desc = decode_html_entities(&pick(if nested_xml.is_empty() { String::new() } else { extract_xml_value(&nested_xml, "desc") }, &datadesc));
    if sourcename.is_empty() && datadesc.is_empty() && datatitle.is_empty() {
        return None;
    }
    let opt = |s: String| if s.is_empty() { None } else { Some(s) };
    Some(ForwardRecord {
        datatype,
        sourcename,
        sourcetime,
        sourceheadurl: opt(sourceheadurl),
        datadesc: opt(datadesc),
        datatitle: opt(datatitle),
        fileext: opt(fileext),
        datasize: if datasize > 0 { Some(datasize) } else { None },
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
        47 => "[表情包]".into(),
        49 | 8 => "[文件]".into(),
        17 => item.chat_record_desc.clone().filter(|d| !d.is_empty()).unwrap_or_else(|| "[聊天记录]".into()),
        _ => "[消息]".into(),
    }
}

fn build_forward_lines(record: &ForwardRecord, depth: usize) -> Vec<String> {
    let indent = if depth > 0 { "  ".repeat(depth.min(8)) } else { String::new() };
    let prefix = if record.sourcename.is_empty() { String::new() } else { format!("{}: ", record.sourcename) };
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
        if !d.trim().is_empty() { d.trim().to_string() } else if !t.trim().is_empty() { t.trim().to_string() } else { "[文件]".into() }
    } else {
        format_forward_item_text(record)
    };
    vec![format!("{indent}{prefix}{text}")]
}

pub fn format_forward_chat_record_content(content: &str) -> String {
    let normalized = normalize_app_message_content(content);
    let name = ["nickname", "title", "des", "displayname"]
        .iter()
        .map(|t| extract_xml_value(&normalized, t))
        .find(|v| !v.is_empty())
        .unwrap_or_else(|| "聊天记录".into());
    let header = if name.is_empty() { "[转发的聊天记录]".to_string() } else { format!("[转发的聊天记录]{name}") };
    match parse_chat_history(&normalized) {
        Some(records) if !records.is_empty() => {
            let lines: Vec<String> = records.iter().flat_map(|r| build_forward_lines(r, 0)).collect();
            format!("{header}\n{}", lines.join("\n"))
        }
        _ => header,
    }
}

// ───────────────────────────── quoted replies ─────────────────────────────

pub struct QuoteInfo {
    pub content: Option<String>,
    pub sender: Option<String>,
    pub quote_type: Option<String>,
    pub svrid: Option<String>,
}

fn refermsg_xml(normalized: &str) -> Option<String> {
    let start = normalized.find("<refermsg>")?;
    let end = normalized.find("</refermsg>")?;
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
        if e >= s && s >= 0 {
            let sliced: String = full.chars().skip(s as usize).take((e - s + 1) as usize).collect();
            let sliced = sliced.trim().to_string();
            if !sliced.is_empty() {
                return sliced;
            }
        }
    }
    String::new()
}

fn extract_preferred_quoted_text(refer_xml: &str) -> String {
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
    let first = if sources[0].is_empty() { refer_xml.to_string() } else { sources[0].clone() };
    let full = sanitize_quoted_content(&extract_xml_value(&first, "content"));
    let partial = extract_partial_quoted_text(&first, &full);
    if !partial.is_empty() {
        return partial;
    }
    let tags = [
        "selectedcontent", "selectedtext", "selectcontent", "selecttext", "quotecontent", "quotetext",
        "partcontent", "parttext", "excerpt", "summary", "preview",
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

pub fn parse_quote_message(content: &str) -> QuoteInfo {
    let empty = QuoteInfo { content: None, sender: None, quote_type: None, svrid: None };
    let normalized = normalize_app_message_content(content);
    let Some(refer) = refermsg_xml(&normalized) else { return empty };
    let mut sender = extract_xml_value(&refer, "displayname");
    if !sender.is_empty() && looks_like_wxid(&sender) {
        sender.clear();
    }
    let refer_content = extract_xml_value(&refer, "content");
    let refer_type = extract_xml_value(&refer, "type");
    let svrid = extract_xml_value(&refer, "svrid");
    let display = match refer_type.as_str() {
        "1" => extract_preferred_quoted_text(&refer),
        "3" => "[图片]".into(),
        "34" => "[语音]".into(),
        "43" => "[视频]".into(),
        "47" => "[表情包]".into(),
        "49" => "[链接]".into(),
        "42" => "[名片]".into(),
        "48" => "[位置]".into(),
        _ => {
            if refer_content.is_empty() || refer_content.contains("wxid_") {
                "[消息]".into()
            } else {
                sanitize_quoted_content(&refer_content)
            }
        }
    };
    let opt = |s: String| if s.is_empty() { None } else { Some(s) };
    QuoteInfo { content: opt(display), sender: opt(sender), quote_type: opt(refer_type), svrid: opt(svrid) }
}

fn format_quoted_reference_preview(content: &str, refer_type: &str) -> String {
    let t: Option<i64> = refer_type.trim().parse().ok();
    let Some(ty) = t else {
        let s = sanitize_quoted_content(content);
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
        return match extract_app_message_type(&normalized).parse::<i64>().unwrap_or(0) {
            6 => "[文件]".into(),
            19 => "[聊天记录]".into(),
            33 | 36 => "[小程序]".into(),
            _ => "[链接]".into(),
        };
    }
    let out = format_plain_export_content(content, ty, &PlainOpts::default(), None, None, None, false, None);
    if out.is_empty() { "[消息]".into() } else { out }
}

#[derive(Clone, Debug)]
pub struct QuotedDisplay {
    pub reply_text: String,
    pub quoted_sender: Option<String>,
    pub quoted_preview: String,
}

pub fn extract_quoted_reply_display(content: &str) -> Option<QuotedDisplay> {
    let normalized = normalize_app_message_content(content);
    let refer = refermsg_xml(&normalized)?;
    let info = parse_quote_message(&normalized);
    let reply_text = strip_sender_prefix(&extract_xml_value(&normalized, "title"));
    let preview = info.content.clone().unwrap_or_else(|| {
        format_quoted_reference_preview(&extract_xml_value(&refer, "content"), &extract_xml_value(&refer, "type"))
    });
    if reply_text.is_empty() && preview.is_empty() {
        return None;
    }
    Some(QuotedDisplay {
        reply_text,
        quoted_sender: info.sender,
        quoted_preview: if preview.is_empty() { "[消息]".into() } else { preview },
    })
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

pub fn extract_reply_to_message_id(content: &str) -> Option<String> {
    let normalized = normalize_app_message_content(content);
    let refer = refermsg_xml(&normalized)?;
    let v = normalize_unsigned_token(&extract_xml_value(&refer, "svrid"));
    if v != "0" { Some(v) } else { None }
}

/// Sender wxid of a quoted message, following `resolveQuotedSenderUsername`.
pub fn quoted_sender_username(content: &str) -> Option<String> {
    let normalized = normalize_app_message_content(content);
    let refer = refermsg_xml(&normalized)?;
    let chat = extract_xml_value(&refer, "chatusr");
    let from = extract_xml_value(&refer, "fromusr");
    if !chat.trim().is_empty() {
        return Some(chat.trim().to_string());
    }
    if from.trim().ends_with("@chatroom") {
        return None;
    }
    let f = from.trim().to_string();
    if f.is_empty() { None } else { Some(f) }
}

// ─────────────────────────────── transfers ───────────────────────────────

fn candidate_ids(values: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in values {
        let t = v.trim();
        if !t.is_empty() && !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    }
    out
}

pub fn is_same_wxid(lhs: &str, rhs: &str) -> bool {
    let left: Vec<String> = candidate_ids(&[lhs]).into_iter().map(|s| s.to_lowercase()).collect();
    if left.is_empty() {
        return false;
    }
    candidate_ids(&[rhs]).iter().any(|r| left.contains(&r.to_lowercase()))
}

pub fn get_transfer_prefix(content: &str, my_wxid: Option<&str>, sender_wxid: Option<&str>) -> &'static str {
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
    let prefix = if content.starts_with("[转账收款]") { "[转账收款]" } else { "[转账]" };
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

// ──────────────────────────── plain text rendering ────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct PlainOpts {
    pub export_voice_as_text: bool,
}

fn parse_duration_seconds(value: &str) -> Option<i64> {
    let n: f64 = value.trim().parse().ok()?;
    if !n.is_finite() || n <= 0.0 {
        return None;
    }
    Some(if n >= 1000.0 { (n / 1000.0).round() as i64 } else { n.round() as i64 })
}

fn extract_amount_from_text(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    rx(r"([¥￥]\s*\d+(?:\.\d+)?|\d+(?:\.\d+)?)")
        .captures(text)
        .map(|c| rx(r"\s+").replace_all(&c[1], "").to_string())
}

/// `parseMessageContent` — text used by ChatLab / JSON / WeClone.
pub fn parse_message_content(
    content: &str,
    local_type: i64,
    my_wxid: Option<&str>,
    sender_wxid: Option<&str>,
    emoji_caption: Option<&str>,
) -> Option<String> {
    if content.is_empty() && local_type == 47 {
        return Some(format_emoji_semantic_text(emoji_caption));
    }
    if content.is_empty() {
        return None;
    }
    let normalized = normalize_app_message_content(content);
    let xml_type = extract_app_message_type(&normalized);
    let with_title = |prefix: &str, title: &str| if title.is_empty() { prefix.to_string() } else { format!("{prefix} {title}") };
    match local_type {
        1 => Some(strip_sender_prefix(content)),
        3 => Some("[图片]".into()),
        34 => Some("[语音消息]".into()),
        42 => Some("[名片]".into()),
        43 => Some("[视频]".into()),
        47 => Some(format_emoji_semantic_text(emoji_caption)),
        48 => Some(location_text(content)),
        49 => {
            let title = extract_xml_value(&normalized, "title");
            let ty = extract_app_message_type(&normalized);
            let song = extract_xml_value(&normalized, "songname");
            if ty == "2000" {
                let feedesc = extract_xml_value(&normalized, "feedesc");
                let memo = extract_xml_value(&normalized, "pay_memo");
                let prefix = get_transfer_prefix(&normalized, my_wxid, sender_wxid);
                if !feedesc.is_empty() {
                    return Some(if memo.is_empty() { format!("{prefix} {feedesc}") } else { format!("{prefix} {feedesc} {memo}") });
                }
                return Some(prefix.to_string());
            }
            Some(match ty.as_str() {
                "3" => if !song.is_empty() { with_title("[音乐]", &song) } else { with_title("[音乐]", &title) },
                "6" => with_title("[文件]", &title),
                "19" => format_forward_chat_record_content(&normalized),
                "33" | "36" => with_title("[小程序]", &title),
                "57" => match extract_quoted_reply_display(content) {
                    Some(d) => build_quoted_reply_text(&d),
                    None => if title.is_empty() { "[引用消息]".into() } else { title },
                },
                _ => with_title("[链接]", &title),
            })
        }
        50 => Some(parse_voip_message(content)),
        10000 | 266287972401 => Some(clean_system_message(content)),
        244813135921 => match extract_quoted_reply_display(content) {
            Some(d) => Some(build_quoted_reply_text(&d)),
            None => {
                let t = extract_xml_value(content, "title");
                Some(if t.is_empty() { "[引用消息]".into() } else { t })
            }
        },
        _ => {
            if !xml_type.is_empty() {
                let title = extract_xml_value(content, "title");
                match xml_type.as_str() {
                    "87" => {
                        let t = extract_xml_value(content, "textannouncement");
                        return Some(if t.is_empty() { "[群公告]".into() } else { format!("[群公告] {t}") });
                    }
                    "2000" => {
                        let feedesc = extract_xml_value(content, "feedesc");
                        let memo = extract_xml_value(content, "pay_memo");
                        let prefix = get_transfer_prefix(content, my_wxid, sender_wxid);
                        if !feedesc.is_empty() {
                            return Some(if memo.is_empty() { format!("{prefix} {feedesc}") } else { format!("{prefix} {feedesc} {memo}") });
                        }
                        return Some(prefix.to_string());
                    }
                    "3" => return Some(with_title("[音乐]", &title)),
                    "6" => return Some(with_title("[文件]", &title)),
                    "19" => return Some(format_forward_chat_record_content(&normalized)),
                    "33" | "36" => return Some(with_title("[小程序]", &title)),
                    "57" => {
                        return Some(match extract_quoted_reply_display(content) {
                            Some(d) => build_quoted_reply_text(&d),
                            None => if title.is_empty() { "[引用消息]".into() } else { title },
                        })
                    }
                    "53" => {
                        if title.is_empty() {
                            return Some("[接龙]".into());
                        }
                        let first = title.lines().map(|l| l.trim()).find(|l| !l.is_empty()).unwrap_or(&title).to_string();
                        return Some(format!("[接龙] {first}"));
                    }
                    "5" | "49" => return Some(with_title("[链接]", &title)),
                    _ => {}
                }
                if !title.is_empty() {
                    return Some(title);
                }
            }
            let s = strip_sender_prefix(&normalized);
            if s.is_empty() { None } else { Some(s) }
        }
    }
}

/// `formatPlainExportContent` — text used by TXT / Excel / HTML.
pub fn format_plain_export_content(
    content: &str,
    local_type: i64,
    opts: &PlainOpts,
    voice_transcript: Option<&str>,
    my_wxid: Option<&str>,
    sender_wxid: Option<&str>,
    _is_send: bool,
    emoji_caption: Option<&str>,
) -> String {
    let readable = extract_readable_system_message_text(content);
    if !readable.is_empty() && is_readable_system_message(local_type, content) {
        return readable;
    }
    match local_type {
        3 => return "[图片]".into(),
        1 => return strip_sender_prefix(content),
        34 => {
            if opts.export_voice_as_text {
                return voice_transcript.filter(|t| !t.is_empty()).map(str::to_string).unwrap_or_else(|| "[语音消息 - 转文字失败]".into());
            }
            return "[其他消息]".into();
        }
        42 => {
            let n = normalize_app_message_content(content);
            let nick = ["nickname", "displayname", "name"].iter().map(|k| extract_xml_value(&n, k)).find(|v| !v.is_empty()).unwrap_or_default();
            return if nick.is_empty() { "[名片]".into() } else { format!("[名片]{nick}") };
        }
        43 => {
            let n = normalize_app_message_content(content);
            let len = ["playlength", "playLength", "length", "duration"].iter().map(|k| extract_xml_value(&n, k)).find(|v| !v.is_empty());
            return match len.and_then(|v| parse_duration_seconds(&v)) {
                Some(s) if s > 0 => format!("[视频]{s}s"),
                _ => "[视频]".into(),
            };
        }
        47 => return format_emoji_semantic_text(emoji_caption),
        48 => return location_text(content),
        50 => return parse_voip_message(content),
        10000 | 266287972401 => return clean_system_message(content),
        _ => {}
    }
    let normalized = normalize_app_message_content(content);
    let is_app = normalized.contains("<appmsg") || normalized.contains("<msg>");
    if local_type == 49 || is_app {
        let sub: i64 = extract_app_message_type(&normalized).parse().unwrap_or(0);
        let title = {
            let t = extract_xml_value(&normalized, "title");
            if !t.is_empty() { t } else { extract_xml_value(&normalized, "appname") }
        };
        if sub == 87 {
            let t = extract_xml_value(&normalized, "textannouncement");
            return if t.is_empty() { "[群公告]".into() } else { format!("[群公告]{t}") };
        }
        if sub == 2000 || title.contains("转账") || normalized.contains("transfer") {
            let feedesc = extract_xml_value(&normalized, "feedesc");
            let memo = extract_xml_value(&normalized, "pay_memo");
            let prefix = get_transfer_prefix(&normalized, my_wxid, sender_wxid);
            if !feedesc.is_empty() {
                return if memo.is_empty() { format!("{prefix}{feedesc}") } else { format!("{prefix}{feedesc} {memo}") };
            }
            let joined: Vec<String> = [
                title.clone(),
                extract_xml_value(&normalized, "des"),
                extract_xml_value(&normalized, "money"),
                extract_xml_value(&normalized, "amount"),
                extract_xml_value(&normalized, "fee"),
            ]
            .into_iter()
            .filter(|v| !v.is_empty())
            .collect();
            return match extract_amount_from_text(&joined.join(" ")) {
                Some(a) => format!("{prefix}{a}"),
                None => prefix.to_string(),
            };
        }
        if sub == 3 || normalized.contains("<musicurl") || normalized.contains("<songname") {
            let song = extract_xml_value(&normalized, "songname");
            let name = if !song.is_empty() { song } else if !title.is_empty() { title.clone() } else { "音乐".into() };
            return format!("[音乐]{name}");
        }
        if sub == 6 {
            let f = extract_xml_value(&normalized, "filename");
            let name = if !f.is_empty() { f } else if !title.is_empty() { title.clone() } else { "文件".into() };
            return format!("[文件]{name}");
        }
        if title.contains("红包") || normalized.contains("hongbao") {
            return format!("[红包]{}", if title.is_empty() { "微信红包" } else { &title });
        }
        if sub == 19 || normalized.contains("<recorditem") {
            return format_forward_chat_record_content(&normalized);
        }
        if sub == 33 || sub == 36 {
            let a = extract_xml_value(&normalized, "appname");
            let name = if !a.is_empty() { a } else if !title.is_empty() { title.clone() } else { "小程序".into() };
            return format!("[小程序]{name}");
        }
        if sub == 57 {
            if let Some(d) = extract_quoted_reply_display(content) {
                return build_quoted_reply_text(&d);
            }
            return if title.is_empty() { "[引用消息]".into() } else { title };
        }
        if !title.is_empty() {
            return format!("[链接]{title}");
        }
        return "[其他消息]".into();
    }
    "[其他消息]".into()
}

// ───────────────────────────────── link cards ─────────────────────────────────

pub struct LinkCard {
    pub title: String,
    pub url: String,
}

pub fn normalize_http_link_url(raw: &str) -> String {
    let value = raw.trim().replace("&amp;", "&");
    if value.is_empty() {
        return String::new();
    }
    let parse_http = |candidate: &str| -> String {
        let lower = candidate.to_ascii_lowercase();
        if (lower.starts_with("http://") || lower.starts_with("https://")) && candidate.len() > 8 && !candidate.contains(' ') {
            candidate.to_string()
        } else {
            String::new()
        }
    };
    if let Some(rest) = value.strip_prefix("//") {
        return parse_http(&format!("https://{rest}"));
    }
    let direct = parse_http(&value);
    if !direct.is_empty() {
        return direct;
    }
    let has_scheme = rx(r"^[a-zA-Z][a-zA-Z0-9+.-]*:").is_match(&value);
    let domain_like = rx(r"^[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}(?:[/:?#].*)?$").is_match(&value);
    if !has_scheme && domain_like {
        return parse_http(&format!("https://{value}"));
    }
    String::new()
}

pub fn extract_html_link_card(content: &str, local_type: i64) -> Option<LinkCard> {
    if content.is_empty() {
        return None;
    }
    let normalized = normalize_app_message_content(content);
    let is_app = local_type == 49 || normalized.contains("<appmsg") || normalized.contains("<msg>");
    if !is_app {
        return None;
    }
    let sub = extract_app_message_type(&normalized);
    if !sub.is_empty() && sub != "5" && sub != "49" {
        return None;
    }
    let url = ["url", "shareurlopen", "shareurloriginal", "shareurl", "shorturl", "dataurl", "lowurl", "streamvideoweburl", "weburl"]
        .iter()
        .map(|k| normalize_http_link_url(&extract_xml_value(&normalized, k)))
        .find(|u| !u.is_empty())?;
    let mut title = extract_xml_value(&normalized, "title");
    if title.is_empty() {
        title = extract_xml_value(&normalized, "des");
    }
    if title.is_empty() {
        title = url.clone();
    }
    let title = strip_sender_prefix(&title);
    let title = if title.is_empty() { url.clone() } else { title };
    Some(LinkCard { title, url })
}

fn link_card_display_title(card: &LinkCard) -> String {
    let t = strip_sender_prefix(card.title.trim());
    if !t.is_empty() {
        t
    } else if !card.url.is_empty() {
        card.url.clone()
    } else {
        "链接".into()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LinkStyle {
    Markdown,
    AppendUrl,
}

pub fn format_link_card_export_text(content: &str, local_type: i64, style: LinkStyle) -> Option<String> {
    let card = extract_html_link_card(content, local_type)?;
    if card.url.is_empty() {
        return None;
    }
    let title = link_card_display_title(&card);
    Some(match style {
        LinkStyle::Markdown => format!("[{title}]({})", card.url),
        LinkStyle::AppendUrl => {
            let prefix = if !title.is_empty() && title != card.url { format!("[链接] {title}") } else { "[链接]".to_string() };
            format!("{prefix}\n{}", card.url)
        }
    })
}

// ───────────────────────────── arkme metadata ─────────────────────────────

fn extract_finder_feed_desc(content: &str) -> String {
    rx(r"(?is)<finderFeed.*?<desc>(.*?)</desc>")
        .captures(content)
        .map(|c| c[1].replace("<![CDATA[", "").replace("]]>", "").trim().to_string())
        .unwrap_or_default()
}

fn first_non_empty(n: &str, tags: &[&str]) -> String {
    tags.iter().map(|t| extract_xml_value(n, t)).find(|v| !v.is_empty()).unwrap_or_default()
}

/// `extractArkmeAppMessageMeta` — extra structured fields for `arkme-json` / `json`.
pub fn extract_arkme_app_message_meta(content: &str, local_type: i64) -> Option<Map<String, Value>> {
    if content.is_empty() {
        return None;
    }
    let n = normalize_app_message_content(content);
    let looks_app = local_type == 49 || local_type == 244813135921 || n.contains("<appmsg") || n.contains("<msg>");
    let has_refer = n.contains("<refermsg>");
    let xml_type = extract_app_message_type(&n);
    let is_finder = xml_type == "51" || n.contains("<finder") || n.contains("finderusername") || n.contains("finderobjectid");
    let is_music = xml_type == "3" || n.contains("<musicurl") || n.contains("<playurl>") || n.contains("<dataurl>");
    if !looks_app && !is_finder && !has_refer {
        return None;
    }
    let kind: Option<&str> = if is_finder {
        Some("finder")
    } else if xml_type == "2001" {
        Some("red-packet")
    } else if is_music {
        Some("music")
    } else if xml_type == "33" || xml_type == "36" {
        Some("miniapp")
    } else if xml_type == "6" {
        Some("file")
    } else if xml_type == "19" {
        Some("chat-record")
    } else if xml_type == "2000" {
        Some("transfer")
    } else if xml_type == "87" {
        Some("announcement")
    } else if xml_type == "57" || has_refer || local_type == 244813135921 {
        Some("quote")
    } else if xml_type == "53" {
        Some("solitaire")
    } else if xml_type == "5" || xml_type == "49" {
        Some("link")
    } else if looks_app {
        Some("card")
    } else {
        None
    };
    let mut meta = Map::new();
    let put = |meta: &mut Map<String, Value>, key: &str, value: String| {
        if !value.is_empty() {
            meta.insert(key.into(), Value::String(value));
        }
    };
    if !xml_type.is_empty() {
        put(&mut meta, "appMsgType", xml_type.clone());
    } else if kind == Some("quote") {
        put(&mut meta, "appMsgType", "57".into());
    }
    if let Some(k) = kind {
        put(&mut meta, "appMsgKind", k.into());
    }
    let desc = first_non_empty(&n, &["des", "desc"]);
    let app_name = extract_xml_value(&n, "appname");
    let source_name = first_non_empty(&n, &["sourcename", "sourcedisplayname"]);
    let source_user = extract_xml_value(&n, "sourceusername");
    let thumb = first_non_empty(&n, &["thumburl", "cdnthumburl", "cover", "coverurl", "thumbUrl", "coverUrl"]);
    put(&mut meta, "appMsgDesc", desc);
    put(&mut meta, "appMsgAppName", app_name.clone());
    put(&mut meta, "appMsgSourceName", source_name.clone());
    put(&mut meta, "appMsgSourceUsername", source_user);
    put(&mut meta, "appMsgThumbUrl", thumb.clone());

    if kind == Some("quote") {
        let q = parse_quote_message(&n);
        put(&mut meta, "quotedContent", q.content.unwrap_or_default());
        put(&mut meta, "quotedSender", q.sender.unwrap_or_default());
        put(&mut meta, "quotedType", q.quote_type.unwrap_or_default());
        put(&mut meta, "quotedSvrid", q.svrid.unwrap_or_default());
    }
    if kind == Some("link") {
        let card = extract_html_link_card(&n, local_type);
        let url = card.as_ref().map(|c| c.url.clone()).filter(|u| !u.is_empty()).unwrap_or_else(|| {
            normalize_http_link_url(&first_non_empty(&n, &["shareurl", "shorturl", "dataurl"]))
        });
        if let Some(c) = &card {
            put(&mut meta, "linkTitle", c.title.clone());
        }
        put(&mut meta, "linkUrl", url);
        put(&mut meta, "linkThumb", thumb.clone());
    }
    if is_music {
        put(&mut meta, "musicTitle", first_non_empty(&n, &["songname", "title"]));
        put(&mut meta, "musicUrl", first_non_empty(&n, &["musicurl", "playurl", "songalbumurl"]));
        put(&mut meta, "musicDataUrl", first_non_empty(&n, &["dataurl", "lowurl"]));
        put(&mut meta, "musicAlbumUrl", extract_xml_value(&n, "songalbumurl"));
        put(&mut meta, "musicCoverUrl", first_non_empty(&n, &["thumburl", "cdnthumburl", "coverurl", "cover"]));
        put(&mut meta, "musicSinger", first_non_empty(&n, &["singername", "artist", "albumartist"]));
        put(&mut meta, "musicAppName", app_name);
        put(&mut meta, "musicSourceName", extract_xml_value(&n, "sourcename"));
        let dur = first_non_empty(&n, &["playlength", "play_length", "duration"]);
        if let Some(d) = parse_duration_seconds(&dur) {
            meta.insert("musicDuration".into(), json!(d));
        }
    }
    if !is_finder {
        return if meta.is_empty() { None } else { Some(meta) };
    }
    let raw_title = extract_xml_value(&n, "title");
    let feed_desc = extract_finder_feed_desc(&n);
    let finder_title = if raw_title.is_empty() || raw_title.contains("不支持") { feed_desc } else { raw_title };
    put(&mut meta, "finderTitle", finder_title);
    put(&mut meta, "finderDesc", first_non_empty(&n, &["des", "desc"]));
    put(&mut meta, "finderUsername", first_non_empty(&n, &["finderusername", "finder_username", "finderuser"]));
    put(&mut meta, "finderNickname", first_non_empty(&n, &["findernickname", "finder_nickname"]));
    put(&mut meta, "finderCoverUrl", first_non_empty(&n, &["thumbUrl", "coverUrl", "thumburl", "coverurl"]));
    put(&mut meta, "finderAvatar", extract_xml_value(&n, "avatar"));
    let dur = first_non_empty(&n, &["videoPlayDuration", "duration"]);
    if let Some(d) = parse_duration_seconds(&dur) {
        meta.insert("finderDuration".into(), json!(d));
    }
    put(&mut meta, "finderObjectId", first_non_empty(&n, &["finderobjectid", "finder_objectid", "objectid", "object_id"]));
    put(&mut meta, "finderUrl", first_non_empty(&n, &["url", "shareurl"]));
    if meta.is_empty() { None } else { Some(meta) }
}

pub fn extract_arkme_contact_card_meta(content: &str, local_type: i64) -> Option<Map<String, Value>> {
    if content.is_empty() || local_type != 42 {
        return None;
    }
    let n = normalize_app_message_content(content);
    let read = |attr: &str| {
        let a = extract_xml_attribute(&n, "msg", attr);
        if !a.is_empty() { a } else { extract_xml_value(&n, attr) }
    };
    let first = |attrs: &[&str]| attrs.iter().map(|a| read(a)).find(|v| !v.is_empty()).unwrap_or_default();
    let mut meta = Map::new();
    meta.insert("cardKind".into(), json!("contact-card"));
    let mut put = |key: &str, value: String| {
        if !value.is_empty() {
            meta.insert(key.into(), Value::String(value));
        }
    };
    put("contactCardWxid", first(&["username", "encryptusername", "encrypt_user_name"]));
    put("contactCardNickname", read("nickname"));
    put("contactCardAlias", read("alias"));
    put("contactCardRemark", read("remark"));
    put("contactCardProvince", read("province"));
    put("contactCardCity", read("city"));
    put("contactCardSignature", first(&["sign", "signature"]));
    put("contactCardAvatar", first(&["smallheadimgurl", "bigheadimgurl", "headimgurl", "avatar"]));
    let sex = read("sex");
    if let Ok(g) = sex.trim().parse::<i64>() {
        if g >= 0 {
            meta.insert("contactCardGender".into(), json!(g));
        }
    }
    Some(meta)
}

// ─────────────────────────── row → ExportMsg ───────────────────────────

pub struct CollectOptions<'a> {
    pub session_id: &'a str,
    pub my_wxid: &'a str,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub sender_filter: Option<&'a str>,
}

/// `collectMessages` (full mode): normalise raw rows and sort chronologically.
pub fn collect_messages(rows: &[Value], opts: &CollectOptions<'_>) -> Vec<ExportMsg> {
    let mut out: Vec<ExportMsg> = Vec::new();
    for row in rows {
        let create_time = get_timestamp_seconds(row);
        if let Some(s) = opts.start.filter(|s| *s > 0) {
            if create_time > 0 && create_time < s {
                continue;
            }
        }
        if let Some(e) = opts.end.filter(|e| *e > 0) {
            if create_time > 0 && create_time > e {
                continue;
            }
        }
        let local_type = row_int(row, &["local_type", "localType", "type", "msg_type", "msgType", "WCDB_CT_local_type"], 1);
        let content = decode_message_content(row);
        let sender_username = row.get("sender_username").map(value_to_string).unwrap_or_default();
        let is_send_raw = row.get("computed_is_send").or_else(|| row.get("is_send")).map(value_to_string).unwrap_or_else(|| "0".into());
        let is_send = is_send_raw.trim().parse::<i64>().unwrap_or(0) == 1;
        let local_id = row_int(row, &["local_id", "localId", "LocalId", "msg_local_id", "msgLocalId", "MsgLocalId", "msg_id", "msgId", "MsgId", "id", "WCDB_CT_local_id"], 0);
        let server_keys = ["server_id", "serverId", "ServerId", "msg_server_id", "msgServerId", "MsgServerId", "svr_id", "svrId", "msg_svr_id", "msgSvrId", "MsgSvrId", "WCDB_CT_server_id"];
        let server_raw = row_field(row, &server_keys).map(value_to_string).map(|v| normalize_unsigned_token(&v)).unwrap_or_else(|| "0".into());
        let server_id = row_int(row, &server_keys, 0);

        let actual_sender = if local_type == 10000 || local_type == 266287972401 {
            let info = extract_revoker_info(&content);
            if info.is_revoke {
                if info.is_self_revoke {
                    opts.my_wxid.to_string()
                } else if let Some(w) = info.revoker_wxid {
                    w
                } else {
                    opts.session_id.to_string()
                }
            } else {
                opts.session_id.to_string()
            }
        } else if is_send {
            opts.my_wxid.to_string()
        } else if !sender_username.is_empty() {
            sender_username.clone()
        } else {
            opts.session_id.to_string()
        };
        if let Some(filter) = opts.sender_filter.filter(|f| !f.trim().is_empty()) {
            if !is_same_wxid(&actual_sender, filter.trim()) {
                continue;
            }
        }

        let mut msg = ExportMsg {
            local_id,
            server_id,
            server_id_raw: if server_raw != "0" { Some(server_raw) } else { None },
            create_time,
            local_type,
            content: content.clone(),
            sender_username: actual_sender,
            is_send,
            ..Default::default()
        };

        if local_type == 48 && !content.is_empty() {
            if let Some((lat, lng, poi, label)) = extract_location_meta(&content, local_type) {
                msg.location_lat = lat;
                msg.location_lng = lng;
                msg.location_poiname = poi;
                msg.location_label = label;
            }
        }
        if local_type == 47 {
            let col = |keys: &[&str]| row_field(row, keys).map(value_to_string).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
            msg.emoji_cdn_url = col(&["emoji_cdn_url", "emojiCdnUrl"]);
            msg.emoji_md5 = col(&["emoji_md5", "emojiMd5"]).and_then(|v| normalize_md5(&v));
            let packed = col(&["packed_info", "packedInfo", "PackedInfo"]).unwrap_or_default();
            let reserved = col(&["reserved0", "Reserved0"]).unwrap_or_default();
            let supplemental = format!("{}\n{}", decode_maybe_compressed(&packed), decode_maybe_compressed(&reserved));
            if !content.is_empty() {
                if msg.emoji_cdn_url.is_none() {
                    msg.emoji_cdn_url = extract_emoji_url(&content);
                }
                if msg.emoji_md5.is_none() {
                    msg.emoji_md5 = extract_emoji_md5(&content);
                }
            }
            if msg.emoji_cdn_url.is_none() {
                msg.emoji_cdn_url = extract_emoji_url(&supplemental);
            }
            if msg.emoji_md5.is_none() {
                msg.emoji_md5 = extract_emoji_md5(&supplemental).or_else(|| extract_loose_hex_md5(&supplemental));
            }
        }
        msg.image_md5 = row_field(row, &["image_md5", "imageMd5"]).map(value_to_string).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        if local_type == 3 {
            msg.image_dat_name = extract_image_dat_name_from_row(row, &content);
            if msg.image_md5.is_none() && !content.is_empty() {
                msg.image_md5 = extract_image_md5(&content);
            }
        }
        if local_type == 43 && !content.is_empty() {
            msg.video_md5 = extract_video_md5(&content);
        }
        let app_like = local_type == 49 || content.contains("<appmsg") || content.contains("&lt;appmsg");
        if !content.is_empty() && app_like {
            if let Some(meta) = extract_file_app_message_meta(&content) {
                msg.xml_type = meta.xml_type;
                msg.file_name = meta.file_name;
                msg.file_size = meta.file_size;
                msg.file_ext = meta.file_ext;
                msg.file_md5 = meta.file_md5;
            }
            let normalized = normalize_app_message_content(&content);
            if extract_app_message_type(&normalized) == "19" {
                msg.chat_record_list = parse_chat_history(&normalized);
            }
        }
        out.push(msg);
    }
    out.sort_by(|a, b| a.create_time.cmp(&b.create_time).then(a.local_id.cmp(&b.local_id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_value_strips_cdata() {
        let xml = "<msg><title><![CDATA[ hello ]]></title></msg>";
        assert_eq!(extract_xml_value(xml, "title"), "hello");
        assert_eq!(extract_xml_attribute(r#"<location x="1.5" y="2" poiname="P"/>"#, "location", "poiname"), "P");
    }

    #[test]
    fn sender_prefix_keeps_urls() {
        assert_eq!(strip_sender_prefix("wxid_abc:hello"), "hello");
        assert_eq!(strip_sender_prefix("http://x.y"), "http://x.y");
        assert_eq!(strip_sender_prefix("  a-b_c:x"), "x");
    }

    #[test]
    fn app_type_ignores_refermsg() {
        let xml = "<msg><appmsg><title>t</title><type>57</type><refermsg><type>1</type></refermsg></appmsg></msg>";
        assert_eq!(extract_app_message_type(xml), "57");
    }

    #[test]
    fn quoted_reply_text() {
        let xml = "<msg><appmsg><title>ok</title><type>57</type><refermsg><type>1</type><displayname>Bob</displayname><content>hi there</content><svrid>123</svrid></refermsg></appmsg></msg>";
        let d = extract_quoted_reply_display(xml).unwrap();
        assert_eq!(build_quoted_reply_text(&d), "ok[引用 Bob：hi there]");
        assert_eq!(extract_reply_to_message_id(xml).as_deref(), Some("123"));
        assert_eq!(parse_message_content(xml, 49, None, None, None).unwrap(), "ok[引用 Bob：hi there]");
    }

    #[test]
    fn partial_quote_uses_indexes() {
        let xml = "<refermsg><type>1</type><content>abcdefg</content><startindex>1</startindex><endindex>3</endindex></refermsg>";
        assert_eq!(extract_preferred_quoted_text(xml), "bcd");
    }

    #[test]
    fn transfer_texts() {
        let xml = "<msg><appmsg><title>转账</title><type>2000</type><wcpayinfo><feedesc>￥1.00</feedesc><paysubtype>1</paysubtype></wcpayinfo></appmsg></msg>";
        assert_eq!(parse_message_content(xml, 49, None, None, None).unwrap(), "[转账] ￥1.00");
        assert_eq!(append_transfer_desc("[转账] ￥1.00", "A 转账给 B"), "[转账] (A 转账给 B) ￥1.00");
        let xml2 = "<appmsg><type>2000</type><paysubtype>3</paysubtype></appmsg>";
        assert_eq!(get_transfer_prefix(xml2, None, None), "[转账收款]");
    }

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
        assert!(is_readable_system_message(10000, "x"));
        let rv = extract_revoker_info("你撤回了一条消息");
        assert!(rv.is_revoke && rv.is_self_revoke);
    }

    #[test]
    fn forward_chat_records() {
        let xml = "<msg><appmsg><title>群聊的聊天记录</title><type>19</type><recorditem><![CDATA[<recordinfo><datalist><dataitem datatype=\"1\"><sourcename>Bob</sourcename><sourcetime>2024-01-01 10:00:00</sourcetime><datadesc>hello</datadesc></dataitem></datalist></recordinfo>]]></recorditem></appmsg></msg>";
        let text = format_forward_chat_record_content(xml);
        assert_eq!(text, "[转发的聊天记录]群聊的聊天记录\nBob: hello");
        assert_eq!(convert_message_type(49, xml), 7);
    }

    #[test]
    fn plain_content_per_type() {
        let opts = PlainOpts::default();
        assert_eq!(format_plain_export_content("wxid_a:hi", 1, &opts, None, None, None, false, None), "hi");
        assert_eq!(format_plain_export_content("", 3, &opts, None, None, None, false, None), "[图片]");
        assert_eq!(format_plain_export_content("x", 34, &opts, None, None, None, false, None), "[其他消息]");
        let v = "<msg><videomsg playlength=\"12\"/><playlength>12</playlength></msg>";
        assert_eq!(format_plain_export_content(v, 43, &opts, None, None, None, false, None), "[视频]12s");
        let loc = "<msg><location x=\"31.2\" y=\"121.4\" poiname=\"Tower\" label=\"Road 1\"/></msg>";
        assert_eq!(format_plain_export_content(loc, 48, &opts, None, None, None, false, None), "[位置] Tower Road 1 (31.2,121.4)");
    }

    #[test]
    fn link_cards() {
        let xml = "<msg><appmsg><title>Hello</title><type>5</type><url>https://example.com/a?b=1&amp;c=2</url></appmsg></msg>";
        let c = extract_html_link_card(xml, 49).unwrap();
        assert_eq!(c.url, "https://example.com/a?b=1&c=2");
        assert_eq!(format_link_card_export_text(xml, 49, LinkStyle::Markdown).unwrap(), "[Hello](https://example.com/a?b=1&c=2)");
        assert_eq!(normalize_http_link_url("example.com/x"), "https://example.com/x");
        assert_eq!(normalize_http_link_url("javascript:alert(1)"), "");
    }

    #[test]
    fn arkme_meta_kinds() {
        let xml = "<msg><appmsg><title>Song</title><type>3</type><songname>S</songname><singername>Z</singername><musicurl>https://m</musicurl></appmsg></msg>";
        let meta = extract_arkme_app_message_meta(xml, 49).unwrap();
        assert_eq!(meta["appMsgKind"], "music");
        assert_eq!(meta["musicSinger"], "Z");
        let card = "<msg username=\"wxid_x\" nickname=\"Nick\" sex=\"1\" city=\"SH\"/>";
        let cm = extract_arkme_contact_card_meta(card, 42).unwrap();
        assert_eq!(cm["contactCardWxid"], "wxid_x");
        assert_eq!(cm["contactCardGender"], 1);
    }

    #[test]
    fn row_normalisation() {
        let rows = vec![
            json!({"local_id": "2", "server_id": "9007199254740993", "create_time": "1700000100", "local_type": "1", "message_content": "wxid_b:second", "sender_username": "wxid_b", "is_send": "0"}),
            json!({"local_id": "1", "create_time": "1700000000", "local_type": "1", "message_content": "first", "is_send": "1"}),
        ];
        let msgs = collect_messages(&rows, &CollectOptions { session_id: "wxid_b", my_wxid: "wxid_me", start: None, end: None, sender_filter: None });
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "first");
        assert_eq!(msgs[0].sender_username, "wxid_me");
        assert_eq!(msgs[1].server_id_raw.as_deref(), Some("9007199254740993"));
        let filtered = collect_messages(&rows, &CollectOptions { session_id: "wxid_b", my_wxid: "wxid_me", start: Some(1700000050), end: None, sender_filter: None });
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn compressed_content_decodes() {
        let payload = "<msg>hello world, this is compressed</msg>";
        let compressed = zstd::encode_all(payload.as_bytes(), 1).unwrap();
        let hex: String = compressed.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(decode_maybe_compressed(&hex), payload);
        let row = json!({"message_content": hex, "WCDB_CT_message_content": "4"});
        assert_eq!(decode_message_content(&row), payload);
    }

    #[test]
    fn timestamps() {
        assert_eq!(normalize_timestamp_seconds(1_700_000_000_000.0), 1_700_000_000);
        assert_eq!(normalize_row_timestamp(&json!("20240102030405")) > 0, true);
        assert_eq!(normalize_unsigned_token("00012"), "12");
        assert_eq!(normalize_unsigned_token("abc"), "0");
        assert_eq!(last_hex_token("zz0123456789abcdef0123456789abcdef00zz", 32, 32).unwrap().len(), 32);
    }
}
