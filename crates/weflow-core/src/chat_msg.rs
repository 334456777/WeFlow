//! The messages of a conversation as the chat commands and the command API give them ([`ChatMessage`]).
//!
//! It turns typed message rows into messages with parsed display text, media tokens, links and quotes.
//! This one backs the HTTP API, the chat commands and the analytics code; the exporter has its own model in
//! `message.rs`, whose text differs on purpose: a chat window's line (`[链接] title`,
//! `[语音消息]`) against an export's (`[引用 …]`, a transfer's direction, a sticker's caption).
//!
//! What a row means is read the same way by both, with `message.rs`'s functions: the sender prefix of a group
//! message, the `<refermsg>` of a quote and its text, a system message's XML. Only what the chat window words
//! differently is here.

use weflow_native::msg_row::MsgRow;
use weflow_native::quote::{reply_text, QUOTE};

use crate::api::js_parse_int;
use crate::message::appmsg::{kind_by_keywords, kind_of_app_type, without_quoted};
use crate::message::emoji::percent_decode;
use crate::message::quote::{extract_preferred_quoted_text, refermsg_xml};
use crate::message::system::{extract_readable_system_message_text, parse_voip_message};
use crate::message::transfer::is_same_wxid;
use crate::message::xml::{
    extract_xml_value, looks_like_wxid, rx, sanitize_quoted_content, strip_sender_line,
    strip_sender_prefix,
};
use crate::xml::decode_entities;

// ───────────────────────── text helpers ─────────────────────────

/// `input` without the C0 and C1 control characters, but for tab, line feed and carriage return.
pub fn clean_utf16(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    input
        .chars()
        .filter(|c| !matches!(*c as u32, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F..=0x9F))
        .collect()
}

fn extract_sender_from_content(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let normalized = clean_utf16(&decode_entities(content));
    let caps = rx(r"(?i)^\s*([A-Za-z0-9_@-]{4,}):\s*(?:\r?\n|<br\s*/?>)").captures(&normalized)?;
    Some(caps[1].trim().to_string()).filter(|s| !s.is_empty())
}

/// The attribute `attr` of the first `<tag>` of `xml`: single or double quotes, whitespace before the name.
fn xml_attr(xml: &str, tag: &str, attr: &str) -> String {
    let re = rx(&format!(
        r#"(?i)<{}[^>]*\s{}\s*=\s*['"]([^'"]*)['"]"#,
        regex::escape(tag),
        regex::escape(attr)
    ));
    re.captures(xml)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

fn first_non_empty(values: &[String]) -> String {
    values
        .iter()
        .find(|v| !v.is_empty())
        .cloned()
        .unwrap_or_default()
}

fn opt(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// The label the chat window shows for a message of `local_type` without text, such as `[图片]`; `[消息]` for a type
/// it does not name.
pub fn message_type_label(local_type: i64) -> &'static str {
    match local_type {
        1 => "[文本]",
        3 => "[图片]",
        34 => "[语音]",
        42 => "[名片]",
        43 => "[视频]",
        47 => "[动画表情]",
        48 => "[位置]",
        49 => "[链接]",
        50 => "[通话]",
        10000 => "[系统消息]",
        244813135921 => "[引用消息]",
        266287972401 => "拍一拍",
        81604378673 => "[聊天记录]",
        154618822705 => "[小程序]",
        8594229559345 => "[红包]",
        8589934592049 => "[转账]",
        34359738417 | 103079215153 | 25769803825 => "[文件]",
        _ => "[消息]",
    }
}

/// The line of a system message: what its `<sysmsg>` says in plain text, else the notice the export reads out of
/// its XML ([`crate::message::system::clean_system_message`]: a revoke's notice, a template with its names filled in,
/// a title), with the entities decoded and the sender and the XML declaration in front of it dropped.
fn system_message_text(content: &str) -> String {
    if content.is_empty() {
        return "[系统消息]".into();
    }
    let normalized = clean_utf16(&decode_entities(content));
    let readable = extract_readable_system_message_text(&normalized);
    if !readable.is_empty() {
        return readable;
    }
    let body = strip_sender_prefix(&normalized);
    crate::message::system::clean_system_message(&rx(r"(?i)<\?xml[^?]*\?>").replace_all(&body, ""))
}

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

// ───────────────────────── type 49 ─────────────────────────

fn app_xml_type(content: &str) -> String {
    let mut xml_type = String::new();
    if let Some(c) = rx(r"(?is)<appmsg.*?>(.*?)</appmsg>").captures(content) {
        let inner = rx(r"(?is)<refermsg.*?</refermsg>")
            .replace_all(&c[1], "")
            .to_string();
        let inner = rx(r"(?is)<patMsg.*?</patMsg>")
            .replace_all(&inner, "")
            .to_string();
        if let Some(t) = rx(r"(?is)<type>(.*?)</type>").captures(&inner) {
            xml_type = t[1].trim().to_string();
        }
    }
    if xml_type.is_empty() {
        xml_type = extract_xml_value(content, "type");
    }
    xml_type
}

fn location_label_of(content: &str) -> String {
    first_non_empty(&[
        xml_attr(content, "location", "label"),
        xml_attr(content, "location", "poiname"),
        extract_xml_value(content, "label"),
        extract_xml_value(content, "poiname"),
    ])
}

/// `parseType49`: the display string for appmsg content. Its kind is the one its own `<type>` gives it, as the
/// chat window's `appMsgKind` and the exports have it; the words of its XML only count when the type says nothing,
/// and never those of the message a reply quotes (#1165).
fn parse_type49(content: &str) -> String {
    let title = extract_xml_value(content, "title");
    let ty = app_xml_type(content);
    let own = without_quoted(content);
    let location_label = location_label_of(&own);
    let kind = kind_of_app_type(&ty).or_else(|| kind_by_keywords(&own));
    let is_finder = kind == Some("finder");
    let is_red_packet = kind == Some("red-packet");
    let is_music = kind == Some("music");

    if ty == "87" {
        let t = extract_xml_value(content, "textannouncement");
        return if t.is_empty() {
            "[群公告]".into()
        } else {
            format!("[群公告] {t}")
        };
    }
    if is_finder {
        return if title.is_empty() {
            "[视频号]".into()
        } else {
            format!("[视频号] {title}")
        };
    }
    if is_red_packet {
        return if title.is_empty() {
            "[红包]".into()
        } else {
            format!("[红包] {title}")
        };
    }
    if !location_label.is_empty() {
        return format!("[位置] {location_label}");
    }
    if is_music {
        return if title.is_empty() {
            "[音乐]".into()
        } else {
            format!("[音乐] {title}")
        };
    }
    if !title.is_empty() {
        return match ty.as_str() {
            "5" | "49" => format!("[链接] {title}"),
            "6" => format!("[文件] {title}"),
            "19" => format!("[聊天记录] {title}"),
            "33" | "36" => format!("[小程序] {title}"),
            "57" => title,
            "53" => {
                let first = title
                    .split('\n')
                    .map(|l| l.trim())
                    .find(|l| !l.is_empty())
                    .unwrap_or(&title)
                    .to_string();
                format!("[接龙] {first}")
            }
            "2000" => format!("[转账] {title}"),
            "2001" => format!("[红包] {title}"),
            _ => title,
        };
    }
    match ty.as_str() {
        "6" => "[文件]",
        "19" => "[聊天记录]",
        "33" | "36" => "[小程序]",
        "2000" => "[转账]",
        "2001" => "[红包]",
        "3" => "[音乐]",
        "5" | "49" => "[链接]",
        "87" => "[群公告]",
        "53" => "[接龙]",
        _ => "[消息]",
    }
    .into()
}

/// The line the chat window shows for any message
/// ([`ChatMessage::parsed_content`]), `sender` the message's ([`ChatMessage::sender_username`]). The exports word
/// theirs in [`crate::message::parse_message_content`].
fn parsed_content(content: &str, local_type: i64, sender: Option<&str>) -> String {
    if content.is_empty() {
        return message_type_label(local_type).into();
    }
    if local_type == QUOTE {
        let t = reply_text(content);
        return if t.is_empty() {
            "[引用消息]".into()
        } else {
            t
        };
    }
    let content = clean_utf16(&decode_entities(content));
    let xml_type = extract_xml_value(&content, "type");
    let looks_app = content.contains("<appmsg") || content.contains("&lt;appmsg");
    match local_type {
        1 => strip_sender_line(&content, sender),
        3 => "[图片]".into(),
        34 => "[语音消息]".into(),
        42 => "[名片]".into(),
        43 => "[视频]".into(),
        47 => "[动画表情]".into(),
        48 => {
            let label = location_label_of(&content);
            if label.is_empty() {
                "[位置]".into()
            } else {
                format!("[位置] {label}")
            }
        }
        49 => parse_type49(&content),
        50 => parse_voip_message(&content),
        10000 => system_message_text(&content),
        266287972401 => clean_pat_message(&content),
        81604378673 => "[聊天记录]".into(),
        8594229559345 => "[红包]".into(),
        8589934592049 => "[转账]".into(),
        _ => {
            if xml_type == "87" {
                let t = extract_xml_value(&content, "textannouncement");
                return if t.is_empty() {
                    "[群公告]".into()
                } else {
                    format!("[群公告] {t}")
                };
            }
            if xml_type == "57" {
                let t = extract_xml_value(&content, "title");
                return if t.is_empty() {
                    "[引用消息]".into()
                } else {
                    t
                };
            }
            if looks_app {
                return parse_type49(&content);
            }
            let generic = extract_xml_value(&content, "title");
            if !generic.is_empty() && generic.chars().count() < 100 {
                return generic;
            }
            if content.chars().count() > 200 {
                return message_type_label(local_type).into();
            }
            let stripped = strip_sender_prefix(&content);
            if stripped.is_empty() {
                message_type_label(local_type).into()
            } else {
                stripped
            }
        }
    }
}

/// What the XML of an app message (type 49) and a few others say, as the chat window shows it. Only what is found
/// is serialized.
#[derive(Default, Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Type49Info {
    /// The `<type>` of the `<appmsg>`: `5` a link, `6` a file, `19` a chat record, `33`/`36` a mini program, `57` a
    /// quote, `2000` a transfer, `2001` a red packet…
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xml_type: Option<String>,
    /// The message's own, serialized by [`ChatMessage`].
    #[serde(skip)]
    pub quoted_content: Option<String>,
    /// The message's own, serialized by [`ChatMessage`].
    #[serde(skip)]
    pub quoted_sender: Option<String>,
    /// The title: a link's or a mini program's, a file's name, a transfer's memo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_title: Option<String>,
    /// The address it opens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_url: Option<String>,
    /// The address of its thumbnail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_thumb: Option<String>,
    /// What it is: `link`, `official-link`, `file`, `music`, `location`, `miniapp`, `chat-record`, `transfer`,
    /// `red-packet`, `gift`, `finder`, `quote`, `announcement`, `solitaire` or `card`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_kind: Option<String>,
    /// Its description (`<des>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_desc: Option<String>,
    /// The name of the app that sent it (`<appname>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_app_name: Option<String>,
    /// The name of the account it comes from (`<sourcename>`), such as an official account's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_source_name: Option<String>,
    /// The username of the account it comes from (`<sourceusername>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_source_username: Option<String>,
    /// The address of its thumbnail or cover.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_thumb_url: Option<String>,
    /// The address of a song (`<musicurl>`, `<playurl>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_music_url: Option<String>,
    /// The address of its data (`<dataurl>`, `<lowurl>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_data_url: Option<String>,
    /// The label of a location it names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_location_label: Option<String>,
    /// The name of a Channels account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finder_nickname: Option<String>,
    /// The username of a Channels account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finder_username: Option<String>,
    /// A Channels video's cover.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finder_cover_url: Option<String>,
    /// The address of a Channels account's avatar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finder_avatar: Option<String>,
    /// A Channels video's length, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finder_duration: Option<i64>,
    /// The message's own, serialized by [`ChatMessage`].
    #[serde(skip)]
    pub location_lat: Option<f64>,
    /// The message's own, serialized by [`ChatMessage`].
    #[serde(skip)]
    pub location_lng: Option<f64>,
    /// The message's own, serialized by [`ChatMessage`].
    #[serde(skip)]
    pub location_poiname: Option<String>,
    /// The message's own, serialized by [`ChatMessage`].
    #[serde(skip)]
    pub location_label: Option<String>,
    /// A song's album cover.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_album_url: Option<String>,
    /// A song's address to play.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_url: Option<String>,
    /// A gift's picture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gift_image_url: Option<String>,
    /// A gift's wish.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gift_wish: Option<String>,
    /// A gift's price, as the message writes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gift_price: Option<String>,
    /// A file's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// A file's size, in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size: Option<i64>,
    /// A file's extension, without its dot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_ext: Option<String>,
    /// A file's MD5, in lowercase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_md5: Option<String>,
    /// Who pays a transfer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_payer_username: Option<String>,
    /// Who receives a transfer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_receiver_username: Option<String>,
    /// A chat record's title; 「聊天记录」 when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_record_title: Option<String>,
}

fn parse_js_float(s: &str) -> Option<f64> {
    crate::sns::parse_float(s)
}

/// `parseType49Message`
fn parse_type49_message(content: &str) -> Type49Info {
    let mut r = Type49Info::default();
    if content.is_empty() {
        return r;
    }
    let xml_type = app_xml_type(content);
    if xml_type.is_empty() {
        return r;
    }
    let ev = |t: &str| extract_xml_value(content, t);
    let title = ev("title");
    let url = ev("url");
    let desc = first_non_empty(&[ev("des"), ev("description")]);
    let app_name = ev("appname");
    let source_name = ev("sourcename");
    let source_username = ev("sourceusername");
    let thumb_url = first_non_empty(&[
        ev("thumburl"),
        ev("cdnthumburl"),
        ev("cover"),
        ev("coverurl"),
        ev("thumb_url"),
    ]);
    let music_url = first_non_empty(&[ev("musicurl"), ev("playurl"), ev("songalbumurl")]);
    let data_url = first_non_empty(&[ev("dataurl"), ev("lowurl")]);
    // the place of the message itself, not of one a reply quotes (#1165)
    let location_label = location_label_of(&without_quoted(content));
    let finder_username = first_non_empty(&[
        ev("finderusername"),
        ev("finder_username"),
        ev("finderuser"),
    ]);
    let finder_nickname = first_non_empty(&[ev("findernickname"), ev("finder_nickname")]);
    // the same table as the exports' (#1165)
    let typed = kind_of_app_type(&xml_type);
    let is_finder = typed == Some("finder");
    let is_red_packet = typed == Some("red-packet");
    let is_music = typed == Some("music");
    let is_location = !location_label.is_empty();

    r.xml_type = Some(xml_type.clone());
    r.link_title = opt(title.clone());
    r.link_url = opt(url.clone());
    r.link_thumb = opt(thumb_url.clone());
    r.app_msg_desc = opt(desc);
    r.app_msg_app_name = opt(app_name.clone());
    r.app_msg_source_name = opt(source_name.clone());
    r.app_msg_source_username = opt(source_username.clone());
    r.app_msg_thumb_url = opt(thumb_url);
    r.app_msg_music_url = opt(music_url.clone());
    r.app_msg_data_url = opt(data_url.clone());
    r.app_msg_location_label = opt(location_label.clone());
    r.finder_username = opt(finder_username);
    r.finder_nickname = opt(finder_nickname);

    if is_finder {
        r.finder_cover_url = opt(first_non_empty(&[
            ev("thumbUrl"),
            ev("coverUrl"),
            ev("thumburl"),
            ev("coverurl"),
        ]));
        r.finder_avatar = opt(ev("avatar"));
        let d = first_non_empty(&[ev("videoPlayDuration"), ev("duration")]);
        if !d.is_empty() {
            if let Some(n) = js_parse_int(&d).filter(|n| *n > 0) {
                r.finder_duration = Some(n);
            }
        }
    }
    if is_location {
        let lat = first_non_empty(&[
            xml_attr(content, "location", "x"),
            xml_attr(content, "location", "latitude"),
        ]);
        let lng = first_non_empty(&[
            xml_attr(content, "location", "y"),
            xml_attr(content, "location", "longitude"),
        ]);
        r.location_lat = parse_js_float(&lat);
        r.location_lng = parse_js_float(&lng);
        r.location_poiname = opt(first_non_empty(&[
            xml_attr(content, "location", "poiname"),
            location_label,
        ]));
        r.location_label = opt(xml_attr(content, "location", "label"));
    }
    if is_music {
        r.music_album_url = opt(ev("songalbumurl"));
        r.music_url = opt(first_non_empty(&[music_url, data_url, url.clone()]));
    }
    let is_gift = xml_type == "115";
    if is_gift {
        r.gift_wish = opt(ev("wishmessage"));
        r.gift_image_url = opt(ev("skuimgurl"));
        r.gift_price = opt(ev("skuprice"));
    }

    let kind = if is_finder {
        "finder"
    } else if is_red_packet {
        "red-packet"
    } else if is_gift {
        "gift"
    } else if is_location {
        "location"
    } else if typed == Some("quote") {
        (r.quoted_content, r.quoted_sender) = quote_of(content);
        "quote"
    } else if let Some(kind) = typed.filter(|k| *k != "link") {
        kind
    } else if typed == Some("link")
        && (source_username.starts_with("gh_")
            || app_name.contains("公众号")
            || !source_name.is_empty())
    {
        "official-link"
    } else if !url.is_empty() {
        "link"
    } else {
        "card"
    };
    r.app_msg_kind = Some(kind.into());

    match xml_type.as_str() {
        "6" => {
            let name = if title.is_empty() {
                ev("filename")
            } else {
                title
            };
            r.link_title = opt(name.clone());
            r.file_name = opt(name.clone());
            let size = first_non_empty(&[ev("totallen"), ev("filesize")]);
            if !size.is_empty() {
                r.file_size = js_parse_int(&size);
            }
            let ext = ev("fileext");
            let md5 = first_non_empty(&[ev("md5"), ev("filemd5")]);
            if !ext.is_empty() {
                r.file_ext = Some(ext);
            } else if let Some(c) = rx(r"\.([^.]+)$").captures(&name) {
                r.file_ext = Some(c[1].to_string());
            }
            if !md5.is_empty() {
                r.file_md5 = Some(md5.to_lowercase());
            }
        }
        "19" => {
            r.chat_record_title = Some(if title.is_empty() {
                "聊天记录".into()
            } else {
                title
            });
        }
        "33" | "36" => {
            r.link_title = opt(title);
            r.link_url = opt(url);
            let thumb = first_non_empty(&[ev("thumburl"), ev("cdnthumburl")]);
            if !thumb.is_empty() {
                r.link_thumb = Some(thumb);
            }
        }
        "2000" => {
            let mut t = if title.is_empty() {
                "[转账]".to_string()
            } else {
                title
            };
            let memo = ev("pay_memo");
            let fee = ev("feedesc");
            if !memo.is_empty() {
                t = memo;
            } else if !fee.is_empty() {
                t = fee;
            }
            r.link_title = Some(t);
            r.transfer_payer_username = opt(ev("payer_username"));
            r.transfer_receiver_username = opt(ev("receiver_username"));
        }
        _ => {
            r.link_title = opt(title);
            r.link_url = opt(url);
            let thumb = first_non_empty(&[ev("thumburl"), ev("cdnthumburl")]);
            if !thumb.is_empty() {
                r.link_thumb = Some(thumb);
            }
        }
    }
    r
}

/// What a reply quotes, as (content, sender).
fn quote_of(content: &str) -> (Option<String>, Option<String>) {
    let Some(refer_xml) = refermsg_xml(&decode_entities(content)) else {
        return (None, None);
    };
    let refer_xml = refer_xml.as_str();
    let mut display_name = extract_xml_value(refer_xml, "displayname");
    if !display_name.is_empty() && looks_like_wxid(&display_name) {
        display_name.clear();
    }
    let refer_content = extract_xml_value(refer_xml, "content");
    let refer_type = extract_xml_value(refer_xml, "type");
    let display_content = match refer_type.as_str() {
        "1" => extract_preferred_quoted_text(refer_xml),
        "3" => "[图片]".into(),
        "34" => "[语音]".into(),
        "43" => "[视频]".into(),
        "47" => "[动画表情]".into(),
        "49" => {
            let inner = parse_type49_message(&decode_entities(&refer_content));
            match (inner.xml_type.as_deref(), inner.link_title) {
                (Some("57"), Some(title)) => title,
                _ => "[链接]".into(),
            }
        }
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
    (Some(display_content), opt(display_name))
}

fn parse_media_quote(content: &str) -> Option<String> {
    let svrid = extract_xml_value(&refermsg_xml(&decode_entities(content))?, "svrid");
    (!svrid.is_empty()).then(|| format!("__SVRID__{svrid}__"))
}

// ───────────────────────── media token helpers ─────────────────────────

/// The token that names a video's file in `value`, a name or path: lowercase hex of 16 to 64 digits, with the `_raw`
/// of an original kept; `None` when there is none.
pub fn normalize_video_file_token(value: &str) -> Option<String> {
    let mut text = value.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }
    text = rx(r"^.*[\\/]").replace(&text, "").to_string();
    text = rx(r"(?i)\.(?:mp4|mov|m4v|avi|mkv|flv|jpg|jpeg|png|gif|dat)$")
        .replace(&text, "")
        .to_string();
    text = rx(r"_thumb$").replace(&text, "").to_string();
    if let Some(c) = rx(r"^([a-f0-9]{16,64})(_raw)?$").captures(&text) {
        return Some(format!(
            "{}{}",
            &c[1],
            c.get(2).map(|m| m.as_str()).unwrap_or("")
        ));
    }
    // `([a-f0-9]{32})(?![a-f0-9])`, then the generic 16–64 variant, emulated on hex runs
    let runs: Vec<&str> = rx(r"[a-f0-9]+")
        .find_iter(&text)
        .map(|m| m.as_str())
        .collect();
    for run in &runs {
        if run.len() >= 32 {
            return Some(run[run.len() - 32..].to_string());
        }
    }
    for run in &runs {
        if run.len() >= 16 {
            let take = run.len().min(64);
            return Some(run[run.len() - take..].to_string());
        }
    }
    None
}

/// A video's file token in a row's packed info: a run of 32 hex digits, else one of 16 to 64.
fn video_token_in_packed(bytes: &[u8]) -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    let mut current = String::new();
    for &b in bytes {
        if b.is_ascii_hexdigit() {
            current.push(b as char);
            continue;
        }
        if current.len() >= 16 {
            candidates.push(std::mem::take(&mut current));
        }
        current.clear();
    }
    if current.len() >= 16 {
        candidates.push(current);
    }
    candidates
        .iter()
        .find(|c| c.len() == 32)
        .or_else(|| candidates.iter().find(|c| c.len() >= 16 && c.len() <= 64))
        .map(|c| c.to_lowercase())
}

fn video_md5_from_content(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let md5 = xml_attr(content, "videomsg", "md5");
    if !md5.is_empty() {
        return Some(md5.to_lowercase());
    }
    let raw = xml_attr(content, "videomsg", "rawmd5");
    if !raw.is_empty() {
        return Some(raw.to_lowercase());
    }
    opt(extract_xml_value(content, "md5")).map(|m| m.to_lowercase())
}

pub(crate) fn parse_voice_duration_seconds(content: &str) -> Option<i64> {
    let caps =
        rx(r#"(?i)(voicelength|length|time|playlength)\s*=\s*['"]?([0-9]+(?:\.[0-9]+)?)['"]?"#)
            .captures(content)?;
    let raw: f64 = caps[2]
        .parse()
        .ok()
        .filter(|f: &f64| f.is_finite() && *f > 0.0)?;
    Some(if raw > 1000.0 {
        crate::num::saturating_i64((raw / 1000.0).round())
    } else {
        crate::num::saturating_i64(raw.round())
    })
}

#[derive(Default)]
struct EmojiInfo {
    cdn_url: Option<String>,
    md5: Option<String>,
}

fn parse_emoji_info(content: &str) -> EmojiInfo {
    let grab = |name: &str| -> Option<String> {
        let quoted = rx(&format!(r#"(?i){name}\s*=\s*['"]([^'"]+)['"]"#)).captures(content);
        let bare = || rx(&format!(r#"(?i){name}\s*=\s*([^'"\s/>]+)"#)).captures(content);
        quoted
            .or_else(bare)
            .map(|c| percent_decode(&c[1].replace("&amp;", "&")))
    };
    let md5 = rx(r#"(?i)md5\s*=\s*['"]([a-fA-F0-9]+)['"]"#)
        .captures(content)
        .or_else(|| rx(r"(?i)md5\s*=\s*([a-fA-F0-9]+)").captures(content))
        .map(|c| c[1].to_string());
    EmojiInfo {
        cdn_url: grab("cdnurl"),
        md5,
    }
}

// ───────────────────────── the model ─────────────────────────

/// A message as the chat window shows it (`chat messages --parsed` and the other parsed reads, #399).
///
/// It holds its text and the keys of its media, links, cards and so on, parsed from the row. An optional field is
/// left out when the message has none; times are Unix seconds.
#[derive(Default, Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    /// The message's id in its conversation's table; with `createTime`, the key of `chat message`.
    pub local_id: i64,
    /// WeChat's id of the message as a number; not serialized, `serverId` is its decimal string.
    #[serde(skip)]
    pub server_id: i64,
    /// WeChat's id of the message, in decimal: a string, as it may pass 2^53.
    #[serde(rename = "serverId")]
    pub server_id_raw: String,
    /// WeChat's type of the message (1 text, 3 image, 34 voice, 43 video, 47 sticker, 49 app message, 10000 system…).
    pub local_type: i64,
    /// When it was sent.
    pub create_time: i64,
    /// WeChat's order of the messages of a conversation: the send time in milliseconds, plus a little.
    pub sort_seq: i64,
    /// 1 when I sent it, 0 when I received it; null when the row does not say.
    pub is_send: Option<i64>,
    /// Who sent it; null when the row does not say.
    pub sender_username: Option<String>,
    /// The text the chat window shows.
    pub parsed_content: String,
    /// The message as WeChat keeps it (often XML).
    pub raw_content: String,
    /// A sticker's address on WeChat's servers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_cdn_url: Option<String>,
    /// A sticker's MD5.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_md5: Option<String>,
    /// What a reply quotes, as the chat window shows it; an image, video or voice that answers a message has
    /// `__SVRID__<server id>__` naming it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quoted_content: Option<String>,
    /// The name of whom a reply quotes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quoted_sender: Option<String>,
    /// An image's MD5, as its XML gives it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_md5: Option<String>,
    /// The name of an image's `.dat` file, from the row's packed info or the XML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_dat_name: Option<String>,
    /// The MD5 that names a video's file, in lowercase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_md5: Option<String>,
    /// A voice's length in seconds, rounded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_duration_seconds: Option<i64>,
    /// An image's AES key on WeChat's servers (`<img aeskey>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aes_key: Option<String>,
    /// An image's `encrypver`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryp_ver: Option<i64>,
    /// The address of an image's or a sticker's thumbnail on WeChat's servers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cdn_thumb_url: Option<String>,
    /// The username a contact card shares.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_username: Option<String>,
    /// The nickname a contact card shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_nickname: Option<String>,
    /// The address of a contact card's avatar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_avatar_url: Option<String>,
    /// A location's latitude.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_lat: Option<f64>,
    /// A location's longitude.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_lng: Option<f64>,
    /// The name of a location's place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_poiname: Option<String>,
    /// A location's label, its address as WeChat shows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_label: Option<String>,
    /// Only app messages and a few others have these: boxed, the others are smaller (#505).
    #[serde(flatten)]
    pub t49: Option<Box<Type49Info>>,
    /// The message's key: the same message has the same key on every machine. It is opaque; only compare it.
    pub message_key: String,
}

// one per message of a page of parsed messages or search hits: its size changes only on purpose (#505)
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<ChatMessage>() == 584);

impl ChatMessage {
    /// [`Type49Info::link_title`], when the message has one.
    pub fn link_title(&self) -> Option<&str> {
        self.t49.as_ref()?.link_title.as_deref()
    }
    /// [`Type49Info::file_name`], when the message has one.
    pub fn file_name(&self) -> Option<&str> {
        self.t49.as_ref()?.file_name.as_deref()
    }
    /// [`Type49Info::file_md5`], when the message has one.
    pub fn file_md5(&self) -> Option<&str> {
        self.t49.as_ref()?.file_md5.as_deref()
    }
    /// [`Type49Info::xml_type`], when the message has one.
    pub fn xml_type(&self) -> Option<&str> {
        self.t49.as_ref()?.xml_type.as_deref()
    }
    /// [`Type49Info::app_msg_kind`], when the message has one.
    pub fn app_msg_kind(&self) -> Option<&str> {
        self.t49.as_ref()?.app_msg_kind.as_deref()
    }
}

// ───────────────────────── message keys ─────────────────────────

/// JS `encodeURIComponent`
fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.trim().bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Where a message is kept: its database by name (`message_0`), never by path (#443), and its table.
struct SourceInfo {
    db_name: String,
    table_name: String,
}

/// The key of a message, scoped by the database's name (`message_0`) instead of its path.
fn build_message_key(
    local_id: i64,
    server_id: i64,
    create_time: i64,
    sort_seq: i64,
    sender: Option<&str>,
    local_type: i64,
    src: &SourceInfo,
) -> String {
    let (local_id, server_id, create_time, sort_seq) = (
        local_id.max(0),
        server_id.max(0),
        create_time.max(0),
        sort_seq.max(0),
    );
    let sender = encode_uri_component(sender.unwrap_or(""));
    // The database's name, not its path as WeFlow's desktop app has it: the path would tell the client this
    // machine's directories and change the key on another one (#443).
    let scope = &src.db_name;
    if local_id > 0 && !scope.is_empty() && !src.table_name.is_empty() {
        return format!(
            "{}:{}:{}",
            encode_uri_component(scope),
            encode_uri_component(&src.table_name),
            local_id
        );
    }
    if local_id > 0 && !scope.is_empty() {
        return format!(
            "local:{}:{local_id}:{create_time}:{sort_seq}:{sender}:{local_type}",
            encode_uri_component(scope)
        );
    }
    if server_id > 0 {
        let scoped = if scope.is_empty() {
            server_id.to_string()
        } else {
            format!("{}:{server_id}", encode_uri_component(scope))
        };
        return format!(
            "server:{scoped}:{create_time}:{sort_seq}:{local_id}:{sender}:{local_type}"
        );
    }
    format!(
        "fallback:{}:{create_time}:{sort_seq}:{local_id}:{sender}:{local_type}",
        encode_uri_component(scope)
    )
}

// ───────────────────────── isSend / sender ─────────────────────────

/// A message whose sender is the account itself ([`is_same_wxid`]) is sent whatever its
/// stored flag says; otherwise the flag stands, a missing one meaning received. Without a sender or an account the
/// flag is kept as it is.
fn resolve_is_send(raw: Option<i64>, sender: Option<&str>, my_wxid: &str) -> Option<i64> {
    let sender = sender.unwrap_or("").trim();
    if sender.is_empty() || my_wxid.trim().is_empty() {
        return raw;
    }
    if is_same_wxid(sender, my_wxid) {
        Some(1)
    } else {
        Some(raw.unwrap_or(0))
    }
}

/// The columns [`chat_message`] turns into a message.
struct RowFields {
    content: String,
    local_type: i64,
    /// `is_send` as stored, before it is checked against the sender.
    raw_is_send: Option<i64>,
    /// The sender's username from its column (trimmed, none when empty).
    sender: Option<String>,
    create_time: i64,
    local_id: i64,
    server_id: i64,
    server_id_raw: String,
    sort_seq: i64,
}

/// The messages (the model fields the API and CLI use) of the typed rows of one conversation, which it
/// takes: their text moves into the messages.
pub fn map_msg_rows(rows: Vec<MsgRow>, session_id: &str, my_wxid: &str) -> Vec<ChatMessage> {
    let table_name = weflow_native::msg::table_name_for(session_id);
    rows.into_iter()
        .map(|row| map_msg_row(row, &table_name, my_wxid))
        .collect()
}

/// One row of [`map_msg_rows`], for callers that map rows one at a time; `table_name` is the conversation's
/// (`table_name_for`).
pub fn map_msg_row(mut row: MsgRow, table_name: &str, my_wxid: &str) -> ChatMessage {
    let fields = typed_fields(&mut row);
    let src = typed_source(&row, table_name);
    chat_message(fields, row.packed_info.as_deref(), src, my_wxid)
}

/// The fields of a typed row, its text moved out; a row without a `sort_seq` is ordered by its send time.
fn typed_fields(row: &mut MsgRow) -> RowFields {
    let create_time = crate::message::msg_row_timestamp(row);
    let server_id = row.server_id.unwrap_or(0);
    RowFields {
        content: std::mem::take(&mut row.content),
        local_type: row.local_type,
        raw_is_send: Some(i64::from(row.is_send)),
        sender: row
            .sender
            .take()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        create_time,
        local_id: row.local_id,
        server_id,
        server_id_raw: if server_id > 0 {
            server_id.to_string()
        } else {
            "0".into()
        },
        sort_seq: row.sort_seq.unwrap_or(create_time),
    }
}

fn typed_source(row: &MsgRow, table_name: &str) -> SourceInfo {
    SourceInfo {
        db_name: weflow_native::msg::shard_name(row.shard),
        table_name: table_name.to_string(),
    }
}

/// The location an app message names, on a message that has none of its own.
fn fill_location(m: &mut ChatMessage, t: &Type49Info) {
    if m.location_lat.is_none() {
        m.location_lat = t.location_lat;
    }
    if m.location_lng.is_none() {
        m.location_lng = t.location_lng;
    }
    if m.location_poiname.is_none() {
        m.location_poiname.clone_from(&t.location_poiname);
    }
    if m.location_label.is_none() {
        m.location_label.clone_from(&t.location_label);
    }
}

/// A message of a row's columns; `packed_info` is the row's, the only place besides its text a media file is named.
fn chat_message(
    fields: RowFields,
    packed_info: Option<&[u8]>,
    src: SourceInfo,
    my_wxid: &str,
) -> ChatMessage {
    let RowFields {
        content,
        local_type,
        raw_is_send: raw,
        sender,
        create_time,
        local_id,
        server_id,
        server_id_raw,
        sort_seq,
    } = fields;
    let sender_raw = sender.or_else(|| extract_sender_from_content(&content));
    let is_send = resolve_is_send(raw, sender_raw.as_deref(), my_wxid);
    let mut m = ChatMessage {
        local_id,
        server_id,
        server_id_raw,
        local_type,
        create_time,
        sort_seq,
        is_send,
        sender_username: sender_raw,
        ..Default::default()
    };
    let app_message = (local_type == 49 || local_type == 8589934592049) && !content.is_empty();

    if local_type == 47 && !content.is_empty() {
        let e = parse_emoji_info(&content);
        m.emoji_cdn_url = e.cdn_url;
        m.emoji_md5 = e.md5;
        m.cdn_thumb_url = rx(r#"(?i)thumburl\s*=\s*['"]([^'"]+)['"]"#)
            .captures(&content)
            .map(|c| percent_decode(&c[1].replace("&amp;", "&")));
    } else if local_type == 3 && !content.is_empty() {
        let md5 = first_non_empty(&[
            extract_xml_value(&content, "md5"),
            xml_attr(&content, "img", "md5"),
        ]);
        m.image_md5 = opt(md5);
        m.aes_key = opt(xml_attr(&content, "img", "aeskey"));
        m.encryp_ver = js_parse_int(&xml_attr(&content, "img", "encrypver"));
        m.cdn_thumb_url = opt(xml_attr(&content, "img", "cdnthumburl"));
        m.image_dat_name = packed_info
            .and_then(crate::message::media::dat_name_in_packed_info)
            .or_else(|| crate::message::media::extract_image_dat_name(&content));
        m.quoted_content = parse_media_quote(&content);
    } else if local_type == 43 {
        m.video_md5 = packed_info.and_then(video_token_in_packed).or_else(|| {
            video_md5_from_content(&content).and_then(|m| normalize_video_file_token(&m))
        });
        m.quoted_content = parse_media_quote(&content);
    } else if local_type == 34 && !content.is_empty() {
        m.voice_duration_seconds = parse_voice_duration_seconds(&content);
        m.quoted_content = parse_media_quote(&content);
    } else if local_type == 42 && !content.is_empty() {
        m.card_username = opt(xml_attr(&content, "msg", "username"));
        m.card_nickname = opt(xml_attr(&content, "msg", "nickname"));
        m.card_avatar_url = opt(first_non_empty(&[
            xml_attr(&content, "msg", "bigheadimgurl"),
            xml_attr(&content, "msg", "smallheadimgurl"),
        ]));
    } else if local_type == 48 && !content.is_empty() {
        let lat = first_non_empty(&[
            xml_attr(&content, "location", "x"),
            xml_attr(&content, "location", "latitude"),
        ]);
        let lng = first_non_empty(&[
            xml_attr(&content, "location", "y"),
            xml_attr(&content, "location", "longitude"),
        ]);
        m.location_lat = parse_js_float(&lat);
        m.location_lng = parse_js_float(&lng);
        m.location_label = opt(first_non_empty(&[
            xml_attr(&content, "location", "label"),
            extract_xml_value(&content, "label"),
        ]));
        m.location_poiname = opt(first_non_empty(&[
            xml_attr(&content, "location", "poiname"),
            extract_xml_value(&content, "poiname"),
        ]));
    } else if app_message {
        let t = parse_type49_message(&content);
        m.quoted_content = t.quoted_content.clone();
        m.quoted_sender = t.quoted_sender.clone();
        m.t49 = Some(Box::new(t));
    } else if local_type == 244813135921 || content.contains("<type>57</type>") {
        (m.quoted_content, m.quoted_sender) = quote_of(&content);
    }

    // a media message or a location is no app message: an `<appmsg>` in it is the message its `<refermsg>` quotes
    let has_appmsg = !crate::message::appmsg::is_non_app_type(local_type)
        && (content.contains("<appmsg") || content.contains("&lt;appmsg"));
    if has_appmsg && app_message {
        // `t49` is the parse of this content already: only the location it names is not the message's yet
        if let Some(t) = m.t49.take() {
            fill_location(&mut m, &t);
            m.t49 = Some(t);
        }
    } else if has_appmsg {
        let t = parse_type49_message(&content);
        let cur = m.t49.get_or_insert_default();
        macro_rules! fill {
            ($f:ident) => {
                if cur.$f.is_none() {
                    cur.$f = t.$f.clone();
                }
            };
        }
        fill!(xml_type);
        fill!(link_title);
        fill!(link_url);
        fill!(link_thumb);
        fill!(file_name);
        fill!(file_size);
        fill!(file_ext);
        fill!(file_md5);
        fill!(app_msg_kind);
        fill!(app_msg_desc);
        fill!(app_msg_app_name);
        fill!(app_msg_source_name);
        fill!(app_msg_source_username);
        fill!(app_msg_thumb_url);
        fill!(app_msg_music_url);
        fill!(app_msg_data_url);
        fill!(app_msg_location_label);
        fill!(finder_nickname);
        fill!(finder_username);
        fill!(finder_cover_url);
        fill!(finder_avatar);
        fill!(finder_duration);
        fill!(music_album_url);
        fill!(music_url);
        fill!(gift_image_url);
        fill!(gift_wish);
        fill!(gift_price);
        fill!(chat_record_title);
        fill!(transfer_payer_username);
        fill!(transfer_receiver_username);
        fill_location(&mut m, &t);
        if m.quoted_content.is_none() {
            m.quoted_content = t.quoted_content.clone();
        }
        if m.quoted_sender.is_none() {
            m.quoted_sender = t.quoted_sender;
        }
    }
    m.parsed_content = parsed_content(&content, local_type, m.sender_username.as_deref());
    m.raw_content = content;
    m.message_key = build_message_key(
        m.local_id,
        m.server_id,
        m.create_time,
        m.sort_seq,
        m.sender_username.as_deref(),
        m.local_type,
        &src,
    );
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sender_is_read_from_the_prefix_of_a_group_message() {
        assert_eq!(
            extract_sender_from_content("wxid_abcd:\nhello").as_deref(),
            Some("wxid_abcd")
        );
        assert_eq!(extract_sender_from_content("wxid_abcd:hello"), None);
    }

    #[test]
    fn a_text_message_is_decoded_once() {
        assert_eq!(
            parsed_content("a &amp;lt;b&amp;gt;", 1, None),
            "a &lt;b&gt;"
        );
        assert_eq!(parsed_content("a &lt;b&gt;", 1, None), "a <b>");
    }

    /// `xml` escaped once, as WeChat puts a quoted message's XML in a reply's `<refermsg><content>`.
    fn escaped(xml: &str) -> String {
        xml.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// A reply saying 好看 to `quoted`, of `<type>` 57.
    fn reply_to(quoted: &str) -> String {
        format!(
            "<msg><appmsg appid=\"\" sdkver=\"0\"><title>好看</title><type>57</type><refermsg><type>49</type>\
             <svrid>7</svrid><fromusr>wxid_b</fromusr><displayname>B</displayname><content>{}</content>\
             </refermsg></appmsg></msg>",
            escaped(quoted)
        )
    }

    const FINDER: &str = "<msg><appmsg><title>一段视频</title><type>51</type><finderFeed>\
        <finderusername>v2_x@finder</finderusername><nickname>频道</nickname><desc>描述</desc></finderFeed>\
        </appmsg></msg>";
    const MUSIC: &str = "<msg><appmsg><title>一首歌</title><type>3</type><url>https://m</url>\
        <musicurl>https://m</musicurl><dataurl>https://m.mp3</dataurl><playurl>https://m.mp3</playurl></appmsg></msg>";
    const DATAURL_LINK: &str =
        "<msg><appmsg><title>一篇文章</title><type>5</type><url>https://a</url>\
        <dataurl>https://a</dataurl></appmsg></msg>";
    const RED_PACKET: &str = "<msg><appmsg><title>恭喜发财</title><type>2001</type><wcpayinfo>\
        <nativeurl>wxpay://c2cbizmessagehandler/hongbao/receivehongbao?x=1</nativeurl></wcpayinfo></appmsg></msg>";
    const LOCATION: &str =
        "<msg><location x=\"31.2\" y=\"121.4\" label=\"外滩\" poiname=\"外滩\"/></msg>";

    /// The kind of an app message is the one its own `<type>` gives it, wherever it is read: the words of its XML do
    /// not make a link a song or a red packet, and those of the message a reply quotes do not make the reply one
    /// (#1165).
    #[test]
    fn an_app_message_is_of_the_kind_its_type_says() {
        let link = |extra: &str| {
            format!(
                "<msg><appmsg><title>一篇文章</title><type>5</type><url>https://a</url>{extra}</appmsg></msg>"
            )
        };
        let hongbao_title = "<msg><appmsg><title>抢 HongBao 的攻略</title><type>5</type>\
            <url>https://a</url></appmsg></msg>";
        // a link: the chat window's text, the exports' text and both kinds agree
        for (xml, text) in [
            (hongbao_title.to_string(), "[链接] 抢 HongBao 的攻略"),
            (link("<dataurl>https://a</dataurl>"), "[链接] 一篇文章"),
            (link("<dataurl></dataurl>"), "[链接] 一篇文章"),
        ] {
            assert_eq!(parsed_content(&xml, 49, None), text, "{xml}");
            assert_eq!(
                crate::message::parse_message_content(&xml, 49, None, None, None).as_deref(),
                Some(text),
                "{xml}"
            );
            assert_eq!(
                crate::message::appmsg::app_message_kind(&xml, 49),
                Some("link"),
                "{xml}"
            );
            assert_eq!(
                parse_type49_message(&xml).app_msg_kind.as_deref(),
                Some("link"),
                "{xml}"
            );
        }
        // a reply, kept as either local type, to a Channels video, a song, a link with a `<dataurl>`, a red packet
        // and a place
        for quoted in [FINDER, MUSIC, DATAURL_LINK, RED_PACKET, LOCATION] {
            let xml = reply_to(quoted);
            for local_type in [49, QUOTE] {
                assert_eq!(parsed_content(&xml, local_type, None), "好看", "{quoted}");
                let kind = crate::message::appmsg::app_message_kind(&xml, local_type);
                assert_eq!(kind, Some("quote"), "{quoted} as {local_type}");
                let meta = crate::message::appmsg::extract_arkme_app_message_meta_in(
                    &crate::message::xml::MsgXml::new(&xml),
                    local_type,
                )
                .unwrap();
                assert!(meta.contains_key("quotedSender"), "{quoted}: {meta:?}");
                assert!(
                    !meta.contains_key("finderTitle") && !meta.contains_key("musicTitle"),
                    "{quoted}: {meta:?}"
                );
            }
            assert_eq!(
                parse_type49_message(&xml).app_msg_kind.as_deref(),
                Some("quote"),
                "{quoted}"
            );
        }
        // the messages themselves are still what they are
        for (xml, kind, text) in [
            (FINDER, "finder", "[视频号] 一段视频"),
            (MUSIC, "music", "[音乐] 一首歌"),
            (RED_PACKET, "red-packet", "[红包] 恭喜发财"),
        ] {
            assert_eq!(parsed_content(xml, 49, None), text);
            assert_eq!(
                crate::message::appmsg::app_message_kind(xml, 49),
                Some(kind)
            );
            assert_eq!(
                parse_type49_message(xml).app_msg_kind.as_deref(),
                Some(kind)
            );
        }
    }

    /// Whatever a reply quotes, and whatever the type of an app message, its kind is the one of its `<type>`, the same
    /// for the chat window and the exports (#1165): every type of the table, against every kind of quoted message.
    #[test]
    fn the_quoted_message_never_decides_the_kind() {
        let quoted = [
            FINDER,
            MUSIC,
            DATAURL_LINK,
            RED_PACKET,
            LOCATION,
            "抢 hongbao 的 <dataurl></dataurl> 文字",
        ];
        for ty in [
            "51", "2001", "3", "33", "36", "6", "19", "2000", "87", "57", "53", "5", "49",
        ] {
            let by_type = kind_of_app_type(ty).unwrap();
            for q in quoted {
                let xml = format!(
                    "<msg><appmsg><title>T</title><type>{ty}</type><url>https://a</url><refermsg><type>49</type>\
                     <svrid>7</svrid><content>{}</content></refermsg></appmsg></msg>",
                    escaped(q)
                );
                // a link with a quote in it is read as the quote
                let arkme = if by_type == "link" { "quote" } else { by_type };
                assert_eq!(
                    crate::message::appmsg::app_message_kind(&xml, 49),
                    Some(arkme),
                    "<type> {ty} quoting {q}"
                );
                let window = parse_type49_message(&xml).app_msg_kind.unwrap();
                assert!(
                    window == by_type || (by_type == "link" && window == "official-link"),
                    "<type> {ty} quoting {q}: {window}"
                );
            }
        }
    }

    /// A message whose `<type>` says nothing is told by the words of its own XML only: a non-empty `<musicurl>`, not
    /// a `<dataurl>`; a red packet's address, not `hongbao` in its text (#1165).
    #[test]
    fn a_message_without_a_type_is_told_by_its_own_words() {
        let app = |inner: &str| format!("<msg><appmsg><title>T</title>{inner}</appmsg></msg>");
        for (inner, kind) in [
            ("<musicurl>https://m</musicurl>", Some("music")),
            ("<musicurl></musicurl><dataurl>https://m</dataurl>", Some("card")),
            ("<des>抢 hongbao</des>", Some("card")),
            (
                "<wcpayinfo><nativeurl>wxpay://c2cbizmessagehandler/hongbao/x</nativeurl></wcpayinfo>",
                Some("red-packet"),
            ),
            ("<finderusername>v2_x@finder</finderusername>", Some("finder")),
        ] {
            let xml = app(inner);
            assert_eq!(
                crate::message::appmsg::app_message_kind(&xml, 49),
                kind,
                "{inner}"
            );
        }
        // the words of what it quotes do not count
        for quoted in [FINDER, MUSIC, RED_PACKET] {
            let own = format!("<appmsg><title>好看</title><refermsg><content>{quoted}</content></refermsg></appmsg>");
            assert_eq!(kind_by_keywords(&without_quoted(&own)), None, "{quoted}");
        }
    }

    #[test]
    fn display_text_per_type() {
        assert_eq!(parsed_content("", 3, None), "[图片]");
        assert_eq!(parsed_content("<msg/>", 3, None), "[图片]");
        assert_eq!(parsed_content("wxid_a:\nhi", 1, Some("wxid_a")), "hi");
        assert_eq!(
            parsed_content("123456@openim:\nhi", 1, Some("123456@openim")),
            "hi"
        );
        assert_eq!(
            parsed_content(
                "<msg><appmsg><title>T</title><type>5</type><url>http://x</url></appmsg></msg>",
                49,
                None
            ),
            "[链接] T"
        );
        assert_eq!(
            parsed_content(
                "<appmsg><title>f.pdf</title><type>6</type></appmsg>",
                49,
                None
            ),
            "[文件] f.pdf"
        );
        assert_eq!(
            parsed_content("<sysmsg><plain>Bob joined</plain></sysmsg>", 10000, None),
            "Bob joined"
        );
        assert_eq!(
            parsed_content("<msg><location label=\"Bund\"/></msg>", 48, None),
            "[位置] Bund"
        );
    }

    #[test]
    fn a_quote_shows_what_it_says() {
        let quote = |title: &str| {
            format!(
                "wxid_bob:\n<msg><appmsg><title>{title}</title><type>57</type>\
                 <refermsg><type>1</type><content>hi</content></refermsg></appmsg></msg>"
            )
        };
        // the other forms of a quote's title are tested in `weflow_native::quote`
        for (title, said) in [
            ("好的 &amp; 收到", "好的 & 收到"),
            ("<![CDATA[好的]]>", "好的"),
            // a reply that writes a tag is that text, not the end of the title
            ("写 &lt;/title&gt; 就行", "写 </title> 就行"),
        ] {
            assert_eq!(
                parsed_content(&quote(title), QUOTE, Some("wxid_bob")),
                said,
                "{title}"
            );
        }
        let untitled =
            "<msg><appmsg><type>57</type><refermsg><content>hi</content></refermsg></appmsg></msg>";
        assert_eq!(parsed_content(untitled, QUOTE, None), "[引用消息]");
        assert_eq!(parsed_content(&quote(""), QUOTE, None), "[引用消息]");
    }

    #[test]
    fn text_that_starts_like_a_sender_keeps_its_start() {
        // only `name:` and a line break, or an id, is a sender: a time, a score or a label is the text itself
        for text in ["4:1", "1:3-1:6", "10:1 abc", "PS: see you", "http://x"] {
            assert_eq!(parsed_content(text, 1, None), text);
        }
        // only a line naming the message's sender is its sender line (#656)
        assert_eq!(
            parsed_content("PS:\nsee you", 1, Some("wxid_bob")),
            "PS:\nsee you"
        );
        assert_eq!(
            parsed_content("wxid_me:\nhi", 1, Some("wxid_bob")),
            "wxid_me:\nhi"
        );
    }

    #[test]
    fn an_app_message_takes_its_fields_and_location_from_its_parse() {
        let contents = [
            r#"<msg><appmsg><title>Bund</title><type>17</type></appmsg><location x="31.2" y="121.5" label="Bund" poiname="外滩"/></msg>"#,
            "<msg><appmsg><title>f.pdf</title><type>6</type><appattach><totallen>12</totallen><fileext>pdf</fileext>\
             </appattach><md5>abc</md5></appmsg></msg>",
            "<msg><appmsg><title>好看</title><type>57</type><refermsg><type>1</type><svrid>9</svrid>\
             <content>原话</content><displayname>Bob</displayname></refermsg></appmsg></msg>",
            "&lt;msg&gt;&lt;appmsg&gt;&lt;title&gt;T&lt;/title&gt;&lt;type&gt;5&lt;/type&gt;\
             &lt;url&gt;http://x&lt;/url&gt;&lt;/appmsg&gt;&lt;/msg&gt;",
        ];
        for local_type in [49, 8_589_934_592_049] {
            for content in contents {
                let row = MsgRow {
                    local_type,
                    content: content.into(),
                    ..MsgRow::default()
                };
                let m = map_msg_rows(vec![row], "wxid_bob", "wxid_me").remove(0);
                let t = parse_type49_message(content);
                assert_eq!(
                    serde_json::to_value(m.t49.as_deref()).unwrap(),
                    serde_json::to_value(Some(&t)).unwrap(),
                    "{content}"
                );
                assert_eq!(
                    (m.location_lat, m.location_lng),
                    (t.location_lat, t.location_lng),
                    "{content}"
                );
                assert_eq!(
                    (m.location_poiname, m.location_label),
                    (t.location_poiname, t.location_label),
                    "{content}"
                );
                assert_eq!(
                    (m.quoted_content, m.quoted_sender),
                    (t.quoted_content, t.quoted_sender),
                    "{content}"
                );
            }
        }
        let location = MsgRow {
            local_type: 49,
            content: contents[0].into(),
            ..MsgRow::default()
        };
        let m = map_msg_rows(vec![location], "wxid_bob", "wxid_me").remove(0);
        assert_eq!((m.location_lat, m.location_lng), (Some(31.2), Some(121.5)));
        assert_eq!(m.location_label.as_deref(), Some("Bund"));
    }

    #[test]
    fn a_text_message_loses_only_the_line_naming_its_sender() {
        // as the chat model reads a row: a private message whose first line is `PS:` keeps it, a group message
        // somebody else sent loses the line naming them (#656)
        let row = |content: &str, sender: &str| MsgRow {
            local_type: 1,
            content: content.into(),
            sender: Some(sender.into()),
            ..MsgRow::default()
        };
        let messages = map_msg_rows(
            vec![
                row("PS:\nsee you", "wxid_bob"),
                row("wxid_carol:\nhi", "wxid_carol"),
            ],
            "room@chatroom",
            "wxid_me",
        );
        assert_eq!(messages[0].parsed_content, "PS:\nsee you");
        assert_eq!(messages[1].parsed_content, "hi");
    }

    #[test]
    fn system_messages_read_the_notice_out_of_their_xml() {
        let revoke = "<sysmsg type=\"revokemsg\"><revokemsg><session>wxid_bob</session><msgid>12</msgid>\
                      <newmsgid>345</newmsgid><replacemsg><![CDATA[\"Bob\" 撤回了一条消息]]></replacemsg>\
                      </revokemsg></sysmsg>";
        assert_eq!(
            parsed_content(revoke, 10000, None),
            "\"Bob\" 撤回了一条消息"
        );
        assert_eq!(
            parsed_content(&format!("room@chatroom:\n{revoke}"), 10000, None),
            "\"Bob\" 撤回了一条消息"
        );
        assert_eq!(
            parsed_content("<?xml version=\"1.0\"?><a>你已添加了 Bob</a>", 10000, None),
            "你已添加了 Bob"
        );
        // as the database keeps a revoke: its time is not part of the notice
        assert_eq!(
            parsed_content(
                "<?xml version=\"1.0\"?><sysmsg type=\"revokemsg\"><revokemsg><content>\"Bob\" 撤回了一条消息</content>\
                 <revoketime>0</revoketime></revokemsg></sysmsg>",
                10000,
                None
            ),
            "\"Bob\" 撤回了一条消息"
        );
        // a notice in plain text is itself, digits at its end included
        assert_eq!(
            parsed_content("群聊人数已达 100", 10000, None),
            "群聊人数已达 100"
        );
        assert_eq!(parsed_content("<a></a>", 10000, None), "[系统消息]");
    }

    #[test]
    fn a_quote_of_a_message_that_quotes_keeps_its_own_sender() {
        // the quoted message's content holds a `<refermsg>` of its own, decoded: the block ends at the last
        // closing tag, not at the inner one
        let xml = "<msg><appmsg><title>好看</title><type>57</type><refermsg><type>1</type><svrid>9</svrid>\
                   <content>原话&lt;refermsg&gt;&lt;svrid&gt;7&lt;/svrid&gt;&lt;/refermsg&gt;</content>\
                   <displayname>Me Nick</displayname></refermsg></appmsg></msg>";
        let (_, sender) = quote_of(xml);
        assert_eq!(sender.as_deref(), Some("Me Nick"));
        assert_eq!(parse_media_quote(xml).as_deref(), Some("__SVRID__9__"));
    }

    #[test]
    fn type49_fields_and_quotes() {
        let xml = "<msg><appmsg><title>ok</title><type>57</type><refermsg><type>1</type><displayname>Bob</displayname><content>hi there</content><svrid>9</svrid></refermsg></appmsg></msg>";
        let t = parse_type49_message(xml);
        assert_eq!(t.app_msg_kind.as_deref(), Some("quote"));
        assert_eq!(t.quoted_content.as_deref(), Some("hi there"));
        assert_eq!(t.quoted_sender.as_deref(), Some("Bob"));
        let f = parse_type49_message("<appmsg><title>a.zip</title><type>6</type><totallen>123</totallen><md5>ABC</md5></appmsg>");
        assert_eq!(f.file_name.as_deref(), Some("a.zip"));
        assert_eq!(f.file_size, Some(123));
        assert_eq!(f.file_ext.as_deref(), Some("zip"));
        assert_eq!(f.file_md5.as_deref(), Some("abc"));
    }

    #[test]
    fn is_send_uses_self_identity() {
        assert_eq!(
            resolve_is_send(Some(0), Some("wxid_me"), "wxid_me"),
            Some(1)
        );
        assert_eq!(
            resolve_is_send(Some(0), Some("wxid_bob"), "wxid_me"),
            Some(0)
        );
        assert_eq!(resolve_is_send(None, Some("wxid_bob"), "wxid_me"), Some(0));
        assert_eq!(resolve_is_send(None, None, "wxid_me"), None);
        // the account folder's `_<4 chars>` suffix, either way round
        assert_eq!(
            resolve_is_send(Some(0), Some("WXID_ME_1a2b"), "wxid_me"),
            Some(1)
        );
        assert_eq!(resolve_is_send(Some(0), Some("john"), "john_1a2b"), Some(1));
        // a custom WeChat id is no prefix of another person's
        assert_eq!(
            resolve_is_send(Some(0), Some("john_smith"), "john"),
            Some(0)
        );
        assert_eq!(
            resolve_is_send(Some(0), Some("john"), "john_smith"),
            Some(0)
        );
        assert_eq!(
            resolve_is_send(Some(1), Some("john_smith"), "john"),
            Some(1)
        );
    }

    #[test]
    fn message_keys_name_the_database_without_its_path() {
        let row = |local_id, server_id, create_time, shard| MsgRow {
            local_id,
            server_id,
            create_time: Some(create_time),
            local_type: 1,
            content: "x".into(),
            shard,
            ..MsgRow::default()
        };
        let m = map_msg_rows(
            vec![row(5, None, 10, 0), row(0, Some(77), 12, 1)],
            "wxid_bob",
            "wxid_me",
        );
        let table = weflow_native::msg::table_name_for("wxid_bob");
        assert_eq!(m[0].message_key, format!("message_0:{table}:5"));
        assert!(
            m[1].message_key.starts_with("server:message_1:77:12:12:0:"),
            "{}",
            m[1].message_key
        );
        let in_db = |db_name: &str| SourceInfo {
            db_name: db_name.into(),
            table_name: String::new(),
        };
        let key = build_message_key(6, 0, 11, 11, None, 1, &in_db("message_1"));
        assert!(key.starts_with("local:message_1:6:11:11:"), "{key}");
        let key = build_message_key(0, 77, 12, 12, None, 1, &in_db(""));
        assert!(key.starts_with("server:77:12:12:0:"), "{key}");
        assert_eq!(encode_uri_component("a b/é"), "a%20b%2F%C3%A9");
    }

    #[test]
    fn a_media_reply_takes_no_app_fields_from_the_app_message_it_quotes() {
        let quote = "<refermsg><type>49</type><svrid>9</svrid><content>&lt;msg&gt;&lt;appmsg&gt;&lt;title&gt;t\
                     &lt;/title&gt;&lt;type&gt;5&lt;/type&gt;&lt;/appmsg&gt;&lt;/msg&gt;</content></refermsg>";
        for (local_type, media) in [
            (3, "<img md5=\"x\"/>"),
            (34, "<voicemsg length=\"1000\"/>"),
            (43, "<videomsg md5=\"x\"/>"),
            (47, "<emoji md5=\"x\"/>"),
        ] {
            let content = format!("<msg>{media}<extcommoninfo>{quote}</extcommoninfo></msg>");
            let row = MsgRow {
                local_type,
                content: content.clone(),
                ..MsgRow::default()
            };
            let m = map_msg_rows(vec![row], "wxid_bob", "wxid_me").remove(0);
            assert!(m.t49.is_none(), "{content}");
            // nor has it an app kind as a resource or in arkme-json
            assert_eq!(
                crate::message::appmsg::app_message_kind(&content, local_type),
                None
            );
            if local_type != 47 {
                assert_eq!(
                    m.quoted_content.as_deref(),
                    Some("__SVRID__9__"),
                    "{content}"
                );
            }
        }
    }

    #[test]
    fn rows_map_to_messages() {
        let rows = vec![
            MsgRow {
                local_id: 5,
                server_id: Some(9_007_199_254_740_993),
                create_time: Some(1_700_000_000),
                local_type: 1,
                content: "wxid_bob:\nhello".into(),
                sender: Some(String::new()),
                ..MsgRow::default()
            },
            MsgRow {
                local_id: 6,
                create_time: Some(1_700_000_100),
                local_type: 3,
                content: r#"<msg><img md5="aabbccddeeff00112233445566778899"/></msg>"#.into(),
                is_send: true,
                ..MsgRow::default()
            },
        ];
        let m = map_msg_rows(rows, "wxid_bob", "wxid_me");
        assert_eq!(m[0].sender_username.as_deref(), Some("wxid_bob"));
        assert_eq!(m[0].parsed_content, "hello");
        assert_eq!(m[0].server_id_raw, "9007199254740993");
        assert_eq!(
            m[1].image_md5.as_deref(),
            Some("aabbccddeeff00112233445566778899")
        );
        assert_eq!(m[1].is_send, Some(1));
        assert_eq!(m[1].sender_username, None);
    }
}
