//! Moments (朋友圈) logic ported from WeFlow's `snsService.ts`:
//! timeline enrichment, XML parsing, HTML / JSON export builders and media decryption helpers.
//! Network and database access live in `services/sns.rs`.

use std::sync::atomic::{AtomicU64, Ordering};

use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyInit, KeyIvInit};
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce};
use md5::{Digest, Md5};
use serde_json::Value;

use crate::contract::{SnsComment, SnsEmoji, SnsLivePhoto, SnsLocation, SnsMedia, SnsPost};
use crate::image::ImageFormat;
use crate::message::rx;

// ───────────────────────── small JS-compat helpers ─────────────────────────

/// `typeof value === 'string' && trim().length > 0 ? trimmed : undefined`
pub fn to_optional_string(v: Option<&Value>) -> Option<String> {
    let t = v?.as_str()?.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn opt_str(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// JS `parseFloat` (leading numeric prefix).
pub fn parse_float(s: &str) -> Option<f64> {
    let m = rx(r"^\s*[+-]?(?:\d+\.?\d*(?:[eE][+-]?\d+)?|\.\d+(?:[eE][+-]?\d+)?)").find(s)?;
    m.as_str()
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|f| f.is_finite())
}

/// JS `parseInt(x)` with NaN → `None`.
fn parse_int(s: &str) -> Option<i64> {
    let m = rx(r"^\s*[+-]?\d+").find(s)?;
    m.as_str().trim().parse::<i64>().ok()
}

/// A JS number as JSON writes it: a whole one as an integer.
fn json_number(f: f64) -> Option<serde_json::Number> {
    if f.fract() == 0.0 && f.abs() < 9.0e15 {
        Some(crate::num::saturating_i64(f).into())
    } else {
        serde_json::Number::from_f64(f)
    }
}

fn decode_xml_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let t = rx(r"(?s)<!\[CDATA\[(.*?)\]\]>")
        .replace_all(text, "$1")
        .to_string();
    let t = rx(r"(?i)&amp;").replace_all(&t, "&").to_string();
    let t = rx(r"(?i)&lt;").replace_all(&t, "<").to_string();
    let t = rx(r"(?i)&gt;").replace_all(&t, ">").to_string();
    let t = rx(r"(?i)&quot;").replace_all(&t, "\"").to_string();
    rx(r"(?i)&#39;").replace_all(&t, "'").to_string()
}

static CMT_COUNTER: AtomicU64 = AtomicU64::new(0);

// ───────────────────────── URL / mime helpers ─────────────────────────

fn fix_sns_url(url: &str, token: Option<&str>, is_video: bool) -> String {
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

/// Whether `url` is an https address of WeChat's Moments media servers: images on `*.qpic.cn` (and
/// `*.c2c.wechat.com` for accounts abroad), videos and their covers on `*.video.qq.com` and `*.tc.qq.com`. A route
/// that fetches media for a page takes only these, so it cannot be made to fetch whatever a page names.
pub fn is_wechat_media_url(url: &str) -> bool {
    const SERVERS: &[&str] = &[".qpic.cn", ".wechat.com", ".video.qq.com", ".tc.qq.com"];
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.port().is_none()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && SERVERS.iter().any(|s| host.ends_with(s))
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
    if let Some(format) = ImageFormat::of(buf) {
        return format.mime().into();
    }
    if buf[0] == 0x42 && buf[1] == 0x4d {
        return "image/bmp".into();
    }
    if buf.len() > 12 && &buf[4..8] == b"ftyp" {
        let window = String::from_utf8_lossy(&buf[8..buf.len().min(64)]).to_lowercase();
        if window.contains("avif") || window.contains("avis") {
            return "image/avif".into();
        }
        if ["heic", "heix", "hevc", "hevx", "mif1", "msf1"]
            .iter()
            .any(|k| window.contains(k))
        {
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
    if ImageFormat::of(buf).is_some() {
        return true;
    }
    if &buf[4..8] == b"ftyp" {
        let window = String::from_utf8_lossy(&buf[8..buf.len().min(64)]).to_lowercase();
        return [
            "avif", "avis", "heic", "heix", "hevc", "hevx", "mif1", "msf1",
        ]
        .iter()
        .any(|k| window.contains(k));
    }
    false
}

/// The extension an emoji is saved under: its format's, `.gif` when it is none of the four.
pub fn image_ext_from_buffer(buf: &[u8]) -> &'static str {
    ImageFormat::of(buf).map_or(".gif", ImageFormat::extension)
}

// ───────────────────────── XML parsing ─────────────────────────

fn extract_video_key(xml: &str) -> Option<String> {
    if xml.is_empty() {
        return None;
    }
    rx(r#"(?i)<enc\s+key="(\d+)""#)
        .captures(xml)
        .map(|c| c[1].to_string())
}

fn tag_value(block: &str, pattern: &str) -> Option<String> {
    rx(pattern).captures(block).map(|c| c[1].to_string())
}

/// `parseCommentsFromXml`: comments incl. emoji payloads and reply references.
pub fn parse_comments_from_xml(xml: &str) -> Vec<SnsComment> {
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

    let mut items: Vec<SnsComment> = Vec::new();
    let item_re = rx(
        r"(?is)<(?:CommentUser|commentUser|comment|user_comment)>(.*?)</(?:CommentUser|commentUser|comment|user_comment)>",
    );
    for m in item_re.captures_iter(&list) {
        let c = &m[1];
        let id = tag_value(
            c,
            r"(?is)<(?:cmtid|commentId|comment_id|id)>([^<]*)</(?:cmtid|commentId|comment_id|id)>",
        );
        let username = tag_value(c, r"(?is)<username>([^<]*)</username>");
        let nickname = tag_value(c, r"(?is)<nickname>([^<]*)</nickname>")
            .or_else(|| tag_value(c, r"(?is)<nickName>([^<]*)</nickName>"));
        let content = tag_value(c, r"(?is)<content>([^<]*)</content>");
        let ref_id = tag_value(
            c,
            r"(?is)<(?:refCommentId|replyCommentId|ref_comment_id)>([^<]*)</(?:refCommentId|replyCommentId|ref_comment_id)>",
        );
        let ref_nick = tag_value(
            c,
            r"(?is)<(?:refNickname|refNickName|replyNickname)>([^<]*)</(?:refNickname|refNickName|replyNickname)>",
        );
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
            let url = url_raw
                .map(|u| u.trim().replace("&amp;", "&"))
                .unwrap_or_default();
            let encrypt_url = enc.map(|u| u.trim().replace("&amp;", "&"));
            let aes_key = aes.map(|a| a.trim().to_string());
            if !url.is_empty() || encrypt_url.as_deref().is_some_and(|u| !u.is_empty()) {
                emojis.push(SnsEmoji {
                    url,
                    md5: md5.map(|m| m.trim().to_string()).unwrap_or_default(),
                    width: w.and_then(|w| parse_int(&w)),
                    height: h.and_then(|h| parse_int(&h)),
                    encrypt_url: encrypt_url.filter(|e| !e.is_empty()),
                    aes_key: aes_key.filter(|a| !a.is_empty()),
                });
            }
        }

        if nickname.is_some() && (content.is_some() || !emojis.is_empty()) {
            let ref_id = ref_id.map(|r| r.trim().to_string()).unwrap_or_default();
            items.push(SnsComment {
                id: id.map(|i| i.trim().to_string()).unwrap_or_else(|| {
                    format!(
                        "cmt_{}_{}",
                        chrono::Utc::now().timestamp_millis(),
                        CMT_COUNTER.fetch_add(1, Ordering::Relaxed)
                    )
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
        if c.ref_nickname.as_deref().is_none_or(str::is_empty) && !c.ref_comment_id.is_empty() {
            if let Some(ru) = c.ref_username.as_deref().filter(|u| !u.is_empty()) {
                c.ref_nickname = user_map.get(ru).cloned();
            }
        }
    }

    items
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

const LIKE_ITEM: &str =
    r"(?is)<(?:LikeUser|likeUser|user_comment)>(.*?)</(?:LikeUser|likeUser|user_comment)>";

pub fn parse_like_users_from_xml(xml: &str) -> Vec<LikeUser> {
    if xml.is_empty() {
        return Vec::new();
    }
    let Some(list) = like_list(xml) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for m in rx(LIKE_ITEM).captures_iter(&list) {
        let block = &m[1];
        let username =
            tag_value(block, r"(?is)<username>([^<]*)</username>").and_then(|u| opt_str(&u));
        let nickname = tag_value(block, r"(?is)<nickname>([^<]*)</nickname>")
            .and_then(|n| opt_str(&n))
            .or_else(|| {
                tag_value(block, r"(?is)<nickName>([^<]*)</nickName>").and_then(|n| opt_str(&n))
            });
        if username.is_some() || nickname.is_some() {
            out.push(LikeUser { username, nickname });
        }
    }
    out
}

// ───────────────────────── location ─────────────────────────

/// `parseLocationFromXml`: the `<location>` of a post, its attributes first and then its child tags; none when it
/// says nothing.
fn parse_location_from_xml(xml: &str) -> Option<SnsLocation> {
    if xml.is_empty() {
        return None;
    }
    let attrs = rx(r"(?i)<location\b([^>]*)>")
        .captures(xml)
        .map(|c| c[1].to_string())
        .unwrap_or_default();
    let read_attr = |name: &str| -> Option<String> {
        if attrs.is_empty() {
            return None;
        }
        let re = rx(&format!(
            r#"(?i){}\s*=\s*["']([\s\S]*?)["']"#,
            regex::escape(name)
        ));
        let m = re.captures(&attrs)?;
        if m[1].is_empty() {
            return None;
        }
        opt_str(&decode_xml_text(&m[1]))
    };
    let read_tag = |name: &str| -> Option<String> {
        let re = rx(&format!(r"(?i)<{0}>([\s\S]*?)</{0}>", regex::escape(name)));
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
    let num = |names: &[&str]| {
        first(names)
            .and_then(|s| parse_float(&s))
            .and_then(json_number)
    };
    let location = SnsLocation {
        latitude: num(&["latitude", "x"]),
        longitude: num(&["longitude", "y"]),
        city: first(&["city"]),
        country: first(&["country"]),
        poi_name: first(&["poiName", "poiname"]),
        poi_address: first(&["poiAddress", "poiaddress"]),
        poi_address_name: first(&["poiAddressName", "poiaddressname"]),
        label: first(&["label"]),
    };
    let said = location.latitude.is_some()
        || location.longitude.is_some()
        || [
            &location.city,
            &location.country,
            &location.poi_name,
            &location.poi_address,
            &location.poi_address_name,
            &location.label,
        ]
        .iter()
        .any(|v| v.is_some());
    said.then_some(location)
}

// ───────────────────────── timeline enrichment ─────────────────────────

#[derive(Clone, Debug, Default)]
pub struct CachedContact {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

/// `getTimeline` post-processing of one post as read: the addresses of its media fixed and their keys, its comments
/// and location from its XML, the poster's name and avatar from `contact`.
pub fn enrich_post(
    post: weflow_native::native_sns::SnsPost,
    contact: Option<&CachedContact>,
) -> SnsPost {
    let is_video_post = post.kind == 15;
    let video_key = extract_video_key(&post.raw_xml).or_else(|| post.video_key.clone());
    let media = post
        .media
        .into_iter()
        .map(|m| {
            // A video in a post of another type (a row of type 1 holding one) opens with the post's video key too:
            // its own key opens neither the video nor its cover (#202)
            let is_video = is_video_post || is_video_url(&m.url);
            let live_photo = m.live_photo.map(|lp| SnsLivePhoto {
                url: fix_sns_url(&lp.url, lp.token.as_deref(), true),
                thumb: fix_sns_url(&lp.thumb, lp.token.as_deref(), false),
                key: video_key.clone().or(lp.key).or_else(|| m.key.clone()),
                token: lp.token,
                md5: lp.md5,
                enc_idx: lp.enc_idx,
                raw_url: None,
                raw_thumb: None,
                resolved_url: None,
                resolved_thumb_url: None,
                proxy_url: None,
                proxy_thumb_url: None,
            });
            SnsMedia {
                url: fix_sns_url(&m.url, m.token.as_deref(), is_video_post),
                thumb: fix_sns_url(&m.thumb, m.token.as_deref(), false),
                md5: m.md5,
                key: if is_video {
                    video_key.clone().or(m.key)
                } else {
                    m.key
                },
                token: m.token,
                enc_idx: m.enc_idx,
                raw_url: None,
                raw_thumb: None,
                resolved_url: None,
                resolved_thumb_url: None,
                proxy_url: None,
                proxy_thumb_url: None,
                live_photo,
            }
        })
        .collect();
    let nickname = contact
        .and_then(|c| c.display_name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| post.username.clone());
    SnsPost {
        comments: parse_comments_from_xml(&post.raw_xml),
        location: parse_location_from_xml(&post.raw_xml),
        id: post.id,
        tid: post.tid,
        username: post.username,
        nickname,
        create_time: post.create_time,
        content_desc: post.content_desc,
        kind: post.kind,
        raw_xml: post.raw_xml,
        media,
        likes: post.likes,
        video_key: post.video_key,
        avatar_url: contact.and_then(|c| c.avatar_url.clone()),
    }
}

/// The files an export saved next to it of one medium of a post: the picture or video, and a live photo's video.
#[derive(Clone, Debug, Default)]
pub struct SavedMedia {
    pub file: Option<String>,
    pub live: Option<String>,
}

// ───────────────────────── HTML export ─────────────────────────

fn normalize_location_text(v: Option<&String>) -> String {
    rx(r"\s+")
        .replace_all(&decode_xml_text(v.map_or("", String::as_str)), " ")
        .trim()
        .to_string()
}

fn resolve_location_text(location: Option<&SnsLocation>) -> String {
    let Some(loc) = location else {
        return String::new();
    };
    let primary = [
        &loc.poi_name,
        &loc.poi_address_name,
        &loc.label,
        &loc.poi_address,
    ]
    .iter()
    .map(|v| normalize_location_text(v.as_ref()))
    .find(|s| !s.is_empty())
    .unwrap_or_default();
    let region = [&loc.country, &loc.city]
        .iter()
        .map(|v| normalize_location_text(v.as_ref()))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !primary.is_empty() && !region.is_empty() && !primary.contains(&region) {
        return format!("{primary} · {region}");
    }
    if primary.is_empty() {
        region
    } else {
        primary
    }
}

/// `toLocaleString('zh-CN')` → `2026/9/30 15:04:05`
fn zh_locale_string(dt: chrono::DateTime<chrono::Local>) -> String {
    use chrono::{Datelike, Timelike};
    format!(
        "{}/{}/{} {:02}:{:02}:{:02}",
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    )
}

pub fn zh_locale_from_ts(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(zh_locale_string)
        .unwrap_or_default()
}

/// The HTML page of `posts` (see [`crate::export_msg::MomentsPage`]); `saved` holds, post by post and medium by medium,
/// the files saved of their media, and `avatars` the avatars saved, by username, as paths from the page.
pub fn generate_html(
    posts: &[SnsPost],
    saved: &[Vec<SavedMedia>],
    keyword: Option<&str>,
    avatars: &std::collections::HashMap<String, String>,
) -> std::io::Result<Vec<u8>> {
    use crate::export_msg::{MomentComment, MomentMedia, MomentMediaKind, MomentPost, MomentsPage};
    let mut page = MomentsPage::new(
        keyword.map(str::to_string),
        chrono::Local::now().timestamp(),
    );
    for (i, post) in posts.iter().enumerate() {
        let u = page.user(
            &post.username,
            &post.nickname,
            avatars.get(&post.username).map(String::as_str),
        );
        let media = post
            .media
            .iter()
            .enumerate()
            .map(|(j, m)| {
                let copy = saved.get(i).and_then(|s| s.get(j));
                let kind = if is_video_url(&m.url) {
                    MomentMediaKind::Video
                } else {
                    MomentMediaKind::Picture
                };
                MomentMedia::new(
                    kind,
                    copy.and_then(|c| c.file.as_deref()),
                    &m.url,
                    copy.and_then(|c| c.live.as_deref()),
                )
            })
            .collect();
        // a reply names whom it answers, or only the comment it answers: then that comment's writer
        let answered = |c: &SnsComment| {
            c.ref_nickname
                .clone()
                .filter(|n| !n.is_empty())
                .or_else(|| {
                    let id = c.ref_comment_id.as_str();
                    (!id.is_empty() && id != "0")
                        .then(|| post.comments.iter().find(|o| o.id == id))
                        .flatten()
                        .map(|o| o.nickname.clone())
                })
        };
        let cmts = post
            .comments
            .iter()
            .map(|c| MomentComment {
                name: c.nickname.clone(),
                text: c.content.clone(),
                to: answered(c),
            })
            .collect();
        page.posts.push(MomentPost {
            t: post.create_time,
            u,
            text: post.content_desc.clone(),
            loc: resolve_location_text(post.location.as_ref()),
            media,
            likes: post.likes.clone(),
            cmts,
        });
    }
    let mut html = Vec::new();
    page.write(&mut html)?;
    Ok(html)
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
        let decoded: Vec<u8> = (0..16)
            .filter_map(|i| u8::from_str_radix(&first32[i * 2..i * 2 + 2], 16).ok())
            .collect();
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
    // not a 12-byte nonce: not this layout
    let n = &Nonce::try_from(nonce).ok()?;
    let mut joined = Vec::with_capacity(ciphertext.len() + tag.len());
    joined.extend_from_slice(ciphertext);
    joined.extend_from_slice(tag);
    let payload = Payload {
        msg: &joined,
        aad: &[],
    };
    let decrypted = match key.len() {
        16 => Aes128Gcm::new_from_slice(key)
            .ok()?
            .decrypt(n, payload)
            .ok()?,
        32 => Aes256Gcm::new_from_slice(key)
            .ok()?
            .decrypt(n, payload)
            .ok()?,
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
            layouts.push(GcmLayout {
                nonce: enc[19..31].to_vec(),
                ciphertext: &payload[..payload.len() - 16],
                tag: &payload[payload.len() - 16..],
            });
        }
    }
    if n > 28 {
        layouts.push(GcmLayout {
            ciphertext: &enc[..n - 28],
            nonce: enc[n - 28..n - 16].to_vec(),
            tag: &enc[n - 16..],
        });
        layouts.push(GcmLayout {
            nonce: enc[..12].to_vec(),
            ciphertext: &enc[12..n - 16],
            tag: &enc[n - 16..],
        });
    }
    if n > 16 {
        layouts.push(GcmLayout {
            nonce: vec![0; 12],
            ciphertext: &enc[..n - 16],
            tag: &enc[n - 16..],
        });
    }
    if n > 28 {
        layouts.push(GcmLayout {
            nonce: enc[..12].to_vec(),
            tag: &enc[12..28],
            ciphertext: &enc[28..],
        });
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
            if let Some(r) =
                try_gcm_decrypt(key, &enc[n - 28..n - 16], &enc[..n - 28], &enc[n - 16..])
            {
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
        if n >= 16 && n.is_multiple_of(16) {
            let mut buf = enc.to_vec();
            if let Ok(dec) = CbcDec::new_from_slices(key, key) {
                if let Ok(plain) = dec.decrypt_padded::<Pkcs7>(&mut buf) {
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
            if buf.len().is_multiple_of(16) {
                if let Ok(dec) = CbcDec::new_from_slices(key, &enc[..16]) {
                    if let Ok(plain) = dec.decrypt_padded::<Pkcs7>(&mut buf) {
                        if is_valid_image_buffer(plain) {
                            return Some(plain.to_vec());
                        }
                    }
                }
            }
        }
        if n.is_multiple_of(16) {
            let mut buf = enc.to_vec();
            if let Ok(Ok(plain)) =
                EcbDec::new_from_slice(key).map(|d| d.decrypt_padded::<Pkcs7>(&mut buf))
            {
                if is_valid_image_buffer(plain) {
                    return Some(plain.to_vec());
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
    fn only_wechat_media_servers_are_fetched_for_a_page() {
        for ok in [
            "https://shmmsns.qpic.cn/mmsns/abc/0?token=T&idx=1",
            "https://mmsns.c2c.wechat.com/mmsns/abc/0",
            "https://szzjwxsns.video.qq.com/102/20202/snsvideodownload?m=x",
            "https://vweixinthumb.tc.qq.com/150/20250/snsvideodownload",
        ] {
            assert!(is_wechat_media_url(ok), "{ok}");
        }
        for refused in [
            "http://shmmsns.qpic.cn/mmsns/abc/0",
            "https://shmmsns.qpic.cn:8443/mmsns/abc/0",
            "https://user@shmmsns.qpic.cn/mmsns/abc/0",
            "https://qpic.cn.example.com/a",
            "https://evilqpic.cn/a",
            "https://127.0.0.1/a",
            "https://www.qq.com/",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert!(!is_wechat_media_url(refused), "{refused}");
        }
    }

    #[test]
    fn fixes_urls_like_the_desktop_app() {
        assert_eq!(
            fix_sns_url("http://x.qq.com/a/150", Some("T"), false),
            "https://x.qq.com/a/0?token=T&idx=1"
        );
        assert_eq!(
            fix_sns_url("http://x.qq.com/v?a=1", Some("T"), true),
            "https://x.qq.com/v?token=T&idx=1&a=1"
        );
        assert_eq!(
            fix_sns_url("https://x/a?token=Z", Some("T"), false),
            "https://x/a?token=Z"
        );
        assert_eq!(
            fix_sns_url("http://x/a/150?q=1", None, false),
            "https://x/a/0?q=1"
        );
        assert!(
            is_video_url("https://x/snsvideodownload?a")
                && !is_video_url("https://vweixinthumb.x/video")
        );
    }

    #[test]
    fn detects_mime() {
        assert_eq!(
            detect_image_mime(&[0xff, 0xd8, 0xff, 0xe0], ""),
            "image/jpeg"
        );
        assert_eq!(detect_image_mime(b"GIF89a....", ""), "image/gif");
        assert_eq!(
            detect_image_mime(b"\0\0\0\x18ftypmp42abcd", ""),
            "video/mp4"
        );
        assert_eq!(detect_image_mime(b"abcd", "image/png"), "image/png");
    }

    const COMMENT_XML: &str = "<CommentUserList><CommentUser><cmtid>1</cmtid><username>wxid_a</username><nickname>A</nickname><content>hi</content></CommentUser>\
<CommentUser><cmtid>2</cmtid><username>wxid_b</username><nickname>B</nickname><content>yo</content><refCommentId>1</refCommentId><ref_username>wxid_a</ref_username>\
<emojiinfo><cdn_url>http://e/x?a=1&amp;b=2</cdn_url><md5>m5</md5><width>10</width><height>20</height><aes_key>k</aes_key></emojiinfo></CommentUser></CommentUserList>";

    #[test]
    fn parses_comments_with_refs_and_emojis() {
        let c = parse_comments_from_xml(COMMENT_XML);
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].ref_nickname.as_deref(), Some("A"));
        assert_eq!(c[1].emojis[0].url, "http://e/x?a=1&b=2");
        assert_eq!(c[1].emojis[0].width, Some(10));
        assert_eq!(c[1].emojis[0].aes_key.as_deref(), Some("k"));
    }

    #[test]
    fn parses_likes_and_location() {
        let xml = "<LikeUserList><LikeUser><username>u1</username><nickname>N1</nickname></LikeUser><LikeUser><nickname>N2</nickname></LikeUser></LikeUserList>";
        let likes = parse_like_users_from_xml(xml);
        assert_eq!(likes.len(), 2);
        assert_eq!(likes[0].username.as_deref(), Some("u1"));
        let nicknames: Vec<Option<&str>> = likes.iter().map(|l| l.nickname.as_deref()).collect();
        assert_eq!(nicknames, [Some("N1"), Some("N2")]);
        let loc = parse_location_from_xml(
            "<location latitude=\"31.2\" longitude=\"121\" city=\"上海\" poiName=\"A &amp; B\"/>",
        )
        .unwrap();
        assert_eq!(loc.latitude.as_ref().and_then(|n| n.as_f64()), Some(31.2));
        assert_eq!(
            loc.longitude.as_ref().and_then(|n| n.as_i64()),
            Some(121),
            "a whole number stays an integer"
        );
        assert_eq!(loc.poi_name.as_deref(), Some("A & B"));
        assert!(parse_location_from_xml("<location/>").is_none());
    }

    fn native_post(kind: i64, raw_xml: &str) -> weflow_native::native_sns::SnsPost {
        weflow_native::native_sns::SnsPost {
            id: "1".into(),
            tid: "1".into(),
            username: "wxid_a".into(),
            create_time: 1700000000,
            kind,
            raw_xml: raw_xml.into(),
            ..Default::default()
        }
    }

    fn native_media(url: &str, thumb: &str, key: &str) -> weflow_native::native_sns::SnsMedia {
        weflow_native::native_sns::SnsMedia {
            url: url.into(),
            thumb: thumb.into(),
            key: Some(key.into()),
            ..Default::default()
        }
    }

    #[test]
    fn enriches_posts_like_get_timeline() {
        let mut post = native_post(
            15,
            "<x><enc key=\"777\"/><location city=\"X\"/><commentUserList>\
             <comment><id>1</id><nickname>A</nickname><content>c</content></comment>\
             <comment><id>2</id><nickname>B</nickname><content>d</content><refNickname>A</refNickname><refCommentId>1</refCommentId></comment>\
             </commentUserList></x>",
        );
        post.media = vec![weflow_native::native_sns::SnsMedia {
            token: Some("T".into()),
            ..native_media("http://v/snsvideodownload?x=1", "http://t/150", "old")
        }];
        let contact = CachedContact {
            display_name: Some("Alice".into()),
            avatar_url: Some("http://a".into()),
        };
        let e = enrich_post(post, Some(&contact));
        assert_eq!(e.nickname, "Alice");
        assert_eq!(e.avatar_url.as_deref(), Some("http://a"));
        assert_eq!(e.media[0].key.as_deref(), Some("777"));
        assert_eq!(
            e.media[0].url,
            "https://v/snsvideodownload?token=T&idx=1&x=1"
        );
        assert_eq!(e.media[0].thumb, "https://t/0?token=T&idx=1");
        assert_eq!(e.comments[1].ref_nickname.as_deref(), Some("A"));
        assert_eq!(e.location.and_then(|l| l.city).as_deref(), Some("X"));
        // no contact: the username
        assert_eq!(enrich_post(native_post(1, ""), None).nickname, "wxid_a");
    }

    #[test]
    fn a_video_in_a_post_of_another_type_takes_the_video_key() {
        // seen on real accounts: a row of type 1 whose medium is a video; only the post's video key opens the
        // video and its cover (#202), while an image of such a row keeps its own
        let mut post = native_post(1, "");
        post.video_key = Some("888".into());
        post.media = vec![
            native_media(
                "http://v.video.qq.com/102/snsvideodownload?m=1",
                "http://v.video.qq.com/t",
                "own",
            ),
            native_media(
                "http://a.qpic.cn/mmsns/x/150",
                "http://a.qpic.cn/mmsns/x/150",
                "image",
            ),
        ];
        let e = enrich_post(post, None);
        assert_eq!(e.media[0].key.as_deref(), Some("888"));
        assert_eq!(e.media[1].key.as_deref(), Some("image"));
    }

    #[test]
    fn the_html_export_puts_each_post_on_the_page() {
        let mut post = enrich_post(
            native_post(
                1,
                "<x><commentUserList><comment><nickname>C</nickname><content>hi</content><refNickname>R</refNickname></comment></commentUserList></x>",
            ),
            None,
        );
        post.username = "wxid_n".into();
        post.nickname = "<Nick>".into();
        post.content_desc = "a\nb".into();
        post.likes = vec!["L".into()];
        post.media = enrich_post(
            weflow_native::native_sns::SnsPost {
                media: vec![
                    native_media("https://x/1.jpg", "", "k"),
                    native_media("https://x/2.jpg", "", "k"),
                ],
                ..native_post(1, "")
            },
            None,
        )
        .media;
        let saved = vec![vec![
            SavedMedia {
                file: Some("media/1_0.jpg".into()),
                live: None,
            },
            SavedMedia::default(),
        ]];
        let avatars = std::collections::HashMap::from([(
            "wxid_n".to_string(),
            "media/avatar_1.jpg".to_string(),
        )]);
        let html = String::from_utf8(generate_html(&[post], &saved, Some("kw"), &avatars).unwrap())
            .unwrap();
        let open = r#"<script id="wf-data" type="application/json">"#;
        let start = html.find(open).unwrap() + open.len();
        let end = start + html[start..].find("</script>").unwrap();
        let d: Value = serde_json::from_str(&html[start..end]).unwrap();
        assert_eq!(d["keyword"], "kw");
        assert_eq!(
            d["users"],
            serde_json::json!([["<Nick>", "media/avatar_1.jpg"]])
        );
        let p = &d["posts"][0];
        assert_eq!(p["text"], "a\nb");
        assert_eq!(p["likes"], serde_json::json!(["L"]));
        assert_eq!(p["cmts"], serde_json::json!([["C", "hi", "R"]]));
        assert_eq!(p["media"][0]["s"], "media/1_0.jpg");
        assert_eq!(
            p["media"][1]["s"], "",
            "not saved: the page offers the address"
        );
        assert_eq!(p["media"][1]["url"], "https://x/2.jpg");
        assert!(!html.contains("__WF_"), "a placeholder is left");
    }

    #[test]
    fn emoji_decrypt_roundtrip_nonce_tail() {
        use aes_gcm::aead::Aead;
        let key = [7u8; 16];
        let nonce = [9u8; 12];
        let plain: Vec<u8> = b"GIF89a"
            .iter()
            .copied()
            .chain(std::iter::repeat_n(1u8, 40))
            .collect();
        let cipher = Aes128Gcm::new_from_slice(&key)
            .unwrap()
            .encrypt(&Nonce::from(nonce), plain.as_slice())
            .unwrap();
        let (ct, tag) = cipher.split_at(cipher.len() - 16);
        let mut enc = ct.to_vec();
        enc.extend_from_slice(&nonce);
        enc.extend_from_slice(tag);
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            decrypt_emoji_aes(&enc, &hex).as_deref(),
            Some(plain.as_slice())
        );
        assert!(decrypt_emoji_aes(&enc, "wrong-key-wrong-key").is_none());
    }

    #[test]
    fn lenient_base64_matches_node() {
        assert_eq!(lenient_base64("aGVsbG8="), b"hello");
        assert_eq!(lenient_base64("aGVs bG8"), b"hello");
    }
}
