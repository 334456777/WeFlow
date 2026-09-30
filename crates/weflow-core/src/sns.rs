//! Moments (朋友圈) logic ported from `electron/services/snsService.ts`:
//! timeline enrichment, XML parsing, HTML / JSON export builders and media decryption helpers.
//! Network and database access live in `services/sns.rs`.

use std::sync::atomic::{AtomicU64, Ordering};

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyInit, KeyIvInit};
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce};
use md5::{Digest, Md5};
use regex::Regex;
use serde_json::{json, Map, Value};

use crate::message::rx;

// ───────────────────────── small JS-compat helpers ─────────────────────────

fn s_of(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_string)
}

/// `typeof value === 'string' && trim().length > 0 ? trimmed : undefined`
pub fn to_optional_string(v: Option<&Value>) -> Option<String> {
    let t = v?.as_str()?.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn opt_str(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// JS `a ?? b ?? c` over JSON values (null and missing both fall through).
fn coalesce<'a>(items: &[Option<&'a Value>]) -> Option<&'a Value> {
    items.iter().flatten().find(|v| !v.is_null()).copied()
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(true),
        Some(_) => true,
    }
}

/// JS `parseFloat` (leading numeric prefix).
pub fn parse_float(s: &str) -> Option<f64> {
    let m = rx(r"^\s*[+-]?(?:\d+\.?\d*(?:[eE][+-]?\d+)?|\.\d+(?:[eE][+-]?\d+)?)").find(s)?;
    m.as_str().trim().parse::<f64>().ok().filter(|f| f.is_finite())
}

/// JS `parseInt(x)` with NaN → `None`.
fn parse_int(s: &str) -> Option<i64> {
    let m = rx(r"^\s*[+-]?\d+").find(s)?;
    m.as_str().trim().parse::<i64>().ok()
}

fn to_optional_number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                parse_float(t)
            }
        }
        _ => None,
    }
}

fn num_value(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < 9.0e15 {
        json!(f as i64)
    } else {
        json!(f)
    }
}

pub fn decode_xml_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let t = rx(r"(?s)<!\[CDATA\[(.*?)\]\]>").replace_all(text, "$1").to_string();
    let t = rx(r"(?i)&amp;").replace_all(&t, "&").to_string();
    let t = rx(r"(?i)&lt;").replace_all(&t, "<").to_string();
    let t = rx(r"(?i)&gt;").replace_all(&t, ">").to_string();
    let t = rx(r"(?i)&quot;").replace_all(&t, "\"").to_string();
    rx(r"(?i)&#39;").replace_all(&t, "'").to_string()
}

static CMT_COUNTER: AtomicU64 = AtomicU64::new(0);

// ───────────────────────── URL / mime helpers ─────────────────────────

pub fn fix_sns_url(url: &str, token: Option<&str>, is_video: bool) -> String {
    if url.is_empty() {
        return url.to_string();
    }
    let mut fixed = url.replacen("http://", "https://", 1);
    if !is_video {
        fixed = rx(r"/150($|\?)").replace(&fixed, "/0$1").to_string();
    }
    let token = match token {
        Some(t) if !t.is_empty() => t,
        _ => return fixed,
    };
    if fixed.contains("token=") {
        return fixed;
    }
    if is_video {
        let mut parts = fixed.split('?');
        let base = parts.next().unwrap_or("");
        let existing = parts.next().map(|p| format!("&{p}")).unwrap_or_default();
        return format!("{base}?token={token}&idx=1{existing}");
    }
    let connector = if fixed.contains('?') { '&' } else { '?' };
    format!("{fixed}{connector}token={token}&idx=1")
}

pub fn is_video_url(url: &str) -> bool {
    if url.is_empty() || url.contains("vweixinthumb") {
        return false;
    }
    url.contains("snsvideodownload") || url.contains("video") || url.contains(".mp4")
}

pub fn detect_image_mime(buf: &[u8], fallback: &str) -> String {
    if buf.len() < 4 {
        return fallback.to_string();
    }
    if buf[..3] == [0xff, 0xd8, 0xff] {
        return "image/jpeg".into();
    }
    if buf.len() >= 8 && buf[..8] == [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a] {
        return "image/png".into();
    }
    if buf.len() >= 6 && (&buf[..6] == b"GIF87a" || &buf[..6] == b"GIF89a") {
        return "image/gif".into();
    }
    if buf.len() >= 12 && &buf[..4] == b"RIFF" && &buf[8..12] == b"WEBP" {
        return "image/webp".into();
    }
    if buf[0] == 0x42 && buf[1] == 0x4d {
        return "image/bmp".into();
    }
    if buf.len() > 12 && &buf[4..8] == b"ftyp" {
        let window = String::from_utf8_lossy(&buf[8..buf.len().min(64)]).to_lowercase();
        if window.contains("avif") || window.contains("avis") {
            return "image/avif".into();
        }
        if ["heic", "heix", "hevc", "hevx", "mif1", "msf1"].iter().any(|k| window.contains(k)) {
            return "image/heic".into();
        }
        return "video/mp4".into();
    }
    if fallback.contains("video") || fallback.contains("mp4") {
        return "video/mp4".into();
    }
    fallback.to_string()
}

/// Valid image header check used for emoji payloads.
pub fn is_valid_image_buffer(buf: &[u8]) -> bool {
    if buf.len() < 12 {
        return false;
    }
    if buf[..3] == [0x47, 0x49, 0x46] || buf[..4] == [0x89, 0x50, 0x4e, 0x47] || buf[..3] == [0xff, 0xd8, 0xff] {
        return true;
    }
    if &buf[..4] == b"RIFF" && &buf[8..12] == b"WEBP" {
        return true;
    }
    if &buf[4..8] == b"ftyp" {
        let window = String::from_utf8_lossy(&buf[8..buf.len().min(64)]).to_lowercase();
        return ["avif", "avis", "heic", "heix", "hevc", "hevx", "mif1", "msf1"].iter().any(|k| window.contains(k));
    }
    false
}

pub fn image_ext_from_buffer(buf: &[u8]) -> &'static str {
    if buf.len() >= 3 && buf[..3] == [0x47, 0x49, 0x46] {
        return ".gif";
    }
    if buf.len() >= 4 && buf[..4] == [0x89, 0x50, 0x4e, 0x47] {
        return ".png";
    }
    if buf.len() >= 3 && buf[..3] == [0xff, 0xd8, 0xff] {
        return ".jpg";
    }
    if buf.len() >= 12 && &buf[..4] == b"RIFF" && &buf[8..12] == b"WEBP" {
        return ".webp";
    }
    ".gif"
}

// ───────────────────────── XML parsing ─────────────────────────

pub fn extract_video_key(xml: &str) -> Option<String> {
    if xml.is_empty() {
        return None;
    }
    rx(r#"(?i)<enc\s+key="(\d+)""#).captures(xml).map(|c| c[1].to_string())
}

fn tag_value(block: &str, pattern: &str) -> Option<String> {
    rx(pattern).captures(block).map(|c| c[1].to_string())
}

/// `parseCommentsFromXml`: comments incl. emoji payloads and reply references.
pub fn parse_comments_from_xml(xml: &str) -> Vec<Value> {
    if xml.is_empty() {
        return Vec::new();
    }
    let list_patterns = [
        r"(?is)<CommentUserList>(.*?)</CommentUserList>",
        r"(?is)<commentUserList>(.*?)</commentUserList>",
        r"(?is)<commentList>(.*?)</commentList>",
        r"(?is)<comment_user_list>(.*?)</comment_user_list>",
    ];
    let list = match list_patterns.iter().find_map(|p| rx(p).captures(xml)) {
        Some(c) => c[1].to_string(),
        None => return Vec::new(),
    };

    struct Item {
        id: String,
        nickname: String,
        username: Option<String>,
        content: String,
        ref_comment_id: String,
        ref_username: Option<String>,
        ref_nickname: Option<String>,
        emojis: Vec<Value>,
    }
    let mut items: Vec<Item> = Vec::new();
    let item_re = rx(r"(?is)<(?:CommentUser|commentUser|comment|user_comment)>(.*?)</(?:CommentUser|commentUser|comment|user_comment)>");
    for m in item_re.captures_iter(&list) {
        let c = &m[1];
        let id = tag_value(c, r"(?is)<(?:cmtid|commentId|comment_id|id)>([^<]*)</(?:cmtid|commentId|comment_id|id)>");
        let username = tag_value(c, r"(?is)<username>([^<]*)</username>");
        let nickname = tag_value(c, r"(?is)<nickname>([^<]*)</nickname>").or_else(|| tag_value(c, r"(?is)<nickName>([^<]*)</nickName>"));
        let content = tag_value(c, r"(?is)<content>([^<]*)</content>");
        let ref_id = tag_value(c, r"(?is)<(?:refCommentId|replyCommentId|ref_comment_id)>([^<]*)</(?:refCommentId|replyCommentId|ref_comment_id)>");
        let ref_nick = tag_value(c, r"(?is)<(?:refNickname|refNickName|replyNickname)>([^<]*)</(?:refNickname|refNickName|replyNickname)>");
        let ref_user = tag_value(c, r"(?is)<ref_username>([^<]*)</ref_username>");

        let mut emojis = Vec::new();
        for em in rx(r"(?is)<emojiinfo>(.*?)</emojiinfo>").captures_iter(c) {
            let ex = &em[1];
            let url_raw = tag_value(ex, r"(?is)<extern_url>([^<]*)</extern_url>")
                .or_else(|| tag_value(ex, r"(?is)<cdn_url>([^<]*)</cdn_url>"))
                .or_else(|| tag_value(ex, r"(?is)<url>([^<]*)</url>"));
            let md5 = tag_value(ex, r"(?is)<md5>([^<]*)</md5>");
            let w = tag_value(ex, r"(?is)<width>([^<]*)</width>");
            let h = tag_value(ex, r"(?is)<height>([^<]*)</height>");
            let enc = tag_value(ex, r"(?is)<encrypt_url>([^<]*)</encrypt_url>");
            let aes = tag_value(ex, r"(?is)<aes_key>([^<]*)</aes_key>");
            let url = url_raw.map(|u| u.trim().replace("&amp;", "&")).unwrap_or_default();
            let encrypt_url = enc.map(|u| u.trim().replace("&amp;", "&"));
            let aes_key = aes.map(|a| a.trim().to_string());
            if !url.is_empty() || encrypt_url.as_deref().map_or(false, |u| !u.is_empty()) {
                let mut o = Map::new();
                o.insert("url".into(), json!(url));
                o.insert("md5".into(), json!(md5.map(|m| m.trim().to_string()).unwrap_or_default()));
                o.insert("width".into(), w.and_then(|w| parse_int(&w)).map(|n| json!(n)).unwrap_or(Value::Null));
                o.insert("height".into(), h.and_then(|h| parse_int(&h)).map(|n| json!(n)).unwrap_or(Value::Null));
                if let Some(e) = encrypt_url.filter(|e| !e.is_empty()) {
                    o.insert("encryptUrl".into(), json!(e));
                }
                if let Some(a) = aes_key.filter(|a| !a.is_empty()) {
                    o.insert("aesKey".into(), json!(a));
                }
                emojis.push(Value::Object(o));
            }
        }

        if nickname.is_some() && (content.is_some() || !emojis.is_empty()) {
            let ref_id = ref_id.map(|r| r.trim().to_string()).unwrap_or_default();
            items.push(Item {
                id: id.map(|i| i.trim().to_string()).unwrap_or_else(|| {
                    format!("cmt_{}_{}", chrono::Utc::now().timestamp_millis(), CMT_COUNTER.fetch_add(1, Ordering::Relaxed))
                }),
                nickname: nickname.map(|n| n.trim().to_string()).unwrap_or_default(),
                username: username.map(|u| u.trim().to_string()),
                content: content.map(|c| c.trim().to_string()).unwrap_or_default(),
                ref_comment_id: if ref_id == "0" { String::new() } else { ref_id },
                ref_username: ref_user.map(|u| u.trim().to_string()),
                ref_nickname: ref_nick.map(|u| u.trim().to_string()),
                emojis,
            });
        }
    }

    // second pass: fill refNickname from refUsername
    let user_map: std::collections::HashMap<String, String> = items
        .iter()
        .filter_map(|c| match (&c.username, c.nickname.as_str()) {
            (Some(u), n) if !u.is_empty() && !n.is_empty() => Some((u.clone(), n.to_string())),
            _ => None,
        })
        .collect();
    for c in items.iter_mut() {
        if c.ref_nickname.as_deref().map_or(true, str::is_empty) && !c.ref_comment_id.is_empty() {
            if let Some(ru) = c.ref_username.as_deref().filter(|u| !u.is_empty()) {
                c.ref_nickname = user_map.get(ru).cloned();
            }
        }
    }

    items
        .into_iter()
        .map(|c| {
            let mut o = Map::new();
            o.insert("id".into(), json!(c.id));
            o.insert("nickname".into(), json!(c.nickname));
            if let Some(u) = c.username {
                o.insert("username".into(), json!(u));
            }
            o.insert("content".into(), json!(c.content));
            o.insert("refCommentId".into(), json!(c.ref_comment_id));
            if let Some(u) = c.ref_username {
                o.insert("refUsername".into(), json!(u));
            }
            if let Some(n) = c.ref_nickname {
                o.insert("refNickname".into(), json!(n));
            }
            if !c.emojis.is_empty() {
                o.insert("emojis".into(), Value::Array(c.emojis));
            }
            Value::Object(o)
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct LikeUser {
    pub username: Option<String>,
    pub nickname: Option<String>,
}

fn like_list(xml: &str) -> Option<String> {
    [
        r"(?is)<LikeUserList>(.*?)</LikeUserList>",
        r"(?is)<likeUserList>(.*?)</likeUserList>",
        r"(?is)<likeList>(.*?)</likeList>",
        r"(?is)<like_user_list>(.*?)</like_user_list>",
    ]
    .iter()
    .find_map(|p| rx(p).captures(xml))
    .map(|c| c[1].to_string())
}

const LIKE_ITEM: &str = r"(?is)<(?:LikeUser|likeUser|user_comment)>(.*?)</(?:LikeUser|likeUser|user_comment)>";

pub fn parse_like_users_from_xml(xml: &str) -> Vec<LikeUser> {
    if xml.is_empty() {
        return Vec::new();
    }
    let Some(list) = like_list(xml) else { return Vec::new() };
    let mut out = Vec::new();
    for m in rx(LIKE_ITEM).captures_iter(&list) {
        let block = &m[1];
        let username = tag_value(block, r"(?is)<username>([^<]*)</username>").and_then(|u| opt_str(&u));
        let nickname = tag_value(block, r"(?is)<nickname>([^<]*)</nickname>")
            .and_then(|n| opt_str(&n))
            .or_else(|| tag_value(block, r"(?is)<nickName>([^<]*)</nickName>").and_then(|n| opt_str(&n)));
        if username.is_some() || nickname.is_some() {
            out.push(LikeUser { username, nickname });
        }
    }
    out
}

/// Legacy nickname-only like parser (`parseLikesFromXml`).
pub fn parse_likes_from_xml(xml: &str) -> Vec<String> {
    if xml.is_empty() {
        return Vec::new();
    }
    let Some(list) = like_list(xml) else { return Vec::new() };
    rx(LIKE_ITEM)
        .captures_iter(&list)
        .filter_map(|m| {
            tag_value(&m[1], r"(?is)<nickname>([^<]*)</nickname>")
                .or_else(|| tag_value(&m[1], r"(?is)<nickName>([^<]*)</nickName>"))
                .map(|n| n.trim().to_string())
        })
        .collect()
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    Regex::new(&format!(r#"(?i){}="([^"]+)""#, regex::escape(name))).ok()?.captures(attrs).map(|c| c[1].to_string())
}

/// `parseMediaFromXml` (only used when the DLL does not hand back parsed media).
pub fn parse_media_from_xml(xml: &str) -> (Vec<Value>, Option<String>) {
    let mut media = Vec::new();
    if xml.is_empty() {
        return (media, None);
    }
    let video_key = extract_video_key(xml);
    for m in rx(r"(?is)<media>(.*?)</media>").captures_iter(xml) {
        let mx = &m[1];
        let url = tag_value(mx, r"(?i)<url[^>]*>([^<]+)</url>");
        let url_attrs = tag_value(mx, r"(?i)<url([^>]*)>").unwrap_or_default();
        let thumb = tag_value(mx, r"(?i)<thumb[^>]*>([^<]+)</thumb>");
        let thumb_attrs = tag_value(mx, r"(?i)<thumb([^>]*)>").unwrap_or_default();

        let (url_token, url_key, url_md5, url_enc) =
            (attr(&url_attrs, "token"), attr(&url_attrs, "key"), attr(&url_attrs, "md5"), attr(&url_attrs, "enc_idx"));
        let (thumb_token, thumb_key, thumb_enc) = (attr(&thumb_attrs, "token"), attr(&thumb_attrs, "key"), attr(&thumb_attrs, "enc_idx"));

        let mut item = Map::new();
        item.insert("url".into(), json!(url.map(|u| u.trim().to_string()).unwrap_or_default()));
        item.insert("thumb".into(), json!(thumb.map(|u| u.trim().to_string()).unwrap_or_default()));
        for (k, v) in [
            ("token", url_token.or(thumb_token)),
            ("key", url_key.or(thumb_key)),
            ("md5", url_md5),
            ("encIdx", url_enc.or(thumb_enc)),
        ] {
            if let Some(v) = v {
                item.insert(k.into(), json!(v));
            }
        }

        if let Some(lp) = rx(r"(?is)<livePhoto>(.*?)</livePhoto>").captures(mx) {
            let lx = &lp[1];
            let lp_url = tag_value(lx, r"(?i)<url[^>]*>([^<]+)</url>");
            let lp_url_attrs = tag_value(lx, r"(?i)<url([^>]*)>").unwrap_or_default();
            let lp_thumb = tag_value(lx, r"(?i)<thumb[^>]*>([^<]+)</thumb>");
            let lp_thumb_attrs = tag_value(lx, r"(?i)<thumb([^>]*)>").unwrap_or_default();
            let token = attr(&lp_url_attrs, "token").or_else(|| attr(&lp_thumb_attrs, "token"));
            let key = attr(&lp_url_attrs, "key").or_else(|| attr(&lp_thumb_attrs, "key"));
            let enc = attr(&lp_url_attrs, "enc_idx");
            let mut live = Map::new();
            live.insert("url".into(), json!(lp_url.map(|u| u.trim().to_string()).unwrap_or_default()));
            live.insert("thumb".into(), json!(lp_thumb.map(|u| u.trim().to_string()).unwrap_or_default()));
            for (k, v) in [("token", token), ("key", key), ("encIdx", enc)] {
                if let Some(v) = v {
                    live.insert(k.into(), json!(v));
                }
            }
            item.insert("livePhoto".into(), Value::Object(live));
        }
        media.push(Value::Object(item));
    }
    (media, video_key)
}

// ───────────────────────── location ─────────────────────────

const LOCATION_FIELDS: [&str; 8] = ["latitude", "longitude", "city", "country", "poiName", "poiAddress", "poiAddressName", "label"];

fn build_location(values: Vec<(&str, Option<Value>)>) -> Option<Value> {
    let mut o = Map::new();
    for key in LOCATION_FIELDS {
        if let Some((_, Some(v))) = values.iter().find(|(k, _)| *k == key) {
            o.insert(key.into(), v.clone());
        }
    }
    (!o.is_empty()).then(|| Value::Object(o))
}

pub fn normalize_location(input: Option<&Value>) -> Option<Value> {
    let row = input?.as_object()?;
    let text = |keys: &[&str]| -> Option<Value> {
        let v = keys.iter().filter_map(|k| row.get(*k)).find(|v| !v.is_null())?;
        let s = v.as_str()?;
        to_optional_string(Some(&json!(decode_xml_text(s)))).map(Value::from)
    };
    let number = |keys: &[&str]| -> Option<Value> {
        let vals: Vec<Option<&Value>> = keys.iter().map(|k| row.get(*k)).collect();
        to_optional_number(coalesce(&vals)).map(num_value)
    };
    build_location(vec![
        ("latitude", number(&["latitude", "lat", "x"])),
        ("longitude", number(&["longitude", "lng", "y"])),
        ("city", text(&["city"])),
        ("country", text(&["country"])),
        ("poiName", text(&["poiName", "poiname"])),
        ("poiAddress", text(&["poiAddress", "poiaddress"])),
        ("poiAddressName", text(&["poiAddressName", "poiaddressname"])),
        ("label", text(&["label"])),
    ])
}

pub fn parse_location_from_xml(xml: &str) -> Option<Value> {
    if xml.is_empty() {
        return None;
    }
    let attrs = rx(r"(?i)<location\b([^>]*)>").captures(xml).map(|c| c[1].to_string()).unwrap_or_default();
    let read_attr = |name: &str| -> Option<String> {
        if attrs.is_empty() {
            return None;
        }
        let re = Regex::new(&format!(r#"(?i){}\s*=\s*["']([\s\S]*?)["']"#, regex::escape(name))).ok()?;
        let m = re.captures(&attrs)?;
        if m[1].is_empty() {
            return None;
        }
        opt_str(&decode_xml_text(&m[1]))
    };
    let read_tag = |name: &str| -> Option<String> {
        let re = Regex::new(&format!(r"(?i)<{0}>([\s\S]*?)</{0}>", regex::escape(name))).ok()?;
        let m = re.captures(xml)?;
        if m[1].is_empty() {
            return None;
        }
        opt_str(&decode_xml_text(&m[1]))
    };
    let first = |names: &[&str]| -> Option<String> {
        for n in names {
            if let Some(v) = read_attr(n) {
                return Some(v);
            }
        }
        for n in names {
            if let Some(v) = read_tag(n) {
                return Some(v);
            }
        }
        None
    };
    // JS order is attr(a) || attr(b) || tag(a) || tag(b)
    let lat = first(&["latitude", "x"]);
    let lng = first(&["longitude", "y"]);
    let num = |s: Option<String>| s.and_then(|s| parse_float(&s)).map(num_value);
    build_location(vec![
        ("latitude", num(lat)),
        ("longitude", num(lng)),
        ("city", first(&["city"]).map(Value::from)),
        ("country", first(&["country"]).map(Value::from)),
        ("poiName", first(&["poiName", "poiname"]).map(Value::from)),
        ("poiAddress", first(&["poiAddress", "poiaddress"]).map(Value::from)),
        ("poiAddressName", first(&["poiAddressName", "poiaddressname"]).map(Value::from)),
        ("label", first(&["label"]).map(Value::from)),
    ])
}

pub fn merge_location(primary: Option<&Value>, fallback: Option<&Value>) -> Option<Value> {
    if primary.is_none() && fallback.is_none() {
        return None;
    }
    let pick = |k: &str| -> Option<Value> {
        primary.and_then(|p| p.get(k)).filter(|v| !v.is_null()).or_else(|| fallback.and_then(|f| f.get(k)).filter(|v| !v.is_null())).cloned()
    };
    build_location(LOCATION_FIELDS.iter().map(|k| (*k, pick(k))).collect())
}

// ───────────────────────── comments / timeline enrichment ─────────────────────────

pub fn fix_comment_refs(comments: &[Value]) -> Vec<Value> {
    let mut id_to_nick = std::collections::HashMap::new();
    for (idx, c) in comments.iter().enumerate() {
        let nick = s_of(c.get("nickname")).unwrap_or_default();
        if let Some(id) = s_of(c.get("id")).filter(|i| !i.is_empty()) {
            id_to_nick.insert(id, nick.clone());
        }
        id_to_nick.insert((idx + 1).to_string(), nick);
    }
    comments
        .iter()
        .map(|c| {
            let ref_id = s_of(c.get("refCommentId")).unwrap_or_default();
            let mut ref_nick = s_of(c.get("refNickname")).unwrap_or_default();
            if !ref_id.is_empty() && ref_id != "0" && ref_nick.is_empty() {
                ref_nick = id_to_nick.get(&ref_id).cloned().unwrap_or_default();
            }
            let emojis: Vec<Value> = c
                .get("emojis")
                .and_then(Value::as_array)
                .map(|a| a.as_slice())
                .unwrap_or(&[])
                .iter()
                .filter(|e| truthy(e.get("url")) || truthy(e.get("encryptUrl")))
                .map(|e| {
                    let mut o = Map::new();
                    o.insert("url".into(), json!(s_of(e.get("url")).unwrap_or_default().replace("&amp;", "&")));
                    o.insert("md5".into(), json!(s_of(e.get("md5")).unwrap_or_default()));
                    o.insert("width".into(), if truthy(e.get("width")) { e["width"].clone() } else { json!(0) });
                    o.insert("height".into(), if truthy(e.get("height")) { e["height"].clone() } else { json!(0) });
                    if let Some(u) = s_of(e.get("encryptUrl")).filter(|u| !u.is_empty()) {
                        o.insert("encryptUrl".into(), json!(u.replace("&amp;", "&")));
                    }
                    if let Some(a) = s_of(e.get("aesKey")).filter(|a| !a.is_empty()) {
                        o.insert("aesKey".into(), json!(a));
                    }
                    Value::Object(o)
                })
                .collect();
            let mut o = Map::new();
            o.insert("id".into(), json!(s_of(c.get("id")).unwrap_or_default()));
            o.insert("nickname".into(), json!(s_of(c.get("nickname")).unwrap_or_default()));
            o.insert("content".into(), json!(s_of(c.get("content")).unwrap_or_default()));
            o.insert("refCommentId".into(), json!(if ref_id == "0" { String::new() } else { ref_id }));
            o.insert("refNickname".into(), json!(ref_nick));
            if !emojis.is_empty() {
                o.insert("emojis".into(), Value::Array(emojis));
            }
            Value::Object(o)
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct CachedContact {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

fn set_or_remove(o: &mut Map<String, Value>, key: &str, v: Option<Value>) {
    match v {
        Some(v) => {
            o.insert(key.into(), v);
        }
        None => {
            o.shift_remove(key);
        }
    }
}

fn str_or_none(s: String) -> Option<Value> {
    Some(Value::String(s))
}

/// `getTimeline` post-processing of one raw DLL post.
pub fn enrich_post(post: &Value, contact: Option<&CachedContact>) -> Value {
    let mut out = post.as_object().cloned().unwrap_or_default();
    let is_video_post = post.get("type").and_then(Value::as_i64) == Some(15);
    let raw_xml = s_of(post.get("rawXml")).unwrap_or_default();
    let video_key = extract_video_key(&raw_xml);
    let location = merge_location(normalize_location(post.get("location")).as_ref(), parse_location_from_xml(&raw_xml).as_ref());

    let fixed_media: Vec<Value> = post
        .get("media")
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .map(|m| {
            let token = s_of(m.get("token"));
            let mut o = Map::new();
            o.insert("url".into(), json!(fix_sns_url(&s_of(m.get("url")).unwrap_or_default(), token.as_deref(), is_video_post)));
            o.insert("thumb".into(), json!(fix_sns_url(&s_of(m.get("thumb")).unwrap_or_default(), token.as_deref(), false)));
            for (k, v) in [
                ("md5", m.get("md5").filter(|v| !v.is_null()).cloned()),
                ("token", m.get("token").filter(|v| !v.is_null()).cloned()),
                (
                    "key",
                    if is_video_post {
                        video_key.clone().map(Value::from).or_else(|| m.get("key").filter(|v| !v.is_null()).cloned())
                    } else {
                        m.get("key").filter(|v| !v.is_null()).cloned()
                    },
                ),
                ("encIdx", coalesce(&[m.get("encIdx"), m.get("enc_idx")]).filter(|v| truthy(Some(v))).cloned()),
            ] {
                if let Some(v) = v {
                    o.insert(k.into(), v);
                }
            }
            if let Some(lp) = m.get("livePhoto").filter(|v| v.is_object()) {
                let mut live = lp.as_object().cloned().unwrap_or_default();
                let lp_token = s_of(lp.get("token"));
                live.insert("url".into(), json!(fix_sns_url(&s_of(lp.get("url")).unwrap_or_default(), lp_token.as_deref(), true)));
                live.insert("thumb".into(), json!(fix_sns_url(&s_of(lp.get("thumb")).unwrap_or_default(), lp_token.as_deref(), false)));
                set_or_remove(&mut live, "token", lp.get("token").filter(|v| !v.is_null()).cloned());
                let key = video_key
                    .clone()
                    .map(Value::from)
                    .or_else(|| lp.get("key").filter(|v| truthy(Some(v))).cloned())
                    .or_else(|| m.get("key").filter(|v| !v.is_null()).cloned());
                set_or_remove(&mut live, "key", key);
                set_or_remove(&mut live, "encIdx", coalesce(&[lp.get("encIdx"), lp.get("enc_idx")]).filter(|v| truthy(Some(v))).cloned());
                o.insert("livePhoto".into(), Value::Object(live));
            }
            Value::Object(o)
        })
        .collect();

    let dll_comments: Vec<Value> = post.get("comments").and_then(Value::as_array).cloned().unwrap_or_default();
    let has_emojis_in_dll = dll_comments.iter().any(|c| c.get("emojis").and_then(Value::as_array).map_or(false, |e| !e.is_empty()));
    let final_comments = if !dll_comments.is_empty() && (has_emojis_in_dll || raw_xml.is_empty()) {
        fix_comment_refs(&dll_comments)
    } else if !raw_xml.is_empty() {
        let xml_comments = parse_comments_from_xml(&raw_xml);
        if xml_comments.is_empty() {
            fix_comment_refs(&dll_comments)
        } else {
            xml_comments
        }
    } else {
        fix_comment_refs(&dll_comments)
    };

    let username = s_of(post.get("username")).unwrap_or_default();
    let nickname = s_of(post.get("nickname")).filter(|n| !n.is_empty()).or_else(|| contact.and_then(|c| c.display_name.clone()).filter(|n| !n.is_empty())).unwrap_or(username);

    set_or_remove(&mut out, "avatarUrl", contact.and_then(|c| c.avatar_url.clone()).map(Value::from));
    set_or_remove(&mut out, "nickname", str_or_none(nickname));
    out.insert("media".into(), Value::Array(fixed_media));
    out.insert("comments".into(), Value::Array(final_comments));
    set_or_remove(&mut out, "location", location);
    Value::Object(out)
}

/// `pickTimelineUsername`
pub fn pick_timeline_username(post: &Value) -> String {
    for k in ["username", "user_name", "userName"] {
        if let Some(v) = post.get(k).filter(|v| !v.is_null()) {
            return v.as_str().map(|s| s.trim().to_string()).unwrap_or_default();
        }
    }
    String::new()
}

// ───────────────────────── HTML export ─────────────────────────

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\n', "<br>")
}

fn normalize_location_text(v: Option<&Value>) -> String {
    let raw = s_of(v).unwrap_or_default();
    rx(r"\s+").replace_all(&decode_xml_text(&raw), " ").trim().to_string()
}

fn resolve_location_text(location: Option<&Value>) -> String {
    let Some(loc) = location.filter(|l| l.is_object()) else { return String::new() };
    let primary = ["poiName", "poiAddressName", "label", "poiAddress"]
        .iter()
        .map(|k| normalize_location_text(loc.get(*k)))
        .find(|s| !s.is_empty())
        .unwrap_or_default();
    let region = ["country", "city"].iter().map(|k| normalize_location_text(loc.get(*k))).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    if !primary.is_empty() && !region.is_empty() && !primary.contains(&region) {
        return format!("{primary} · {region}");
    }
    if primary.is_empty() {
        region
    } else {
        primary
    }
}

fn format_post_time(ts: i64, now: chrono::DateTime<chrono::Local>) -> String {
    use chrono::{Datelike, TimeZone, Timelike};
    let d = chrono::Local.timestamp_opt(ts, 0).single().unwrap_or(now);
    let time = format!("{:02}:{:02}", d.hour(), d.minute());
    if d.year() == now.year() {
        format!("{}月{}日 {}", d.month(), d.day(), time)
    } else {
        format!("{}年{}月{}日 {}", d.year(), d.month(), d.day(), time)
    }
}

/// `toLocaleString('zh-CN')` → `2026/9/30 15:04:05`
pub fn zh_locale_string(dt: chrono::DateTime<chrono::Local>) -> String {
    use chrono::{Datelike, Timelike};
    format!("{}/{}/{} {:02}:{:02}:{:02}", dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), dt.second())
}

pub fn zh_locale_from_ts(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(ts, 0).single().map(zh_locale_string).unwrap_or_default()
}

const HTML_STYLE: &str = include_str!("sns_export.css");

pub fn generate_html(posts: &[Value], usernames: &[String], keyword: Option<&str>, avatar_map: &std::collections::HashMap<String, String>) -> String {
    let now = chrono::Local::now();
    let mut filter_info = String::new();
    if let Some(k) = keyword.filter(|k| !k.is_empty()) {
        filter_info.push_str(&format!("关键词: \"{}\" ", escape_html(k)));
    }
    if !usernames.is_empty() {
        filter_info.push_str(&format!("筛选用户: {} 人", usernames.len()));
    }

    let posts_html: Vec<String> = posts
        .iter()
        .map(|post| {
            let media = post.get("media").and_then(Value::as_array).cloned().unwrap_or_default();
            let grid = match media.len() {
                1 => "grid-1",
                2 | 4 => "grid-2",
                _ => "grid-3",
            };
            let media_html: String = media
                .iter()
                .map(|m| {
                    let url = s_of(m.get("url")).unwrap_or_default();
                    match s_of(m.get("localPath")).filter(|p| !p.is_empty()) {
                        Some(local) if is_video_url(&url) => {
                            format!("<div class=\"mi\"><video src=\"{}\" controls preload=\"metadata\"></video></div>", escape_html(&local))
                        }
                        Some(local) => format!("<div class=\"mi\"><img src=\"{}\" loading=\"lazy\" onclick=\"openLb(this.src)\" alt=\"\"></div>", escape_html(&local)),
                        None => format!("<div class=\"mi ml\"><a href=\"{}\" target=\"_blank\">查看媒体</a></div>", escape_html(&url)),
                    }
                })
                .collect();

            let link_title = s_of(post.get("linkTitle")).unwrap_or_default();
            let link_url = s_of(post.get("linkUrl")).unwrap_or_default();
            let link_html = if !link_title.is_empty() && !link_url.is_empty() {
                format!(
                    "<a class=\"lk\" href=\"{}\" target=\"_blank\"><span class=\"lk-t\">{}</span><span class=\"lk-a\">›</span></a>",
                    escape_html(&link_url),
                    escape_html(&link_title)
                )
            } else {
                String::new()
            };
            let loc_text = resolve_location_text(post.get("location"));
            let loc_html = if loc_text.is_empty() {
                String::new()
            } else {
                format!("<div class=\"loc\"><span class=\"loc-i\">📍</span><span class=\"loc-t\">{}</span></div>", escape_html(&loc_text))
            };

            let likes: Vec<String> = post.get("likes").and_then(Value::as_array).map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let likes_html = if likes.is_empty() {
                String::new()
            } else {
                format!(
                    "<div class=\"interactions\"><div class=\"likes\">♥ {}</div></div>",
                    likes.iter().map(|l| format!("<span>{}</span>", escape_html(l))).collect::<Vec<_>>().join("、")
                )
            };

            let comments = post.get("comments").and_then(Value::as_array).cloned().unwrap_or_default();
            let comments_html = if comments.is_empty() {
                String::new()
            } else {
                let inner: String = comments
                    .iter()
                    .map(|c| {
                        let ref_nick = s_of(c.get("refNickname")).unwrap_or_default();
                        let r = if ref_nick.is_empty() { String::new() } else { format!("<span class=\"re\">回复</span><b>{}</b>", escape_html(&ref_nick)) };
                        format!(
                            "<div class=\"cmt\"><b>{}</b>{}：{}</div>",
                            escape_html(&s_of(c.get("nickname")).unwrap_or_default()),
                            r,
                            escape_html(&s_of(c.get("content")).unwrap_or_default())
                        )
                    })
                    .collect();
                format!("<div class=\"interactions{}\"><div class=\"cmts\">{}</div></div>", if likes.is_empty() { "" } else { " cmt-border" }, inner)
            };

            let username = s_of(post.get("username")).unwrap_or_default();
            let nickname = s_of(post.get("nickname")).unwrap_or_default();
            let avatar_html = match avatar_map.get(&username) {
                Some(src) => format!("<div class=\"avatar\"><img src=\"{}\" alt=\"\"></div>", escape_html(src)),
                None => {
                    let ch = nickname.chars().next().map(|c| c.to_string()).unwrap_or_else(|| "?".into());
                    format!("<div class=\"avatar\">{}</div>", escape_html(&ch))
                }
            };
            let content_desc = s_of(post.get("contentDesc")).unwrap_or_default();
            let create_time = post.get("createTime").and_then(Value::as_i64).unwrap_or(0);

            format!(
                "<div class=\"post\">\n{avatar}\n<div class=\"body\">\n<div class=\"hd\"><span class=\"nick\">{nick}</span><span class=\"tm\">{tm}</span></div>\n{txt}\n{loc}\n{mg}\n{lk}\n{likes}\n{cmts}\n</div></div>",
                avatar = avatar_html,
                nick = escape_html(&nickname),
                tm = format_post_time(create_time, now),
                txt = if content_desc.is_empty() { String::new() } else { format!("<div class=\"txt\">{}</div>", escape_html(&content_desc)) },
                loc = loc_html,
                mg = if media_html.is_empty() { String::new() } else { format!("<div class=\"mg {grid}\">{media_html}</div>") },
                lk = link_html,
                likes = likes_html,
                cmts = comments_html,
            )
        })
        .collect();

    format!(
        "<!DOCTYPE html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"UTF-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n<title>朋友圈导出</title>\n<style>\n{style}</style>\n</head>\n<body>\n<div class=\"container\">\n    <div class=\"feed-hd\"><h2>朋友圈</h2><span class=\"info\">共 {count} 条{filter}</span></div>\n    {posts}\n    <div class=\"ft\">由 WeFlow 导出 · {now}</div>\n</div>\n<div class=\"lb\" id=\"lb\" onclick=\"closeLb()\"><img id=\"lbi\" src=\"\"></div>\n<button class=\"btt\" id=\"btt\" onclick=\"scrollTo({{top:0,behavior:'smooth'}})\">↑</button>\n<script>\nfunction openLb(s){{document.getElementById('lbi').src=s;document.getElementById('lb').classList.add('on');document.body.style.overflow='hidden'}}\nfunction closeLb(){{document.getElementById('lb').classList.remove('on');document.body.style.overflow=''}}\ndocument.addEventListener('keydown',function(e){{if(e.key==='Escape')closeLb()}})\nwindow.addEventListener('scroll',function(){{document.getElementById('btt').classList.toggle('show',window.scrollY>600)}})\n</script>\n</body>\n</html>",
        style = HTML_STYLE,
        count = posts.len(),
        filter = if filter_info.is_empty() { String::new() } else { format!(" · {filter_info}") },
        posts = posts_html.join("\n"),
        now = zh_locale_string(now),
    )
}

// ───────────────────────── emoji decryption ─────────────────────────

fn md5_bytes(data: &[u8]) -> Vec<u8> {
    let mut h = Md5::new();
    h.update(data);
    h.finalize().to_vec()
}

/// Lenient base64 (Node's `Buffer.from(x, 'base64')` skips unknown characters and stops at `=`).
pub fn lenient_base64(input: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in input.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' | '-' => 62,
            '/' | '_' => 63,
            '=' => break,
            _ => continue,
        };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    out
}

fn build_key_tries(aes_key: &str) -> Vec<Vec<u8>> {
    let mut tries = Vec::new();
    let hex: String = aes_key.chars().filter(|c| !c.is_whitespace()).collect();
    if hex.len() >= 32 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let first32 = &hex[..32];
        let decoded: Vec<u8> = (0..16).filter_map(|i| u8::from_str_radix(&first32[i * 2..i * 2 + 2], 16).ok()).collect();
        if decoded.len() == 16 {
            tries.push(decoded);
        }
        tries.push(first32.as_bytes().to_vec());
    }
    if aes_key.chars().count() >= 16 {
        let bytes = aes_key.as_bytes();
        tries.push(bytes[..16.min(bytes.len())].to_vec());
    }
    tries.push(md5_bytes(aes_key.as_bytes()));
    let b64 = lenient_base64(aes_key);
    if b64.len() >= 16 {
        tries.push(b64[..16].to_vec());
    }
    tries
}

fn inflate_variants(data: &[u8]) -> Vec<Vec<u8>> {
    use flate2::read::{GzDecoder, ZlibDecoder};
    use std::io::Read;
    let mut out = Vec::new();
    let mut buf = Vec::new();
    if ZlibDecoder::new(data).read_to_end(&mut buf).is_ok() {
        out.push(std::mem::take(&mut buf));
    }
    let mut buf = Vec::new();
    if GzDecoder::new(data).read_to_end(&mut buf).is_ok() {
        out.push(buf);
    }
    out
}

fn try_gcm_decrypt(key: &[u8], nonce: &[u8], ciphertext: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
    if nonce.len() != 12 {
        return None;
    }
    let mut joined = Vec::with_capacity(ciphertext.len() + tag.len());
    joined.extend_from_slice(ciphertext);
    joined.extend_from_slice(tag);
    let payload = Payload { msg: &joined, aad: &[] };
    let n = Nonce::from_slice(nonce);
    let decrypted = match key.len() {
        16 => Aes128Gcm::new_from_slice(key).ok()?.decrypt(n, payload).ok()?,
        32 => Aes256Gcm::new_from_slice(key).ok()?.decrypt(n, payload).ok()?,
        _ => return None,
    };
    if is_valid_image_buffer(&decrypted) {
        return Some(decrypted);
    }
    for d in inflate_variants(&decrypted) {
        if is_valid_image_buffer(&d) {
            return Some(d);
        }
    }
    Some(decrypted)
}

struct GcmLayout<'a> {
    nonce: Vec<u8>,
    ciphertext: &'a [u8],
    tag: &'a [u8],
}

fn build_gcm_layouts(enc: &[u8]) -> Vec<GcmLayout<'_>> {
    let mut layouts = Vec::new();
    let n = enc.len();
    if n > 63 && enc[0] == 0xab && enc[8] == 0xab && enc[9] == 0x00 {
        let payload_size = u32::from_le_bytes([enc[10], enc[11], enc[12], enc[13]]) as usize;
        if payload_size > 16 && 63 + payload_size <= n {
            let payload = &enc[63..63 + payload_size];
            layouts.push(GcmLayout { nonce: enc[19..31].to_vec(), ciphertext: &payload[..payload.len() - 16], tag: &payload[payload.len() - 16..] });
        }
    }
    if n > 28 {
        layouts.push(GcmLayout { ciphertext: &enc[..n - 28], nonce: enc[n - 28..n - 16].to_vec(), tag: &enc[n - 16..] });
        layouts.push(GcmLayout { nonce: enc[..12].to_vec(), ciphertext: &enc[12..n - 16], tag: &enc[n - 16..] });
    }
    if n > 16 {
        layouts.push(GcmLayout { nonce: vec![0; 12], ciphertext: &enc[..n - 16], tag: &enc[n - 16..] });
    }
    if n > 28 {
        layouts.push(GcmLayout { nonce: enc[..12].to_vec(), tag: &enc[12..28], ciphertext: &enc[28..] });
    }
    layouts
}

/// `decryptEmojiAes`: GCM layouts → CBC/ECB fallbacks across several key derivations.
pub fn decrypt_emoji_aes(enc: &[u8], aes_key: &str) -> Option<Vec<u8>> {
    if enc.len() <= 16 {
        return None;
    }
    let keys = build_key_tries(aes_key);
    let usable = |k: &&Vec<u8>| k.len() == 16 || k.len() == 32;
    let n = enc.len();
    let (tag, ciphertext) = (&enc[n - 16..], &enc[..n - 16]);

    if n > 28 {
        for key in keys.iter().filter(usable) {
            if let Some(r) = try_gcm_decrypt(key, &enc[n - 28..n - 16], &enc[..n - 28], &enc[n - 16..]) {
                return Some(r);
            }
        }
    }
    for key in keys.iter().filter(usable) {
        if let Some(r) = try_gcm_decrypt(key, &key[..12], ciphertext, tag) {
            return Some(r);
        }
    }
    for layout in build_gcm_layouts(enc) {
        for key in keys.iter().filter(usable) {
            if let Some(r) = try_gcm_decrypt(key, &layout.nonce, layout.ciphertext, layout.tag) {
                return Some(r);
            }
        }
    }

    type CbcDec = cbc::Decryptor<aes::Aes128>;
    type EcbDec = ecb::Decryptor<aes::Aes128>;
    for key in keys.iter().filter(|k| k.len() == 16) {
        if n >= 16 && n % 16 == 0 {
            let mut buf = enc.to_vec();
            if let Ok(dec) = CbcDec::new_from_slices(key, key) {
                if let Ok(plain) = dec.decrypt_padded_mut::<Pkcs7>(&mut buf) {
                    if is_valid_image_buffer(plain) {
                        return Some(plain.to_vec());
                    }
                    for d in inflate_variants(plain) {
                        if is_valid_image_buffer(&d) {
                            return Some(d);
                        }
                    }
                }
            }
        }
        if n > 32 {
            let mut buf = enc[16..].to_vec();
            if buf.len() % 16 == 0 {
                if let Ok(dec) = CbcDec::new_from_slices(key, &enc[..16]) {
                    if let Ok(plain) = dec.decrypt_padded_mut::<Pkcs7>(&mut buf) {
                        if is_valid_image_buffer(plain) {
                            return Some(plain.to_vec());
                        }
                    }
                }
            }
        }
        if n % 16 == 0 {
            let mut buf = enc.to_vec();
            if let Ok(plain) = EcbDec::new_from_slice(key).map(|d| d.decrypt_padded_mut::<Pkcs7>(&mut buf)) {
                if let Ok(plain) = plain {
                    if is_valid_image_buffer(plain) {
                        return Some(plain.to_vec());
                    }
                }
            }
        }
    }
    None
}

// ───────────────────────── tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_urls_like_the_desktop_app() {
        assert_eq!(fix_sns_url("http://x.qq.com/a/150", Some("T"), false), "https://x.qq.com/a/0?token=T&idx=1");
        assert_eq!(fix_sns_url("http://x.qq.com/v?a=1", Some("T"), true), "https://x.qq.com/v?token=T&idx=1&a=1");
        assert_eq!(fix_sns_url("https://x/a?token=Z", Some("T"), false), "https://x/a?token=Z");
        assert_eq!(fix_sns_url("http://x/a/150?q=1", None, false), "https://x/a/0?q=1");
        assert!(is_video_url("https://x/snsvideodownload?a") && !is_video_url("https://vweixinthumb.x/video"));
    }

    #[test]
    fn detects_mime() {
        assert_eq!(detect_image_mime(&[0xff, 0xd8, 0xff, 0xe0], ""), "image/jpeg");
        assert_eq!(detect_image_mime(b"GIF89a....", ""), "image/gif");
        assert_eq!(detect_image_mime(b"\0\0\0\x18ftypmp42abcd", ""), "video/mp4");
        assert_eq!(detect_image_mime(b"abcd", "image/png"), "image/png");
    }

    const COMMENT_XML: &str = "<CommentUserList><CommentUser><cmtid>1</cmtid><username>wxid_a</username><nickname>A</nickname><content>hi</content></CommentUser>\
<CommentUser><cmtid>2</cmtid><username>wxid_b</username><nickname>B</nickname><content>yo</content><refCommentId>1</refCommentId><ref_username>wxid_a</ref_username>\
<emojiinfo><cdn_url>http://e/x?a=1&amp;b=2</cdn_url><md5>m5</md5><width>10</width><height>20</height><aes_key>k</aes_key></emojiinfo></CommentUser></CommentUserList>";

    #[test]
    fn parses_comments_with_refs_and_emojis() {
        let c = parse_comments_from_xml(COMMENT_XML);
        assert_eq!(c.len(), 2);
        assert_eq!(c[1]["refNickname"], "A");
        assert_eq!(c[1]["emojis"][0]["url"], "http://e/x?a=1&b=2");
        assert_eq!(c[1]["emojis"][0]["width"], 10);
        assert_eq!(c[1]["emojis"][0]["aesKey"], "k");
    }

    #[test]
    fn parses_likes_and_location() {
        let xml = "<LikeUserList><LikeUser><username>u1</username><nickname>N1</nickname></LikeUser><LikeUser><nickname>N2</nickname></LikeUser></LikeUserList>";
        let likes = parse_like_users_from_xml(xml);
        assert_eq!(likes.len(), 2);
        assert_eq!(likes[0].username.as_deref(), Some("u1"));
        assert_eq!(parse_likes_from_xml(xml), vec!["N1", "N2"]);
        let loc = parse_location_from_xml("<location latitude=\"31.2\" longitude=\"121.5\" city=\"上海\" poiName=\"A &amp; B\"/>").unwrap();
        assert_eq!(loc["latitude"], 31.2);
        assert_eq!(loc["poiName"], "A & B");
        let merged = merge_location(Some(&json!({"city": "X"})), Some(&loc)).unwrap();
        assert_eq!(merged["city"], "X");
        assert_eq!(merged["longitude"], 121.5);
    }

    #[test]
    fn enriches_posts_like_get_timeline() {
        let post = json!({
            "id": "1", "username": "wxid_a", "nickname": "", "createTime": 1700000000, "type": 15,
            "rawXml": "<x><enc key=\"777\"/><location city=\"X\"/></x>",
            "media": [{"url": "http://v/snsvideodownload?x=1", "thumb": "http://t/150", "token": "T", "key": "old"}],
            "comments": [{"id": "1", "nickname": "A", "content": "c"}, {"id": "2", "nickname": "B", "content": "d", "refCommentId": "1"}]
        });
        let contact = CachedContact { display_name: Some("Alice".into()), avatar_url: Some("http://a".into()) };
        let e = enrich_post(&post, Some(&contact));
        assert_eq!(e["nickname"], "Alice");
        assert_eq!(e["avatarUrl"], "http://a");
        assert_eq!(e["media"][0]["key"], "777");
        assert_eq!(e["media"][0]["url"], "https://v/snsvideodownload?token=T&idx=1&x=1");
        assert_eq!(e["media"][0]["thumb"], "https://t/0?token=T&idx=1");
        assert_eq!(e["comments"][1]["refNickname"], "A");
        assert_eq!(e["location"]["city"], "X");
    }

    #[test]
    fn html_export_escapes_and_embeds() {
        let posts = vec![json!({"username": "u", "nickname": "<Nick>", "createTime": 1700000000, "contentDesc": "a\nb",
            "media": [{"url": "https://x/1.jpg", "localPath": "media/1_0.jpg"}], "likes": ["L"], "comments": [{"nickname": "C", "content": "hi", "refNickname": "R"}]})];
        let html = generate_html(&posts, &[], Some("kw"), &Default::default());
        assert!(html.contains("&lt;Nick&gt;"));
        assert!(html.contains("a<br>b"));
        assert!(html.contains("src=\"media/1_0.jpg\""));
        assert!(html.contains("<span class=\"re\">回复</span><b>R</b>"));
        assert!(html.contains("关键词: \"kw\""));
    }

    #[test]
    fn emoji_decrypt_roundtrip_nonce_tail() {
        use aes_gcm::aead::Aead;
        let key = [7u8; 16];
        let nonce = [9u8; 12];
        let plain: Vec<u8> = b"GIF89a".iter().copied().chain(std::iter::repeat(1u8).take(40)).collect();
        let cipher = Aes128Gcm::new_from_slice(&key).unwrap().encrypt(Nonce::from_slice(&nonce), plain.as_slice()).unwrap();
        let (ct, tag) = cipher.split_at(cipher.len() - 16);
        let mut enc = ct.to_vec();
        enc.extend_from_slice(&nonce);
        enc.extend_from_slice(tag);
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(decrypt_emoji_aes(&enc, &hex).as_deref(), Some(plain.as_slice()));
        assert!(decrypt_emoji_aes(&enc, "wrong-key-wrong-key").is_none());
    }

    #[test]
    fn lenient_base64_matches_node() {
        assert_eq!(lenient_base64("aGVsbG8="), b"hello");
        assert_eq!(lenient_base64("aGVs bG8"), b"hello");
    }
}
