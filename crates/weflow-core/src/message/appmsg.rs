//! app 消息：种类、各种类的 Arkme 元数据，以及名片的元数据。

use serde_json::{json, Map, Value};

use super::link::{extract_html_link_card, normalize_http_link_url};
use super::parse_duration_seconds;
use super::xml::{
    extract_xml_value, normalize_app_message_content, rx, xml_attr_text, xml_text, MsgXml,
};

/// `<finderFeed>` 之后第一个 `<desc>` 的文字，即视频号的描述；没有时为空字符串。
fn extract_finder_feed_desc(content: &str) -> String {
    rx(r"(?is)<finderFeed.*?<desc>.*?</desc>")
        .find(content)
        .map(|m| xml_text(m.as_str(), "desc"))
        .unwrap_or_default()
}

/// `n` 中 `tags` 里第一个有文字的元素的文字，解码成给人读的形式（[`xml_text`]，#1199）。
fn first_non_empty(n: &str, tags: &[&str]) -> String {
    tags.iter()
        .map(|t| xml_text(n, t))
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

/// [`extract_arkme_app_message_meta_in`] 在读取各字段之前求出的判定。
struct AppMessageClass {
    /// 规范化后的内容。
    n: String,
    /// app 消息的 `<type>`，没有时为空。
    xml_type: String,
    is_finder: bool,
    is_music: bool,
    /// 元数据的 `appMsgKind`。
    kind: Option<&'static str>,
}

/// app 消息自己的 `<type>` 决定的类型，是一张表中的一行：聊天窗口、导出、两者的文字、导出的类型名和 ChatLab
/// 都读这一张表（#1165、#1180）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AppType {
    /// `appMsgKind`；没有自己的种类的类型（表情、笔记、拍一拍）为 `None`，它们的种类与表中没有的类型一样按 XML
    /// 推断（[`kind_by_keywords`]）。
    pub(crate) kind: Option<&'static str>,
    /// 这种消息的文字开头的标签。
    pub(crate) tag: &'static str,
    /// 导出（json、excel、sql）中的类型名。
    pub(crate) name: &'static str,
    /// ChatLab 的 `type`。
    pub(crate) chatlab: i64,
}

/// `xml_type`（app 消息自己的 `<type>`）在表中的那一行；表中没有这个类型时为 `None`。
pub(crate) fn app_type(xml_type: &str) -> Option<AppType> {
    let row = |kind, tag, name, chatlab| AppType {
        kind,
        tag,
        name,
        chatlab,
    };
    Some(match xml_type {
        // 76 是较新的音乐分享，字段与 3 相同（#1167）
        "3" | "76" => row(Some("music"), "[音乐]", "音乐消息", 7),
        // 4 是视频的链接
        "4" | "5" | "49" => row(Some("link"), "[链接]", "链接消息", 7),
        "6" => row(Some("file"), "[文件]", "文件消息", 4),
        "8" => row(None, "[表情]", "动画表情", 7),
        "19" => row(Some("chat-record"), "[聊天记录]", "聊天记录", 7),
        "24" => row(None, "[笔记]", "笔记", 7),
        "33" | "36" => row(Some("miniapp"), "[小程序]", "小程序消息", 24),
        "51" => row(Some("finder"), "[视频号]", "视频号消息", 7),
        "53" => row(Some("solitaire"), "[接龙]", "接龙", 7),
        "57" => row(Some("quote"), "[引用消息]", "引用消息", 25),
        // 拍一拍，导出时按系统消息写（#1196）
        "62" => row(None, "[拍一拍]", "系统消息", 80),
        "87" => row(Some("announcement"), "[群公告]", "群公告", 7),
        "2000" => row(Some("transfer"), "[转账]", "转账消息", 99),
        "2001" => row(Some("red-packet"), "[红包]", "红包消息", 7),
        _ => return None,
    })
}

/// `local_type` 在低 32 位之上保存的 `<type>` 在表中的那一行（微信 4 把 app 消息存为 `(<type> << 32) | 49`），
/// 供只按 `local_type` 判断的地方用（#1263）；没有保存 `<type>` 的（49 本身、不是 app 消息的）和表中没有的为 `None`。
pub(crate) fn row_kept_in(local_type: i64) -> Option<AppType> {
    let kept = local_type >> 32;
    if local_type & 0xFFFF_FFFF != 49 || kept == 0 {
        return None;
    }
    app_type(&kept.to_string())
}

/// 种类为 `kind` 的类型在表中的那一行（[`type_of_kind`]），用于种类由 XML 而不是 `<type>` 判出的消息；
/// 没有对应行的种类（`card`、`gift`、`location`）为 `None`。
pub(crate) fn row_of_kind(kind: &str) -> Option<AppType> {
    app_type(type_of_kind(kind)?)
}

/// 表中种类为 `kind` 的一个 `<type>`（公众号文章算链接）；没有对应行的种类（`card`、`gift`、`location`）为 `None`。
pub(crate) fn type_of_kind(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "music" => "3",
        "link" | "official-link" => "5",
        "file" => "6",
        "chat-record" => "19",
        "miniapp" => "33",
        "finder" => "51",
        "solitaire" => "53",
        "quote" => "57",
        "announcement" => "87",
        "transfer" => "2000",
        "red-packet" => "2001",
        _ => return None,
    })
}

/// app 消息自己的 `<type>` 给出的种类（`finder`、`music`、`quote` 等），聊天窗口、导出和两者的文字都相同（#1165）；
/// 类型没有给出种类时为 `None`，种类由 [`kind_by_keywords`] 推断。
pub(crate) fn kind_of_app_type(xml_type: &str) -> Option<&'static str> {
    app_type(xml_type).and_then(|row| row.kind)
}

/// 去掉被引用消息后的 app 消息 XML：回复的 `<refermsg>` 里是被引用消息的 XML，其中的词说明的是被引用的消息，
/// 不是回复本身。
pub(crate) fn without_quoted(xml: &str) -> std::borrow::Cow<'_, str> {
    crate::xml::remove_blocks(xml, "<refermsg", "</refermsg>")
}

/// `<type>` 没有给出种类的 app 消息，按它自己的 XML（`own`，[`without_quoted`]）推断种类：有 `<finderFeed>`，
/// 或 `<finderUsername>`、`<finderObjectID>` 不为空的是视频号；`<nativeurl>` 是微信打开红包的地址的是红包；
/// `<musicurl>` 或 `<playurl>` 不为空的是音乐。`<dataurl>` 不算音乐的标志，链接和文件也有它，而且常常为空；
/// 空的视频号元素（`<finderGuarantee>`、`<finderUsername></finderUsername>`）也不算视频号的标志，表情和小程序的
/// XML 里也有它们（#1241）。
pub(crate) fn kind_by_keywords(own: &str) -> Option<&'static str> {
    // 标签不区分大小写；内容按原样读，不先转成小写，因为转成小写的 `<![cdata[` 会被当作文字
    if crate::xml::find_ci(own, 0, "<finderfeed").is_some()
        || ["finderusername", "finderobjectid"]
            .iter()
            .any(|tag| !extract_xml_value(own, tag).is_empty())
    {
        return Some("finder");
    }
    if extract_xml_value(own, "nativeurl")
        .trim()
        .to_ascii_lowercase()
        .starts_with("wxpay://c2cbizmessagehandler/hongbao")
    {
        return Some("red-packet");
    }
    if ["musicurl", "playurl"]
        .iter()
        .any(|tag| !extract_xml_value(own, tag).trim().is_empty())
    {
        return Some("music");
    }
    None
}

/// app 消息自己的 XML（`own`，[`without_quoted`]）中的地点：依次取 `<location>` 的 `label`、`poiname` 属性，
/// 再取 `<label>`、`<poiname>` 元素；没有地点时为空字符串。
pub(crate) fn location_label(own: &str) -> String {
    [
        xml_attr_text(own, "location", "label"),
        xml_attr_text(own, "location", "poiname"),
        xml_text(own, "label"),
        xml_text(own, "poiname"),
    ]
    .into_iter()
    .find(|v| !v.is_empty())
    .unwrap_or_default()
}

/// 链接（`<type>` 5 或 49）是否为公众号文章：来源是公众号（`sourceusername` 为 `gh_…`）、应用名含「公众号」，
/// 或者有来源名（`sourcename`）。
fn is_official_article(n: &str, xml_type: &str) -> bool {
    matches!(xml_type, "5" | "49")
        && (xml_text(n, "sourceusername").starts_with("gh_")
            || xml_text(n, "appname").contains("公众号")
            || !xml_text(n, "sourcename").is_empty())
}

/// `kind` 是否为链接，包括公众号文章。
pub(crate) fn is_link_kind(kind: Option<&str>) -> bool {
    matches!(kind, Some("link" | "official-link"))
}

/// [`extract_arkme_app_message_meta_in`] 的 `appMsgKind`，不读其他字段；图片、语音、视频、表情和位置不是 app 消息，
/// 为 `None`。
pub fn app_message_kind(content: &str, local_type: i64) -> Option<&'static str> {
    app_message_kind_in(&MsgXml::new(content), local_type)
}

/// 用消息已有的 [`MsgXml`] 求 [`app_message_kind`]。
pub(crate) fn app_message_kind_in(x: &MsgXml<'_>, local_type: i64) -> Option<&'static str> {
    classify_app_message(x, local_type)?.kind
}

/// 从消息的 [`MsgXml`] 中取出的 app 消息字段，即 `arkme-json` 和 `json` 额外带的结构化字段。图片、语音、视频、
/// 表情和位置没有这些字段，它们回复某条消息时只带被引用消息的字段。
pub fn extract_arkme_app_message_meta_in(
    x: &MsgXml<'_>,
    local_type: i64,
) -> Option<Map<String, Value>> {
    if is_non_app_type(local_type) {
        return reply_quote_fields(x);
    }
    app_message_fields(classify_app_message(x, local_type)?, local_type, x)
}

/// 只对引用、链接，以及图片、语音、视频、表情和位置的回复求 [`extract_arkme_app_message_meta_in`]；
/// 其他消息为 `None`，不读它们的字段。
pub fn quote_or_link_app_message_meta_in(
    x: &MsgXml<'_>,
    local_type: i64,
) -> Option<Map<String, Value>> {
    if is_non_app_type(local_type) {
        return reply_quote_fields(x);
    }
    let content = x.content();
    // 引用有 `<refermsg>`、引用的 `local_type` 或 `<type>` 57，链接有 `<type>`（4、5、49）。
    // 这几个词在规范化后仍在（见 `normalized_may_contain`）；从 `<appmsg>` 中切掉 `<refermsg>`、`<patmsg>` 块时，
    // 拼出的 `<type>` 也只会出现在有这些词的地方：三个词都没有时，消息既不是引用也不是链接。
    if local_type != 244813135921
        && ["type", "refermsg", "patmsg"]
            .iter()
            .all(|word| crate::xml::find_ci(content, 0, word).is_none())
    {
        return None;
    }
    let class = classify_app_message(x, local_type)?;
    if class.kind != Some("quote") && !is_link_kind(class.kind) {
        return None;
    }
    app_message_fields(class, local_type, x)
}

/// 图片、语音、视频、表情和位置的基本类型（`local_type` 的低 32 位）。
const NON_APP_TYPES: [i64; 5] = [3, 34, 43, 47, 48];

/// `local_type` 是否为图片、语音、视频、表情或位置：它们的 XML 与 app 消息一样是 `<msg>`，也可能带回复的
/// `<refermsg>`，但它们不是 app 消息。
pub(crate) fn is_non_app_type(local_type: i64) -> bool {
    NON_APP_TYPES.contains(&(local_type & 0xFFFF_FFFF))
}

/// [`is_non_app_type`] 的消息回复某条消息时（如用图片回复）的元数据：只有它的 `<refermsg>` 所引用消息的字段，
/// 与 app 消息的引用相同；不是回复时为 `None`。
fn reply_quote_fields(x: &MsgXml<'_>) -> Option<Map<String, Value>> {
    // `refermsg` 这个词在规范化后仍在（见 `normalized_may_contain`）：原文中没有它，就没有 `<refermsg>` 可读
    if !x.content().contains("refermsg") || !x.once().contains("<refermsg>") {
        return None;
    }
    let mut meta = Map::new();
    put_quote(&mut meta, x);
    (!meta.is_empty()).then_some(meta)
}

/// `value` 不为空时把它放进 `meta` 的 `key`。
fn put(meta: &mut Map<String, Value>, key: &str, value: String) {
    if !value.is_empty() {
        meta.insert(key.into(), Value::String(value));
    }
}

/// 把 `x` 引用的消息放进 `meta`：`quotedContent`、`quotedSender`、`quotedType`、`quotedSvrid`，有哪项放哪项。
fn put_quote(meta: &mut Map<String, Value>, x: &MsgXml<'_>) {
    let q = x.quote_twice().clone();
    put(meta, "quotedContent", q.content.unwrap_or_default());
    put(meta, "quotedSender", q.sender.unwrap_or_default());
    put(meta, "quotedType", q.quote_type.unwrap_or_default());
    put(meta, "quotedSvrid", q.svrid.unwrap_or_default());
}

/// [`extract_arkme_app_message_meta_in`] 对 app 消息的第一步：判出种类；消息完全没有 app 消息的元数据时为 `None`，
/// [`is_non_app_type`] 的消息就没有。这里判出的种类是 app 消息唯一的种类，聊天窗口、命令 API 和导出都读它（#1263）。
fn classify_app_message(x: &MsgXml<'_>, local_type: i64) -> Option<AppMessageClass> {
    if x.content().is_empty() || is_non_app_type(local_type) {
        return None;
    }
    let n = x.once().to_string();
    let looks_app = local_type == 49 || local_type == 244813135921 || n.contains("<appmsg");
    let has_refer = n.contains("<refermsg>");
    let replies = has_refer || local_type == 244813135921;
    let xml_type = x.app_type_twice().to_string();
    let own = without_quoted(&n);
    // 先看消息自己的 `<type>`；类型没有给出种类时才看它自己的 XML 里的词（#1165）
    let kind = match kind_of_app_type(&xml_type) {
        Some("link") if replies => Some("quote"),
        Some(kind) => Some(kind),
        None => match kind_by_keywords(&own) {
            Some(kind) => Some(kind),
            None if replies => Some("quote"),
            None if looks_app => Some("card"),
            None => None,
        },
    };
    // 聊天窗口单独显示的几种：礼物、位置、公众号文章；导出的叫法相同（#1263）
    let kind = match kind {
        Some("finder" | "red-packet") => kind,
        _ if xml_type == "115" => Some("gift"),
        _ if !location_label(&own).is_empty() => Some("location"),
        Some("link") if is_official_article(&n, &xml_type) => Some("official-link"),
        kind => kind,
    };
    let is_finder = kind == Some("finder");
    let is_music = kind == Some("music");
    if !looks_app && !is_finder && !has_refer {
        return None;
    }
    Some(AppMessageClass {
        n,
        xml_type,
        is_finder,
        is_music,
        kind,
    })
}

/// 按 `class` 的判定取出 [`extract_arkme_app_message_meta_in`] 的各字段。
fn app_message_fields(
    class: AppMessageClass,
    local_type: i64,
    x: &MsgXml<'_>,
) -> Option<Map<String, Value>> {
    let AppMessageClass {
        n,
        xml_type,
        is_finder,
        is_music,
        kind,
    } = class;
    let mut meta = Map::new();
    if !xml_type.is_empty() {
        put(&mut meta, "appMsgType", xml_type);
    } else if kind == Some("quote") {
        put(&mut meta, "appMsgType", "57".into());
    }
    if let Some(k) = kind {
        put(&mut meta, "appMsgKind", k.into());
    }
    let desc = first_non_empty(&n, &["des", "desc"]);
    let app_name = xml_text(&n, "appname");
    let source_name = first_non_empty(&n, &["sourcename", "sourcedisplayname"]);
    let source_user = xml_text(&n, "sourceusername");
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
        put_quote(&mut meta, x);
    }
    if is_link_kind(kind) {
        let card = extract_html_link_card(&n, local_type);
        let url = card
            .as_ref()
            .map(|c| c.url.clone())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| {
                normalize_http_link_url(&first_non_empty(
                    &n,
                    &["url", "shareurl", "shorturl", "dataurl"],
                ))
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
            // 不取 `<songalbumurl>`：它是封面，写在 `musicAlbumUrl`（#1170）
            first_non_empty(&n, &["musicurl", "playurl"]),
        );
        put(
            &mut meta,
            "musicDataUrl",
            first_non_empty(&n, &["dataurl", "lowurl"]),
        );
        put(&mut meta, "musicAlbumUrl", xml_text(&n, "songalbumurl"));
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
        put(&mut meta, "musicSourceName", xml_text(&n, "sourcename"));
        let dur = first_non_empty(&n, &["playlength", "play_length", "duration"]);
        if let Some(d) = parse_duration_seconds(&dur) {
            meta.insert("musicDuration".into(), json!(d));
        }
    }
    if !is_finder {
        return if meta.is_empty() { None } else { Some(meta) };
    }
    let raw_title = xml_text(&n, "title");
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
    put(&mut meta, "finderAvatar", xml_text(&n, "avatar"));
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

/// 名片消息（`local_type` 42）的 Arkme 元数据：`cardKind` 为 `contact-card`，以及名片上的用户名、昵称、备注、
/// 地区、签名、头像和性别；各项取 `<msg>` 的属性，没有时取同名元素。不是名片时为 `None`。
pub fn extract_arkme_contact_card_meta(
    content: &str,
    local_type: i64,
) -> Option<Map<String, Value>> {
    if content.is_empty() || local_type != 42 {
        return None;
    }
    let n = normalize_app_message_content(content);
    let read = |attr: &str| {
        let a = xml_attr_text(&n, "msg", attr);
        if !a.is_empty() {
            a
        } else {
            xml_text(&n, attr)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn arkme_meta_kinds() {
        let xml = "<msg><appmsg><title>Song</title><type>3</type><songname>S</songname><singername>Z</singername><musicurl>https://m</musicurl></appmsg></msg>";
        let meta = extract_arkme_app_message_meta_in(&MsgXml::new(xml), 49).unwrap();
        assert_eq!(meta["appMsgKind"], "music");
        assert_eq!(meta["musicSinger"], "Z");
        let card = "<msg username=\"wxid_x\" nickname=\"Nick\" sex=\"1\" city=\"SH\"/>";
        let cm = extract_arkme_contact_card_meta(card, 42).unwrap();
        assert_eq!(cm["contactCardWxid"], "wxid_x");
        assert_eq!(cm["contactCardGender"], 1);
    }

    #[test]
    fn only_app_messages_have_an_app_kind() {
        let not_apps = [
            (
                3,
                "<?xml version=\"1.0\"?>\n<msg><img md5=\"aabb\" length=\"10\"/></msg>",
            ),
            (
                34,
                "<msg><voicemsg length=\"1000\" voicelength=\"1\"/></msg>",
            ),
            (43, "<msg><videomsg md5=\"aabb\" playlength=\"3\"/></msg>"),
            (47, "<msg><emoji md5=\"aabb\" cdnurl=\"http://e\"/></msg>"),
            (
                48,
                "<msg><location x=\"31.2\" y=\"121.4\" label=\"Bund\" poiname=\"Bund\"/></msg>",
            ),
            (1, "a <msg> in a text"),
        ];
        for (t, xml) in not_apps {
            let x = MsgXml::new(xml);
            assert_eq!(app_message_kind(xml, t), None, "{xml:?} as {t}");
            assert_eq!(
                extract_arkme_app_message_meta_in(&x, t),
                None,
                "{xml:?} as {t}"
            );
            assert_eq!(
                quote_or_link_app_message_meta_in(&x, t),
                None,
                "{xml:?} as {t}"
            );
        }
        let apps = [
            (
                49,
                "<msg><appmsg><title>t</title><type>5</type><url>http://a</url></appmsg></msg>",
                "link",
            ),
            (
                25_769_803_825,
                "<msg><appmsg><title>f.pdf</title><type>6</type></appmsg></msg>",
                "file",
            ),
            (
                244_813_135_921,
                "<msg><appmsg><title>ok</title><refermsg><svrid>1</svrid></refermsg></appmsg></msg>",
                "quote",
            ),
            // 类型没有种类，或 XML 没有写出类型的 app 消息
            (
                49,
                "<msg><appmsg><title>t</title><type>999</type></appmsg></msg>",
                "card",
            ),
            (49, "<msg><voicemsg length=\"1\"/></msg>", "card"),
            (
                21_474_836_529,
                "<msg><appmsg><title>t</title><type>5</type></appmsg></msg>",
                "link",
            ),
        ];
        for (t, xml, kind) in apps {
            assert_eq!(app_message_kind(xml, t), Some(kind), "{xml:?} as {t}");
            let meta = extract_arkme_app_message_meta_in(&MsgXml::new(xml), t);
            assert_eq!(
                meta.as_ref().and_then(|m| m["appMsgKind"].as_str()),
                Some(kind),
                "{xml:?} as {t}"
            );
        }
    }

    #[test]
    fn a_media_reply_has_the_fields_of_its_quote_and_no_app_fields() {
        let refers = [
            "<refermsg><type>1</type><svrid>9001</svrid><displayname>Bob</displayname><content>hi there</content></refermsg>",
            // 引用的是 app 消息，它的 XML 会让回复看起来像 app 消息
            "<refermsg><type>49</type><svrid>9003</svrid><displayname>Bob</displayname><content>&lt;msg&gt;&lt;appmsg&gt;\
             &lt;title&gt;A&lt;/title&gt;&lt;des&gt;d&lt;/des&gt;&lt;type&gt;5&lt;/type&gt;&lt;/appmsg&gt;&lt;/msg&gt;</content></refermsg>",
            // 引用的是图片，它的 `<type>`（3）会让回复被当成音乐
            "<refermsg><type>3</type><svrid>9002</svrid><displayname>Bob</displayname><content>&lt;msg&gt;&lt;img/&gt;&lt;/msg&gt;</content></refermsg>",
            "<refermsg><svrid>9005</svrid></refermsg>",
        ];
        for refer in refers {
            // app 消息引用同一条消息时字段相同
            let quote =
                format!("<msg><appmsg><title>ok</title><type>57</type>{refer}</appmsg></msg>");
            let mut want = extract_arkme_app_message_meta_in(&MsgXml::new(&quote), 49).unwrap();
            want.retain(|key, _| key.starts_with("quoted"));
            assert!(want.contains_key("quotedSvrid"), "{refer}");
            for t in NON_APP_TYPES {
                let xml =
                    format!("<msg><img md5=\"x\"/><extcommoninfo>{refer}</extcommoninfo></msg>");
                let x = MsgXml::new(&xml);
                assert_eq!(app_message_kind(&xml, t), None, "{xml:?} as {t}");
                assert_eq!(
                    extract_arkme_app_message_meta_in(&x, t).as_ref(),
                    Some(&want),
                    "{xml:?} as {t}"
                );
                assert_eq!(
                    quote_or_link_app_message_meta_in(&x, t).as_ref(),
                    Some(&want),
                    "{xml:?} as {t}"
                );
            }
        }
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
        // 切掉 `<patmsg>` 块之后才出现的 `<type>` 和 `</type>`
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
            let whole = extract_arkme_app_message_meta_in(&MsgXml::new(&s), t);
            let kind = app_message_kind(&s, t);
            assert_eq!(
                kind,
                whole.as_ref().and_then(|m| m.get("appMsgKind")?.as_str()),
                "{s:?} as {t}"
            );
            if is_non_app_type(t) {
                assert!(
                    whole.iter().flatten().all(|(k, _)| k.starts_with("quoted")),
                    "{s:?} as {t}: {whole:?}"
                );
            }
            // 媒体回复的元数据就是它的引用：`json` 导出也保留它
            let quote_or_link =
                whole.filter(|_| kind == Some("quote") || is_link_kind(kind) || is_non_app_type(t));
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
