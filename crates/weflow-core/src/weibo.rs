//! Port of `electron/services/social/weiboService.ts`: recent public Weibo posts of a bound UID,
//! used as extra context for AI insights.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use md5::Digest;
use serde_json::Value;

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_POSTS: usize = 5;
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";
const MOBILE_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";

#[derive(Debug, Clone, PartialEq)]
pub struct WeiboPost {
    pub id: String,
    pub created_at: String,
    pub url: String,
    pub text: String,
    pub screen_name: Option<String>,
}

type CacheEntry = (Instant, Vec<WeiboPost>);

fn cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<String, CacheEntry>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

pub fn clear_cache() {
    cache().lock().unwrap().clear();
}

/// `normalizeWeiboCookieInput`: a browser-exported JSON cookie array, or a raw `Cookie:` header.
pub fn normalize_cookie_input(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(trimmed) {
        let mut picked: Vec<(String, String)> = Vec::new();
        for e in &entries {
            let name = e.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let value = e.get("value").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let domain = e.get("domain").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
            if name.is_empty() || value.is_empty() {
                continue;
            }
            if !domain.is_empty() && !domain.contains("weibo.com") && !domain.contains("weibo.cn") {
                continue;
            }
            match picked.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = value,
                None => picked.push((name, value)),
            }
        }
        let joined = picked.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join("; ");
        if joined.is_empty() {
            return Err("no usable Weibo cookie entries found in the cookie JSON".into());
        }
        return Ok(joined);
    }
    let lower = trimmed.to_lowercase();
    let body = if lower.starts_with("cookie:") { &trimmed["cookie:".len()..] } else { trimmed };
    Ok(body.trim().to_string())
}

/// `normalizeWeiboUid`: a numeric UID or a `weibo.com/u/<uid>` / `m.weibo.cn/u/<uid>` link.
pub fn normalize_uid(input: &str) -> Result<String, String> {
    let t = input.trim();
    if t.len() >= 5 && t.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(t.to_string());
    }
    let re = crate::message::rx(r"(?i)(?:weibo\.com|m\.weibo\.cn)/u/(\d{5,})");
    if let Some(c) = re.captures(t) {
        return Ok(c[1].to_string());
    }
    Err("please enter a valid Weibo UID (digits only)".into())
}

/// `sanitizeWeiboText`
pub fn sanitize_text(text: &str) -> String {
    let t: String = text.chars().filter(|c| !matches!(c, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}')).collect();
    let t = crate::message::rx(r"https?://t\.cn/[A-Za-z0-9]+").replace_all(&t, " ").to_string();
    let t = crate::message::rx(r" +").replace_all(&t, " ").to_string();
    let t = crate::message::rx(r"\n{3,}").replace_all(&t, "\n\n").to_string();
    t.trim().to_string()
}

fn merge_retweet_text(item: &Value) -> String {
    let base = sanitize_text(item.get("text_raw").and_then(Value::as_str).unwrap_or(""));
    let retweet = sanitize_text(item.pointer("/retweeted_status/text_raw").and_then(Value::as_str).unwrap_or(""));
    if retweet.is_empty() {
        return base;
    }
    if base.is_empty() || base == "转发微博" {
        return format!("转发：{retweet}");
    }
    format!("{base}\n\n转发内容：{retweet}")
}

async fn request_json(url: &str, cookie: Option<&str>, referer: &str, user_agent: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder().timeout(TIMEOUT).build().map_err(|e| e.to_string())?;
    let mut req = client.get(url).header("Accept", "application/json, text/plain, */*").header("Referer", referer).header("User-Agent", user_agent).header("X-Requested-With", "XMLHttpRequest");
    if let Some(c) = cookie.filter(|c| !c.is_empty()) {
        req = req.header("Cookie", c);
    }
    let resp = req.send().await.map_err(|e| if e.is_timeout() { "Weibo request timed out".to_string() } else { e.to_string() })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("Weibo API returned an unexpected status {}", status.as_u16()));
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|_| "Weibo API returned a non-JSON response".to_string())
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn fetch_timeline(uid: &str, cookie: &str) -> Result<Vec<Value>, String> {
    let v = request_json(&format!("https://weibo.com/ajax/profile/getWaterFallContent?uid={}", enc(uid)), Some(cookie), &format!("https://weibo.com/u/{}", enc(uid)), USER_AGENT).await?;
    match (v.get("ok").and_then(Value::as_i64), v.pointer("/data/list").and_then(Value::as_array)) {
        (Some(1), Some(list)) => Ok(list.clone()),
        _ => Err("failed to load the Weibo timeline, check that the cookie is still valid".into()),
    }
}

async fn fetch_mobile_timeline(uid: &str) -> Result<Vec<Value>, String> {
    let container = format!("107603{uid}");
    let v = request_json(&format!("https://m.weibo.cn/api/container/getIndex?type=uid&value={}&containerid={}", enc(uid), enc(&container)), None, &format!("https://m.weibo.cn/u/{}", enc(uid)), MOBILE_USER_AGENT).await?;
    let cards = match (v.get("ok").and_then(Value::as_i64), v.pointer("/data/cards").and_then(Value::as_array)) {
        (Some(1), Some(c)) => c,
        _ => return Err("failed to load the Weibo timeline, try again later".into()),
    };
    let mut rows = Vec::new();
    for card in cards {
        if let Some(m) = card.get("mblog").filter(|m| !m.is_null()) {
            rows.push(m.clone());
        }
        for sub in card.get("card_group").and_then(Value::as_array).map(|a| a.as_slice()).unwrap_or(&[]) {
            if let Some(m) = sub.get("mblog").filter(|m| !m.is_null()) {
                rows.push(m.clone());
            }
        }
    }
    if rows.is_empty() {
        return Err("this Weibo account has no recent public posts to read".into());
    }
    Ok(rows)
}

async fn fetch_detail(id: &str, cookie: &str) -> Result<Value, String> {
    let v = request_json(&format!("https://weibo.com/ajax/statuses/show?id={}&isGetLongText=true", enc(id)), Some(cookie), &format!("https://weibo.com/detail/{}", enc(id)), USER_AGENT).await?;
    if v.get("id").map_or(true, |x| x.is_null()) && v.get("idstr").map_or(true, |x| x.is_null()) {
        return Err("failed to load the Weibo post".into());
    }
    Ok(v)
}

/// `validateUid`: `(uid, screen name)`; without a cookie only the UID format is checked.
pub async fn validate_uid(uid_input: &str, cookie_input: &str) -> Result<(String, Option<String>), String> {
    let uid = normalize_uid(uid_input)?;
    let cookie = normalize_cookie_input(cookie_input)?;
    if cookie.is_empty() {
        return Ok((uid, None));
    }
    let list = fetch_timeline(&uid, &cookie).await?;
    let first = list.first().ok_or_else(|| "this Weibo account has no recent public posts, or the cookie has expired".to_string())?;
    Ok((uid, first.pointer("/user/screen_name").and_then(Value::as_str).map(str::to_string)))
}

fn js_string(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// `fetchRecentPosts` with a 30-minute cache per (uid, count, cookie).
pub async fn fetch_recent_posts(uid_input: &str, cookie_input: &str, requested_count: i64) -> Result<Vec<WeiboPost>, String> {
    let uid = normalize_uid(uid_input)?;
    let cookie = normalize_cookie_input(cookie_input)?;
    let has_cookie = !cookie.is_empty();
    let count = requested_count.clamp(1, MAX_POSTS as i64) as usize;
    let cookie_hash: String = md5::Md5::digest(if has_cookie { cookie.as_bytes() } else { b"__no_cookie_mobile__" }).iter().map(|b| format!("{b:02x}")).collect();
    let key = format!("{uid}:{count}:{cookie_hash}");
    if let Some((at, posts)) = cache().lock().unwrap().get(&key) {
        if at.elapsed() < CACHE_TTL {
            return Ok(posts.clone());
        }
    }
    let raw = if has_cookie { fetch_timeline(&uid, &cookie).await? } else { fetch_mobile_timeline(&uid).await? };
    let mut posts = Vec::new();
    for item in &raw {
        if posts.len() >= count {
            break;
        }
        let id = {
            let s = js_string(item.get("idstr"));
            if s.is_empty() { js_string(item.get("id")) } else { s }.trim().to_string()
        };
        if id.is_empty() {
            continue;
        }
        let mut text = merge_retweet_text(item);
        if item.get("isLongText").and_then(Value::as_bool) == Some(true) && has_cookie {
            if let Ok(detail) = fetch_detail(&id, &cookie).await {
                text = merge_retweet_text(&detail);
            }
        }
        let text = sanitize_text(&text);
        if text.is_empty() {
            continue;
        }
        posts.push(WeiboPost { url: format!("https://m.weibo.cn/detail/{id}"), id, created_at: js_string(item.get("created_at")), text, screen_name: item.pointer("/user/screen_name").and_then(Value::as_str).map(str::to_string) });
    }
    cache().lock().unwrap().insert(key, (Instant::now(), posts.clone()));
    Ok(posts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_input_accepts_headers_and_browser_json() {
        assert_eq!(normalize_cookie_input("  Cookie: a=1; b=2 ").unwrap(), "a=1; b=2");
        assert_eq!(normalize_cookie_input("").unwrap(), "");
        let json = r#"[{"domain":".weibo.com","name":"SUB","value":"x"},{"domain":"example.com","name":"evil","value":"y"},{"name":"SUB","value":"z"},{"name":"","value":"q"}]"#;
        assert_eq!(normalize_cookie_input(json).unwrap(), "SUB=z");
        assert!(normalize_cookie_input(r#"[{"domain":"example.com","name":"a","value":"b"}]"#).is_err());
    }

    #[test]
    fn uids_come_from_digits_or_profile_links() {
        assert_eq!(normalize_uid("1234567890").unwrap(), "1234567890");
        assert_eq!(normalize_uid("https://weibo.com/u/7654321").unwrap(), "7654321");
        assert_eq!(normalize_uid("m.weibo.cn/u/55555").unwrap(), "55555");
        assert!(normalize_uid("1234").is_err());
        assert!(normalize_uid("abc").is_err());
    }

    #[test]
    fn post_text_is_cleaned_and_retweets_merged() {
        assert_eq!(sanitize_text("hi\u{200b} http://t.cn/AbC123   there\n\n\n\nend"), "hi there\n\nend");
        let item = serde_json::json!({ "text_raw": "转发微博", "retweeted_status": { "text_raw": "original" } });
        assert_eq!(merge_retweet_text(&item), "转发：original");
        let item = serde_json::json!({ "text_raw": "mine", "retweeted_status": { "text_raw": "theirs" } });
        assert_eq!(merge_retweet_text(&item), "mine\n\n转发内容：theirs");
        assert_eq!(merge_retweet_text(&serde_json::json!({ "text_raw": "solo" })), "solo");
    }
}
