//! 导出用的消息解码和格式化。
//!
//! 输入数据库的消息行（[`MsgRow`]），输出 [`ExportMsg`] 和给人读的文字。
//! 文字与桌面版的导出相同，两边的导出可以逐条对照。`[图片]` 这类占位文字是数据，与桌面版一致，不随 CLI 的语言变化。

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
use quote::build_quoted_reply_text;
use row::unsigned_token;
use system::{clean_system_message, extract_revoker_info, parse_voip_message, pat_text};
use time::{normalize_timestamp_seconds, parse_compact_datetime};
use transfer::{is_same_wxid, transfer_text};
use xml::{
    decode_html_entities, extract_xml_value, rx, strip_sender_line, strip_sender_prefix, xml_text,
    MsgXml,
};

// ───────────────────────────── 导出的消息 ─────────────────────────────

/// 转发的聊天记录中的一条消息（`<recorditem>` 的 `<dataitem>`）。
#[derive(Clone, Debug, Default)]
pub struct ForwardRecord {
    /// 条目的 `datatype`，用聊天记录自己的编号（1 文字、2 图片、3 语音、4 视频、5 链接、8 文件、17 嵌套的聊天记录、
    /// 19 小程序、37 表情等，由 `RecordKind` 解读）；没有时为 0。
    pub datatype: i64,
    /// 发送者的名字。
    pub sourcename: String,
    /// 发送时间，即聊天记录里写的原文。
    pub sourcetime: String,
    /// 条目的文字（`<datadesc>`，没有时取 `<content>`）。
    pub datadesc: Option<String>,
    /// 条目的标题，如链接或文件的标题。
    pub datatitle: Option<String>,
    /// 嵌套的聊天记录的标题，没有时为条目自己的标题。
    pub chat_record_title: Option<String>,
    /// 嵌套的聊天记录的描述，没有时为条目的文字。
    pub chat_record_desc: Option<String>,
    /// 嵌套的聊天记录中的消息（`datatype` 17）。
    pub chat_record_list: Option<Vec<ForwardRecord>>,
}

/// 复制到导出文件旁边的媒体文件：导出的消息指向它，不再写占位文字。
#[derive(Clone, Debug, Default)]
pub struct MediaRef {
    /// 媒体的种类：`image`、`voice`、`video`、`emoji` 或 `file`。
    pub kind: &'static str,
    /// 文件相对于导出文件的路径，以 `/` 分隔。
    pub relative_path: String,
}

/// 导出读到的一条消息：行的字段，以及从它的 XML 中取出的内容。
#[derive(Clone, Debug, Default)]
pub struct ExportMsg {
    /// 本地 ID，即消息在所在分片的会话表中的行号。
    pub local_id: i64,
    /// 服务器 ID；没有时为 0。
    pub server_id: i64,
    /// 服务器 ID 的十进制文字，保留全部数字；行中没有时为 `None`。
    pub server_id_raw: Option<String>,
    /// 发送时间，Unix 秒。
    pub create_time: i64,
    /// 消息类型（`local_type`），app 消息的子类型在高 32 位。
    pub local_type: i64,
    /// 文字；不是文字的消息为它的 XML。
    pub content: String,
    /// 导出所写的发送者：系统消息和拍一拍中，撤回提示为撤回者，其他为会话；其余消息中，账号自己发的为账号，
    /// 其他为行中的发送者，没有时为会话。
    pub sender_username: String,
    /// 是否为账号所有者发的。
    pub is_send: bool,
    /// 图片的 MD5。
    pub image_md5: Option<String>,
    /// 图片的 `.dat` 文件名。
    pub image_dat_name: Option<String>,
    /// 表情在微信服务器上的地址。
    pub emoji_cdn_url: Option<String>,
    /// 表情的 MD5。
    pub emoji_md5: Option<String>,
    /// 以 `<type>` 为 8 的 app 消息发出的表情的 MD5（`appmsg_emoticon_md5_in`）。
    pub sticker_md5: Option<String>,
    /// 表情的描述，取自微信的表情库。
    pub emoji_caption: Option<String>,
    /// 视频的 MD5，视频文件以它命名。
    pub video_md5: Option<String>,
    /// app 消息的 `<appmsg><type>`，读取时求出，各格式的导出器经 `MsgXml::of_export` 沿用。
    pub xml_type: Option<String>,
    /// 文件名；没有 `<filename>` 时为 `<title>`。
    pub file_name: Option<String>,
    /// 文件大小，单位为字节。
    pub file_size: Option<i64>,
    /// 文件的扩展名，不带点。
    pub file_ext: Option<String>,
    /// 文件的 MD5。
    pub file_md5: Option<String>,
    /// 位置的纬度。
    pub location_lat: Option<f64>,
    /// 位置的经度。
    pub location_lng: Option<f64>,
    /// 位置的地点名称（`poiname`）。
    pub location_poiname: Option<String>,
    /// 位置的标签（`label`），即微信显示的地址。
    pub location_label: Option<String>,
    /// 聊天记录中的消息。
    pub chat_record_list: Option<Vec<ForwardRecord>>,
    /// 导出要求复制媒体并且找到了文件时才有。
    pub media: Option<MediaRef>,
}

// 导出的一页中每条消息各占一个：断言它的大小，使大小只在有意修改时改变（#505）
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<ExportMsg>() == 536);

impl ExportMsg {
    /// 消息的 JSON 形式，即 [`crate::contract::MessageView`]。
    pub fn to_json(&self, my_wxid: &str) -> Value {
        serde_json::to_value(crate::contract::MessageView::of(self, my_wxid)).unwrap_or_default()
    }

    /// 十进制、不带前导零的服务器 ID；没有时为 `None`。
    pub fn platform_message_id(&self) -> Option<String> {
        match &self.server_id_raw {
            Some(raw) => unsigned_token(raw),
            None => unsigned_token(&self.server_id.to_string()),
        }
    }
}

// ──────────────────────────── 纯文本渲染 ────────────────────────────

/// 时长的文字换算成整秒：不小于 1000 的值按毫秒算；不是正数时为 `None`。
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

/// 解码 `text` 中 `&#x0A;`、`&#32;` 这类数字实体（笔记的正文这样保存换行和空格）。
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

/// 微信笔记（app 消息的 `<type>` 24）的文字。
///
/// `<des>` 只有截断的预览，完整的正文在 `<recorditem>` 里：`datatype="1"` 的条目是一段文字，各有一个 `<datadesc>`；
/// `datatype="2"` 的条目是图片。条目中没有文字时依次用标题和 `<des>`。
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

/// 消息的文字：各格式导出的文字和命令 API 的消息（[`crate::contract::MessageView`]）都在它的基础上写。
/// 没有可写的文字时为 `None`。
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

/// 用消息已有的 `MsgXml` 求 [`parse_message_content`]。
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
        // 拍一拍的标题带转义的实体：与聊天窗口一样由 `pat_text` 先解码整条内容的实体，再取 `<title>`（#1196）
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
        // app 消息：不论 `local_type` 是否在高位保存 `<type>`（微信 4 存为 `(<type> << 32) | 49`），文字都相同（#1180）
        l if l & 0xFFFF_FFFF == 49 => Some(
            app_message_text(x, local_type, my_wxid, sender_wxid).unwrap_or_else(|| {
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
                if let Some(text) = app_message_text(x, local_type, my_wxid, sender_wxid) {
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

/// app 消息的文字写多少。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextForm {
    /// 全文，即导出的写法：回复连同被引用的内容、聊天记录中的消息、笔记的正文。
    Full,
    /// 一行，即聊天窗口的写法：回复本身的文字，聊天记录或笔记的标题。
    Line,
}

/// 按 [`TextForm::Full`] 求 [`app_message_text_as`]。
fn app_message_text(
    x: &MsgXml<'_>,
    local_type: i64,
    my_wxid: Option<&str>,
    sender_wxid: Option<&str>,
) -> Option<String> {
    app_message_text_as(x, local_type, my_wxid, sender_wxid, TextForm::Full)
}

/// app 消息的文字。种类（[`appmsg::app_message_kind_in`]）为 `location` 时写 `[位置] <地点>`；其余按它自己的
/// `<type>` 在 [`appmsg::app_type`] 表中的那一行写，用那一行的标签，`<type>` 不在表中时按种类在表中的那一行。
/// 聊天窗口和导出都读它，聊天窗口用 [`TextForm::Line`]（#1263）。`<type>` 和种类在表中都没有对应的行时为 `None`。
pub(crate) fn app_message_text_as(
    x: &MsgXml<'_>,
    local_type: i64,
    my_wxid: Option<&str>,
    sender_wxid: Option<&str>,
    form: TextForm,
) -> Option<String> {
    let normalized = x.once();
    let kind = appmsg::app_message_kind_in(x, local_type);
    if kind == Some("location") {
        let place = appmsg::location_label(&appmsg::without_quoted(normalized));
        return Some(format!("[位置] {place}"));
    }
    let own = x.app_type_twice();
    let ty = match appmsg::app_type(own) {
        Some(_) => own,
        None => kind.and_then(appmsg::type_of_kind)?,
    };
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
        "24" | "19" if form == TextForm::Line => with_title(&title),
        "24" => note_text(normalized, &title),
        "87" => with_title(&xml_text(normalized, "textannouncement")),
        "3" | "76" => {
            let song = xml_text(normalized, "songname");
            with_title(if song.is_empty() { &title } else { &song })
        }
        "19" => format_forward_chat_record_content(normalized),
        "57" => match x.quoted_display().filter(|_| form == TextForm::Full) {
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

// ─────────────────────────── 行 → ExportMsg ───────────────────────────

/// [`collect_rows`] 取哪些行，以及行所属的会话和账号。
#[derive(Debug)]
pub struct CollectOptions<'a> {
    /// 行所属的会话。
    pub session_id: &'a str,
    /// 账号的 wxid。
    pub my_wxid: &'a str,
    /// Unix 秒：只取这个时间及以后的消息；`None` 或 0 时不限。
    pub start: Option<i64>,
    /// Unix 秒：只取这个时间及以前的消息；`None` 或 0 时不限。
    pub end: Option<i64>,
    /// 只取这个发送者的消息；为空时不限。
    pub sender_filter: Option<&'a str>,
}

/// 导出所写的消息发送者：系统消息和拍一拍中，撤回提示为撤回者，其他为会话本身；其余消息中，账号自己发的为账号，
/// 其他为行中的发送者，没有时为会话本身。
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

/// 以整数保存的时间换算成 Unix 秒：8、12、14 位的先按紧凑日期（`YYYYMMDD`、`YYYYMMDDhhmm`、`YYYYMMDDhhmmss`）读，
/// 读不出时与其余的一样按秒、毫秒或更细的单位截成秒；不是正数时为 0。
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
        reason = "与解析单元格的文字时相同：超过 2^53 的值不会是需要保留精度的时间"
    )]
    normalize_timestamp_seconds(value as f64)
}

/// 类型化的消息行的发送时间（Unix 秒）：取 `create_time`；`create_time` 早于 2000 年而 `sort_seq` 晚于 2000 年时，
/// 或者 `create_time` 缺失、不是正数时，取 `sort_seq`。
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

/// 类型化的消息行的发送者，规则同 `message_sender`。
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

/// 把类型化的消息行转换成导出的消息：只留时间范围内、指定发送者的消息，按时间排序，时间相同的按 `local_id`。
/// 行的所有权交给它，行的文字直接移进消息，不复制。
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
            // 只解析一次，下面各项共用
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
        // 没有 recorditem 时取 <des>
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

    /// 导出读取消息时记下的 `<appmsg>` 类型与从 XML 重新求出的相同：普通和转义的 app 消息、
    /// 引用了另一条带自己类型的消息的回复、带拍一拍的、没有类型的，以及只是提到 `<appmsg>` 的文字。
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
