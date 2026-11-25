//! App messages: their kind, the Arkme metadata of each kind and contact cards.

use serde_json::{json, Map, Value};

use super::link::{extract_html_link_card, normalize_http_link_url};
use super::parse_duration_seconds;
use super::xml::{
    extract_xml_value, normalize_app_message_content, rx, xml_attr_text, xml_text, MsgXml,
};

fn extract_finder_feed_desc(content: &str) -> String {
    rx(r"(?is)<finderFeed.*?<desc>.*?</desc>")
        .find(content)
        .map(|m| xml_text(m.as_str(), "desc"))
        .unwrap_or_default()
}

/// The text of the first of `tags` that `n` has, for people to read ([`xml_text`], #1199).
fn first_non_empty(n: &str, tags: &[&str]) -> String {
    tags.iter()
        .map(|t| xml_text(n, t))
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

/// What [`extract_arkme_app_message_meta_in`] works out before it reads any field.
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

/// What an app message's own `<type>` makes it: one row of the one table that the chat window, the exports, the
/// text of both, the exports' type names and ChatLab read (#1165, #1180).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AppType {
    /// Its `appMsgKind`; `None` for a type with no kind of its own (a sticker, a note), whose kind
    /// [`kind_by_keywords`] guesses, as for a type not in the table.
    pub(crate) kind: Option<&'static str>,
    /// The tag the text of such a message starts with.
    pub(crate) tag: &'static str,
    /// The name of its type in the exports (json, excel, sql).
    pub(crate) name: &'static str,
    /// Its ChatLab type.
    pub(crate) chatlab: i64,
}

/// The row of `xml_type` (an app message's own `<type>`), or `None` for a type the table does not know.
pub(crate) fn app_type(xml_type: &str) -> Option<AppType> {
    let row = |kind, tag, name, chatlab| AppType {
        kind,
        tag,
        name,
        chatlab,
    };
    Some(match xml_type {
        // 76 is the newer music share, with the same fields as 3 (#1167)
        "3" | "76" => row(Some("music"), "[音乐]", "音乐消息", 7),
        // 4 is a video's link
        "4" | "5" | "49" => row(Some("link"), "[链接]", "链接消息", 7),
        "6" => row(Some("file"), "[文件]", "文件消息", 4),
        "8" => row(None, "[表情]", "动画表情", 7),
        "19" => row(Some("chat-record"), "[聊天记录]", "聊天记录", 7),
        "24" => row(None, "[笔记]", "笔记", 7),
        "33" | "36" => row(Some("miniapp"), "[小程序]", "小程序消息", 24),
        "51" => row(Some("finder"), "[视频号]", "视频号消息", 7),
        "53" => row(Some("solitaire"), "[接龙]", "接龙", 7),
        "57" => row(Some("quote"), "[引用消息]", "引用消息", 25),
        // a pat, which the exports keep as a system message (#1196)
        "62" => row(None, "[拍一拍]", "系统消息", 80),
        "87" => row(Some("announcement"), "[群公告]", "群公告", 7),
        "2000" => row(Some("transfer"), "[转账]", "转账消息", 99),
        "2001" => row(Some("red-packet"), "[红包]", "红包消息", 7),
        _ => return None,
    })
}

/// The kind an app message's own `<type>` gives it (`finder`, `music`, `quote`, …), the same for the chat window,
/// the exports and the text of both (#1165); `None` for a type that does not say, whose kind
/// [`kind_by_keywords`] guesses.
pub(crate) fn kind_of_app_type(xml_type: &str) -> Option<&'static str> {
    app_type(xml_type).and_then(|row| row.kind)
}

/// An app message's XML without the messages it quotes: a reply's `<refermsg>` holds the quoted message's XML, whose
/// words say what the quoted message is, not the reply.
pub(crate) fn without_quoted(xml: &str) -> std::borrow::Cow<'_, str> {
    crate::xml::remove_blocks(xml, "<refermsg", "</refermsg>")
}

/// The kind an app message whose `<type>` says nothing looks like by its own XML (`own`, [`without_quoted`]): a
/// Channels video by its `<finderFeed>` or a non-empty `<finderUsername>` or `<finderObjectID>`, a red packet by the
/// address WeChat opens it with, a song by a non-empty `<musicurl>` or `<playurl>`. `<dataurl>` is no sign of a song,
/// as links and files have it too, often empty; nor are empty finder elements (`<finderGuarantee>`,
/// `<finderUsername></finderUsername>`) a sign of a video, as the XML of a sticker or a mini program has them too
/// (#1241).
pub(crate) fn kind_by_keywords(own: &str) -> Option<&'static str> {
    // tags match in any case; the content is read as it is, as a lowered `<![cdata[` would be text
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

/// `appMsgKind` of [`extract_arkme_app_message_meta_in`], without reading the other fields; `None` for an image, a
/// voice message, a video, a sticker and a location, which are not app messages.
pub fn app_message_kind(content: &str, local_type: i64) -> Option<&'static str> {
    classify_app_message(&MsgXml::new(content), local_type)?.kind
}

/// The app-message fields of a message's [`MsgXml`]: extra structured fields for `arkme-json` / `json`. An image, a
/// voice message, a video, a sticker or a location has none of them: when it replies to a message, only the fields of
/// what it quotes.
pub fn extract_arkme_app_message_meta_in(
    x: &MsgXml<'_>,
    local_type: i64,
) -> Option<Map<String, Value>> {
    if is_non_app_type(local_type) {
        return reply_quote_fields(x);
    }
    app_message_fields(classify_app_message(x, local_type)?, local_type, x)
}

/// [`extract_arkme_app_message_meta_in`] of a quote, a link, or a reply that is an image, a voice message, a video,
/// a sticker or a location; `None` for every other message, whose fields are not read.
pub fn quote_or_link_app_message_meta_in(
    x: &MsgXml<'_>,
    local_type: i64,
) -> Option<Map<String, Value>> {
    if is_non_app_type(local_type) {
        return reply_quote_fields(x);
    }
    let content = x.content();
    // A quote has a `<refermsg>`, the quote's local type or `<type>` 57, a link a `<type>` (4, 5, 49). Those words
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

/// The base types (the low 32 bits of the local type) of an image, a voice message, a video, a sticker and a location.
const NON_APP_TYPES: [i64; 5] = [3, 34, 43, 47, 48];

/// Whether `local_type` is an image's, a voice message's, a video's, a sticker's or a location's: never an app
/// message, though its XML is a `<msg>` as an app message's is, and the `<refermsg>` of a reply can quote one.
pub(crate) fn is_non_app_type(local_type: i64) -> bool {
    NON_APP_TYPES.contains(&(local_type & 0xFFFF_FFFF))
}

/// The meta of a message of [`is_non_app_type`] that replies to a message (an image answering one, say): only the
/// fields of what its `<refermsg>` quotes, as an app message's quote has them; `None` for one that replies to none.
fn reply_quote_fields(x: &MsgXml<'_>) -> Option<Map<String, Value>> {
    // `refermsg` survives normalizing (see normalized_may_contain): without it, there is no `<refermsg>` to read
    if !x.content().contains("refermsg") || !x.once().contains("<refermsg>") {
        return None;
    }
    let mut meta = Map::new();
    put_quote(&mut meta, x);
    (!meta.is_empty()).then_some(meta)
}

/// Puts `value` under `key` in `meta`, unless it is empty.
fn put(meta: &mut Map<String, Value>, key: &str, value: String) {
    if !value.is_empty() {
        meta.insert(key.into(), Value::String(value));
    }
}

/// Puts what `x` quotes in `meta`: `quotedContent`, `quotedSender`, `quotedType` and `quotedSvrid`, those it has.
fn put_quote(meta: &mut Map<String, Value>, x: &MsgXml<'_>) {
    let q = x.quote_twice().clone();
    put(meta, "quotedContent", q.content.unwrap_or_default());
    put(meta, "quotedSender", q.sender.unwrap_or_default());
    put(meta, "quotedType", q.quote_type.unwrap_or_default());
    put(meta, "quotedSvrid", q.svrid.unwrap_or_default());
}

/// The first part of [`extract_arkme_app_message_meta_in`] for an app message; `None` when the message has no
/// app-message meta at all, as one of [`is_non_app_type`] has none.
fn classify_app_message(x: &MsgXml<'_>, local_type: i64) -> Option<AppMessageClass> {
    if x.content().is_empty() || is_non_app_type(local_type) {
        return None;
    }
    let n = x.once().to_string();
    let looks_app = local_type == 49 || local_type == 244813135921 || n.contains("<appmsg");
    let has_refer = n.contains("<refermsg>");
    let replies = has_refer || local_type == 244813135921;
    let xml_type = x.app_type_twice().to_string();
    // the message's own `<type>` first; the words of its own XML only when the type says nothing (#1165)
    let kind = match kind_of_app_type(&xml_type) {
        Some("link") if replies => Some("quote"),
        Some(kind) => Some(kind),
        None => match kind_by_keywords(&without_quoted(&n)) {
            Some(kind) => Some(kind),
            None if replies => Some("quote"),
            None if looks_app => Some("card"),
            None => None,
        },
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

/// The fields of [`extract_arkme_app_message_meta_in`] for a message `class` describes.
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
    if kind == Some("link") {
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
            // not `<songalbumurl>`, the cover, which is `musicAlbumUrl` (#1170)
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
            // an app message of a type it has no kind of, or whose XML names none
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
            // a quote of an app message, whose XML would make the reply look like one
            "<refermsg><type>49</type><svrid>9003</svrid><displayname>Bob</displayname><content>&lt;msg&gt;&lt;appmsg&gt;\
             &lt;title&gt;A&lt;/title&gt;&lt;des&gt;d&lt;/des&gt;&lt;type&gt;5&lt;/type&gt;&lt;/appmsg&gt;&lt;/msg&gt;</content></refermsg>",
            // a quote of an image, whose `<type>` would make it music
            "<refermsg><type>3</type><svrid>9002</svrid><displayname>Bob</displayname><content>&lt;msg&gt;&lt;img/&gt;&lt;/msg&gt;</content></refermsg>",
            "<refermsg><svrid>9005</svrid></refermsg>",
        ];
        for refer in refers {
            // an app message's quote of the same message has the same fields
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
            // a media reply's meta is its quote: the detailed JSON keeps it too
            let quote_or_link =
                whole.filter(|_| matches!(kind, Some("quote" | "link")) || is_non_app_type(t));
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
