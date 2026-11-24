//! Message decoding and formatting for the exports.
//!
//! Raw WCDB rows go in, [`ExportMsg`] values and human-readable text come out. The text is
//! the desktop app's, so exports stay comparable with its exports. Placeholder strings such as
//! `[图片]` are data and intentionally stay as in the desktop exports.

use serde_json::Value;
use weflow_native::msg_row::MsgRow;

pub(crate) mod appmsg;
pub mod emoji;
pub(crate) mod forward;
pub(crate) mod kind;
pub(crate) mod link;
pub(crate) mod media;
pub(crate) mod quote;
pub(crate) mod row;
pub(crate) mod system;
pub mod time;
pub mod transfer;
pub(crate) mod xml;

use emoji::{
    appmsg_emoticon_md5_in, extract_emoji_md5, extract_emoji_url, format_emoji_semantic_text,
};
use forward::{format_forward_chat_record_content, parse_chat_history};
use media::{
    dat_name_in_packed_info, extract_file_app_message_meta_in, extract_image_dat_name,
    extract_image_md5, extract_location_meta, extract_video_md5, location_text,
};
use quote::{build_quoted_reply_text, extract_quoted_reply_display};
use row::unsigned_token;
use system::{
    clean_system_message, extract_readable_system_message_text, extract_revoker_info,
    is_readable_system_message, parse_voip_message, pat_text,
};
use time::{normalize_timestamp_seconds, parse_compact_datetime};
use transfer::{get_transfer_prefix, is_same_wxid, transfer_text};
use xml::{
    decode_html_entities, extract_app_message_type, extract_xml_value,
    normalize_app_message_content, rx, strip_sender_line, strip_sender_prefix, xml_text, MsgXml,
};

// ───────────────────────────── message struct ─────────────────────────────

/// A message of a forwarded chat record (`<recorditem>`'s `<dataitem>`).
#[derive(Clone, Debug, Default)]
pub struct ForwardRecord {
    /// Its `datatype`, in the record's own numbers (1 text, 2 image, 3 voice, 4 video, 5 link, 8 file, 17 a chat
    /// record of its own, 19 mini program, 37 sticker…, read by `RecordKind`); 0 when it gives none.
    pub datatype: i64,
    /// The name of who sent it.
    pub sourcename: String,
    /// When it was sent, as the record writes it.
    pub sourcetime: String,
    /// Its text (`datadesc`, else `content`).
    pub datadesc: Option<String>,
    /// Its title, such as a link's or a file's.
    pub datatitle: Option<String>,
    /// A nested record's title, else the item's own.
    pub chat_record_title: Option<String>,
    /// A nested record's description, else the item's text.
    pub chat_record_desc: Option<String>,
    /// A nested record's messages (`datatype` 17).
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

/// A message as the exports read it: the row's fields and what its XML names.
#[derive(Clone, Debug, Default)]
pub struct ExportMsg {
    /// The message's id in its conversation's table.
    pub local_id: i64,
    /// WeChat's id of the message; 0 when it has none.
    pub server_id: i64,
    /// WeChat's id as the row gives it, every digit kept; none when the row has none.
    pub server_id_raw: Option<String>,
    /// When it was sent, in Unix seconds.
    pub create_time: i64,
    /// WeChat's type of the message, with an app message's sub-type in the high 32 bits.
    pub local_type: i64,
    /// The text, or the XML of a message that is not text.
    pub content: String,
    /// Who sent it, as the exports name it: the revoker of a revoke notice, the account for its own messages, else
    /// the row's sender, else the conversation.
    pub sender_username: String,
    /// Whether the account's owner sent it.
    pub is_send: bool,
    /// An image's MD5.
    pub image_md5: Option<String>,
    /// The name of an image's `.dat` file.
    pub image_dat_name: Option<String>,
    /// A sticker's address on WeChat's servers.
    pub emoji_cdn_url: Option<String>,
    /// A sticker's MD5.
    pub emoji_md5: Option<String>,
    /// The md5 of a sticker sent as an app message of type 8 (`appmsg_emoticon_md5_in`).
    pub sticker_md5: Option<String>,
    /// A sticker's caption, from WeChat's sticker database.
    pub emoji_caption: Option<String>,
    /// The MD5 that names a video's file.
    pub video_md5: Option<String>,
    /// An app message's `<type>`.
    pub xml_type: Option<String>,
    /// A file's name.
    pub file_name: Option<String>,
    /// A file's size, in bytes.
    pub file_size: Option<i64>,
    /// A file's extension, without its dot.
    pub file_ext: Option<String>,
    /// A file's MD5.
    pub file_md5: Option<String>,
    /// A location's latitude.
    pub location_lat: Option<f64>,
    /// A location's longitude.
    pub location_lng: Option<f64>,
    /// The name of a location's place.
    pub location_poiname: Option<String>,
    /// A location's label, its address as WeChat shows it.
    pub location_label: Option<String>,
    /// A chat record's messages.
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

    /// WeChat's id of the message in decimal without leading zeros; `None` when it has none.
    pub fn platform_message_id(&self) -> Option<String> {
        match &self.server_id_raw {
            Some(raw) => unsigned_token(raw),
            None => unsigned_token(&self.server_id.to_string()),
        }
    }
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

/// `text` with numeric entities such as `&#x0A;` and `&#32;` decoded (a note's body stores line breaks and spaces
/// this way).
fn decode_numeric_entities(text: &str) -> String {
    rx(r"&#(?:[xX]([0-9a-fA-F]+)|([0-9]+));")
        .replace_all(text, |c: &regex::Captures<'_>| {
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

/// The text of a WeChat note (appmsg type 24).
///
/// `<des>` holds only a truncated preview; the full body is in `<recorditem>`, one `<datadesc>` per `datatype="1"`
/// item (a text section), while a `datatype="2"` item is an image.
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
        xml_text(normalized, "des")
    };
    if body.is_empty() {
        "[笔记]".into()
    } else {
        format!("[笔记]\n{body}")
    }
}

/// The text of a message for ChatLab / JSON / WeClone.
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

/// [`parse_message_content`] of a message's `MsgXml`.
pub fn parse_message_content_in(
    x: &MsgXml<'_>,
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
    match local_type {
        1 => Some(strip_sender_line(content, sender_wxid)),
        3 => Some("[图片]".into()),
        34 => Some("[语音消息]".into()),
        42 => Some("[名片]".into()),
        43 => Some("[视频]".into()),
        47 => Some(format_emoji_semantic_text(emoji_caption)),
        48 => Some(location_text(content)),
        50 => Some(parse_voip_message(content)),
        10000 => Some(clean_system_message(content)),
        // a pat's `<title>` is escaped XML, as the chat window reads it (#1196)
        266287972401 => Some(pat_text(content)),
        244813135921 => match x.quoted_display() {
            Some(d) => Some(build_quoted_reply_text(d)),
            None => {
                let t = xml_text(content, "title");
                Some(if t.is_empty() {
                    "[引用消息]".into()
                } else {
                    t
                })
            }
        },
        // an app message, whether `local_type` keeps its `<type>` in the high bits (`(<type> << 32) | 49`, as
        // WeChat 4 does) or not: the same text either way (#1180)
        l if l & 0xFFFF_FFFF == 49 => Some(
            app_message_text(x, my_wxid, sender_wxid).unwrap_or_else(|| {
                let title = xml_text(normalized, "title");
                if title.is_empty() {
                    "[消息]".into()
                } else {
                    title
                }
            }),
        ),
        _ => {
            if !xml_type.is_empty() {
                if let Some(text) = app_message_text(x, my_wxid, sender_wxid) {
                    return Some(text);
                }
                let title = xml_text(content, "title");
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

/// The text of an app message by its own `<type>`, with the tag of its row of
/// [`appmsg::app_type`](crate::message::appmsg::app_type); `None` for a type not in the table.
fn app_message_text(
    x: &MsgXml<'_>,
    my_wxid: Option<&str>,
    sender_wxid: Option<&str>,
) -> Option<String> {
    let normalized = x.once();
    let ty = x.app_type_twice();
    let row = appmsg::app_type(ty)?;
    let title = xml_text(normalized, "title");
    let with_title = |title: &str| {
        if title.is_empty() {
            row.tag.to_string()
        } else {
            format!("{} {title}", row.tag)
        }
    };
    Some(match ty {
        "2000" => transfer_text(normalized, my_wxid, sender_wxid),
        "8" => row.tag.into(),
        "24" => note_text(normalized, &title),
        "87" => with_title(&xml_text(normalized, "textannouncement")),
        "3" | "76" => {
            let song = xml_text(normalized, "songname");
            with_title(if song.is_empty() { &title } else { &song })
        }
        "19" => format_forward_chat_record_content(normalized),
        "57" => match x.quoted_display() {
            Some(d) => build_quoted_reply_text(d),
            None if title.is_empty() => row.tag.into(),
            None => title,
        },
        "53" => with_title(
            title
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or(&title),
        ),
        _ => with_title(&title),
    })
}

/// The text of a message for TXT / Excel / HTML.
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
                .map(|k| xml_text(&n, k))
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
        10000 => return clean_system_message(content),
        266287972401 => return pat_text(content),
        _ => {}
    }
    let normalized = normalize_app_message_content(content);
    let is_app = normalized.contains("<appmsg") || normalized.contains("<msg>");
    if local_type == 49 || is_app {
        let sub: i64 = extract_app_message_type(&normalized).parse().unwrap_or(0);
        let title = {
            let t = xml_text(&normalized, "title");
            if !t.is_empty() {
                t
            } else {
                xml_text(&normalized, "appname")
            }
        };
        if sub == 8 {
            return "[表情]".into();
        }
        if sub == 24 {
            return note_text(&normalized, &title);
        }
        if sub == 87 {
            let t = xml_text(&normalized, "textannouncement");
            return if t.is_empty() {
                "[群公告]".into()
            } else {
                format!("[群公告]{t}")
            };
        }
        if sub == 2000 || title.contains("转账") || normalized.contains("transfer") {
            let feedesc = xml_text(&normalized, "feedesc");
            let memo = xml_text(&normalized, "pay_memo");
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
        if sub == 3
            || sub == 76
            || normalized.contains("<musicurl")
            || normalized.contains("<songname")
        {
            let song = xml_text(&normalized, "songname");
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
            let f = xml_text(&normalized, "filename");
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
            let a = xml_text(&normalized, "appname");
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

// ─────────────────────────── row → ExportMsg ───────────────────────────

/// Which rows [`collect_rows`] takes, and whose they are.
#[derive(Debug)]
pub struct CollectOptions<'a> {
    /// The conversation the rows are of.
    pub session_id: &'a str,
    /// The account's wxid.
    pub my_wxid: &'a str,
    /// Unix seconds: only messages from then on; none or 0 for no bound.
    pub start: Option<i64>,
    /// Unix seconds: only messages up to then; none or 0 for no bound.
    pub end: Option<i64>,
    /// Only the messages of this sender.
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

/// `message_sender` of a typed row.
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

/// The export messages of typed rows, which it takes (their text moves into the messages): the messages
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
    use super::xml::last_hex_token;
    use super::*;

    #[test]
    fn wechat_note_exports_the_full_text() {
        let xml = "<msg><appmsg><title /><des>预览被截断…</des><type>24</type><recorditem><![CDATA[<recordinfo><desc>预览</desc><datalist count=\"3\"><dataitem datatype=\"8\" dataid=\"a\"><fileext>.htm</fileext></dataitem><dataitem datatype=\"1\" dataid=\"b\"><datadesc>第一段&#x0A;第&#x20;二行</datadesc></dataitem><dataitem datatype=\"2\" dataid=\"c\"></dataitem></datalist></recordinfo>]]></recorditem></appmsg></msg>";
        let t = parse_message_content(xml, 103_079_215_153, None, None, None).unwrap();
        assert_eq!(t, "[笔记]\n第一段\n第 二行\n[图片]");
        // without a recorditem, the <des>
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

    /// The `<appmsg>` type an export takes from reading a message is the one its XML gives, for app messages plain
    /// and escaped, quoting another message with a type of its own, with a pat, without a type, and for text that
    /// only mentions `<appmsg>`.
    #[test]
    fn the_app_type_read_is_the_one_worked_out_again() {
        let contents = [
            (49, "<msg><appmsg><title>a</title><type>5</type></appmsg></msg>"),
            (49, "&lt;msg&gt;&lt;appmsg&gt;&lt;type&gt;6&lt;/type&gt;&lt;md5&gt;x&lt;/md5&gt;&lt;/appmsg&gt;&lt;/msg&gt;"),
            (49, "<msg><appmsg><title>b</title><refermsg><type>3</type></refermsg><type>57</type></appmsg></msg>"),
            (49, "<msg><appmsg><patmsg><type>1</type></patmsg><type>62</type></appmsg></msg>"),
            (49, "<msg><appmsg><title>c</title></appmsg></msg>"),
            (49, "<msg><appmsg><type>19</type><recorditem>x</recorditem></appmsg></msg>"),
            (1, "a text about <appmsg> and <type>5</type>"),
            (1, "plain"),
        ];
        let rows = contents
            .iter()
            .zip(1i64..)
            .map(|(&(local_type, content), local_id)| MsgRow {
                local_id,
                create_time: Some(1_700_000_000 + local_id),
                local_type,
                content: content.into(),
                ..MsgRow::default()
            })
            .collect();
        let msgs = collect_rows(
            rows,
            &CollectOptions {
                session_id: "wxid_b",
                my_wxid: "wxid_me",
                start: None,
                end: None,
                sender_filter: None,
            },
        );
        assert_eq!(msgs.len(), contents.len());
        assert!(msgs.iter().filter(|m| m.xml_type.is_some()).count() >= 5);
        for m in &msgs {
            assert_eq!(
                MsgXml::of_export(m).app_type_twice(),
                MsgXml::new(&m.content).app_type_twice(),
                "{}",
                m.content
            );
        }
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
}
