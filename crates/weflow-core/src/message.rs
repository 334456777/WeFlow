//! Message decoding and formatting, ported from WeFlow's desktop app's `exportService.ts`.
//!
//! Raw WCDB rows go in, [`ExportMsg`] values and human-readable text come out. The
//! functions mirror their TypeScript counterparts so exports stay comparable with the
//! desktop app. Placeholder strings such as `[图片]` are data and intentionally stay as
//! in the desktop exports.

use serde_json::{json, Map, Value};
use weflow_native::msg_row::MsgRow;

pub(crate) mod kind;
pub(crate) mod row;
pub mod time;
pub(crate) mod xml;

use row::unsigned_token;
use time::{normalize_timestamp_seconds, parse_compact_datetime};
use xml::{
    decode_html_entities, decode_html_entities_cow, extract_app_message_type,
    extract_xml_attribute, extract_xml_value, last_hex_token, looks_like_wxid,
    normalize_app_message_content, normalize_app_message_cow, rx, sanitize_quoted_content,
    sender_prefix_len, strip_sender_line, strip_sender_prefix, MsgXml,
};

// ───────────────────────────── message struct ─────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct ForwardRecord {
    pub datatype: i64,
    pub sourcename: String,
    pub sourcetime: String,
    pub datadesc: Option<String>,
    pub datatitle: Option<String>,
    pub chat_record_title: Option<String>,
    pub chat_record_desc: Option<String>,
    pub chat_record_list: Option<Vec<ForwardRecord>>,
}

/// A media file copied next to the export: what the exported message points at instead of its placeholder text.
#[derive(Clone, Debug, Default)]
pub struct MediaRef {
    /// `image`, `voice`, `video`, `emoji` or `file`.
    pub kind: &'static str,
    /// Path of the file relative to the export file, with `/` separators.
    pub relative_path: String,
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
    /// The md5 of a sticker sent as an app message of type 8 ([`appmsg_emoticon_md5_in`]).
    pub sticker_md5: Option<String>,
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
    /// Set when the export was asked to embed media and the file could be found.
    pub media: Option<MediaRef>,
}

// one per message of an exported page: its size changes only on purpose (#505)
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<ExportMsg>() == 536);

impl ExportMsg {
    /// JSON view used by the CLI query commands ([`crate::contract::MessageView`]).
    pub fn to_json(&self, my_wxid: &str) -> Value {
        serde_json::to_value(crate::contract::MessageView::of(self, my_wxid)).unwrap_or_default()
    }

    pub fn platform_message_id(&self) -> Option<String> {
        match &self.server_id_raw {
            Some(raw) => unsigned_token(raw),
            None => unsigned_token(&self.server_id.to_string()),
        }
    }
}

// ────────────────────────── system / revoke / voip ──────────────────────────

struct RevokeInfo {
    pub is_revoke: bool,
    pub is_self_revoke: bool,
    pub revoker_wxid: Option<String>,
}

fn extract_revoker_info(content: &str) -> RevokeInfo {
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

fn is_readable_system_message(local_type: i64, content: &str) -> bool {
    local_type == 10000 || has_sysmsg(&normalize_app_message_cow(content))
}

/// Whether normalized content opens a `<sysmsg>` after its sender prefix (see [`is_readable_system_message`]).
fn has_sysmsg(normalized: &str) -> bool {
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

// ─────────────────────────────── emoji helpers ───────────────────────────────

fn normalize_md5(value: &str) -> Option<String> {
    let md5 = value.trim().to_lowercase();
    if md5.len() == 32 && md5.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(md5)
    } else {
        None
    }
}

fn extract_loose_hex_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    // `(?i)(?:emoji|sticker|md5)[^a-fA-F0-9]{0,32}([a-fA-F0-9]{32})`: the leftmost keyword that has one
    let keyed = ["emoji", "sticker", "md5"]
        .into_iter()
        .filter_map(|key| {
            crate::xml::find_all_ci(content, 0, key)
                .find_map(|p| hex32_after_gap(content, p + key.len()).map(|hex| (p, hex)))
        })
        .min_by_key(|(p, _)| *p);
    if let Some((_, hex)) = keyed {
        return normalize_md5(hex);
    }
    first_hex32(content).and_then(normalize_md5)
}

/// Whether `n` hex digits start at `i`.
fn hex_at(h: &[u8], i: usize, n: usize) -> bool {
    h.get(i..i + n)
        .is_some_and(|w| w.iter().all(u8::is_ascii_hexdigit))
}

/// `[^a-fA-F0-9]{0,32}([a-fA-F0-9]{32})` at `i`.
fn hex32_after_gap(s: &str, i: usize) -> Option<&str> {
    let (_, (offset, _)) = s[i..]
        .char_indices()
        .enumerate()
        .take(33)
        .find(|(_, (_, c))| c.is_ascii_hexdigit())?;
    let start = i + offset;
    hex_at(s.as_bytes(), start, 32).then(|| &s[start..start + 32])
}

/// `([a-fA-F0-9]{32})`: the first 32 hex digits in a row.
fn first_hex32(s: &str) -> Option<&str> {
    let mut run = 0;
    for (i, b) in s.bytes().enumerate() {
        run = if b.is_ascii_hexdigit() { run + 1 } else { 0 };
        if run == 32 {
            return Some(&s[i + 1 - 32..=i]);
        }
    }
    None
}

pub fn extract_emoji_url(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let h = content.as_bytes();
    // `(?i)cdnurl\s*=\s*['"]([^'"]+)['"]`
    let quoted = crate::xml::find_all_ci(content, 0, "cdnurl").find_map(|p| {
        let quote = crate::xml::assignment_end(content, p + "cdnurl".len())?;
        if !matches!(h.get(quote), Some(b'\'' | b'"')) {
            return None;
        }
        let start = quote + 1;
        let end = start + h[start..].iter().position(|b| matches!(b, b'\'' | b'"'))?;
        (end > start).then(|| &content[start..end])
    });
    if let Some(url) = quoted {
        return Some(percent_decode(&url.replace("&amp;", "&")));
    }
    // `(?i)cdnurl[^>]*>([^<]+)`
    crate::xml::find_all_ci(content, 0, "cdnurl").find_map(|p| {
        let start = p + 1 + memchr::memchr(b'>', &h[p..])?;
        let end = memchr::memchr(b'<', &h[start..]).map_or(h.len(), |r| start + r);
        (end > start).then(|| content[start..end].to_string())
    })
}

pub fn extract_emoji_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let h = content.as_bytes();
    let assigned = || {
        crate::xml::find_all_ci(content, 0, "md5")
            .filter_map(|p| crate::xml::assignment_end(content, p + "md5".len()))
    };
    // `(?i)md5\s*=\s*['"]([a-fA-F0-9]{32})['"]`
    let found = assigned()
        .find(|&q| {
            matches!(h.get(q), Some(b'\'' | b'"'))
                && hex_at(h, q + 1, 32)
                && matches!(h.get(q + 33), Some(b'\'' | b'"'))
        })
        .map(|q| &content[q + 1..q + 33])
        // `(?i)md5\s*=\s*([a-fA-F0-9]{32})`
        .or_else(|| {
            assigned()
                .find(|&q| hex_at(h, q, 32))
                .map(|q| &content[q..q + 32])
        })
        // `(?i)<md5>([a-fA-F0-9]{32})</md5>`
        .or_else(|| {
            crate::xml::find_all_ci(content, 0, "<md5>")
                .map(|p| p + "<md5>".len())
                .find(|&q| hex_at(h, q, 32) && crate::xml::is_close_at(h, q + 32, b"md5"))
                .map(|q| &content[q..q + 32])
        });
    found
        .and_then(normalize_md5)
        .or_else(|| extract_loose_hex_md5(content))
}

/// `s` with its `%XX` escapes decoded; `s` itself when they do not spell UTF-8.
pub(crate) fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    String::from_utf8(percent_decode_bytes(s)).unwrap_or_else(|_| s.to_string())
}

/// `%XX` escapes (two hex digits) turned into bytes; anything else is kept as it is.
pub fn percent_decode_bytes(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

pub fn format_emoji_semantic_text(caption: Option<&str>) -> String {
    match caption.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => format!("[表情：{c}]"),
        None => "[表情]".into(),
    }
}

/// The md5 (`<emoticonmd5>`) of a sticker sent as an app message of type 8, which looks up the sticker's caption.
fn appmsg_emoticon_md5_in(x: &MsgXml) -> Option<String> {
    // the tag is matched ASCII case-insensitively; see normalized_may_contain for why `content` can rule it out
    crate::xml::find_ci(x.content(), 0, "emoticonmd5")?;
    if x.app_type_twice() != "8" {
        return None;
    }
    let md5 = extract_xml_value(x.once(), "emoticonmd5").to_ascii_lowercase();
    (md5.len() == 32 && md5.chars().all(|c| c.is_ascii_hexdigit())).then_some(md5)
}

// ─────────────────────────── image / video / file ───────────────────────────

fn normalize_image_dat_name_token(value: &str) -> Option<String> {
    let mut text = value.trim().to_string();
    if text.is_empty() {
        return None;
    }
    text = text.replace("&amp;", "&");
    text = percent_decode(&text);
    if let Some(name) = hex_dat_name(&text) {
        return Some(name);
    }
    let no_query = text.split(['?', '#']).next().unwrap_or("").to_string();
    let base = rx(r"^.*[\\/]").replace(&no_query, "").to_string();
    // the regex below needs `.dat`, and no character but these letters' ASCII cases folds to them
    let base = if crate::xml::find_ci(&base, 0, ".dat").is_some() {
        rx(r"(?i)\.(?:t\.)?dat$").replace(&base, "").to_string()
    } else {
        base
    };
    let base = base.trim().to_string();
    if base.is_empty() {
        return None;
    }
    let cdn_token = if base.contains('_') {
        base.split('_').next().unwrap_or("").to_string()
    } else {
        base
    };
    // `^[a-fA-F0-9]{16,64}$`
    if (16..=64).contains(&cdn_token.len()) && cdn_token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(cdn_token.to_lowercase());
    }
    if let Some(t) = last_hex_token(&cdn_token, 32, 32) {
        return Some(t.to_lowercase());
    }
    last_hex_token(&cdn_token, 16, 64).map(|t| t.to_lowercase())
}

pub(crate) fn extract_image_dat_name(content: &str) -> Option<String> {
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
    let pick = candidates
        .into_iter()
        .find(|c| !c.is_empty())
        .unwrap_or_default();
    normalize_image_dat_name_token(&pick)
}

/// The name of an image's `.dat` file in a row's packed info: its printable bytes as text, a `<hex>[.t].dat` name
/// or else a long run of hex digits.
pub(crate) fn dat_name_in_packed_info(bytes: &[u8]) -> Option<String> {
    let text: String = bytes
        .iter()
        .map(|b| {
            if (0x20..=0x7e).contains(b) {
                *b as char
            } else {
                ' '
            }
        })
        .collect();
    if let Some(name) = hex_dat_name(&text) {
        return Some(name);
    }
    // `([0-9a-fA-F]{16,})`: the first run of at least 16 hex digits
    let h = text.as_bytes();
    let mut i = 0;
    while i < h.len() {
        let run = h[i..].iter().take_while(|b| b.is_ascii_hexdigit()).count();
        if run >= 16 {
            return Some(text[i..i + run].to_lowercase());
        }
        i += run.max(1);
    }
    None
}

/// `(?i)([0-9a-fA-F]{8,})(?:\.t)?\.dat`: the hex name of a `.dat` file in `text`, lowercased.
fn hex_dat_name(text: &str) -> Option<String> {
    // the regex needs `.dat`, and no character but these letters' ASCII cases folds to them
    crate::xml::find_ci(text, 0, ".dat")?;
    rx(r"(?i)([0-9a-fA-F]{8,})(?:\.t)?\.dat")
        .captures(text)
        .map(|c| c[1].to_lowercase())
}

pub fn extract_image_md5(content: &str) -> Option<String> {
    crate::xml::spaced_hex_attribute(content, "<img", "md5")
        .map(str::to_ascii_lowercase)
        .or_else(|| crate::xml::text_inner(content, "md5").map(str::to_lowercase))
}

fn extract_video_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    crate::xml::spaced_hex_attribute(content, "<videomsg", "md5")
        .map(str::to_ascii_lowercase)
        .or_else(|| crate::xml::text_inner(content, "md5").map(str::to_lowercase))
}

struct FileMeta {
    pub xml_type: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub file_ext: Option<String>,
    pub file_md5: Option<String>,
}

/// The file an app message carries ([`FileMeta`]), read from its [`MsgXml`].
fn extract_file_app_message_meta_in(x: &MsgXml) -> Option<FileMeta> {
    let normalized = x.once();
    if normalized.is_empty() || (!normalized.contains("<appmsg") && !normalized.contains("<msg>")) {
        return None;
    }
    let xml_type = x.app_type_twice().to_string();
    if xml_type.is_empty() {
        return None;
    }
    let first = |keys: &[&str]| {
        keys.iter()
            .map(|k| extract_xml_value(normalized, k))
            .find(|v| !v.is_empty())
            .unwrap_or_default()
    };
    let raw_name = first(&["filename", "title"]);
    let raw_ext = extract_xml_value(normalized, "fileext");
    let raw_size = first(&["totallen", "datasize", "filesize"]);
    let mut raw_md5 = extract_xml_value(normalized, "md5");
    if raw_md5.is_empty() {
        raw_md5 = extract_xml_attribute(normalized, "appattach", "md5");
    }
    if raw_md5.is_empty() {
        raw_md5 = extract_loose_hex_md5(normalized).unwrap_or_default();
    }
    let size = raw_size.trim().parse::<i64>().ok().filter(|n| *n > 0);
    let non_empty = |s: String| {
        let t = decode_html_entities(&s).trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    };
    Some(FileMeta {
        xml_type: Some(xml_type),
        file_name: non_empty(raw_name),
        file_size: size,
        file_ext: non_empty(raw_ext),
        file_md5: if raw_md5.len() == 32 && raw_md5.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(raw_md5.to_lowercase())
        } else {
            None
        },
    })
}

/// (latitude, longitude, label, poi name) of a location message.
type LocationMeta = (Option<f64>, Option<f64>, Option<String>, Option<String>);

fn extract_location_meta(content: &str, local_type: i64) -> Option<LocationMeta> {
    if content.is_empty() || local_type != 48 {
        return None;
    }
    let n = normalize_app_message_content(content);
    let pick = |a: String, b: String| if !a.is_empty() { a } else { b };
    let raw_lat = pick(
        extract_xml_attribute(&n, "location", "x"),
        extract_xml_attribute(&n, "location", "latitude"),
    );
    let raw_lng = pick(
        extract_xml_attribute(&n, "location", "y"),
        extract_xml_attribute(&n, "location", "longitude"),
    );
    let poiname = {
        let a = extract_xml_attribute(&n, "location", "poiname");
        if !a.is_empty() {
            a
        } else {
            let b = extract_xml_value(&n, "poiname");
            if !b.is_empty() {
                b
            } else {
                extract_xml_value(&n, "poiName")
            }
        }
    };
    let label = pick(
        extract_xml_attribute(&n, "location", "label"),
        extract_xml_value(&n, "label"),
    );
    let lat = raw_lat.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    let lng = raw_lng.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    let poi = if poiname.is_empty() {
        None
    } else {
        Some(poiname)
    };
    let lab = if label.is_empty() { None } else { Some(label) };
    if lat.is_none() && lng.is_none() && poi.is_none() && lab.is_none() {
        return None;
    }
    Some((lat, lng, poi, lab))
}

fn location_text(content: &str) -> String {
    let n = normalize_app_message_content(content);
    let first = |vals: Vec<String>| vals.into_iter().find(|v| !v.is_empty()).unwrap_or_default();
    let poiname = first(vec![
        extract_xml_attribute(&n, "location", "poiname"),
        extract_xml_value(&n, "poiname"),
        extract_xml_value(&n, "poiName"),
    ]);
    let label = first(vec![
        extract_xml_attribute(&n, "location", "label"),
        extract_xml_value(&n, "label"),
    ]);
    let lat = first(vec![
        extract_xml_attribute(&n, "location", "x"),
        extract_xml_attribute(&n, "location", "latitude"),
    ]);
    let lng = first(vec![
        extract_xml_attribute(&n, "location", "y"),
        extract_xml_attribute(&n, "location", "longitude"),
    ]);
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

fn parse_chat_history(content: &str) -> Option<Vec<ForwardRecord>> {
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

fn format_forward_chat_record_content(content: &str) -> String {
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

// ───────────────────────────── quoted replies ─────────────────────────────

#[derive(Clone)]
struct QuoteInfo {
    pub content: Option<String>,
    pub sender: Option<String>,
    pub quote_type: Option<String>,
    pub svrid: Option<String>,
}

impl QuoteInfo {
    fn empty() -> Self {
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
    // 被引用的是「带引用的图片」等消息时，其 content 里还嵌着一层（已解码的）<refermsg>，
    // 取最后一个结束标签才是外层，否则 type / displayname 会被截掉。
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

/// The quoted text of a `<refermsg>` block: the part the reply picked when it names one, else all of it.
pub(crate) fn extract_preferred_quoted_text(refer_xml: &str) -> String {
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
    let first = if sources[0].is_empty() {
        refer_xml.to_string()
    } else {
        sources[0].clone()
    };
    let full = sanitize_quoted_content(&extract_xml_value(&first, "content"));
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
            let v = sanitize_quoted_content(&extract_xml_value(source, tag));
            if !v.is_empty() {
                return v;
            }
        }
    }
    full
}

/// The quote a `<refermsg>` block (of the normalized content) holds.
fn quote_info_of(refer: impl AsRef<str>) -> QuoteInfo {
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
        "49" => "[链接]".into(),
        "42" => "[名片]".into(),
        "48" => "[位置]".into(),
        _ => quoted_other_preview(&refer_content),
    };
    // 被引用的文字本身是 XML（如引用了一条夹带图片的消息）时，换成可读预览
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

/// 被引用消息的内容本身是 XML（引用的是另一条引用、文件、图片、笔记……）时的可读预览。
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

/// 微信会把过长的被引用内容截断，XML 可能没有结尾标签，所以只看开头。
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

/// 引用里「被引用内容」的文本；XML 内容改成可读预览，普通文本去掉清洗后原样返回。
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

fn format_quoted_reference_preview(content: &str, refer_type: &str) -> String {
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
    let out =
        format_plain_export_content(content, ty, &PlainOpts::default(), None, None, None, None);
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

fn extract_quoted_reply_display(content: &str) -> Option<QuotedDisplay> {
    MsgXml::new(content).quoted_display().cloned()
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

/// Whether the message is a reply (a text quote): only local type 244813135921 and app message type 57 are.
/// Images, forwarded chat records and others may carry a `<refermsg>` too, and are not text quotes.
pub fn is_text_quote_message_in(local_type: i64, x: &MsgXml) -> bool {
    if local_type == 244_813_135_921 {
        return true;
    }
    if matches!(local_type & 0xFFFF_FFFF, 3 | 34 | 43 | 47) {
        return false;
    }
    x.app_type_twice() == "57"
}

/// Sender wxid of a quoted message, following `resolveQuotedSenderUsername`.
pub fn quoted_sender_username(content: &str) -> Option<String> {
    MsgXml::new(content).quoted_sender_username()
}

// ─────────────────────────────── transfers ───────────────────────────────

/// Whether two identifiers are the same person, by the database layer's rule
/// ([`weflow_native::db::same_identity`]): case-insensitive, and a bare wxid matches its account folder name
/// (`wxid_abc` and `wxid_abc_1a2b`), so the export and the database agree on who sent what.
pub fn is_same_wxid(lhs: &str, rhs: &str) -> bool {
    weflow_native::db::same_identity(lhs, rhs)
}

fn get_transfer_prefix(
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

// ──────────────────────────── plain text rendering ────────────────────────────

#[derive(Clone, Debug, Default)]
struct PlainOpts {
    pub export_voice_as_text: bool,
}

fn parse_duration_seconds(value: &str) -> Option<i64> {
    let n: f64 = value.trim().parse().ok()?;
    if !n.is_finite() || n <= 0.0 {
        return None;
    }
    Some(if n >= 1000.0 {
        crate::num::saturating_i64((n / 1000.0).round())
    } else {
        crate::num::saturating_i64(n.round())
    })
}

fn extract_amount_from_text(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    rx(r"([¥￥]\s*\d+(?:\.\d+)?|\d+(?:\.\d+)?)")
        .captures(text)
        .map(|c| rx(r"\s+").replace_all(&c[1], "").to_string())
}

/// 解码 `&#x0A;` / `&#32;` 这类数字实体（笔记正文里的换行、空格就是这样存的）。
fn decode_numeric_entities(text: &str) -> String {
    rx(r"&#(?:[xX]([0-9a-fA-F]+)|([0-9]+));")
        .replace_all(text, |c: &regex::Captures| {
            let code = match (c.get(1), c.get(2)) {
                (Some(h), _) => u32::from_str_radix(h.as_str(), 16).ok(),
                (_, Some(d)) => d.as_str().parse::<u32>().ok(),
                _ => None,
            };
            code.and_then(char::from_u32)
                .map(String::from)
                .unwrap_or_else(|| c[0].to_string())
        })
        .into_owned()
}

/// appmsg type 24 是微信「笔记」。`<des>` 只有被截断的预览，完整正文在 `<recorditem>` 里
/// 每个 `datatype="1"` 的 `<datadesc>`（文本段），`datatype="2"` 是图片。
fn note_text(normalized: &str, title: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in rx(r#"(?is)<dataitem\b[^>]*\bdatatype\s*=\s*["']?(\d+)["']?[^>]*>(.*?)</dataitem>"#)
        .captures_iter(normalized)
    {
        match &c[1] {
            "1" => {
                let t = extract_xml_value(&c[2], "datadesc");
                let t = decode_html_entities(&decode_numeric_entities(&t));
                if !t.trim().is_empty() {
                    parts.push(t.trim().to_string());
                }
            }
            "2" => parts.push("[图片]".into()),
            _ => {}
        }
    }
    let body = if !parts.is_empty() {
        parts.join("\n")
    } else if !title.is_empty() {
        title.to_string()
    } else {
        decode_numeric_entities(&extract_xml_value(normalized, "des"))
            .trim()
            .to_string()
    };
    if body.is_empty() {
        "[笔记]".into()
    } else {
        format!("[笔记]\n{body}")
    }
}

/// `parseMessageContent` — text used by ChatLab / JSON / WeClone.
pub fn parse_message_content(
    content: &str,
    local_type: i64,
    my_wxid: Option<&str>,
    sender_wxid: Option<&str>,
    emoji_caption: Option<&str>,
) -> Option<String> {
    parse_message_content_in(
        &MsgXml::new(content),
        local_type,
        my_wxid,
        sender_wxid,
        emoji_caption,
    )
}

/// [`parse_message_content`] of a message's [`MsgXml`].
pub fn parse_message_content_in(
    x: &MsgXml,
    local_type: i64,
    my_wxid: Option<&str>,
    sender_wxid: Option<&str>,
    emoji_caption: Option<&str>,
) -> Option<String> {
    let content = x.content();
    if content.is_empty() && local_type == 47 {
        return Some(format_emoji_semantic_text(emoji_caption));
    }
    if content.is_empty() {
        return None;
    }
    let normalized = x.once();
    let xml_type = x.app_type_twice();
    let with_title = |prefix: &str, title: &str| {
        if title.is_empty() {
            prefix.to_string()
        } else {
            format!("{prefix} {title}")
        }
    };
    match local_type {
        1 => Some(strip_sender_line(content, sender_wxid)),
        3 => Some("[图片]".into()),
        34 => Some("[语音消息]".into()),
        42 => Some("[名片]".into()),
        43 => Some("[视频]".into()),
        47 => Some(format_emoji_semantic_text(emoji_caption)),
        48 => Some(location_text(content)),
        49 => {
            let title = extract_xml_value(normalized, "title");
            let ty = xml_type;
            let song = extract_xml_value(normalized, "songname");
            if ty == "2000" {
                let feedesc = extract_xml_value(normalized, "feedesc");
                let memo = extract_xml_value(normalized, "pay_memo");
                let prefix = get_transfer_prefix(normalized, my_wxid, sender_wxid);
                if !feedesc.is_empty() {
                    return Some(if memo.is_empty() {
                        format!("{prefix} {feedesc}")
                    } else {
                        format!("{prefix} {feedesc} {memo}")
                    });
                }
                return Some(prefix.to_string());
            }
            Some(match ty {
                "8" => "[表情]".into(),
                "24" => note_text(normalized, &title),
                "3" => {
                    if !song.is_empty() {
                        with_title("[音乐]", &song)
                    } else {
                        with_title("[音乐]", &title)
                    }
                }
                "6" => with_title("[文件]", &title),
                "19" => format_forward_chat_record_content(normalized),
                "33" | "36" => with_title("[小程序]", &title),
                "57" => match x.quoted_display() {
                    Some(d) => build_quoted_reply_text(d),
                    None => {
                        if title.is_empty() {
                            "[引用消息]".into()
                        } else {
                            title
                        }
                    }
                },
                _ => with_title("[链接]", &title),
            })
        }
        50 => Some(parse_voip_message(content)),
        10000 | 266287972401 => Some(clean_system_message(content)),
        244813135921 => match x.quoted_display() {
            Some(d) => Some(build_quoted_reply_text(d)),
            None => {
                let t = extract_xml_value(content, "title");
                Some(if t.is_empty() {
                    "[引用消息]".into()
                } else {
                    t
                })
            }
        },
        _ => {
            if !xml_type.is_empty() {
                let title = extract_xml_value(content, "title");
                match xml_type {
                    "8" => return Some("[表情]".into()),
                    "24" => return Some(note_text(normalized, &title)),
                    "87" => {
                        let t = extract_xml_value(content, "textannouncement");
                        return Some(if t.is_empty() {
                            "[群公告]".into()
                        } else {
                            format!("[群公告] {t}")
                        });
                    }
                    "2000" => {
                        let feedesc = extract_xml_value(content, "feedesc");
                        let memo = extract_xml_value(content, "pay_memo");
                        let prefix = get_transfer_prefix(content, my_wxid, sender_wxid);
                        if !feedesc.is_empty() {
                            return Some(if memo.is_empty() {
                                format!("{prefix} {feedesc}")
                            } else {
                                format!("{prefix} {feedesc} {memo}")
                            });
                        }
                        return Some(prefix.to_string());
                    }
                    "3" => return Some(with_title("[音乐]", &title)),
                    "6" => return Some(with_title("[文件]", &title)),
                    "19" => return Some(format_forward_chat_record_content(normalized)),
                    "33" | "36" => return Some(with_title("[小程序]", &title)),
                    "57" => {
                        return Some(match x.quoted_display() {
                            Some(d) => build_quoted_reply_text(d),
                            None => {
                                if title.is_empty() {
                                    "[引用消息]".into()
                                } else {
                                    title
                                }
                            }
                        })
                    }
                    "53" => {
                        if title.is_empty() {
                            return Some("[接龙]".into());
                        }
                        let first = title
                            .lines()
                            .map(|l| l.trim())
                            .find(|l| !l.is_empty())
                            .unwrap_or(&title)
                            .to_string();
                        return Some(format!("[接龙] {first}"));
                    }
                    "5" | "49" => return Some(with_title("[链接]", &title)),
                    _ => {}
                }
                if !title.is_empty() {
                    return Some(title);
                }
            }
            let s = strip_sender_prefix(normalized);
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
    }
}

/// `formatPlainExportContent` — text used by TXT / Excel / HTML.
fn format_plain_export_content(
    content: &str,
    local_type: i64,
    opts: &PlainOpts,
    voice_transcript: Option<&str>,
    my_wxid: Option<&str>,
    sender_wxid: Option<&str>,
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
                return voice_transcript
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| "[语音消息 - 转文字失败]".into());
            }
            return "[其他消息]".into();
        }
        42 => {
            let n = normalize_app_message_content(content);
            let nick = ["nickname", "displayname", "name"]
                .iter()
                .map(|k| extract_xml_value(&n, k))
                .find(|v| !v.is_empty())
                .unwrap_or_default();
            return if nick.is_empty() {
                "[名片]".into()
            } else {
                format!("[名片]{nick}")
            };
        }
        43 => {
            let n = normalize_app_message_content(content);
            let len = ["playlength", "playLength", "length", "duration"]
                .iter()
                .map(|k| extract_xml_value(&n, k))
                .find(|v| !v.is_empty());
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
            if !t.is_empty() {
                t
            } else {
                extract_xml_value(&normalized, "appname")
            }
        };
        if sub == 8 {
            return "[表情]".into();
        }
        if sub == 24 {
            return note_text(&normalized, &title);
        }
        if sub == 87 {
            let t = extract_xml_value(&normalized, "textannouncement");
            return if t.is_empty() {
                "[群公告]".into()
            } else {
                format!("[群公告]{t}")
            };
        }
        if sub == 2000 || title.contains("转账") || normalized.contains("transfer") {
            let feedesc = extract_xml_value(&normalized, "feedesc");
            let memo = extract_xml_value(&normalized, "pay_memo");
            let prefix = get_transfer_prefix(&normalized, my_wxid, sender_wxid);
            if !feedesc.is_empty() {
                return if memo.is_empty() {
                    format!("{prefix}{feedesc}")
                } else {
                    format!("{prefix}{feedesc} {memo}")
                };
            }
            let joined: Vec<String> = [
                title,
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
            let name = if !song.is_empty() {
                song
            } else if !title.is_empty() {
                title
            } else {
                "音乐".into()
            };
            return format!("[音乐]{name}");
        }
        if sub == 6 {
            let f = extract_xml_value(&normalized, "filename");
            let name = if !f.is_empty() {
                f
            } else if !title.is_empty() {
                title
            } else {
                "文件".into()
            };
            return format!("[文件]{name}");
        }
        if title.contains("红包") || normalized.contains("hongbao") {
            return format!(
                "[红包]{}",
                if title.is_empty() {
                    "微信红包"
                } else {
                    &title
                }
            );
        }
        if sub == 19 || normalized.contains("<recorditem") {
            return format_forward_chat_record_content(&normalized);
        }
        if sub == 33 || sub == 36 {
            let a = extract_xml_value(&normalized, "appname");
            let name = if !a.is_empty() {
                a
            } else if !title.is_empty() {
                title
            } else {
                "小程序".into()
            };
            return format!("[小程序]{name}");
        }
        if sub == 57 {
            if let Some(d) = extract_quoted_reply_display(content) {
                return build_quoted_reply_text(&d);
            }
            return if title.is_empty() {
                "[引用消息]".into()
            } else {
                title
            };
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

fn normalize_http_link_url(raw: &str) -> String {
    let value = raw.trim().replace("&amp;", "&");
    if value.is_empty() {
        return String::new();
    }
    let parse_http = |candidate: &str| -> String {
        let lower = candidate.to_ascii_lowercase();
        if (lower.starts_with("http://") || lower.starts_with("https://"))
            && candidate.len() > 8
            && !candidate.contains(' ')
        {
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
    let url = [
        "url",
        "shareurlopen",
        "shareurloriginal",
        "shareurl",
        "shorturl",
        "dataurl",
        "lowurl",
        "streamvideoweburl",
        "weburl",
    ]
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

pub fn format_link_card_export_text(
    content: &str,
    local_type: i64,
    style: LinkStyle,
) -> Option<String> {
    let card = extract_html_link_card(content, local_type)?;
    if card.url.is_empty() {
        return None;
    }
    let title = link_card_display_title(&card);
    Some(match style {
        LinkStyle::Markdown => format!("[{title}]({})", card.url),
        LinkStyle::AppendUrl => {
            let prefix = if !title.is_empty() && title != card.url {
                format!("[链接] {title}")
            } else {
                "[链接]".to_string()
            };
            format!("{prefix}\n{}", card.url)
        }
    })
}

// ───────────────────────────── arkme metadata ─────────────────────────────

fn extract_finder_feed_desc(content: &str) -> String {
    rx(r"(?is)<finderFeed.*?<desc>(.*?)</desc>")
        .captures(content)
        .map(|c| {
            c[1].replace("<![CDATA[", "")
                .replace("]]>", "")
                .trim()
                .to_string()
        })
        .unwrap_or_default()
}

fn first_non_empty(n: &str, tags: &[&str]) -> String {
    tags.iter()
        .map(|t| extract_xml_value(n, t))
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

/// What [`extract_arkme_app_message_meta`] works out before it reads any field.
struct AppMessageClass {
    /// The normalized content.
    n: String,
    /// `<type>` of the app message, or empty.
    xml_type: String,
    is_finder: bool,
    is_music: bool,
    /// The `appMsgKind` of the meta.
    kind: Option<&'static str>,
}

/// `appMsgKind` of [`extract_arkme_app_message_meta`], without reading the other fields.
pub fn app_message_kind(content: &str, local_type: i64) -> Option<&'static str> {
    classify_app_message(&MsgXml::new(content), local_type)?.kind
}

/// `extractArkmeAppMessageMeta` — extra structured fields for `arkme-json` / `json`.
pub fn extract_arkme_app_message_meta(
    content: &str,
    local_type: i64,
) -> Option<Map<String, Value>> {
    extract_arkme_app_message_meta_in(&MsgXml::new(content), local_type)
}

/// [`extract_arkme_app_message_meta`] of a message's [`MsgXml`].
pub fn extract_arkme_app_message_meta_in(
    x: &MsgXml,
    local_type: i64,
) -> Option<Map<String, Value>> {
    app_message_fields(classify_app_message(x, local_type)?, local_type, x)
}

/// [`extract_arkme_app_message_meta`] of a quote or a link; `None` for every other message, whose fields are not
/// read.
pub fn quote_or_link_app_message_meta_in(
    x: &MsgXml,
    local_type: i64,
) -> Option<Map<String, Value>> {
    let content = x.content();
    // A quote has a `<refermsg>`, the quote's local type or `<type>` 57, a link `<type>` 5 or 49. Those words
    // survive normalizing (see normalized_may_contain), and cutting `<refermsg>` and `<patmsg>` blocks out of
    // the `<appmsg>` can join text into a `<type>` only where those words are: without any of the three, the
    // message is neither.
    if local_type != 244813135921
        && ["type", "refermsg", "patmsg"]
            .iter()
            .all(|word| crate::xml::find_ci(content, 0, word).is_none())
    {
        return None;
    }
    let class = classify_app_message(x, local_type)?;
    if !matches!(class.kind, Some("quote" | "link")) {
        return None;
    }
    app_message_fields(class, local_type, x)
}

/// The first part of [`extract_arkme_app_message_meta`]; `None` when the message has no meta at all.
fn classify_app_message(x: &MsgXml, local_type: i64) -> Option<AppMessageClass> {
    if x.content().is_empty() {
        return None;
    }
    let n = x.once().to_string();
    let looks_app = local_type == 49
        || local_type == 244813135921
        || n.contains("<appmsg")
        || n.contains("<msg>");
    let has_refer = n.contains("<refermsg>");
    let xml_type = x.app_type_twice().to_string();
    let is_finder = xml_type == "51"
        || n.contains("<finder")
        || n.contains("finderusername")
        || n.contains("finderobjectid");
    let is_music = xml_type == "3"
        || n.contains("<musicurl")
        || n.contains("<playurl>")
        || n.contains("<dataurl>");
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
    Some(AppMessageClass {
        n,
        xml_type,
        is_finder,
        is_music,
        kind,
    })
}

/// The fields of [`extract_arkme_app_message_meta`] for a message `class` describes.
fn app_message_fields(
    class: AppMessageClass,
    local_type: i64,
    x: &MsgXml,
) -> Option<Map<String, Value>> {
    let AppMessageClass {
        n,
        xml_type,
        is_finder,
        is_music,
        kind,
    } = class;
    let mut meta = Map::new();
    let put = |meta: &mut Map<String, Value>, key: &str, value: String| {
        if !value.is_empty() {
            meta.insert(key.into(), Value::String(value));
        }
    };
    if !xml_type.is_empty() {
        put(&mut meta, "appMsgType", xml_type);
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
    let thumb = first_non_empty(
        &n,
        &[
            "thumburl",
            "cdnthumburl",
            "cover",
            "coverurl",
            "thumbUrl",
            "coverUrl",
        ],
    );
    put(&mut meta, "appMsgDesc", desc);
    put(&mut meta, "appMsgAppName", app_name.clone());
    put(&mut meta, "appMsgSourceName", source_name);
    put(&mut meta, "appMsgSourceUsername", source_user);
    put(&mut meta, "appMsgThumbUrl", thumb.clone());

    if kind == Some("quote") {
        let q = x.quote_twice().clone();
        put(&mut meta, "quotedContent", q.content.unwrap_or_default());
        put(&mut meta, "quotedSender", q.sender.unwrap_or_default());
        put(&mut meta, "quotedType", q.quote_type.unwrap_or_default());
        put(&mut meta, "quotedSvrid", q.svrid.unwrap_or_default());
    }
    if kind == Some("link") {
        let card = extract_html_link_card(&n, local_type);
        let url = card
            .as_ref()
            .map(|c| c.url.clone())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| {
                normalize_http_link_url(&first_non_empty(&n, &["shareurl", "shorturl", "dataurl"]))
            });
        if let Some(c) = &card {
            put(&mut meta, "linkTitle", c.title.clone());
        }
        put(&mut meta, "linkUrl", url);
        put(&mut meta, "linkThumb", thumb);
    }
    if is_music {
        put(
            &mut meta,
            "musicTitle",
            first_non_empty(&n, &["songname", "title"]),
        );
        put(
            &mut meta,
            "musicUrl",
            first_non_empty(&n, &["musicurl", "playurl", "songalbumurl"]),
        );
        put(
            &mut meta,
            "musicDataUrl",
            first_non_empty(&n, &["dataurl", "lowurl"]),
        );
        put(
            &mut meta,
            "musicAlbumUrl",
            extract_xml_value(&n, "songalbumurl"),
        );
        put(
            &mut meta,
            "musicCoverUrl",
            first_non_empty(&n, &["thumburl", "cdnthumburl", "coverurl", "cover"]),
        );
        put(
            &mut meta,
            "musicSinger",
            first_non_empty(&n, &["singername", "artist", "albumartist"]),
        );
        put(&mut meta, "musicAppName", app_name);
        put(
            &mut meta,
            "musicSourceName",
            extract_xml_value(&n, "sourcename"),
        );
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
    let finder_title = if raw_title.is_empty() || raw_title.contains("不支持") {
        feed_desc
    } else {
        raw_title
    };
    put(&mut meta, "finderTitle", finder_title);
    put(
        &mut meta,
        "finderDesc",
        first_non_empty(&n, &["des", "desc"]),
    );
    put(
        &mut meta,
        "finderUsername",
        first_non_empty(&n, &["finderusername", "finder_username", "finderuser"]),
    );
    put(
        &mut meta,
        "finderNickname",
        first_non_empty(&n, &["findernickname", "finder_nickname"]),
    );
    put(
        &mut meta,
        "finderCoverUrl",
        first_non_empty(&n, &["thumbUrl", "coverUrl", "thumburl", "coverurl"]),
    );
    put(&mut meta, "finderAvatar", extract_xml_value(&n, "avatar"));
    let dur = first_non_empty(&n, &["videoPlayDuration", "duration"]);
    if let Some(d) = parse_duration_seconds(&dur) {
        meta.insert("finderDuration".into(), json!(d));
    }
    put(
        &mut meta,
        "finderObjectId",
        first_non_empty(
            &n,
            &["finderobjectid", "finder_objectid", "objectid", "object_id"],
        ),
    );
    put(
        &mut meta,
        "finderUrl",
        first_non_empty(&n, &["url", "shareurl"]),
    );
    if meta.is_empty() {
        None
    } else {
        Some(meta)
    }
}

pub fn extract_arkme_contact_card_meta(
    content: &str,
    local_type: i64,
) -> Option<Map<String, Value>> {
    if content.is_empty() || local_type != 42 {
        return None;
    }
    let n = normalize_app_message_content(content);
    let read = |attr: &str| {
        let a = extract_xml_attribute(&n, "msg", attr);
        if !a.is_empty() {
            a
        } else {
            extract_xml_value(&n, attr)
        }
    };
    let first = |attrs: &[&str]| {
        attrs
            .iter()
            .map(|a| read(a))
            .find(|v| !v.is_empty())
            .unwrap_or_default()
    };
    let mut meta = Map::new();
    meta.insert("cardKind".into(), json!("contact-card"));
    let mut put = |key: &str, value: String| {
        if !value.is_empty() {
            meta.insert(key.into(), Value::String(value));
        }
    };
    put(
        "contactCardWxid",
        first(&["username", "encryptusername", "encrypt_user_name"]),
    );
    put("contactCardNickname", read("nickname"));
    put("contactCardAlias", read("alias"));
    put("contactCardRemark", read("remark"));
    put("contactCardProvince", read("province"));
    put("contactCardCity", read("city"));
    put("contactCardSignature", first(&["sign", "signature"]));
    put(
        "contactCardAvatar",
        first(&["smallheadimgurl", "bigheadimgurl", "headimgurl", "avatar"]),
    );
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

/// Who sent a message, as the exports name it: the revoker of a revoke notice, the account for its own messages,
/// else the row's sender, else the conversation itself.
fn message_sender(
    local_type: i64,
    content: &str,
    is_send: bool,
    sender_username: &str,
    session_id: &str,
    my_wxid: &str,
) -> String {
    if local_type == 10000 || local_type == 266287972401 {
        let info = extract_revoker_info(content);
        if !info.is_revoke {
            session_id.to_string()
        } else if info.is_self_revoke {
            my_wxid.to_string()
        } else {
            info.revoker_wxid.unwrap_or_else(|| session_id.to_string())
        }
    } else if is_send {
        my_wxid.to_string()
    } else if !sender_username.is_empty() {
        sender_username.to_string()
    } else {
        session_id.to_string()
    }
}

/// A Unix time stored as an integer: a compact date (`YYYYMMDD`, `YYYYMMDDhhmm`, `YYYYMMDDhhmmss`), else seconds, milliseconds or finer cut to seconds.
fn int_timestamp_seconds(value: i64) -> i64 {
    if value <= 0 {
        return 0;
    }
    if matches!(value.ilog10() + 1, 8 | 12 | 14) {
        let compact = parse_compact_datetime(&value.to_string());
        if compact > 0 {
            return compact;
        }
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "as the text of the cell is parsed: a time above 2^53 is not a time to keep exact"
    )]
    normalize_timestamp_seconds(value as f64)
}

/// A typed row's send time in Unix seconds: its `create_time`, or its `sort_seq` when that is the only one that looks
/// like a time.
pub fn msg_row_timestamp(row: &MsgRow) -> i64 {
    let primary = row.create_time.map_or(0, int_timestamp_seconds);
    let sort_seq = row.sort_seq.map_or(0, int_timestamp_seconds);
    if primary > 0 && primary < 946_684_800 && sort_seq > 946_684_800 {
        return sort_seq;
    }
    if primary > 0 {
        return primary;
    }
    sort_seq.max(0)
}

/// [`message_sender`] of a typed row.
pub fn msg_row_sender(row: &MsgRow, session_id: &str, my_wxid: &str) -> String {
    message_sender(
        row.local_type,
        if row.local_type == 10000 || row.local_type == 266287972401 {
            &row.content
        } else {
            ""
        },
        row.is_send,
        row.sender.as_deref().unwrap_or(""),
        session_id,
        my_wxid,
    )
}

/// `collectMessages` (full mode) of typed rows, which it takes (their text moves into the messages): the messages
/// in the time range and of the sender asked for, sorted chronologically.
pub fn collect_rows(rows: Vec<MsgRow>, opts: &CollectOptions<'_>) -> Vec<ExportMsg> {
    let mut out: Vec<ExportMsg> = Vec::new();
    for row in rows {
        let create_time = msg_row_timestamp(&row);
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
        let MsgRow {
            local_id,
            server_id,
            local_type,
            content,
            sender,
            is_send,
            packed_info,
            ..
        } = row;
        let server_id = server_id.unwrap_or(0);
        let server_id_raw = (server_id > 0).then(|| server_id.to_string());

        let actual_sender = message_sender(
            local_type,
            &content,
            is_send,
            sender.as_deref().unwrap_or(""),
            opts.session_id,
            opts.my_wxid,
        );
        if let Some(filter) = opts.sender_filter.filter(|f| !f.trim().is_empty()) {
            if !is_same_wxid(&actual_sender, filter.trim()) {
                continue;
            }
        }

        let mut msg = ExportMsg {
            local_id,
            server_id,
            server_id_raw,
            create_time,
            local_type,
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
        if local_type == 47 && !content.is_empty() {
            msg.emoji_cdn_url = extract_emoji_url(&content);
            msg.emoji_md5 = extract_emoji_md5(&content);
        }
        if local_type == 3 {
            msg.image_dat_name = packed_info
                .as_deref()
                .and_then(dat_name_in_packed_info)
                .or_else(|| extract_image_dat_name(&content));
            if !content.is_empty() {
                msg.image_md5 = extract_image_md5(&content);
            }
        }
        if local_type == 43 && !content.is_empty() {
            msg.video_md5 = extract_video_md5(&content);
        }
        {
            // read once for all of it
            let x = MsgXml::new(&content);
            let app_like =
                local_type == 49 || content.contains("<appmsg") || content.contains("&lt;appmsg");
            if !content.is_empty() && app_like {
                if let Some(meta) = extract_file_app_message_meta_in(&x) {
                    msg.xml_type = meta.xml_type;
                    msg.file_name = meta.file_name;
                    msg.file_size = meta.file_size;
                    msg.file_ext = meta.file_ext;
                    msg.file_md5 = meta.file_md5;
                }
                if x.app_type_twice() == "19" {
                    msg.chat_record_list = parse_chat_history(x.once());
                }
            }
            if local_type != 47 {
                msg.sticker_md5 = appmsg_emoticon_md5_in(&x);
            }
        }
        msg.content = content;
        out.push(msg);
    }
    out.sort_by(|a, b| {
        a.create_time
            .cmp(&b.create_time)
            .then(a.local_id.cmp(&b.local_id))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::kind::convert_message_type_in;
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn wechat_note_exports_the_full_text() {
        let xml = "<msg><appmsg><title /><des>预览被截断…</des><type>24</type><recorditem><![CDATA[<recordinfo><desc>预览</desc><datalist count=\"3\"><dataitem datatype=\"8\" dataid=\"a\"><fileext>.htm</fileext></dataitem><dataitem datatype=\"1\" dataid=\"b\"><datadesc>第一段&#x0A;第&#x20;二行</datadesc></dataitem><dataitem datatype=\"2\" dataid=\"c\"></dataitem></datalist></recordinfo>]]></recorditem></appmsg></msg>";
        let t = parse_message_content(xml, 103_079_215_153, None, None, None).unwrap();
        assert_eq!(t, "[笔记]\n第一段\n第 二行\n[图片]");
        // 没有 recorditem 时退回 <des>
        let t = parse_message_content(
            "<msg><appmsg><title /><des>只有预览</des><type>24</type></appmsg></msg>",
            103_079_215_153,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(t, "[笔记]\n只有预览");
    }

    #[test]
    fn emoji_text_carries_the_caption() {
        assert_eq!(format_emoji_semantic_text(None), "[表情]");
        assert_eq!(format_emoji_semantic_text(Some("  ")), "[表情]");
        assert_eq!(format_emoji_semantic_text(Some("打call")), "[表情：打call]");
        let xml = "<msg><appmsg><title /><type>8</type><appattach><emoticonmd5>35A036EDED53A8D41C043215C66FFDA3</emoticonmd5></appattach></appmsg></msg>";
        assert_eq!(
            appmsg_emoticon_md5_in(&MsgXml::new(xml)).as_deref(),
            Some("35a036eded53a8d41c043215c66ffda3")
        );
        assert_eq!(
            appmsg_emoticon_md5_in(&MsgXml::new("<msg><appmsg><type>5</type></appmsg></msg>")),
            None
        );
    }

    #[test]
    fn quoted_reply_text() {
        let xml = "<msg><appmsg><title>ok</title><type>57</type><refermsg><type>1</type><displayname>Bob</displayname><content>hi there</content><svrid>123</svrid></refermsg></appmsg></msg>";
        let d = extract_quoted_reply_display(xml).unwrap();
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

    #[test]
    fn forward_chat_records() {
        let xml = "<msg><appmsg><title>群聊的聊天记录</title><type>19</type><recorditem><![CDATA[<recordinfo><datalist><dataitem datatype=\"1\"><sourcename>Bob</sourcename><sourcetime>2024-01-01 10:00:00</sourcetime><datadesc>hello</datadesc></dataitem></datalist></recordinfo>]]></recorditem></appmsg></msg>";
        let text = format_forward_chat_record_content(xml);
        assert_eq!(text, "[转发的聊天记录]群聊的聊天记录\nBob: hello");
        assert_eq!(convert_message_type_in(49, &MsgXml::new(xml)), 7);
    }

    #[test]
    fn plain_content_per_type() {
        let opts = PlainOpts::default();
        assert_eq!(
            format_plain_export_content("wxid_a:hi", 1, &opts, None, None, None, None),
            "hi"
        );
        assert_eq!(
            format_plain_export_content("", 3, &opts, None, None, None, None),
            "[图片]"
        );
        assert_eq!(
            format_plain_export_content("x", 34, &opts, None, None, None, None),
            "[其他消息]"
        );
        let v = "<msg><videomsg playlength=\"12\"/><playlength>12</playlength></msg>";
        assert_eq!(
            format_plain_export_content(v, 43, &opts, None, None, None, None),
            "[视频]12s"
        );
        let loc =
            "<msg><location x=\"31.2\" y=\"121.4\" poiname=\"Tower\" label=\"Road 1\"/></msg>";
        assert_eq!(
            format_plain_export_content(loc, 48, &opts, None, None, None, None),
            "[位置] Tower Road 1 (31.2,121.4)"
        );
    }

    #[test]
    fn link_cards() {
        let xml = "<msg><appmsg><title>Hello</title><type>5</type><url>https://example.com/a?b=1&amp;c=2</url></appmsg></msg>";
        let c = extract_html_link_card(xml, 49).unwrap();
        assert_eq!(c.url, "https://example.com/a?b=1&c=2");
        assert_eq!(
            format_link_card_export_text(xml, 49, LinkStyle::Markdown).unwrap(),
            "[Hello](https://example.com/a?b=1&c=2)"
        );
        assert_eq!(
            normalize_http_link_url("example.com/x"),
            "https://example.com/x"
        );
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
            MsgRow {
                local_id: 2,
                server_id: Some(9_007_199_254_740_993),
                create_time: Some(1_700_000_100),
                local_type: 1,
                content: "wxid_b:second".into(),
                sender: Some("wxid_b".into()),
                ..MsgRow::default()
            },
            MsgRow {
                local_id: 1,
                create_time: Some(1_700_000_000),
                local_type: 1,
                content: "first".into(),
                is_send: true,
                ..MsgRow::default()
            },
        ];
        let msgs = collect_rows(
            rows.clone(),
            &CollectOptions {
                session_id: "wxid_b",
                my_wxid: "wxid_me",
                start: None,
                end: None,
                sender_filter: None,
            },
        );
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "first");
        assert_eq!(msgs[0].sender_username, "wxid_me");
        assert_eq!(msgs[1].server_id_raw.as_deref(), Some("9007199254740993"));
        let filtered = collect_rows(
            rows,
            &CollectOptions {
                session_id: "wxid_b",
                my_wxid: "wxid_me",
                start: Some(1700000050),
                end: None,
                sender_filter: None,
            },
        );
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn timestamps() {
        assert_eq!(
            normalize_timestamp_seconds(1_700_000_000_000.0),
            1_700_000_000
        );
        assert!(int_timestamp_seconds(20_240_102_030_405) > 0);
        assert_eq!(unsigned_token("00012").as_deref(), Some("12"));
        assert_eq!(unsigned_token(" 12.9 ").as_deref(), Some("12"));
        assert_eq!(
            unsigned_token("18446744073709551615").as_deref(),
            Some("18446744073709551615"),
            "an id above i64::MAX keeps its digits"
        );
        for none in ["", "000", "0.5", "-3", "abc"] {
            assert_eq!(unsigned_token(none), None, "{none:?}");
        }
        assert_eq!(
            last_hex_token("zz0123456789abcdef0123456789abcdef00zz", 32, 32)
                .unwrap()
                .len(),
            32
        );
    }

    #[test]
    fn percent_decoding_never_splits_characters() {
        assert_eq!(percent_decode_bytes("%E4%B8%AD%2F"), "中/".as_bytes());
        assert_eq!(
            percent_decode_bytes("%a中"),
            "%a中".as_bytes(),
            "a multi-byte character after % is not a hex digit"
        );
        assert_eq!(percent_decode_bytes("%中x"), "%中x".as_bytes());
        assert_eq!(
            percent_decode_bytes("%+5"),
            b"%+5",
            "a sign is not a hex digit"
        );
        assert_eq!(percent_decode_bytes("a%4"), b"a%4");
        assert_eq!(
            percent_decode("%ff"),
            "%ff",
            "invalid UTF-8 keeps the input"
        );
    }

    #[test]
    fn the_kind_and_the_quote_or_link_meta_agree_with_the_whole_meta() {
        use rand::{rngs::StdRng, RngExt, SeedableRng};
        const TOKENS: &[&str] = &[
            "<msg>",
            "</msg>",
            "<appmsg>",
            "<appmsg appid=\"wx1\">",
            "</appmsg>",
            "<type>",
            "</type>",
            "<refermsg>",
            "</refermsg>",
            "<patmsg>",
            "</patmsg>",
            "<finder",
            "finderusername",
            "finderobjectid",
            "<musicurl",
            "<playurl>",
            "<dataurl>",
            "<des>d</des>",
            "<title>t</title>",
            "<url>http://a</url>",
            "<svrid>12</svrid>",
            "<appname>app</appname>",
            "<ty",
            "pe>",
            "TYPE",
            "<PATMSG>",
            "</patmsg>",
            "<REFERMSG>",
            "57",
            "&lt;",
            "&gt;",
            "&amp;",
            " ",
            "x",
        ];
        const TYPES: &[&str] = &[
            "57", "5", "49", "6", "3", "51", "2001", "33", "19", "2000", "87", "53", "8", "",
        ];
        const LOCAL_TYPES: &[i64] = &[1, 3, 47, 49, 10000, 244813135921, 21474836529];
        // a `<type>` and `</type>` that only exist once the `<patmsg>` blocks are cut out
        let joined =
            "<msg><appmsg><ty<patmsg>a</patmsg>pe>5</ty<patmsg>b</patmsg>pe></appmsg></msg>";
        assert_eq!(app_message_kind(joined, 1), Some("link"));
        assert!(quote_or_link_app_message_meta_in(&MsgXml::new(joined), 1).is_some());
        let mut rng = StdRng::seed_from_u64(546);
        let mut kinds: HashMap<Option<&'static str>, usize> = HashMap::new();
        for _ in 0..20_000 {
            let mut s = String::new();
            if rng.random_bool(0.6) {
                s.push_str("<msg><appmsg><type>");
                s.push_str(TYPES[rng.random_range(0..TYPES.len())]);
                s.push_str("</type>");
            }
            for _ in 0..rng.random_range(0..8) {
                s.push_str(TOKENS[rng.random_range(0..TOKENS.len())]);
            }
            if rng.random_bool(0.3) {
                s = s.replace('<', "&lt;").replace('>', "&gt;");
            }
            let t = LOCAL_TYPES[rng.random_range(0..LOCAL_TYPES.len())];
            let whole = extract_arkme_app_message_meta(&s, t);
            let kind = app_message_kind(&s, t);
            assert_eq!(
                kind,
                whole.as_ref().and_then(|m| m.get("appMsgKind")?.as_str()),
                "{s:?} as {t}"
            );
            let quote_or_link = whole.filter(|_| matches!(kind, Some("quote" | "link")));
            assert_eq!(
                quote_or_link_app_message_meta_in(&MsgXml::new(&s), t),
                quote_or_link,
                "{s:?} as {t}"
            );
            *kinds.entry(kind).or_default() += 1;
        }
        for kind in [
            None,
            Some("quote"),
            Some("link"),
            Some("card"),
            Some("finder"),
            Some("music"),
            Some("file"),
        ] {
            let hits = kinds.get(&kind).copied().unwrap_or(0);
            assert!(hits >= 100, "{kind:?} came up {hits} times");
        }
    }
}
