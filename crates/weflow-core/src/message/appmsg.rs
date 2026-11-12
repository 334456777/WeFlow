//! App messages: their kind, the Arkme metadata of each kind and contact cards.

use serde_json::{json, Map, Value};

use super::link::{extract_html_link_card, normalize_http_link_url};
use super::parse_duration_seconds;
use super::xml::{
    extract_xml_attribute, extract_xml_value, normalize_app_message_content, rx, MsgXml,
};

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
