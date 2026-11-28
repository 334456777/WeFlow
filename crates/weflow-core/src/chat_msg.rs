//! 聊天命令和命令 API 给出的会话消息（[`ChatMessage`]）。
//!
//! 把类型化的消息行转换成带解析好的显示文字、媒体标识、链接和引用的消息。HTTP API、聊天命令和统计代码都用它；
//! 导出在 `message.rs` 中有自己的模型，两者的文字不同：聊天窗口写一行（`[链接] 标题`、`[语音消息]`），
//! 导出写全文（`[引用 …]`、表情的描述）。
//!
//! 两者用 `message.rs` 的同一批函数读出一行的含义：群消息的发送者前缀、引用的 `<refermsg>` 及其文字、
//! 系统消息的 XML。这里只有聊天窗口写法不同的部分。

use weflow_native::msg_row::MsgRow;
use weflow_native::quote::{reply_text, QUOTE};

use crate::api::js_parse_int;
use crate::message::appmsg::{
    is_non_app_type, kind_by_keywords, kind_of_app_type, row_kept_in, without_quoted,
};
use crate::message::emoji::percent_decode;
use crate::message::system::{extract_readable_system_message_text, parse_voip_message, pat_text};
use crate::message::transfer::is_same_wxid;
use crate::message::xml::{
    decode_text_entities, extract_xml_value, rx, strip_sender_line, strip_sender_prefix, xml_text,
};
use crate::message::{app_message_text_as, TextForm};
use crate::xml::decode_entities;

// ───────────────────────── 文字的辅助函数 ─────────────────────────

/// 去掉 `input` 中的 C0、C1 控制字符和 DEL，保留制表符、换行和回车。
pub fn clean_utf16(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    input
        .chars()
        .filter(|c| !matches!(*c as u32, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F..=0x9F))
        .collect()
}

/// 内容开头 `<发送者>:` 加换行（或 `<br>`）中的发送者，名字至少 4 个字符；没有时为 `None`。
fn extract_sender_from_content(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let normalized = clean_utf16(&decode_entities(content));
    let caps = rx(r"(?i)^\s*([A-Za-z0-9_@-]{4,}):\s*(?:\r?\n|<br\s*/?>)").captures(&normalized)?;
    Some(caps[1].trim().to_string()).filter(|s| !s.is_empty())
}

/// `xml` 中第一个 `<tag>` 的属性 `attr`：单引号、双引号都可以，属性名前面要有空白。
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

/// 解码成给人读的文字的 [`xml_attr`]：实体解码一次（#1199）。
fn attr_text(xml: &str, tag: &str, attr: &str) -> String {
    decode_text_entities(&xml_attr(xml, tag, attr)).into_owned()
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

/// 聊天窗口给没有内容的 `local_type` 消息显示的标签，如 `[图片]`，与有内容时文字开头的标签相同：
/// 存为 `(<type> << 32) | 49` 的 app 消息取表中这个 `<type>` 的标签，表中没有时与存为 49 的一样为 `[链接]`（#1263）；
/// 不认识的类型为 `[消息]`。
pub fn message_type_label(local_type: i64) -> &'static str {
    if local_type & 0xFFFF_FFFF == 49 {
        return row_kept_in(local_type).map_or("[链接]", |row| row.tag);
    }
    match local_type {
        1 => "[文本]",
        3 => "[图片]",
        34 => "[语音消息]",
        42 => "[名片]",
        43 => "[视频]",
        47 => "[表情]",
        48 => "[位置]",
        50 => "[通话]",
        10000 => "[系统消息]",
        _ => "[消息]",
    }
}

/// 只放得下一行时代表一条消息的文字：会话列表中公众号会话的最后一条消息，以及消息推送（#1263）。
/// app 消息不论存为 49 还是 `(<type> << 32) | 49`，写链接的标题或文件名，没有时写聊天窗口的一行；
/// 名片写昵称；图片、语音、视频、表情和位置写标签；其他消息写聊天窗口的一行，没有时写去掉发送者行的内容。
/// 开头的 `[视频号]` 不写。什么都不剩时为 `None`。
pub fn summary_line(m: &ChatMessage) -> Option<String> {
    let non_empty = |s: &str| (!s.trim().is_empty()).then(|| s.to_string());
    let line = match m.local_type {
        42 => m
            .card_nickname
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "[名片]".into()),
        3 | 34 | 43 | 47 | 48 => message_type_label(m.local_type).into(),
        t if t & 0xFFFF_FFFF == 49 => m
            .link_title()
            .and_then(non_empty)
            .or_else(|| m.file_name().and_then(non_empty))
            .or_else(|| non_empty(&m.parsed_content))
            .unwrap_or_else(|| "[消息]".into()),
        _ if !m.parsed_content.trim().is_empty() => m.parsed_content.clone(),
        // 内容本身，去掉写着发送者的一行（链接的 `https:` 不算发送者）
        _ => match rx(r"(?i)^\s*([a-zA-Z0-9_@-]+):(?:\s*(?:\r?\n|<br\s*/?>)\s*|\s*)")
            .find(&m.raw_content)
        {
            Some(found) if !m.raw_content[found.end()..].starts_with("//") => {
                m.raw_content[found.end()..].to_string()
            }
            _ => m.raw_content.clone(),
        },
    };
    let line = clean_utf16(&line);
    let line = line.trim();
    let shown = rx(r"^\s*\[视频号\]\s*").replace(line, "");
    let shown = if shown.trim().is_empty() {
        line
    } else {
        shown.trim()
    };
    non_empty(shown)
}

/// 系统消息的一行：`<sysmsg>` 中的纯文字；没有时为导出从 XML 中读出的提示，即撤回提示、填上名字的模板或标题，
/// 见 [`crate::message::system::clean_system_message`]。读之前解码实体，并去掉前面的发送者和 XML 声明。
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

// ───────────────────────── 类型 49 ─────────────────────────

/// app 消息自己的 `<type>`，读法与导出相同（#1241）。
fn app_xml_type(content: &str) -> String {
    crate::message::xml::MsgXml::new(content)
        .app_type_twice()
        .to_string()
}

/// 位置中的地点，从已解码实体的内容中读。
fn location_label_of(content: &str) -> String {
    location_label_by(
        |a| xml_attr(content, "location", a),
        |t| extract_xml_value(content, t),
    )
}

/// 位置中的地点：依次取 `<location>` 的 `label`、`poiname` 属性（由 `attr` 读），再取 `<label>`、`<poiname>`
/// 元素（由 `text` 读）。
fn location_label_by(attr: impl Fn(&str) -> String, text: impl Fn(&str) -> String) -> String {
    first_non_empty(&[
        attr("label"),
        attr("poiname"),
        text("label"),
        text("poiname"),
    ])
}

/// 聊天窗口给 app 消息显示的一行：导出的文字（[`app_message_text_as`]）的 [`TextForm::Line`] 写法，
/// 所以回复写它本身的文字，聊天记录和笔记写标题（#1263）；`sender` 和 `my_wxid` 用来判断转账的方向（#1185）。
/// `<type>` 和种类在表中都没有对应的行时写标题，没有标题时为 `[消息]`。
fn parse_type49(content: &str, sender: Option<&str>, my_wxid: Option<&str>) -> String {
    let x = crate::message::xml::MsgXml::new(content);
    app_message_text_as(&x, 49, my_wxid, sender, TextForm::Line).unwrap_or_else(|| {
        let title = xml_text(x.once(), "title");
        if title.is_empty() {
            "[消息]".into()
        } else {
            title
        }
    })
}

/// 聊天窗口给任意消息显示的一行（[`ChatMessage::parsed_content`]）。
/// `sender` 是消息的发送者（[`ChatMessage::sender_username`]），`my_wxid` 是账号的 wxid；
/// 导出的文字见 [`crate::message::parse_message_content`]。
fn parsed_content(
    content: &str,
    local_type: i64,
    sender: Option<&str>,
    my_wxid: Option<&str>,
) -> String {
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
    // 拍一拍，写法与导出相同（#1196）
    if local_type == 266287972401 {
        return pat_text(content);
    }
    // 文本按输入的原样保留，不做反转义：其中的 `&lt;` 就是输入的文字，与导出相同（#1208）
    if local_type == 1 {
        return strip_sender_line(&clean_utf16(content), sender);
    }
    // app 消息的读法与导出相同，实体解码一次（#1263）
    let raw = clean_utf16(content);
    let content = clean_utf16(&decode_entities(content));
    let xml_type = extract_xml_value(&content, "type");
    let looks_app = content.contains("<appmsg") || content.contains("&lt;appmsg");
    match local_type {
        3 => "[图片]".into(),
        34 => "[语音消息]".into(),
        42 => "[名片]".into(),
        43 => "[视频]".into(),
        // 导出的标签，不带聊天窗口不读的表情描述（#1263）
        47 => crate::message::emoji::format_emoji_semantic_text(None),
        48 => {
            let label = location_label_of(&content);
            if label.is_empty() {
                "[位置]".into()
            } else {
                format!("[位置] {label}")
            }
        }
        49 => parse_type49(&raw, sender, my_wxid),
        50 => parse_voip_message(&content),
        10000 => system_message_text(&content),
        // 存为 `(<type> << 32) | 49` 的 app 消息与存为 49 的读法相同（#1180）
        l if l & 0xFFFF_FFFF == 49 && looks_app => parse_type49(&raw, sender, my_wxid),
        // 没有 `<appmsg>` 的按存下的类型写标签
        l if row_kept_in(l).is_some() => message_type_label(l).into(),
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
                return parse_type49(&raw, sender, my_wxid);
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

/// app 消息（类型 49）和其他几种消息的 XML 中的信息，取聊天窗口显示的那些。只输出找到的字段。
#[derive(Default, Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Type49Info {
    /// `<appmsg>` 的 `<type>`：`5` 链接，`6` 文件，`19` 聊天记录，`33`、`36` 小程序，`57` 引用，`2000` 转账，
    /// `2001` 红包等。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xml_type: Option<String>,
    /// 消息自己的字段，由 [`ChatMessage`] 输出。
    #[serde(skip)]
    pub quoted_content: Option<String>,
    /// 消息自己的字段，由 [`ChatMessage`] 输出。
    #[serde(skip)]
    pub quoted_sender: Option<String>,
    /// 标题：链接或小程序的标题、文件名、转账的备注（没有备注时为金额）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_title: Option<String>,
    /// 点开时打开的地址。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_url: Option<String>,
    /// 缩略图的地址。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_thumb: Option<String>,
    /// 种类：`link`、`official-link`、`file`、`music`、`location`、`miniapp`、`chat-record`、`transfer`、
    /// `red-packet`、`gift`、`finder`、`quote`、`announcement`、`solitaire` 或 `card`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_kind: Option<String>,
    /// 描述（`<des>`，没有时为 `<description>`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_desc: Option<String>,
    /// 发出它的应用的名称（`<appname>`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_app_name: Option<String>,
    /// 来源账号的名称（`<sourcename>`），如公众号的名称。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_source_name: Option<String>,
    /// 来源账号的用户名（`<sourceusername>`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_source_username: Option<String>,
    /// 缩略图或封面的地址。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_thumb_url: Option<String>,
    /// 歌曲的地址（`<musicurl>`、`<playurl>`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_music_url: Option<String>,
    /// 数据的地址（`<dataurl>`、`<lowurl>`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_data_url: Option<String>,
    /// 消息自己的 XML 中写的地点（`<location>` 的 `label` 或 `poiname`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_location_label: Option<String>,
    /// 视频号账号的名称。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finder_nickname: Option<String>,
    /// 视频号账号的用户名。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finder_username: Option<String>,
    /// 视频号视频的封面。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finder_cover_url: Option<String>,
    /// 视频号账号头像的地址。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finder_avatar: Option<String>,
    /// 视频号视频的时长，单位为秒。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finder_duration: Option<i64>,
    /// 消息自己的字段，由 [`ChatMessage`] 输出。
    #[serde(skip)]
    pub location_lat: Option<f64>,
    /// 消息自己的字段，由 [`ChatMessage`] 输出。
    #[serde(skip)]
    pub location_lng: Option<f64>,
    /// 消息自己的字段，由 [`ChatMessage`] 输出。
    #[serde(skip)]
    pub location_poiname: Option<String>,
    /// 消息自己的字段，由 [`ChatMessage`] 输出。
    #[serde(skip)]
    pub location_label: Option<String>,
    /// 歌曲的专辑封面。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_album_url: Option<String>,
    /// 歌曲的播放地址：依次取 `<musicurl>`、`<playurl>`、`<dataurl>`、`<lowurl>`、`<url>`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_url: Option<String>,
    /// 礼物的图片。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gift_image_url: Option<String>,
    /// 礼物的祝福语。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gift_wish: Option<String>,
    /// 礼物的价格，即消息中写的原文。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gift_price: Option<String>,
    /// 文件名。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// 文件大小，单位为字节。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size: Option<i64>,
    /// 文件的扩展名，不带点。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_ext: Option<String>,
    /// 文件的 MD5，小写。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_md5: Option<String>,
    /// 转账的付款方。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_payer_username: Option<String>,
    /// 转账的收款方。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_receiver_username: Option<String>,
    /// 聊天记录的标题；没有时为「聊天记录」。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_record_title: Option<String>,
}

/// 像 JavaScript 的 `parseFloat` 一样读出开头的数字。
fn parse_js_float(s: &str) -> Option<f64> {
    crate::sns::parse_float(s)
}

/// 对应桌面版的 `parseType49Message`：从 app 消息的 XML 中读出 [`Type49Info`]。
fn parse_type49_message(content: &str) -> Type49Info {
    let mut r = Type49Info::default();
    if content.is_empty() {
        return r;
    }
    let xml_type = app_xml_type(content);
    if xml_type.is_empty() {
        // 没有 `<type>`：按 XML 里的词判定种类，与导出相同（#1263）
        r.app_msg_kind = crate::message::appmsg::app_message_kind(content, 49).map(Into::into);
        if r.app_msg_kind.as_deref() == Some("quote") {
            (r.quoted_content, r.quoted_sender) = quote_of(content);
        }
        return r;
    }
    // 各字段是给人读的文字，实体已解码（#1199）
    let ev = |t: &str| xml_text(content, t);
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
    // 只取歌曲本身的地址：`<songalbumurl>` 是封面，放在 `musicAlbumUrl`（#1170）
    let music_url = first_non_empty(&[ev("musicurl"), ev("playurl")]);
    let data_url = first_non_empty(&[ev("dataurl"), ev("lowurl")]);
    // 消息本身的地点，不是回复所引用的消息的（#1165）
    let own = without_quoted(content);
    let location_label =
        location_label_by(|a| attr_text(&own, "location", a), |t| xml_text(&own, t));
    let finder_username = first_non_empty(&[
        ev("finderusername"),
        ev("finder_username"),
        ev("finderuser"),
    ]);
    let finder_nickname = first_non_empty(&[ev("findernickname"), ev("finder_nickname")]);
    // 与导出用同一张表；类型没有给出种类时，同样看消息自己的 XML 里的词（#1165、#1241）
    let typed = kind_of_app_type(&xml_type).or_else(|| kind_by_keywords(&own));
    let is_finder = typed == Some("finder");
    let is_music = typed == Some("music");
    let is_location = !location_label.is_empty();

    r.xml_type = Some(xml_type.clone());
    r.link_title = opt(title.clone());
    r.link_url = opt(url.clone());
    r.link_thumb = opt(thumb_url.clone());
    r.app_msg_desc = opt(desc);
    r.app_msg_app_name = opt(app_name);
    r.app_msg_source_name = opt(source_name);
    r.app_msg_source_username = opt(source_username);
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
            attr_text(content, "location", "poiname"),
            location_label,
        ]));
        r.location_label = opt(attr_text(content, "location", "label"));
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

    // 与导出判出的是同一个种类（#1263）
    let kind = crate::message::appmsg::app_message_kind(content, 49).unwrap_or("card");
    if kind == "quote" {
        (r.quoted_content, r.quoted_sender) = quote_of(content);
    }
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

/// 回复所引用的内容和发送者 `(content, sender)`：即导出写在回复后面的预览（#1263）。
fn quote_of(content: &str) -> (Option<String>, Option<String>) {
    let x = crate::message::xml::MsgXml::new(content);
    match x.quoted_display() {
        Some(d) => (Some(d.quoted_preview.clone()), d.quoted_sender.clone()),
        None => (None, None),
    }
}

// ───────────────────────── 媒体标识的辅助函数 ─────────────────────────

/// `value`（文件名或路径）中视频文件的标识：16～64 位的小写十六进制，原视频的 `_raw` 后缀保留；没有时为 `None`。
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
    // 先按 `([a-f0-9]{32})(?![a-f0-9])`，再按 16～64 位的通用写法，在成段的十六进制上模拟这两个正则
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

/// 行的 `packed_info` 中视频文件的标识（小写）：长 32 的一段十六进制数字，没有时取长 16～64 的一段。
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

/// 视频消息的 MD5（小写）：`<videomsg>` 的 `md5` 属性，没有时取 `rawmd5` 属性，再没有时取 `<md5>` 元素。
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

/// 语音的时长（整秒）：取第一个出现的 `voicelength`、`length`、`time` 或 `playlength` 属性，
/// 大于 1000 的值按毫秒算；读不出时为 `None`。
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

/// 表情消息的地址和 MD5。
#[derive(Default)]
struct EmojiInfo {
    cdn_url: Option<String>,
    md5: Option<String>,
}

/// 从表情消息的 XML 中读出 `cdnurl`（解码 `&amp;` 和百分号转义）和 `md5`。
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

// ───────────────────────── 消息的模型 ─────────────────────────

/// 聊天窗口显示的一条消息（`chat messages --parsed` 和其他解析过的读取，#399）。
///
/// 其中有从行中解析出的文字，以及媒体、链接、卡片等的标识。消息没有的可选字段不输出；时间都是 Unix 秒。
#[derive(Default, Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    /// 本地 ID，即消息在所在分片的会话表中的行号；与 `createTime` 一起是 `chat message` 的键。
    pub local_id: i64,
    /// 数字形式的服务器 ID；不输出，输出的是十进制字符串 `serverId`。
    #[serde(skip)]
    pub server_id: i64,
    /// 十进制的服务器 ID：可能超过 2^53，所以是字符串。
    #[serde(rename = "serverId")]
    pub server_id_raw: String,
    /// 消息类型（1 文本、3 图片、34 语音、43 视频、47 表情、49 app 消息、10000 系统消息等）。
    pub local_type: i64,
    /// 发送时间。
    pub create_time: i64,
    /// 消息在聊天窗口中的排序位置，通常是毫秒的发送时间加一个计数。
    pub sort_seq: i64,
    /// 账号自己发的为 1，收到的为 0；行中没有时为 null。
    pub is_send: Option<i64>,
    /// 发送者：取行的发送者列，没有时取内容开头的 `<发送者>:`；都没有时为 null。
    pub sender_username: Option<String>,
    /// 聊天窗口显示的文字。
    pub parsed_content: String,
    /// 微信保存的消息原文（常是 XML）。
    pub raw_content: String,
    /// 表情在微信服务器上的地址。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_cdn_url: Option<String>,
    /// 表情的 MD5。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_md5: Option<String>,
    /// 回复所引用的内容，与导出写的预览相同：回复，以及回复某条消息的图片、语音、视频、表情和位置都有这一项。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quoted_content: Option<String>,
    /// 被引用的人的名字。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quoted_sender: Option<String>,
    /// 图片的 MD5，取自它的 XML。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_md5: Option<String>,
    /// 图片的 `.dat` 文件名，取自行的 `packed_info` 或 XML。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_dat_name: Option<String>,
    /// 视频的 MD5（小写），视频文件以它命名。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_md5: Option<String>,
    /// 语音的时长，四舍五入到秒。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_duration_seconds: Option<i64>,
    /// 图片在微信服务器上的 AES 密钥（`<img aeskey>`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aes_key: Option<String>,
    /// 图片的 `encrypver`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryp_ver: Option<i64>,
    /// 图片或表情的缩略图在微信服务器上的地址。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cdn_thumb_url: Option<String>,
    /// 名片分享的用户名。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_username: Option<String>,
    /// 名片上显示的昵称。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_nickname: Option<String>,
    /// 名片头像的地址。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_avatar_url: Option<String>,
    /// 位置的纬度。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_lat: Option<f64>,
    /// 位置的经度。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_lng: Option<f64>,
    /// 位置的地点名称（`poiname`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_poiname: Option<String>,
    /// 位置的标签（`label`），即微信显示的地址。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_label: Option<String>,
    /// 只有 app 消息和其他几种消息有这些字段：放在 `Box` 里，其他消息占的内存更小（#505）。
    #[serde(flatten)]
    pub t49: Option<Box<Type49Info>>,
    /// 消息的键：同一条消息在每台机器上的键都相同。键的内容不透明，只用来比较。
    pub message_key: String,
}

// 一页解析过的消息或搜索结果中每条消息各占一个：断言它的大小，使大小只在有意修改时改变（#505）
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<ChatMessage>() == 584);

impl ChatMessage {
    /// 消息的 [`Type49Info::link_title`]；没有时为 `None`。
    pub fn link_title(&self) -> Option<&str> {
        self.t49.as_ref()?.link_title.as_deref()
    }
    /// 消息的 [`Type49Info::file_name`]；没有时为 `None`。
    pub fn file_name(&self) -> Option<&str> {
        self.t49.as_ref()?.file_name.as_deref()
    }
    /// 消息的 [`Type49Info::file_md5`]；没有时为 `None`。
    pub fn file_md5(&self) -> Option<&str> {
        self.t49.as_ref()?.file_md5.as_deref()
    }
    /// 消息的 [`Type49Info::xml_type`]；没有时为 `None`。
    pub fn xml_type(&self) -> Option<&str> {
        self.t49.as_ref()?.xml_type.as_deref()
    }
    /// 消息的 [`Type49Info::app_msg_kind`]；没有时为 `None`。
    pub fn app_msg_kind(&self) -> Option<&str> {
        self.t49.as_ref()?.app_msg_kind.as_deref()
    }
}

// ───────────────────────── 消息的键 ─────────────────────────

/// 相当于 JavaScript 的 `encodeURIComponent`，编码前先去掉首尾空白。
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

/// 消息保存的位置：数据库的名字（`message_0`，不用路径，#443）和表名。
struct SourceInfo {
    db_name: String,
    table_name: String,
}

/// 消息的键，以数据库的名字（`message_0`）而不是路径限定范围。
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
    // 用数据库的名字，不像桌面版那样用路径：路径会把本机的目录告诉客户端，换一台机器键也会变（#443）
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

// ───────────────────────── isSend 与发送者 ─────────────────────────

/// 发送者就是账号本身（[`is_same_wxid`]）的消息，不论保存的标志是什么都算自己发的；否则按标志，没有标志时算收到的。
/// 没有发送者或账号时标志保持原样。
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

/// [`chat_message`] 转换成消息的各列。
struct RowFields {
    content: String,
    local_type: i64,
    /// 保存的 `is_send`，还没有与发送者核对。
    raw_is_send: Option<i64>,
    /// 发送者列中的用户名（去掉首尾空白，为空时为 `None`）。
    sender: Option<String>,
    create_time: i64,
    local_id: i64,
    server_id: i64,
    server_id_raw: String,
    sort_seq: i64,
}

/// 把一个会话的类型化消息行转换成消息（API 和 CLI 用的模型字段）；行的所有权交给它，行的文字直接移进消息。
pub fn map_msg_rows(rows: Vec<MsgRow>, session_id: &str, my_wxid: &str) -> Vec<ChatMessage> {
    let table_name = weflow_native::msg::table_name_for(session_id);
    rows.into_iter()
        .map(|row| map_msg_row(row, &table_name, my_wxid))
        .collect()
}

/// 转换一行，同 [`map_msg_rows`]，供逐行转换的调用方用；`table_name` 是会话的表名（`table_name_for`）。
pub fn map_msg_row(mut row: MsgRow, table_name: &str, my_wxid: &str) -> ChatMessage {
    let fields = typed_fields(&mut row);
    let src = typed_source(&row, table_name);
    chat_message(fields, row.packed_info.as_deref(), src, my_wxid)
}

/// 类型化的行的各字段，文字从行中移出；没有 `sort_seq` 的行按发送时间排序。
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

/// 行所在分片的名字和会话的表名。
fn typed_source(row: &MsgRow, table_name: &str) -> SourceInfo {
    SourceInfo {
        db_name: weflow_native::msg::shard_name(row.shard),
        table_name: table_name.to_string(),
    }
}

/// 消息自己没有的位置字段，填上 app 消息中写的位置。
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

/// 由一行的各列组成的消息；`packed_info` 是这一行的，除了文字之外只有它里面写着媒体文件的名字。
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
    // 8589934592049 即 `(2000 << 32) | 49`，存成组合类型的转账
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
    } else if local_type == 43 {
        m.video_md5 = packed_info.and_then(video_token_in_packed).or_else(|| {
            video_md5_from_content(&content).and_then(|m| normalize_video_file_token(&m))
        });
    } else if local_type == 34 && !content.is_empty() {
        m.voice_duration_seconds = parse_voice_duration_seconds(&content);
    } else if local_type == 42 && !content.is_empty() {
        m.card_username = opt(xml_attr(&content, "msg", "username"));
        m.card_nickname = opt(attr_text(&content, "msg", "nickname"));
        m.card_avatar_url = opt(first_non_empty(&[
            attr_text(&content, "msg", "bigheadimgurl"),
            attr_text(&content, "msg", "smallheadimgurl"),
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
            attr_text(&content, "location", "label"),
            xml_text(&content, "label"),
        ]));
        m.location_poiname = opt(first_non_empty(&[
            attr_text(&content, "location", "poiname"),
            xml_text(&content, "poiname"),
        ]));
    } else if app_message {
        let t = parse_type49_message(&content);
        m.quoted_content = t.quoted_content.clone();
        m.quoted_sender = t.quoted_sender.clone();
        m.t49 = Some(Box::new(t));
    } else if local_type == 244813135921 || content.contains("<type>57</type>") {
        (m.quoted_content, m.quoted_sender) = quote_of(&content);
    }
    // 回复某条消息的图片、语音、视频、表情或位置：被引用的内容，与导出写的预览相同（#1263）
    if is_non_app_type(local_type) {
        (m.quoted_content, m.quoted_sender) = quote_of(&content);
    }

    // 媒体消息和位置不是 app 消息：其中的 `<appmsg>` 是它的 `<refermsg>` 所引用的消息
    let has_appmsg = !is_non_app_type(local_type)
        && (content.contains("<appmsg") || content.contains("&lt;appmsg"));
    if has_appmsg && app_message {
        // `t49` 已经是这段内容的解析结果：只有其中写的位置还没有填到消息上
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
    m.parsed_content = parsed_content(
        &content,
        local_type,
        m.sender_username.as_deref(),
        Some(my_wxid),
    );
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
    use crate::message::appmsg::app_type;

    /// 不带账号的 wxid 读出的聊天窗口文字。
    fn parsed_content(content: &str, local_type: i64, sender: Option<&str>) -> String {
        super::parsed_content(content, local_type, sender, None)
    }

    #[test]
    fn the_sender_is_read_from_the_prefix_of_a_group_message() {
        assert_eq!(
            extract_sender_from_content("wxid_abcd:\nhello").as_deref(),
            Some("wxid_abcd")
        );
        assert_eq!(extract_sender_from_content("wxid_abcd:hello"), None);
    }

    /// 微信按输入的原样保存文本，不做转义：其中像实体的文字就是输入的内容，聊天窗口显示的与导出写的相同（#1208）。
    #[test]
    fn a_text_message_is_shown_as_it_was_typed() {
        for (content, sender, text) in [
            ("a &amp;lt;b&amp;gt;", None, "a &amp;lt;b&amp;gt;"),
            ("a &lt;b&gt;", None, "a &lt;b&gt;"),
            ("1 < 2 && x > 0", None, "1 < 2 && x > 0"),
            ("<b>粗</b> & Tom's", None, "<b>粗</b> & Tom's"),
            ("wxid_bob:\nR&D &amp; QA", Some("wxid_bob"), "R&D &amp; QA"),
        ] {
            assert_eq!(parsed_content(content, 1, sender), text, "{content:?}");
            assert_eq!(
                crate::message::parse_message_content(content, 1, None, sender, None).as_deref(),
                Some(text),
                "{content:?}"
            );
        }
    }

    /// 转义一次的 `xml`，与微信放在回复的 `<refermsg><content>` 里的被引用消息的 XML 一样。
    fn escaped(xml: &str) -> String {
        xml.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// 对 `quoted` 说「好看」的回复，`<type>` 为 57。
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

    /// app 消息的种类在哪里读都是它自己的 `<type>` 给出的：XML 里的词不会让链接变成音乐或红包，
    /// 回复所引用的消息里的词也不会让回复变成那一种（#1165）。
    #[test]
    fn an_app_message_is_of_the_kind_its_type_says() {
        let link = |extra: &str| {
            format!(
                "<msg><appmsg><title>一篇文章</title><type>5</type><url>https://a</url>{extra}</appmsg></msg>"
            )
        };
        let hongbao_title = "<msg><appmsg><title>抢 HongBao 的攻略</title><type>5</type>\
            <url>https://a</url></appmsg></msg>";
        // 链接：聊天窗口的文字、导出的文字和两边的种类都一致
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
        // 回复视频号、音乐、带 `<dataurl>` 的链接、红包和位置的消息，两种 `local_type` 都试
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
        // 被引用的消息本身仍是原来的种类
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

    /// 0 到 2100 的每个 `<type>`，不论存为 `local_type` 49 还是 `(<type> << 32) | 49`，在哪里读都只有一个种类、
    /// 一个类型名和一个 ChatLab 类型。表中有的取那一行，聊天窗口的文字和导出的文字都以那一行的标签开头（#1180）；
    /// 表中没有的算 `card`（115 算 `gift`），不论怎样存读法都相同（#1263）。
    #[test]
    fn every_app_type_reads_the_same_everywhere() {
        let mut known = 0;
        for n in 0..=2100_i64 {
            let ty = n.to_string();
            let ty = ty.as_str();
            let xml = format!(
                "<msg><appmsg><title>T</title><type>{ty}</type><url>https://a</url></appmsg></msg>"
            );
            let x = crate::message::xml::MsgXml::new(&xml);
            let kept_as = (n << 32) | 49;
            let Some(row) = app_type(ty) else {
                let [as_49, as_kept] = [49, kept_as].map(|local_type| {
                    (
                        crate::message::appmsg::app_message_kind(&xml, local_type),
                        parse_type49_message(&xml).app_msg_kind,
                        crate::message::kind::message_type_name_in(local_type, &x),
                        crate::message::kind::convert_message_type_in(local_type, &x),
                        parsed_content(&xml, local_type, None),
                        crate::message::parse_message_content(&xml, local_type, None, None, None),
                    )
                });
                let kind = if n == 115 { "gift" } else { "card" };
                assert_eq!(as_49.0, Some(kind), "<type> {ty}");
                assert_eq!(as_49.1.as_deref(), Some(kind), "<type> {ty}");
                assert_eq!(as_49, as_kept, "<type> {ty}");
                continue;
            };
            known += 1;
            for local_type in [49, kept_as] {
                let at = format!("<type> {ty} as {local_type}");
                let chat = parsed_content(&xml, local_type, None);
                let export =
                    crate::message::parse_message_content(&xml, local_type, None, None, None)
                        .unwrap();
                match ty {
                    // 转发的聊天记录的条目只有导出写
                    "19" => {}
                    // 没有金额的转账只写方向
                    "2000" => {
                        assert_eq!((chat.as_str(), export.as_str()), (row.tag, row.tag), "{at}");
                    }
                    "24" => {
                        assert!(
                            chat.starts_with(row.tag) && export.starts_with(row.tag),
                            "{at}: {chat} / {export}"
                        );
                    }
                    "57" => assert_eq!((chat.as_str(), export.as_str()), ("T", "T"), "{at}"),
                    // 存为 `(62 << 32) | 49` 的拍一拍按系统消息只写标题（#1196）
                    "62" if local_type == kept_as => {
                        assert_eq!((chat.as_str(), export.as_str()), ("T", "T"), "{at}");
                    }
                    "8" | "87" => {
                        assert_eq!((chat.as_str(), export.as_str()), (row.tag, row.tag), "{at}");
                    }
                    _ => {
                        let tagged = format!("{} T", row.tag);
                        assert_eq!(
                            (chat.as_str(), export.as_str()),
                            (tagged.as_str(), tagged.as_str()),
                            "{at}"
                        );
                    }
                }
                let arkme = crate::message::appmsg::app_message_kind(&xml, local_type);
                assert_eq!(arkme, Some(row.kind.unwrap_or("card")), "{at}");
                assert_eq!(
                    parse_type49_message(&xml).app_msg_kind.as_deref(),
                    arkme,
                    "{at}"
                );
                assert_eq!(
                    crate::message::kind::message_type_name_in(local_type, &x),
                    row.name,
                    "{at}"
                );
                assert_eq!(
                    crate::message::kind::convert_message_type_in(local_type, &x),
                    row.chatlab,
                    "{at}"
                );
            }
        }
        // 表中的行取自表本身
        assert!(known > 10, "{known} types");
    }

    /// 性质测试用的 app 消息 XML 元素：`<type>` 没有给出种类时，能凭其中的词判出种类的（视频号、红包、音乐，
    /// #1241），以及判不出的，其中有空的元素。
    const WORDS: &[&str] = &[
        "<finderFeed><objectId>1</objectId></finderFeed>",
        "<finderUsername>v2_x@finder</finderUsername>",
        "<finderUsername></finderUsername>",
        "<finderObjectID>12</finderObjectID>",
        "<finderGuarantee></finderGuarantee>",
        "<nativeurl>wxpay://c2cbizmessagehandler/hongbao/receivehongbao?a=1</nativeurl>",
        "<musicurl>https://m</musicurl>",
        "<musicurl> </musicurl>",
        "<playurl>https://p</playurl>",
        "<dataurl>https://d</dataurl>",
        "<dataurl></dataurl>",
        "<des>d</des>",
        "<url>https://a</url>",
    ];

    /// 性质测试生成的 app 消息：它自己的 `<type>`（表中有的、表中没有的，或者没有），[`WORDS`] 中的几个元素，
    /// 以及回复时所引用的消息的 `<type>` 和元素。
    type AppParts = (Option<i64>, Vec<&'static str>, Option<Quoted>);

    /// [`AppParts`] 所引用的消息：被引用消息的 `<type>` 和元素，
    /// 以及它的 XML 在 `<refermsg>` 的 `<content>` 中是否像微信那样转义。
    type Quoted = (i64, Vec<&'static str>, bool);

    /// 生成 [`AppParts`] 的策略。
    fn app_message() -> impl proptest::strategy::Strategy<Value = AppParts> {
        use proptest::prelude::*;
        let known: Vec<i64> = (1..=2100)
            .filter(|n: &i64| app_type(&n.to_string()).is_some())
            .collect();
        let ty = prop_oneof![
            3 => proptest::sample::select(known.clone()).prop_map(Some),
            1 => (1..5000_i64).prop_map(Some),
            1 => Just(None),
        ];
        let words = || proptest::sample::subsequence(WORDS, 0..=4);
        let quoted =
            proptest::option::of((proptest::sample::select(known), words(), any::<bool>()));
        (ty, words(), quoted)
    }

    /// [`AppParts`] 对应的 XML。
    fn app_xml(ty: Option<i64>, words: &[&str], quoted: Option<&Quoted>) -> String {
        let ty = ty.map_or(String::new(), |n| format!("<type>{n}</type>"));
        let refer = quoted.map_or(String::new(), |(n, words, escaped)| {
            let content = format!(
                "<msg><appmsg><title>Q</title><type>{n}</type>{}</appmsg></msg>",
                words.concat()
            );
            let content = if *escaped {
                content.replace('<', "&lt;").replace('>', "&gt;")
            } else {
                content
            };
            format!(
                "<refermsg><type>49</type><svrid>1</svrid><content>{content}</content></refermsg>"
            )
        });
        format!(
            "<msg><appmsg><title>T</title>{ty}{}{refer}</appmsg></msg>",
            words.concat()
        )
    }

    /// app 消息文字开头的 `[…]`；没有时为 `None`。
    fn tag_of(text: &str) -> Option<&str> {
        let end = text.find(']')?;
        (text.starts_with('[') && end <= 30).then(|| &text[..=end])
    }

    proptest::proptest! {
        #![proptest_config(crate::property_cases(512))]

        /// 任意 app 消息不论怎样存、在哪里读，都只有一个种类、类型名和标签：种类取它自己的 `<type>` 给出的，
        /// 没有时取它自己的 XML 里的词给出的，从不取被引用消息的（#1165、#1241、#1263）。
        #[test]
        fn any_app_message_has_one_kind_wherever_it_is_read(
            (ty, words, quoted) in app_message(),
        ) {
            use proptest::prop_assert_eq;
            let xml = app_xml(ty, &words, quoted.as_ref());
            let x = crate::message::xml::MsgXml::new(&xml);
            let kind = crate::message::appmsg::app_message_kind(&xml, 49);
            let chat_kind = parse_type49_message(&xml).app_msg_kind;
            prop_assert_eq!(chat_kind.as_deref(), kind, "{}", xml);
            // 先看类型，类型没有给出种类时才看消息自己的 XML 里的词
            let replies = quoted.is_some();
            let want = match ty.and_then(|n| kind_of_app_type(&n.to_string())) {
                Some("link") if replies => "quote",
                Some(by_type) => by_type,
                None => kind_by_keywords(&words.concat())
                    .unwrap_or(if replies { "quote" } else { "card" }),
            };
            let want = match want {
                "finder" | "red-packet" => want,
                _ if ty == Some(115) => "gift",
                _ => want,
            };
            prop_assert_eq!(kind, Some(want), "{}", xml);
            // 被引用的消息不影响它的种类
            if let Some((n, _, escaped)) = &quoted {
                let bare = app_xml(ty, &words, Some(&(*n, Vec::new(), *escaped)));
                prop_assert_eq!(crate::message::appmsg::app_message_kind(&bare, 49), kind, "{}", xml);
            }
            // 聊天窗口的一行和导出的文字以同样的标签开头；转发的聊天记录除外，导出在它的标签后面还写其中的条目
            let chat = parsed_content(&xml, 49, None);
            let export = crate::message::parse_message_content(&xml, 49, None, None, None)
                .unwrap_or_default();
            if kind != Some("chat-record") {
                prop_assert_eq!(tag_of(&chat), tag_of(&export), "{}: {} / {}", xml, chat, export);
            }
            // 不论怎样存
            if let Some(n) = ty {
                let kept = (n << 32) | 49;
                prop_assert_eq!(crate::message::appmsg::app_message_kind(&xml, kept), kind, "{}", xml);
                prop_assert_eq!(
                    crate::message::kind::message_type_name_in(kept, &x),
                    crate::message::kind::message_type_name_in(49, &x),
                    "{}", xml
                );
                prop_assert_eq!(
                    crate::message::kind::convert_message_type_in(kept, &x),
                    crate::message::kind::convert_message_type_in(49, &x),
                    "{}", xml
                );
            }
        }
    }

    /// 转账在聊天窗口和导出中的文字相同，不论怎样存、从哪一方看：写方向、金额和备注，不写标题（#1185）。
    #[test]
    fn a_transfer_reads_the_same_in_the_chat_and_the_exports() {
        let transfer = |subtype: &str, memo: &str| {
            format!(
                "<msg><appmsg><title>转账</title><type>2000</type><wcpayinfo><paysubtype>{subtype}</paysubtype>\
                 <feedesc>￥12.34</feedesc><pay_memo>{memo}</pay_memo>\
                 <payer_username>wxid_payer</payer_username><receiver_username>wxid_receiver</receiver_username>\
                 </wcpayinfo></appmsg></msg>"
            )
        };
        let cases = [
            // `paysubtype` 决定方向
            ("1", "", None, None, "[转账] ￥12.34"),
            ("3", "饭钱", None, None, "[转账收款] ￥12.34 饭钱"),
            // 没有时按发送者，再按本账号
            ("", "", Some("wxid_payer"), None, "[转账收款] ￥12.34"),
            ("", "", Some("wxid_receiver"), None, "[转账] ￥12.34"),
            ("", "", None, Some("wxid_payer"), "[转账收款] ￥12.34"),
            ("", "", None, Some("wxid_receiver"), "[转账] ￥12.34"),
        ];
        for (subtype, memo, sender, me, text) in cases {
            let xml = transfer(subtype, memo);
            for local_type in [49, (2000 << 32) | 49] {
                let at = format!("{subtype:?} {sender:?} {me:?} as {local_type}");
                assert_eq!(
                    super::parsed_content(&xml, local_type, sender, me),
                    text,
                    "{at}"
                );
                assert_eq!(
                    crate::message::parse_message_content(&xml, local_type, me, sender, None)
                        .as_deref(),
                    Some(text),
                    "{at}"
                );
            }
        }
    }

    /// 不论回复引用了什么、app 消息是什么类型，它的种类都是它的 `<type>` 给出的，聊天窗口与导出相同（#1165）：
    /// 表中的每个类型对每种被引用的消息都试一遍。
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
            "51", "2001", "3", "76", "33", "36", "6", "19", "2000", "87", "57", "53", "4", "5",
            "49",
        ] {
            let by_type = kind_of_app_type(ty).unwrap();
            for q in quoted {
                let xml = format!(
                    "<msg><appmsg><title>T</title><type>{ty}</type><url>https://a</url><refermsg><type>49</type>\
                     <svrid>7</svrid><content>{}</content></refermsg></appmsg></msg>",
                    escaped(q)
                );
                // 带引用的链接算引用，聊天窗口也一样（#1263）
                let kind = if by_type == "link" { "quote" } else { by_type };
                assert_eq!(
                    crate::message::appmsg::app_message_kind(&xml, 49),
                    Some(kind),
                    "<type> {ty} quoting {q}"
                );
                let window = parse_type49_message(&xml).app_msg_kind.unwrap();
                assert_eq!(window, kind, "<type> {ty} quoting {q}");
            }
        }
    }

    /// 礼物、位置、公众号文章和没有地址的链接，在聊天窗口和导出中是同一个种类（#1263）。
    #[test]
    fn the_chat_window_and_the_exports_name_each_app_message_alike() {
        for (xml, kind) in [
            (
                "<msg><appmsg><title>T</title><type>115</type><wishmessage>w</wishmessage></appmsg></msg>",
                "gift",
            ),
            (
                "<msg><appmsg><title>T</title><type>5</type><url>https://a</url>\
                 <sourceusername>gh_x</sourceusername></appmsg></msg>",
                "official-link",
            ),
            (
                "<msg><appmsg><title>T</title><type>5</type><url>https://a</url></appmsg></msg>",
                "link",
            ),
            (
                "<msg><appmsg><title>T</title><type>5</type></appmsg></msg>",
                "link",
            ),
            (
                "<msg><appmsg><title>T</title><type>17</type><location label=\"西湖\" x=\"1\" y=\"2\"/>\
                 </appmsg></msg>",
                "location",
            ),
        ] {
            assert_eq!(
                crate::message::appmsg::app_message_kind(xml, 49),
                Some(kind),
                "{xml}"
            );
            assert_eq!(
                parse_type49_message(xml).app_msg_kind.as_deref(),
                Some(kind),
                "{xml}"
            );
        }
    }

    /// 没有内容的消息的标签与有内容的相同，存为 `(<type> << 32) | 49` 的 app 消息按表中这个 `<type>` 的标签：
    /// 这样存的表情或笔记不会被当成文件（#1263）。
    #[test]
    fn a_label_is_the_tag_of_the_type_kept() {
        let kept = |ty: i64| (ty << 32) | 49;
        for (local_type, label) in [
            (3, "[图片]"),
            (34, "[语音消息]"),
            (47, "[表情]"),
            (49, "[链接]"),
            (kept(6), "[文件]"),
            (kept(8), "[表情]"),
            (kept(24), "[笔记]"),
            (kept(19), "[聊天记录]"),
            (kept(57), "[引用消息]"),
            (kept(62), "[拍一拍]"),
            (kept(2000), "[转账]"),
            (kept(2001), "[红包]"),
            (kept(999), "[链接]"),
        ] {
            assert_eq!(message_type_label(local_type), label, "{local_type}");
            assert_eq!(parsed_content("", local_type, None), label, "{local_type}");
        }
    }

    /// 会话列表和消息推送对 app 消息的写法相同，不论存为 49 还是 `(<type> << 32) | 49`（#1263）。
    #[test]
    fn a_summary_is_the_same_however_the_message_is_kept() {
        let transfer = "<msg><appmsg><title>微信转账</title><type>2000</type><wcpayinfo>\
                        <feedesc>￥12.34</feedesc><paysubtype>1</paysubtype></wcpayinfo></appmsg></msg>";
        let mut lines = [49, (2000 << 32) | 49].map(|local_type| {
            let row = MsgRow {
                local_type,
                content: transfer.into(),
                ..MsgRow::default()
            };
            summary_line(&map_msg_rows(vec![row], "wxid_bob", "wxid_me")[0])
        });
        assert_eq!(lines[0], lines[1]);
        assert!(lines[0].take().is_some_and(|l| !l.is_empty()));
        let voice = ChatMessage {
            local_type: 34,
            ..ChatMessage::default()
        };
        assert_eq!(summary_line(&voice).as_deref(), Some("[语音消息]"));
    }

    /// 回复所引用的内容在聊天窗口和导出中相同（#1263）：被引用的 app 消息按它自己的 XML 写，
    /// 回复写它的文字，链接写标题，文件写文件名。
    #[test]
    fn a_quote_is_previewed_alike_everywhere() {
        let reply = |quoted_type: &str, quoted: &str| {
            format!(
                "<msg><appmsg><title>好</title><type>57</type><refermsg><type>{quoted_type}</type>\
                 <displayname>Bob</displayname><content>{}</content><svrid>1</svrid></refermsg></appmsg></msg>",
                escaped(quoted)
            )
        };
        for (quoted_type, quoted, preview) in [
            ("1", "早", "早"),
            ("3", "<msg><img/></msg>", "[图片]"),
            ("34", "<msg><voicemsg/></msg>", "[语音消息]"),
            ("47", "<msg><emoji/></msg>", "[表情]"),
            (
                "49",
                "<msg><appmsg><title>原来那句</title><type>57</type></appmsg></msg>",
                "原来那句",
            ),
            (
                "49",
                "<msg><appmsg><title>一篇文章</title><type>5</type></appmsg></msg>",
                "一篇文章",
            ),
            (
                "49",
                "<msg><appmsg><title>a.pdf</title><type>6</type></appmsg></msg>",
                "[文件] a.pdf",
            ),
        ] {
            let xml = reply(quoted_type, quoted);
            let window = parse_type49_message(&xml);
            assert_eq!(window.quoted_content.as_deref(), Some(preview), "{quoted}");
            assert_eq!(window.quoted_sender.as_deref(), Some("Bob"), "{quoted}");
            assert_eq!(
                crate::message::parse_message_content(&xml, 49, None, None, None).as_deref(),
                Some(format!("好[引用 Bob：{preview}]").as_str()),
                "{quoted}"
            );
        }
    }

    /// 聊天窗口按同一张表写 app 消息的文字，与导出相同（#1263），但只写一行：回复写它本身的文字，
    /// 聊天记录和笔记写标题。
    #[test]
    fn the_chat_window_words_app_messages_as_the_exports_do() {
        let export = |xml: &str| crate::message::parse_message_content(xml, 49, None, None, None);
        for ty in [
            "3", "76", "4", "5", "49", "6", "8", "33", "36", "51", "53", "62", "87", "2001", "115",
            "999",
        ] {
            let xml = format!(
                "<msg><appmsg><title>T</title><type>{ty}</type><url>https://a</url></appmsg></msg>"
            );
            assert_eq!(
                Some(super::parsed_content(&xml, 49, None, None)),
                export(&xml),
                "<type> {ty}"
            );
        }
        // 表中没有的类型按 XML 判出的种类写
        let finder = "<msg><appmsg><title>直播</title><type>63</type>\
                      <finderUsername>v2_x</finderUsername></appmsg></msg>";
        assert_eq!(
            super::parsed_content(finder, 49, None, None),
            "[视频号] 直播"
        );
        assert_eq!(export(finder).as_deref(), Some("[视频号] 直播"));
        // 聊天窗口写一行，导出写全文
        let record = "<msg><appmsg><title>群聊的聊天记录</title><type>19</type><recorditem>\
                      &lt;recordinfo&gt;&lt;datalist&gt;&lt;dataitem datatype=\"1\"&gt;&lt;sourcename&gt;A\
                      &lt;/sourcename&gt;&lt;datadesc&gt;早&lt;/datadesc&gt;&lt;/dataitem&gt;&lt;/datalist&gt;\
                      &lt;/recordinfo&gt;</recorditem></appmsg></msg>";
        assert_eq!(
            super::parsed_content(record, 49, None, None),
            "[聊天记录] 群聊的聊天记录"
        );
        assert_ne!(export(record).as_deref(), Some("[聊天记录] 群聊的聊天记录"));
        let note = "<msg><appmsg><title>清单</title><type>24</type><des>买菜</des></appmsg></msg>";
        assert_eq!(super::parsed_content(note, 49, None, None), "[笔记] 清单");
        assert_eq!(
            super::parsed_content("<msg><emoji md5=\"a\"/></msg>", 47, None, None),
            "[表情]"
        );
    }

    /// 对微信实际保存的几种写法，聊天窗口和导出判出的种类相同（#1241）：写在 CDATA 里的 `<type>`，
    /// 写在被引用文件的整段 XML 或聊天记录的条目之后的 `<type>`，表情或其他类型的模板里空的视频号元素，
    /// 表中没有的类型，以及回复某条消息的表情。
    #[test]
    fn an_app_message_has_one_kind_however_its_xml_is_written() {
        let finder_template = "<finderGuarantee><scene><![CDATA[0]]></scene></finderGuarantee>\
            <finderUsername><![CDATA[]]></finderUsername><finderObjectID><![CDATA[]]></finderObjectID>\
            <findernamecard><username /></findernamecard>";
        let quoted_file = escaped("<msg><appmsg><title>a.pdf</title><type>6</type></appmsg></msg>");
        let cases = [
            (
                "<msg><appmsg><title>恭喜发财</title><type><![CDATA[2001]]></type><url>https://a</url></appmsg></msg>"
                    .to_string(),
                "2001",
                "red-packet",
            ),
            (
                format!(
                    "<msg><appmsg><title>好</title><refermsg><type>49</type><svrid>7</svrid>\
                     <content>{quoted_file}</content></refermsg><type>57</type></appmsg></msg>"
                ),
                "57",
                "quote",
            ),
            (
                format!(
                    "<msg><appmsg><title>记录</title><recorditem>{}</recorditem><type>19</type></appmsg></msg>",
                    escaped("<recordinfo><datalist><dataitem><refermsgitem><type>1</type></refermsgitem>\
                             </dataitem></datalist></recordinfo>")
                ),
                "19",
                "chat-record",
            ),
            (
                format!("<msg><appmsg><title>T</title><type>8</type>{finder_template}</appmsg></msg>"),
                "8",
                "card",
            ),
            (
                format!(
                    "<msg><appmsg><title>T</title><type>92</type><url>https://a</url>{finder_template}</appmsg></msg>"
                ),
                "92",
                "card",
            ),
            (
                "<msg><appmsg><title>T</title><type>44</type><url>https://a</url></appmsg></msg>".to_string(),
                "44",
                "card",
            ),
            (
                "<msg><appmsg><title>直播</title><type>63</type><url>https://a</url>\
                 <finderUsername>v2_x@finder</finderUsername><finderObjectID>1</finderObjectID></appmsg></msg>"
                    .to_string(),
                "63",
                "finder",
            ),
            (
                "<msg><appmsg><title>T</title><type>8</type><refermsg><type>1</type><svrid>7</svrid>\
                 <content>hi</content></refermsg></appmsg></msg>"
                    .to_string(),
                "8",
                "quote",
            ),
        ];
        for (xml, ty, kind) in cases {
            let window = parse_type49_message(&xml);
            assert_eq!(window.xml_type.as_deref(), Some(ty), "{xml}");
            assert_eq!(window.app_msg_kind.as_deref(), Some(kind), "{xml}");
            assert_eq!(
                crate::message::appmsg::app_message_kind(&xml, 49),
                Some(kind),
                "{xml}"
            );
        }
    }

    /// `<type>` 没有给出种类的消息只按它自己的 XML 里的词判定：看不为空的 `<musicurl>`，不看 `<dataurl>`；
    /// 看红包的地址，不看文字里的 `hongbao`（#1165）。
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
        // 被引用消息里的词不算
        for quoted in [FINDER, MUSIC, RED_PACKET] {
            let own = format!("<appmsg><title>好看</title><refermsg><content>{quoted}</content></refermsg></appmsg>");
            assert_eq!(kind_by_keywords(&without_quoted(&own)), None, "{quoted}");
        }
        // 聊天窗口的文字也不受影响：回复一段视频号视频，两者都没有 `<type>`
        let video = "<msg><appmsg><title>一段视频</title><finderFeed><finderusername>v2_x@finder</finderusername>\
            </finderFeed></appmsg></msg>";
        let own = format!(
            "<msg><appmsg><title>好看</title><refermsg><content>{}</content></refermsg></appmsg></msg>",
            escaped(video)
        );
        assert_eq!(parsed_content(&own, 49, None), "好看");
    }

    /// 以 `<type>` 3 或 76 分享的歌曲，按微信保存的样子：有 `<musicShareItem>` 和 `<dataurl>`，没有 `<musicurl>`
    /// 和 `<songname>`。两者在聊天窗口、导出和各自的文字中都是音乐（#1167）。
    #[test]
    fn a_song_is_music_by_either_type() {
        for ty in ["3", "76"] {
            let xml = format!(
                "<msg><appmsg appid=\"wx1\" sdkver=\"0\"><title>一首歌</title><des>歌手</des><type>{ty}</type>\
                 <url>https://m/page</url><dataurl>https://m/a.mp3</dataurl><songalbumurl>https://m/c.jpg</songalbumurl>\
                 <musicShareItem><mid>1</mid></musicShareItem></appmsg></msg>"
            );
            let kept_as: i64 = ty.parse::<i64>().unwrap() << 32 | 49;
            for local_type in [49, kept_as] {
                assert_eq!(
                    parsed_content(&xml, local_type, None),
                    "[音乐] 一首歌",
                    "{ty} as {local_type}"
                );
                assert_eq!(
                    crate::message::parse_message_content(&xml, local_type, None, None, None)
                        .as_deref(),
                    Some("[音乐] 一首歌"),
                    "{ty} as {local_type}"
                );
                assert_eq!(
                    crate::message::appmsg::app_message_kind(&xml, local_type),
                    Some("music"),
                    "{ty} as {local_type}"
                );
                assert_eq!(
                    crate::message::kind::message_type_name_in(
                        local_type,
                        &crate::message::xml::MsgXml::new(&xml)
                    ),
                    "音乐消息",
                    "{ty} as {local_type}"
                );
            }
            let window = parse_type49_message(&xml);
            assert_eq!(window.app_msg_kind.as_deref(), Some("music"), "{ty}");
            assert_eq!(
                window.app_msg_data_url.as_deref(),
                Some("https://m/a.mp3"),
                "{ty}"
            );
            assert_eq!(
                window.music_album_url.as_deref(),
                Some("https://m/c.jpg"),
                "{ty}"
            );
            // 歌曲的地址从不是封面（#1170）：它没有自己的播放地址，所以聊天窗口取音频文件，导出没有这一项
            assert_eq!(window.app_msg_music_url, None, "{ty}");
            assert_eq!(window.music_url.as_deref(), Some("https://m/a.mp3"), "{ty}");
            let meta = crate::message::appmsg::extract_arkme_app_message_meta_in(
                &crate::message::xml::MsgXml::new(&xml),
                49,
            )
            .unwrap();
            assert!(!meta.contains_key("musicUrl"), "{ty}: {meta:?}");
            assert_eq!(meta["musicDataUrl"], "https://m/a.mp3", "{ty}");
            assert_eq!(meta["musicAlbumUrl"], "https://m/c.jpg", "{ty}");
            // 有自己的播放地址的，各处都链接到它
            let playable = xml.replace("</url>", "</url><musicurl>https://m/play</musicurl>");
            let window = parse_type49_message(&playable);
            assert_eq!(window.app_msg_music_url.as_deref(), Some("https://m/play"));
            assert_eq!(window.music_url.as_deref(), Some("https://m/play"));
            let meta = crate::message::appmsg::extract_arkme_app_message_meta_in(
                &crate::message::xml::MsgXml::new(&playable),
                49,
            )
            .unwrap();
            assert_eq!(meta["musicUrl"], "https://m/play", "{ty}");
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
        // 引用标题的其他写法在 `weflow_native::quote` 中测试
        for (title, said) in [
            ("好的 &amp; 收到", "好的 & 收到"),
            ("<![CDATA[好的]]>", "好的"),
            // 回复里写的标签是文字，不是标题的结束
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
        // 只有 `名字:` 加换行，或者像 ID 的名字，才是发送者：时间、比分或标签是正文本身
        for text in ["4:1", "1:3-1:6", "10:1 abc", "PS: see you", "http://x"] {
            assert_eq!(parsed_content(text, 1, None), text);
        }
        // 只有写着这条消息发送者的一行才是发送者行（#656）
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

    /// 回复所引用的文字在 `<refermsg>` 中转义了两次，换行写成又转义了一层的 `&#x0A;` 或 `&#10;`：
    /// 聊天窗口、导出的文字和 Arkme 都显示解码后的文字，换行显示为空格，用户输入的 `&amp;` 不再多解码（#1212）。
    #[test]
    fn a_quoted_text_with_line_breaks_reads_the_same_everywhere() {
        use crate::message::appmsg::extract_arkme_app_message_meta_in;
        use crate::message::xml::MsgXml;

        // 微信保存的引用：被引用消息自己的 msgsource 转义后放在它的文字旁边
        let reply = |quoted: &str| {
            format!(
                "<msg><appmsg><title>好的</title><type>57</type><refermsg><type>1</type><svrid>7</svrid>\
                 <displayname>Bob</displayname><content>{quoted}</content>\
                 <msgsource>&lt;msgsource&gt;&lt;/msgsource&gt;</msgsource></refermsg></appmsg></msg>"
            )
        };
        for (quoted, text) in [
            ("第一行&amp;#x0A;第二行", "第一行 第二行"),
            ("a&amp;#10;b", "a b"),
            ("R&amp;amp;amp;D", "R&amp;D"),
            // 与实际数据中换行和空格还多转义了一层的引用一样
            (
                "第一行&amp;amp;#x0A;第二行&amp;amp;#x20;三",
                "第一行 第二行 三",
            ),
            // 多数引用的换行保持原样
            ("第一行\n第二行", "第一行 第二行"),
        ] {
            let xml = reply(quoted);
            let row = MsgRow {
                local_type: QUOTE,
                content: xml.clone(),
                ..MsgRow::default()
            };
            let m = map_msg_rows(vec![row], "wxid_bob", "wxid_me").remove(0);
            assert_eq!(m.quoted_content.as_deref(), Some(text), "{quoted}");
            assert_eq!(
                crate::message::parse_message_content(&xml, QUOTE, None, None, None).as_deref(),
                Some(format!("好的[引用 Bob：{text}]").as_str()),
                "{quoted}"
            );
            let meta =
                extract_arkme_app_message_meta_in(&MsgXml::new(&xml), QUOTE).unwrap_or_default();
            assert_eq!(
                meta.get("quotedContent").and_then(|v| v.as_str()),
                Some(text),
                "{quoted}"
            );
        }
    }

    /// 链接、文件、名片和位置的字段在聊天窗口、导出的文字和 Arkme 中都是给人读的文字：
    /// 实体解码一次（`&amp;amp;` 得到 `&amp;`），CDATA 里的文字原样保留，
    /// 不论 app 消息存为 `local_type` 49 还是 `(<type> << 32) | 49`（#1199）。
    #[test]
    fn the_fields_of_a_message_have_their_entities_decoded_once() {
        use crate::message::appmsg::extract_arkme_app_message_meta_in;
        use crate::message::xml::MsgXml;

        let read = |local_type, content: &str| {
            let row = MsgRow {
                local_type,
                content: content.into(),
                ..MsgRow::default()
            };
            map_msg_rows(vec![row], "wxid_bob", "wxid_me").remove(0)
        };
        let export = |local_type, content: &str| {
            crate::message::parse_message_content(content, local_type, None, None, None)
                .unwrap_or_default()
        };
        let arkme = |local_type, content: &str| {
            let meta = extract_arkme_app_message_meta_in(&MsgXml::new(content), local_type)
                .unwrap_or_default();
            move |key: &str| meta.get(key).and_then(|v| v.as_str()).map(str::to_owned)
        };

        let link =
            "<msg><appmsg><title>A &amp; B</title><des>&lt;C &amp;amp; D</des><type>5</type>\
                    <url>https://mp.example/s?__biz=X&amp;mid=1</url>\
                    <thumburl>https://t.example/a?x=1&amp;y=2</thumburl></appmsg></msg>";
        let cdata = "<msg><appmsg><title><![CDATA[a &amp; b]]></title><type>5</type>\
                     <url>https://a.example/</url></appmsg></msg>";
        let file = "<msg><appmsg><title>a&amp;b&#233;.pdf</title><type>6</type>\
                    <appattach><totallen>3</totallen></appattach></appmsg></msg>";
        for (ty, local_types) in [("5", [49, (5 << 32) | 49]), ("6", [49, (6 << 32) | 49])] {
            for local_type in local_types {
                let at = format!("<type> {ty} as {local_type}");
                if ty == "6" {
                    let m = read(local_type, file);
                    assert_eq!(m.file_name(), Some("a&bé.pdf"), "{at}");
                    assert_eq!(export(local_type, file), "[文件] a&bé.pdf", "{at}");
                    continue;
                }
                let m = read(local_type, link);
                let t = m.t49.as_deref().unwrap();
                assert_eq!(t.link_title.as_deref(), Some("A & B"), "{at}");
                assert_eq!(
                    t.link_url.as_deref(),
                    Some("https://mp.example/s?__biz=X&mid=1"),
                    "{at}"
                );
                assert_eq!(
                    t.link_thumb.as_deref(),
                    Some("https://t.example/a?x=1&y=2"),
                    "{at}"
                );
                assert_eq!(t.app_msg_desc.as_deref(), Some("<C &amp; D"), "{at}");
                assert_eq!(export(local_type, link), "[链接] A & B", "{at}");
                let field = arkme(local_type, link);
                assert_eq!(field("linkTitle").as_deref(), Some("A & B"), "{at}");
                assert_eq!(field("appMsgDesc").as_deref(), Some("<C &amp; D"), "{at}");
                assert_eq!(
                    field("linkThumb").as_deref(),
                    Some("https://t.example/a?x=1&y=2"),
                    "{at}"
                );
                assert_eq!(
                    field("linkUrl").as_deref(),
                    Some("https://mp.example/s?__biz=X&mid=1"),
                    "{at}"
                );
                // CDATA 里的文字按原样保留
                let m = read(local_type, cdata);
                assert_eq!(m.link_title(), Some("a &amp; b"), "{at}");
                assert_eq!(export(local_type, cdata), "[链接] a &amp; b", "{at}");
            }
        }

        let card = r#"<msg username="wxid_c" nickname="C &amp; D" bigheadimgurl="https://h.example/a?x=1&amp;y=2"/>"#;
        let m = read(42, card);
        assert_eq!(m.card_nickname.as_deref(), Some("C & D"));
        assert_eq!(
            m.card_avatar_url.as_deref(),
            Some("https://h.example/a?x=1&y=2")
        );
        let card_meta =
            crate::message::appmsg::extract_arkme_contact_card_meta(card, 42).unwrap_or_default();
        assert_eq!(
            card_meta
                .get("contactCardNickname")
                .and_then(|v| v.as_str()),
            Some("C & D")
        );

        let location = r#"<msg><location x="1" y="2" label="Caf&#233; &amp; Bar" poiname="P &lt;1&gt;"/></msg>"#;
        let m = read(48, location);
        assert_eq!(m.location_label.as_deref(), Some("Café & Bar"));
        assert_eq!(m.location_poiname.as_deref(), Some("P <1>"));
        let text = export(48, location);
        assert!(
            text.contains("P <1>") && text.contains("Café & Bar") && !text.contains("&amp;"),
            "{text}"
        );
    }
    #[test]
    fn a_text_message_loses_only_the_line_naming_its_sender() {
        // 与聊天模型读一行时一样（#656）：私聊中第一行是 `PS:` 的消息保留这一行，
        // 群里别人发的消息去掉写着发送者的一行
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
        // 数据库中保存的撤回：撤回时间不属于提示
        assert_eq!(
            parsed_content(
                "<?xml version=\"1.0\"?><sysmsg type=\"revokemsg\"><revokemsg><content>\"Bob\" 撤回了一条消息</content>\
                 <revoketime>0</revoketime></revokemsg></sysmsg>",
                10000,
                None
            ),
            "\"Bob\" 撤回了一条消息"
        );
        // 纯文字的提示就是它本身，末尾的数字也保留
        assert_eq!(
            parsed_content("群聊人数已达 100", 10000, None),
            "群聊人数已达 100"
        );
        assert_eq!(parsed_content("<a></a>", 10000, None), "[系统消息]");
    }

    #[test]
    fn a_quote_of_a_message_that_quotes_keeps_its_own_sender() {
        // 被引用消息的内容里有它自己的（已解码的）`<refermsg>`：块到最后一个结束标签为止，不是里面那个
        let xml = "<msg><appmsg><title>好看</title><type>57</type><refermsg><type>1</type><svrid>9</svrid>\
                   <content>原话&lt;refermsg&gt;&lt;svrid&gt;7&lt;/svrid&gt;&lt;/refermsg&gt;</content>\
                   <displayname>Me Nick</displayname></refermsg></appmsg></msg>";
        let (_, sender) = quote_of(xml);
        assert_eq!(sender.as_deref(), Some("Me Nick"));
        assert_eq!(
            crate::message::xml::MsgXml::new(xml)
                .reply_to_message_id()
                .as_deref(),
            Some("9")
        );
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
        // 账号目录名的 `_<4 个字符>` 后缀，哪一边带都算
        assert_eq!(
            resolve_is_send(Some(0), Some("WXID_ME_1a2b"), "wxid_me"),
            Some(1)
        );
        assert_eq!(resolve_is_send(Some(0), Some("john"), "john_1a2b"), Some(1));
        // 自定义的微信号不当作别人的微信号的前缀
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

    /// 回复某条消息的图片、语音、视频、表情或位置不是 app 消息，引用的内容与导出写的相同（#1263）。
    #[test]
    fn a_media_reply_takes_no_app_fields_from_the_app_message_it_quotes() {
        let quote = "<refermsg><type>49</type><svrid>9</svrid><displayname>Bob</displayname>\
                     <content>&lt;msg&gt;&lt;appmsg&gt;&lt;title&gt;t&lt;/title&gt;&lt;type&gt;5&lt;/type&gt;\
                     &lt;/appmsg&gt;&lt;/msg&gt;</content></refermsg>";
        for (local_type, media) in [
            (3, "<img md5=\"x\"/>"),
            (34, "<voicemsg length=\"1000\"/>"),
            (43, "<videomsg md5=\"x\"/>"),
            (47, "<emoji md5=\"x\"/>"),
            (48, "<location x=\"1\" y=\"2\"/>"),
        ] {
            let content = format!("<msg>{media}<extcommoninfo>{quote}</extcommoninfo></msg>");
            let row = MsgRow {
                local_type,
                content: content.clone(),
                ..MsgRow::default()
            };
            let m = map_msg_rows(vec![row], "wxid_bob", "wxid_me").remove(0);
            assert!(m.t49.is_none(), "{content}");
            // 作为资源或在 arkme-json 中也没有 app 消息的种类
            assert_eq!(
                crate::message::appmsg::app_message_kind(&content, local_type),
                None
            );
            let meta = crate::message::appmsg::extract_arkme_app_message_meta_in(
                &crate::message::xml::MsgXml::new(&content),
                local_type,
            )
            .unwrap();
            assert_eq!(
                (m.quoted_content.as_deref(), m.quoted_sender.as_deref()),
                (Some("t"), Some("Bob")),
                "{content}"
            );
            assert_eq!(
                (
                    meta["quotedContent"].as_str(),
                    meta["quotedSender"].as_str()
                ),
                (Some("t"), Some("Bob")),
                "{content}"
            );
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
