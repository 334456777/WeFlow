//! Moments service: ports the stateful parts of `electron/services/snsService.ts`
//! (timeline queries, stats caches, media download/decrypt, export, emoji download).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use md5::{Digest, Md5};
use serde_json::{json, Map, Value};

use super::*;
use crate::isaac64;
use crate::sns::{self, CachedContact};

const EXPORT_STATS_TTL: Duration = Duration::from_secs(5 * 60);
const USER_COUNTS_TTL: Duration = Duration::from_secs(5 * 60);
const TIMELINE_FALLBACK_COOLDOWN: Duration = Duration::from_secs(3 * 60);
const IMAGE_CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const IMAGE_CACHE_MAX: usize = 120;
const VIDEO_HEADER_BYTES: usize = 131_072;

#[derive(Default)]
pub(crate) struct SnsState {
    export_stats: Option<(Value, Instant)>,
    user_counts: Option<(HashMap<String, i64>, Instant)>,
    last_timeline_fallback: Option<Instant>,
    image_cache: HashMap<String, (String, Instant)>,
    image_order: VecDeque<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SnsTimelineQuery {
    pub limit: i32,
    pub offset: i32,
    pub usernames: Vec<String>,
    pub keyword: Option<String>,
    pub start: i64,
    pub end: i64,
}

#[derive(Clone, Debug, Default)]
pub struct SnsExportOptions {
    pub output_dir: PathBuf,
    /// `json` | `html` | `arkmejson`
    pub format: String,
    pub usernames: Vec<String>,
    pub keyword: Option<String>,
    pub export_media: bool,
    pub export_images: Option<bool>,
    pub export_live_photos: Option<bool>,
    pub export_videos: Option<bool>,
    pub start: i64,
    pub end: i64,
}

/// Result of `downloadImage` / `fetchAndDecryptImage`.
#[derive(Clone, Debug, Default)]
pub struct SnsMediaFetch {
    pub data: Option<Vec<u8>>,
    pub content_type: String,
    pub cache_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum SnsProxyResult {
    DataUrl(String),
    VideoPath(Option<PathBuf>),
}

fn md5_hex(data: &[u8]) -> String {
    let mut h = Md5::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn err_native(e: impl ToString) -> AppError {
    AppError::native(e.to_string())
}

fn truthy_key(key: Option<&str>) -> Option<&str> {
    key.map(str::trim).filter(|k| !k.is_empty())
}

fn export_stats_value(total_posts: i64, total_friends: i64, my_posts: Option<i64>) -> Value {
    json!({ "totalPosts": total_posts, "totalFriends": total_friends, "myPosts": my_posts })
}

impl ServiceHub {
    // ── paths ──

    fn cache_base(&self) -> PathBuf {
        self.profile()
            .ok()
            .and_then(|p| p.cache_path.clone())
            .filter(|p| !p.trim().is_empty())
            .map(|p| crate::config::expand_home(&p))
            .unwrap_or_else(|| self.ctx.cache_dir())
    }

    pub fn sns_cache_dir(&self) -> PathBuf {
        let dir = self.cache_base().join("sns_cache");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn emoji_cache_dir(&self) -> PathBuf {
        let dir = self.cache_base().join("Emojis");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// Directory the HTTP API exports media into (`api-media`).
    pub fn api_media_dir(&self) -> PathBuf {
        self.cache_base().join("api-media")
    }

    fn sns_cache_file(&self, url: &str) -> PathBuf {
        let ext = if sns::is_video_url(url) { ".mp4" } else { ".jpg" };
        self.sns_cache_dir().join(format!("{}{}", md5_hex(url.as_bytes()), ext))
    }

    // ── contacts ──

    /// Desktop `contacts.json` cache (if present) topped up from WCDB for the given users.
    pub(super) fn contact_book(&self, wcdb: &weflow_native::wcdb::Wcdb, usernames: &[String]) -> HashMap<String, CachedContact> {
        let mut book: HashMap<String, CachedContact> = HashMap::new();
        if let Ok(raw) = std::fs::read_to_string(self.cache_base().join("contacts.json")) {
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&raw) {
                for (k, v) in map {
                    let avatar = v.get("avatarUrl").and_then(Value::as_str).filter(|a| !a.contains("base64,ffd8")).map(str::to_string);
                    book.insert(k, CachedContact { display_name: v.get("displayName").and_then(Value::as_str).map(str::to_string), avatar_url: avatar });
                }
            }
        }
        let missing: Vec<&String> = usernames.iter().filter(|u| book.get(*u).map_or(true, |c| c.display_name.is_none() && c.avatar_url.is_none())).collect();
        if !missing.is_empty() {
            let payload = serde_json::to_string(&missing).unwrap_or_else(|_| "[]".into());
            if let Ok(Value::Object(names)) = wcdb.display_names(&payload) {
                for (k, v) in names {
                    if let Some(n) = v.as_str().filter(|n| !n.is_empty()) {
                        book.entry(k).or_default().display_name = Some(n.to_string());
                    }
                }
            }
            if let Ok(Value::Object(urls)) = wcdb.avatar_urls(&payload) {
                for (k, v) in urls {
                    if let Some(u) = v.as_str().filter(|u| !u.is_empty()) {
                        book.entry(k).or_default().avatar_url = Some(u.to_string());
                    }
                }
            }
        }
        book
    }

    fn contact_identity(&self, wcdb: &weflow_native::wcdb::Wcdb, book: &HashMap<String, CachedContact>, cache: &mut HashMap<String, Option<Value>>, username: &str) -> Option<Value> {
        let username = username.trim();
        if username.is_empty() {
            return None;
        }
        if let Some(hit) = cache.get(username) {
            return hit.clone();
        }
        let contact = wcdb.contact(username).ok().filter(Value::is_object);
        let pick = |keys: &[&str]| -> Option<String> {
            let c = contact.as_ref()?;
            keys.iter().find_map(|k| sns::to_optional_string(c.get(*k)))
        };
        let alias = pick(&["alias", "Alias"]);
        let remark = pick(&["remark", "Remark"]);
        let nick = pick(&["nickName", "nick_name", "nickname", "NickName"]);
        let display = remark
            .clone()
            .or_else(|| nick.clone())
            .or_else(|| alias.clone())
            .or_else(|| book.get(username).and_then(|c| c.display_name.clone()).filter(|d| !d.is_empty()))
            .unwrap_or_else(|| username.to_string());
        let mut o = Map::new();
        o.insert("username".into(), json!(username));
        o.insert("wxid".into(), json!(username));
        for (k, v) in [("alias", alias.clone()), ("wechatId", alias), ("remark", remark), ("nickName", nick)] {
            if let Some(v) = v {
                o.insert(k.into(), json!(v));
            }
        }
        o.insert("displayName".into(), json!(display));
        let v = Some(Value::Object(o));
        cache.insert(username.to_string(), v.clone());
        v
    }

    // ── timeline ──

    /// `snsService.getTimeline`: DLL rows enriched with fixed URLs, comments, location, avatar.
    pub fn sns_timeline_query(&self, q: &SnsTimelineQuery) -> AppResult<Vec<Value>> {
        let wcdb = self.open_wcdb()?;
        self.sns_timeline_with(&wcdb, q)
    }

    fn sns_timeline_with(&self, wcdb: &weflow_native::wcdb::Wcdb, q: &SnsTimelineQuery) -> AppResult<Vec<Value>> {
        let usernames_json = if q.usernames.is_empty() { None } else { Some(serde_json::to_string(&q.usernames).unwrap()) };
        let raw = wcdb
            .sns_timeline(q.limit, q.offset, usernames_json.as_deref(), q.keyword.as_deref(), q.start as i32, q.end as i32)
            .map_err(err_native)?;
        let rows = raw.as_array().cloned().unwrap_or_default();
        if rows.is_empty() {
            return Ok(rows);
        }
        let users: Vec<String> = {
            let mut seen = std::collections::BTreeSet::new();
            rows.iter().filter_map(|p| p.get("username").and_then(Value::as_str)).filter(|u| seen.insert(u.to_string())).map(str::to_string).collect()
        };
        let book = self.contact_book(wcdb, &users);
        Ok(rows.iter().map(|p| sns::enrich_post(p, p.get("username").and_then(Value::as_str).and_then(|u| book.get(u)))).collect())
    }

    pub fn sns_usernames_list(&self) -> AppResult<Vec<String>> {
        let wcdb = self.open_wcdb()?;
        let raw = wcdb.sns_usernames().map_err(err_native)?;
        let direct: Vec<String> = raw.as_array().map(|a| a.iter().filter_map(|u| u.as_str()).map(|u| u.trim().to_string()).filter(|u| !u.is_empty()).collect()).unwrap_or_default();
        if !direct.is_empty() {
            return Ok(direct);
        }
        if let Ok(from_timeline) = self.collect_timeline_usernames(&wcdb, 2000) {
            if !from_timeline.is_empty() {
                return Ok(from_timeline);
            }
        }
        Ok(direct)
    }

    fn scan_timeline_rows(&self, wcdb: &weflow_native::wcdb::Wcdb, max_rounds: usize, mut visit: impl FnMut(&Value)) -> AppResult<()> {
        let page = 500;
        let mut offset = 0;
        for _ in 0..max_rounds {
            let raw = wcdb.sns_timeline(page, offset, None, None, 0, 0).map_err(err_native)?;
            let Some(rows) = raw.as_array() else {
                return Err(AppError::native("failed to read the Moments timeline"));
            };
            if rows.is_empty() {
                break;
            }
            rows.iter().for_each(&mut visit);
            if (rows.len() as i32) < page {
                break;
            }
            offset += rows.len() as i32;
        }
        Ok(())
    }

    fn collect_timeline_usernames(&self, wcdb: &weflow_native::wcdb::Wcdb, max_rounds: usize) -> AppResult<Vec<String>> {
        let mut seen = std::collections::BTreeSet::new();
        let mut order = Vec::new();
        self.scan_timeline_rows(wcdb, max_rounds, |row| {
            let u = sns::pick_timeline_username(row);
            if !u.is_empty() && seen.insert(u.clone()) {
                order.push(u);
            }
        })?;
        Ok(order)
    }

    // ── stats ──

    fn export_stats_from_table(&self, wcdb: &weflow_native::wcdb::Wcdb, my_wxid: Option<&str>) -> (i64, i64, Option<i64>) {
        match wcdb.sns_export_stats(my_wxid) {
            Ok(raw) if raw.is_object() => {
                let n = |k: &str| raw.get(k).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0);
                let my = match raw.get("my_posts") {
                    None | Some(Value::Null) => None,
                    Some(_) => Some(n("my_posts")),
                };
                (n("total_posts"), n("total_friends"), my)
            }
            _ => (0, 0, my_wxid.map(|_| 0)),
        }
    }

    fn export_stats_from_timeline(&self, wcdb: &weflow_native::wcdb::Wcdb, my_wxid: Option<&str>) -> AppResult<(i64, i64, Option<i64>)> {
        let (mut total, mut mine) = (0i64, 0i64);
        let mut users = std::collections::HashSet::new();
        self.scan_timeline_rows(wcdb, 2000, |row| {
            total += 1;
            let u = sns::pick_timeline_username(row);
            if !u.is_empty() {
                if my_wxid == Some(u.as_str()) {
                    mine += 1;
                }
                users.insert(u);
            }
        })?;
        Ok((total, users.len() as i64, my_wxid.map(|_| mine)))
    }

    /// `getExportStats` / `getExportStatsFast`: returns `{totalPosts, totalFriends, myPosts}`.
    pub fn sns_export_stats(&self, fast: bool) -> AppResult<Value> {
        let (allow_fallback, prefer_cache) = if fast { (false, true) } else { (true, false) };
        let my_wxid = Some(self.my_wxid_cleaned()).filter(|w| !w.trim().is_empty());
        let cached = self.sns_state.lock().unwrap().export_stats.clone();
        if prefer_cache {
            if let Some((v, at)) = &cached {
                if at.elapsed() <= EXPORT_STATS_TTL {
                    return Ok(v.clone());
                }
            }
        }
        let wcdb = self.open_wcdb()?;
        let (mut total, mut friends, mut mine) = self.export_stats_from_table(&wcdb, my_wxid.as_deref());
        let mut fallback_error: Option<String> = None;
        let mut fallback_attempted = false;
        let cooled_down = self.sns_state.lock().unwrap().last_timeline_fallback.map_or(true, |t| t.elapsed() >= TIMELINE_FALLBACK_COOLDOWN);
        if allow_fallback && (total <= 0 || friends <= 0) && cooled_down {
            fallback_attempted = true;
            match self.export_stats_from_timeline(&wcdb, my_wxid.as_deref()) {
                Ok((t, f, m)) => {
                    self.sns_state.lock().unwrap().last_timeline_fallback = Some(Instant::now());
                    if t > 0 {
                        total = t;
                    }
                    if f > 0 {
                        friends = f;
                    }
                    if m.is_some() {
                        mine = m;
                    }
                }
                Err(e) => fallback_error = Some(e.message),
            }
        }
        let normalized = export_stats_value(total.max(0), friends.max(0), if my_wxid.is_some() { mine.map(|m| m.max(0)) } else { None });
        let has_data = total > 0 || friends > 0;
        let cache_has_data = cached.as_ref().map_or(false, |(v, _)| v["totalPosts"].as_i64().unwrap_or(0) > 0 || v["totalFriends"].as_i64().unwrap_or(0) > 0);
        if !has_data && cache_has_data {
            return Ok(cached.unwrap().0);
        }
        if !has_data && fallback_attempted {
            if let Some(e) = fallback_error {
                return Err(AppError::native(e));
            }
        }
        self.sns_state.lock().unwrap().export_stats = Some((normalized.clone(), Instant::now()));
        Ok(normalized)
    }

    pub fn sns_user_post_counts(&self, prefer_cache: bool) -> AppResult<HashMap<String, i64>> {
        if prefer_cache {
            if let Some((c, at)) = &self.sns_state.lock().unwrap().user_counts {
                if at.elapsed() <= USER_COUNTS_TTL {
                    return Ok(c.clone());
                }
            }
        }
        let wcdb = self.open_wcdb()?;
        let mut counts: HashMap<String, i64> = HashMap::new();
        let res = self.scan_timeline_rows(&wcdb, 2000, |row| {
            let u = sns::pick_timeline_username(row);
            if !u.is_empty() {
                *counts.entry(u).or_default() += 1;
            }
        });
        match res {
            Ok(()) => {
                self.sns_state.lock().unwrap().user_counts = Some((counts.clone(), Instant::now()));
                Ok(counts)
            }
            Err(e) => match &self.sns_state.lock().unwrap().user_counts {
                Some((c, _)) => Ok(c.clone()),
                None => Err(e),
            },
        }
    }

    pub fn sns_user_post_stats(&self, username: &str) -> AppResult<Value> {
        let username = username.trim();
        if username.is_empty() {
            return Err(AppError::usage("username must not be empty"));
        }
        let counts = self.sns_user_post_counts(true)?;
        Ok(json!({ "username": username, "totalPosts": counts.get(username).copied().unwrap_or(0).max(0) }))
    }

    // ── block-delete trigger / delete ──

    pub fn sns_block_delete_status(&self) -> AppResult<Value> {
        let v = self.sns_block_delete("check")?;
        Ok(json!({ "success": true, "installed": v["installed"].as_bool().unwrap_or(false) }))
    }

    pub fn sns_block_delete_install(&self) -> AppResult<Value> {
        let _ = self.sns_block_delete("install")?;
        Ok(json!({ "success": true }))
    }

    pub fn sns_block_delete_uninstall(&self) -> AppResult<Value> {
        let _ = self.sns_block_delete("uninstall")?;
        Ok(json!({ "success": true }))
    }

    /// `deleteSnsPost`: also drops the stats caches.
    pub fn sns_delete_post(&self, post_id: &str) -> AppResult<Value> {
        let v = self.sns_delete(post_id)?;
        let mut st = self.sns_state.lock().unwrap();
        st.user_counts = None;
        st.export_stats = None;
        Ok(json!({ "success": true, "result": v }))
    }

    // ── media download / decrypt ──

    fn sns_http_client(&self, insecure: bool) -> AppResult<reqwest::Client> {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .danger_accept_invalid_certs(insecure)
            .build()
            .map_err(|e| AppError::runtime(format!("failed to create HTTP client: {e}")))
    }

    /// `fetchAndDecryptImage`: downloads a Moments image/video, decrypting with the ISAAC64 key.
    pub async fn sns_fetch_media(&self, url: &str, key: Option<&str>) -> AppResult<SnsMediaFetch> {
        if url.is_empty() {
            return Err(AppError::usage("url must not be empty"));
        }
        let is_video = sns::is_video_url(url);
        let cache_path = self.sns_cache_file(url);

        if cache_path.exists() {
            if is_video {
                return Ok(SnsMediaFetch { data: None, content_type: "video/mp4".into(), cache_path: Some(cache_path) });
            }
            match std::fs::read(&cache_path) {
                Ok(data) if sns::detect_image_mime(&data, "").starts_with("image/") => {
                    let ct = sns::detect_image_mime(&data, "image/jpeg");
                    return Ok(SnsMediaFetch { data: Some(data), content_type: ct, cache_path: Some(cache_path) });
                }
                Ok(_) => {
                    let _ = std::fs::remove_file(&cache_path);
                }
                Err(_) => {}
            }
        }

        let client = self.sns_http_client(false)?;
        let mut req = client.get(url).header("User-Agent", "MicroMessenger Client").header("Accept", "*/*").header("Connection", "keep-alive");
        if !is_video {
            req = req.header("Accept-Language", "zh-CN,zh;q=0.9");
        }
        let resp = req.send().await.map_err(|e| AppError::runtime(if e.is_timeout() { "request timed out".to_string() } else { e.to_string() }))?;
        let status = resp.status();
        if status.as_u16() != 200 && status.as_u16() != 206 {
            return Err(AppError::runtime(format!("HTTP {}", status.as_u16())));
        }
        let x_enc = resp.headers().get("x-enc").and_then(|v| v.to_str().ok()).unwrap_or("").trim().to_string();
        let header_ct = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("image/jpeg").to_string();
        let mut raw = resp.bytes().await.map_err(|e| AppError::runtime(e.to_string()))?.to_vec();
        let key = truthy_key(key);

        if is_video {
            if let Some(k) = key {
                isaac64::xor_in_place(&mut raw, k, Some(VIDEO_HEADER_BYTES));
            }
            std::fs::write(&cache_path, &raw).map_err(|e| AppError::runtime(format!("failed to write {}: {e}", cache_path.display())))?;
            return Ok(SnsMediaFetch { data: Some(raw), content_type: "video/mp4".into(), cache_path: Some(cache_path) });
        }

        let mut decoded = raw.clone();
        let raw_is_image = sns::detect_image_mime(&raw, "").starts_with("image/");
        let should_decrypt = (x_enc == "1" || key.is_some()) && key.is_some();
        if should_decrypt {
            let key = key.unwrap();
            if key.chars().all(|c| c.is_ascii_digit()) {
                let mut decrypted = raw.clone();
                isaac64::xor_in_place(&mut decrypted, key, None);
                if sns::detect_image_mime(&decrypted, "").starts_with("image/") || !raw_is_image {
                    decoded = decrypted;
                }
            }
        }
        if !sns::detect_image_mime(&decoded, "").starts_with("image/") {
            return Err(AppError::runtime("image decryption failed: unrecognized image format"));
        }
        let _ = std::fs::write(&cache_path, &decoded);
        let ct = sns::detect_image_mime(&decoded, &header_ct);
        Ok(SnsMediaFetch { data: Some(decoded), content_type: ct, cache_path: Some(cache_path) })
    }

    /// `proxyImage`: image → `data:` URL (memoised for 15 minutes), video → local path.
    pub async fn sns_proxy_image(&self, url: &str, key: Option<&str>) -> AppResult<SnsProxyResult> {
        if url.is_empty() {
            return Err(AppError::usage("url must not be empty"));
        }
        let cache_key = format!("{url}|{}", key.unwrap_or(""));
        {
            let mut st = self.sns_state.lock().unwrap();
            if let Some((data_url, at)) = st.image_cache.get(&cache_key).cloned() {
                let valid = at.elapsed() <= IMAGE_CACHE_TTL
                    && data_url.split(',').nth(1).map_or(false, |b64| {
                        let bytes = sns::lenient_base64(b64);
                        sns::detect_image_mime(&bytes, "").starts_with("image/")
                    });
                if valid {
                    st.image_order.retain(|k| k != &cache_key);
                    st.image_order.push_back(cache_key.clone());
                    st.image_cache.insert(cache_key, (data_url.clone(), Instant::now()));
                    return Ok(SnsProxyResult::DataUrl(data_url));
                }
                st.image_cache.remove(&cache_key);
                st.image_order.retain(|k| k != &cache_key);
            }
        }
        let fetched = self.sns_fetch_media(url, key).await?;
        if fetched.content_type.starts_with("video/") {
            return Ok(SnsProxyResult::VideoPath(fetched.cache_path));
        }
        let data = fetched.data.ok_or_else(|| AppError::runtime("empty image data"))?;
        if !sns::detect_image_mime(&data, "").starts_with("image/") {
            return Err(AppError::runtime("invalid image data (wrong key or corrupt cache)"));
        }
        use base64::Engine;
        let data_url = format!("data:{};base64,{}", fetched.content_type, base64::engine::general_purpose::STANDARD.encode(&data));
        let mut st = self.sns_state.lock().unwrap();
        st.image_order.retain(|k| k != &cache_key);
        st.image_order.push_back(cache_key.clone());
        st.image_cache.insert(cache_key, (data_url.clone(), Instant::now()));
        let now = Instant::now();
        let expired: Vec<String> = st.image_cache.iter().filter(|(_, (_, at))| now.duration_since(*at) > IMAGE_CACHE_TTL).map(|(k, _)| k.clone()).collect();
        for k in expired {
            st.image_cache.remove(&k);
            st.image_order.retain(|o| o != &k);
        }
        while st.image_cache.len() > IMAGE_CACHE_MAX {
            match st.image_order.pop_front() {
                Some(oldest) => {
                    st.image_cache.remove(&oldest);
                }
                None => break,
            }
        }
        Ok(SnsProxyResult::DataUrl(data_url))
    }

    // ── emoji ──

    async fn download_raw(&self, target_url: &str, cache_key: &str, dir: &Path) -> Option<PathBuf> {
        let client = self.sns_http_client(true).ok()?;
        let mut url = target_url.replace("&amp;", "&");
        for _ in 0..5 {
            let resp = client
                .get(&url)
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 MicroMessenger/7.0.20.1781(0x67001431)")
                .header("Accept", "*/*")
                .send()
                .await
                .ok()?;
            let code = resp.status().as_u16();
            if [301, 302, 303, 307].contains(&code) {
                let loc = resp.headers().get("location")?.to_str().ok()?.to_string();
                url = if loc.starts_with("http") {
                    loc
                } else {
                    let base = reqwest::Url::parse(&url).ok()?;
                    format!("{}://{}{}", base.scheme(), base.host_str()?, loc)
                };
                continue;
            }
            if code != 200 {
                return None;
            }
            let bytes = resp.bytes().await.ok()?;
            if bytes.is_empty() {
                return None;
            }
            let ext = if sns::is_valid_image_buffer(&bytes) { sns::image_ext_from_buffer(&bytes) } else { ".bin" };
            let path = dir.join(format!("{cache_key}{ext}"));
            std::fs::write(&path, &bytes).ok()?;
            return Some(path);
        }
        None
    }

    /// `downloadSnsEmoji`: cached download of a comment emoji, decrypting with the AES key when needed.
    pub async fn sns_download_emoji(&self, url: &str, encrypt_url: Option<&str>, aes_key: Option<&str>) -> AppResult<Value> {
        let encrypt_url = encrypt_url.filter(|u| !u.is_empty());
        let aes_key = aes_key.filter(|k| !k.is_empty());
        if url.is_empty() && encrypt_url.is_none() {
            return Err(AppError::usage("url must not be empty"));
        }
        let cache_key = md5_hex(if url.is_empty() { encrypt_url.unwrap() } else { url }.as_bytes());
        let dir = self.emoji_cache_dir();
        for ext in [".gif", ".png", ".webp", ".jpg", ".jpeg"] {
            let p = dir.join(format!("{cache_key}{ext}"));
            if p.exists() {
                return Ok(json!({ "success": true, "localPath": p }));
            }
        }
        let save_decrypted = |buf: &[u8]| -> Option<PathBuf> {
            let ext = if sns::is_valid_image_buffer(buf) { sns::image_ext_from_buffer(buf) } else { ".gif" };
            let p = dir.join(format!("{cache_key}{ext}"));
            std::fs::write(&p, buf).ok().map(|_| p)
        };

        if let (Some(enc_url), Some(key)) = (encrypt_url, aes_key) {
            if let Some(enc_path) = self.download_raw(enc_url, &format!("{cache_key}_enc"), &dir).await {
                if let Ok(enc) = std::fs::read(&enc_path) {
                    if sns::is_valid_image_buffer(&enc) {
                        let p = dir.join(format!("{cache_key}{}", sns::image_ext_from_buffer(&enc)));
                        if std::fs::write(&p, &enc).is_ok() {
                            let _ = std::fs::remove_file(&enc_path);
                            return Ok(json!({ "success": true, "localPath": p }));
                        }
                    }
                    if let Some(dec) = sns::decrypt_emoji_aes(&enc, key) {
                        let _ = std::fs::remove_file(&enc_path);
                        if let Some(p) = save_decrypted(&dec) {
                            return Ok(json!({ "success": true, "localPath": p }));
                        }
                        return Ok(json!({ "success": false }));
                    }
                }
                let _ = std::fs::remove_file(&enc_path);
            }
        }
        if !url.is_empty() {
            if let Some(path) = self.download_raw(url, &cache_key, &dir).await {
                if let Ok(buf) = std::fs::read(&path) {
                    if sns::is_valid_image_buffer(&buf) {
                        return Ok(json!({ "success": true, "localPath": path }));
                    }
                    if let Some(key) = aes_key {
                        if let Some(dec) = sns::decrypt_emoji_aes(&buf, key) {
                            let _ = std::fs::remove_file(&path);
                            if let Some(p) = save_decrypted(&dec) {
                                return Ok(json!({ "success": true, "localPath": p }));
                            }
                            return Ok(json!({ "success": false }));
                        }
                    }
                }
                let _ = std::fs::remove_file(&path);
            }
        }
        Err(AppError::runtime("failed to download the emoji"))
    }

    // ── export ──

    /// `exportTimeline`: json / html / arkmejson with optional media download.
    pub async fn sns_export_timeline(&self, opts: &SnsExportOptions) -> AppResult<Value> {
        use futures::StreamExt;
        let explicit = opts.export_images.is_some() || opts.export_live_photos.is_some() || opts.export_videos.is_some();
        let pick = |v: Option<bool>| if explicit { v == Some(true) } else { opts.export_media };
        let (want_images, want_live, want_videos) = (pick(opts.export_images), pick(opts.export_live_photos), pick(opts.export_videos));
        let want_media = want_images || want_live || want_videos;

        std::fs::create_dir_all(&opts.output_dir).map_err(|e| AppError::runtime(format!("failed to create {}: {e}", opts.output_dir.display())))?;
        let usernames = &opts.usernames;
        let keyword = opts.keyword.as_deref().filter(|k| !k.is_empty());

        // 1. load every post page by page
        let wcdb = self.open_wcdb()?;
        let mut posts: Vec<Value> = Vec::new();
        let page = 50;
        let mut end_ts = opts.end;
        self.emit_progress("sns", "loading Moments…", 0, 0);
        loop {
            let q = SnsTimelineQuery { limit: page, offset: 0, usernames: usernames.clone(), keyword: keyword.map(str::to_string), start: opts.start, end: end_ts };
            let batch = match self.sns_timeline_with(&wcdb, &q) {
                Ok(b) if !b.is_empty() => b,
                _ => break,
            };
            let last_ts = batch.last().and_then(|p| p.get("createTime").and_then(Value::as_i64)).unwrap_or(0) - 1;
            let n = batch.len();
            posts.extend(batch);
            end_ts = last_ts;
            self.emit_progress("sns", "loading Moments…", posts.len(), 0);
            if (n as i32) < page || (opts.start > 0 && last_ts < opts.start) {
                break;
            }
        }
        if posts.is_empty() {
            return Ok(json!({ "success": true, "filePath": "", "postCount": 0, "mediaCount": 0 }));
        }

        // 2. media
        let media_dir = opts.output_dir.join("media");
        let mut media_count = 0usize;
        if want_media {
            std::fs::create_dir_all(&media_dir).map_err(|e| AppError::runtime(e.to_string()))?;
            #[derive(Clone)]
            struct Task {
                kind: &'static str,
                url: String,
                key: Option<String>,
                post_idx: usize,
                media_idx: usize,
                post_id: String,
            }
            let mut tasks = Vec::new();
            for (pi, post) in posts.iter().enumerate() {
                let post_id = post.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                for (mi, m) in post.get("media").and_then(Value::as_array).cloned().unwrap_or_default().iter().enumerate() {
                    let url = m.get("url").and_then(Value::as_str).unwrap_or("").to_string();
                    let key = m.get("key").and_then(Value::as_str).map(str::to_string);
                    let video = sns::is_video_url(&url);
                    if want_images && !video && !url.is_empty() {
                        tasks.push(Task { kind: "image", url: url.clone(), key: key.clone(), post_idx: pi, media_idx: mi, post_id: post_id.clone() });
                    }
                    if want_videos && video && !url.is_empty() {
                        tasks.push(Task { kind: "video", url: url.clone(), key: key.clone(), post_idx: pi, media_idx: mi, post_id: post_id.clone() });
                    }
                    if let Some(lp) = m.get("livePhoto").filter(|_| want_live) {
                        if let Some(lurl) = lp.get("url").and_then(Value::as_str).filter(|u| !u.is_empty()) {
                            tasks.push(Task {
                                kind: "livephoto",
                                url: lurl.to_string(),
                                key: lp.get("key").and_then(Value::as_str).filter(|k| !k.is_empty()).map(str::to_string).or(key.clone()),
                                post_idx: pi,
                                media_idx: mi,
                                post_id: post_id.clone(),
                            });
                        }
                    }
                }
            }
            let total = tasks.len();
            let mut done = 0usize;
            let media_dir_ref = &media_dir;
            let results: Vec<(Task, Option<String>)> = futures::stream::iter(tasks)
                .map(|task| async move {
                    let is_video = task.kind == "video" || task.kind == "livephoto" || sns::is_video_url(&task.url);
                    let ext = if is_video { "mp4" } else { "jpg" };
                    let suffix = if task.kind == "livephoto" { "_live" } else { "" };
                    let file_name = format!("{}_{}{}.{}", task.post_id, task.media_idx, suffix, ext);
                    let path = media_dir_ref.join(&file_name);
                    if path.exists() {
                        return (task, Some(format!("media/{file_name}")));
                    }
                    let ok = match self.sns_fetch_media(&task.url, task.key.as_deref()).await {
                        Ok(f) => match (f.data, f.cache_path) {
                            (Some(d), _) => std::fs::write(&path, d).is_ok(),
                            (None, Some(c)) => std::fs::copy(c, &path).is_ok(),
                            _ => false,
                        },
                        Err(_) => false,
                    };
                    (task, ok.then(|| format!("media/{file_name}")))
                })
                .buffer_unordered(5)
                .inspect(|_| {
                    done += 1;
                    self.emit_progress("sns-media", "downloading media", done, total);
                })
                .collect()
                .await;
            for (task, local) in results {
                let Some(local) = local else { continue };
                media_count += 1;
                if let Some(m) = posts[task.post_idx].get_mut("media").and_then(|m| m.get_mut(task.media_idx)).and_then(Value::as_object_mut) {
                    if task.kind == "livephoto" {
                        if let Some(lp) = m.get_mut("livePhoto").and_then(Value::as_object_mut) {
                            lp.insert("localPath".into(), json!(local));
                        }
                    } else {
                        m.insert("localPath".into(), json!(local));
                    }
                }
            }
        }

        // 2.5 avatars (html only)
        let mut avatar_map: HashMap<String, String> = HashMap::new();
        if opts.format == "html" {
            std::fs::create_dir_all(&media_dir).map_err(|e| AppError::runtime(e.to_string()))?;
            let mut unique: Vec<(String, String)> = Vec::new();
            for p in &posts {
                if let (Some(u), Some(a)) = (p.get("username").and_then(Value::as_str), p.get("avatarUrl").and_then(Value::as_str).filter(|a| !a.is_empty())) {
                    if !unique.iter().any(|(x, _)| x == u) {
                        unique.push((u.to_string(), a.to_string()));
                    }
                }
            }
            let total = unique.len();
            let media_dir_ref = &media_dir;
            let fetched: Vec<(String, Option<String>)> = futures::stream::iter(unique)
                .map(|(user, avatar)| async move {
                    let file_name = format!("avatar_{}.jpg", &md5_hex(user.as_bytes())[..8]);
                    let path = media_dir_ref.join(&file_name);
                    if path.exists() {
                        return (user, Some(format!("media/{file_name}")));
                    }
                    let saved = match self.sns_fetch_media(&avatar, None).await {
                        Ok(f) => f.data.map_or(false, |d| std::fs::write(&path, d).is_ok()),
                        Err(_) => false,
                    };
                    (user, saved.then(|| format!("media/{file_name}")))
                })
                .buffer_unordered(5)
                .collect()
                .await;
            self.emit_progress("sns-avatar", "downloading avatars", total, total);
            for (user, local) in fetched {
                if let Some(l) = local {
                    avatar_map.insert(user, l);
                }
            }
        }

        // 3. output file
        let now_utc = chrono::Utc::now();
        let stamp = now_utc.format("%Y-%m-%dT%H-%M-%S").to_string();
        let export_time = now_utc.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        let filters = json!({ "usernames": usernames, "keyword": keyword.unwrap_or("") });
        let out_path;
        match opts.format.as_str() {
            "json" => {
                out_path = opts.output_dir.join(format!("朋友圈导出_{stamp}.json"));
                let posts_json: Vec<Value> = posts.iter().map(|p| export_post_json(p, false)).collect();
                let data = json!({ "exportTime": export_time, "totalPosts": posts.len(), "filters": filters, "posts": posts_json });
                std::fs::write(&out_path, serde_json::to_string_pretty(&data).unwrap()).map_err(|e| AppError::runtime(e.to_string()))?;
            }
            "arkmejson" => {
                out_path = opts.output_dir.join(format!("朋友圈导出_{stamp}.json"));
                let mut identity_cache: HashMap<String, Option<Value>> = HashMap::new();
                let uniq: Vec<String> = {
                    let mut seen = std::collections::BTreeSet::new();
                    posts.iter().filter_map(|p| p.get("username").and_then(Value::as_str)).filter(|u| seen.insert(u.to_string())).map(str::to_string).collect()
                };
                let book = self.contact_book(&wcdb, &uniq);
                let mut built = Vec::with_capacity(posts.len());
                for (i, post) in posts.iter().enumerate() {
                    let username = post.get("username").and_then(Value::as_str).unwrap_or("");
                    let author = self.contact_identity(&wcdb, &book, &mut identity_cache, username).unwrap_or_else(|| {
                        json!({ "username": username, "wxid": username, "displayName": post.get("nickname").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(username) })
                    });
                    let (likes_detail, comments_detail) = self.arkme_interaction_details(&wcdb, &book, &mut identity_cache, post);
                    let base = export_post_json(post, true);
                    let b = base.as_object().cloned().unwrap_or_default();
                    let mut o = Map::new();
                    for k in ["id", "username", "nickname"] {
                        o.insert(k.into(), b.get(k).cloned().unwrap_or(Value::Null));
                    }
                    o.insert("author".into(), author);
                    for k in ["createTime", "createTimeStr", "contentDesc", "type", "media", "likes", "comments", "location"] {
                        if let Some(v) = b.get(k) {
                            o.insert(k.into(), v.clone());
                        }
                    }
                    o.insert("likesDetail".into(), Value::Array(likes_detail));
                    o.insert("commentsDetail".into(), Value::Array(comments_detail));
                    for k in ["linkTitle", "linkUrl"] {
                        if let Some(v) = b.get(k) {
                            o.insert(k.into(), v.clone());
                        }
                    }
                    built.push(Value::Object(o));
                    if (i + 1) % 20 == 0 || i + 1 == posts.len() {
                        self.emit_progress("sns-arkme", "building ArkmeJSON", i + 1, posts.len());
                    }
                }
                let owner_wxid = Some(self.my_wxid_cleaned()).filter(|w| !w.is_empty());
                let record_owner = match owner_wxid.as_deref() {
                    Some(w) => self.contact_identity(&wcdb, &book, &mut identity_cache, w).unwrap_or_else(|| json!({ "username": w, "wxid": w, "displayName": w })),
                    None => json!({ "username": "", "wxid": "", "displayName": "" }),
                };
                let data = json!({
                    "exportTime": export_time,
                    "format": "arkmejson",
                    "schemaVersion": "1.0.0",
                    "recordOwner": record_owner,
                    "mediaSelection": { "images": want_images, "livePhotos": want_live, "videos": want_videos },
                    "totalPosts": posts.len(),
                    "filters": filters,
                    "posts": built
                });
                std::fs::write(&out_path, serde_json::to_string_pretty(&data).unwrap()).map_err(|e| AppError::runtime(e.to_string()))?;
            }
            _ => {
                out_path = opts.output_dir.join(format!("朋友圈导出_{stamp}.html"));
                let html = sns::generate_html(&posts, usernames, keyword, &avatar_map);
                std::fs::write(&out_path, html).map_err(|e| AppError::runtime(e.to_string()))?;
            }
        }
        self.emit_progress("sns", "export finished", posts.len(), posts.len());
        Ok(json!({ "success": true, "filePath": out_path, "postCount": posts.len(), "mediaCount": media_count }))
    }

    /// `buildArkmeInteractionDetails`
    fn arkme_interaction_details(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        book: &HashMap<String, CachedContact>,
        cache: &mut HashMap<String, Option<Value>>,
        post: &Value,
    ) -> (Vec<Value>, Vec<Value>) {
        let raw_xml = post.get("rawXml").and_then(Value::as_str).unwrap_or("");
        let xml_likes = sns::parse_like_users_from_xml(raw_xml);
        let legacy_likes: Vec<sns::LikeUser> = post
            .get("likes")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(|n| sns::LikeUser { username: None, nickname: Some(n.to_string()) }).collect())
            .unwrap_or_default();
        let (like_candidates, like_source) = if xml_likes.is_empty() { (legacy_likes, "legacy") } else { (xml_likes, "xml") };
        let mut likes_detail = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for like in like_candidates {
            let identity = like.username.as_deref().and_then(|u| self.contact_identity(wcdb, book, cache, u));
            let idv = |k: &str| identity.as_ref().and_then(|i| i.get(k)).and_then(Value::as_str).map(str::to_string);
            let nickname = like.nickname.clone().filter(|n| !n.is_empty()).or_else(|| idv("displayName")).or_else(|| like.username.clone()).unwrap_or_default();
            let username = idv("username").or_else(|| like.username.clone());
            let key = format!("{}|{}", username.clone().unwrap_or_default(), nickname);
            if !seen.insert(key) {
                continue;
            }
            let mut o = Map::new();
            o.insert("nickname".into(), json!(nickname));
            for (k, v) in [
                ("username", username.clone()),
                ("wxid", username.clone()),
                ("alias", idv("alias")),
                ("wechatId", idv("wechatId")),
                ("remark", idv("remark")),
                ("nickName", idv("nickName")),
            ] {
                if let Some(v) = v {
                    o.insert(k.into(), json!(v));
                }
            }
            o.insert("displayName".into(), json!(idv("displayName").filter(|d| !d.is_empty()).or_else(|| Some(nickname.clone()).filter(|n| !n.is_empty())).or(username).unwrap_or_default()));
            o.insert("source".into(), json!(like_source));
            likes_detail.push(Value::Object(o));
        }

        let xml_comments = sns::parse_comments_from_xml(raw_xml);
        let post_comments: Vec<Value> = post.get("comments").and_then(Value::as_array).cloned().unwrap_or_default();
        let by_id: HashMap<String, &Value> = post_comments.iter().filter_map(|c| c.get("id").and_then(Value::as_str).filter(|i| !i.is_empty()).map(|i| (i.to_string(), c))).collect();
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let non_empty_arr = |v: Option<&Value>| v.filter(|e| e.as_array().map_or(false, |a| !a.is_empty())).cloned();

        let mut base: Vec<Value> = Vec::new();
        if !xml_comments.is_empty() {
            for c in &xml_comments {
                let fb = c.get("id").and_then(Value::as_str).and_then(|i| by_id.get(i)).copied();
                let pick = |k: &str| -> String {
                    let own = s(c, k);
                    if !own.is_empty() {
                        own
                    } else {
                        fb.map(|f| s(f, k)).unwrap_or_default()
                    }
                };
                let mut o = Map::new();
                o.insert("id".into(), json!(pick("id")));
                o.insert("nickname".into(), json!(pick("nickname")));
                if let Some(u) = c.get("username") {
                    o.insert("username".into(), u.clone());
                }
                o.insert("content".into(), json!(pick("content")));
                o.insert("refCommentId".into(), json!(pick("refCommentId")));
                if let Some(u) = c.get("refUsername") {
                    o.insert("refUsername".into(), u.clone());
                }
                let ref_nick = c.get("refNickname").filter(|v| truthy_json(v)).cloned().or_else(|| fb.and_then(|f| f.get("refNickname")).cloned());
                if let Some(r) = ref_nick {
                    o.insert("refNickname".into(), r);
                }
                if let Some(e) = non_empty_arr(c.get("emojis")).or_else(|| fb.and_then(|f| f.get("emojis")).cloned()) {
                    o.insert("emojis".into(), e);
                }
                base.push(Value::Object(o));
            }
            let mapped: std::collections::HashSet<String> = base.iter().filter_map(|c| c.get("id").and_then(Value::as_str).filter(|i| !i.is_empty()).map(str::to_string)).collect();
            for c in &post_comments {
                if c.get("id").and_then(Value::as_str).map_or(false, |i| !i.is_empty() && mapped.contains(i)) {
                    continue;
                }
                base.push(legacy_comment(c));
            }
        } else {
            base.extend(post_comments.iter().map(legacy_comment));
        }

        let comment_source = if xml_comments.is_empty() { "legacy" } else { "xml" };
        let mut comments_detail = Vec::new();
        for c in &base {
            let username = c.get("username").and_then(Value::as_str).filter(|u| !u.is_empty()).map(str::to_string);
            let ref_username = c.get("refUsername").and_then(Value::as_str).filter(|u| !u.is_empty()).map(str::to_string);
            let actor = username.as_deref().and_then(|u| self.contact_identity(wcdb, book, cache, u));
            let ref_actor = ref_username.as_deref().and_then(|u| self.contact_identity(wcdb, book, cache, u));
            let av = |a: &Option<Value>, k: &str| a.as_ref().and_then(|i| i.get(k)).and_then(Value::as_str).map(str::to_string);
            let nickname = Some(s(c, "nickname")).filter(|n| !n.is_empty()).or_else(|| av(&actor, "displayName")).or_else(|| username.clone()).unwrap_or_default();
            let eff_username = av(&actor, "username").or_else(|| username.clone());
            let eff_ref_username = av(&ref_actor, "username").or_else(|| ref_username.clone());
            let mut o = Map::new();
            o.insert("id".into(), json!(s(c, "id")));
            o.insert("nickname".into(), json!(nickname));
            let mut put = |k: &str, v: Option<String>| {
                if let Some(v) = v {
                    o.insert(k.into(), json!(v));
                }
            };
            put("username", eff_username.clone());
            put("wxid", eff_username.clone());
            put("alias", av(&actor, "alias"));
            put("wechatId", av(&actor, "wechatId"));
            put("remark", av(&actor, "remark"));
            put("nickName", av(&actor, "nickName"));
            o.insert("displayName".into(), json!(av(&actor, "displayName").filter(|d| !d.is_empty()).or_else(|| Some(nickname.clone()).filter(|n| !n.is_empty())).or(eff_username).unwrap_or_default()));
            o.insert("content".into(), json!(s(c, "content")));
            o.insert("refCommentId".into(), json!(s(c, "refCommentId")));
            let ref_nickname = Some(s(c, "refNickname")).filter(|n| !n.is_empty()).or_else(|| av(&ref_actor, "displayName"));
            let mut put2 = |k: &str, v: Option<String>| {
                if let Some(v) = v {
                    o.insert(k.into(), json!(v));
                }
            };
            put2("refNickname", ref_nickname);
            put2("refUsername", eff_ref_username.clone());
            put2("refWxid", eff_ref_username);
            put2("refAlias", av(&ref_actor, "alias"));
            put2("refWechatId", av(&ref_actor, "wechatId"));
            put2("refRemark", av(&ref_actor, "remark"));
            put2("refNickName", av(&ref_actor, "nickName"));
            put2("refDisplayName", av(&ref_actor, "displayName"));
            if let Some(e) = c.get("emojis").filter(|v| !v.is_null()) {
                o.insert("emojis".into(), e.clone());
            }
            o.insert("source".into(), json!(comment_source));
            comments_detail.push(Value::Object(o));
        }
        (likes_detail, comments_detail)
    }

    /// Adds `media.proxyUrl` style fields used by the HTTP timeline (`enrichSnsTimelineMedia`).
    pub async fn sns_enrich_timeline_media(&self, posts: Vec<Value>, proxy_base: &str, inline: bool, replace: bool) -> Vec<Value> {
        let mut out = Vec::with_capacity(posts.len());
        for post in posts {
            let media_list = post.get("media").and_then(Value::as_array).cloned().unwrap_or_default();
            if media_list.is_empty() {
                out.push(post);
                continue;
            }
            let mut next_media = Vec::with_capacity(media_list.len());
            for media in &media_list {
                let raw_url = media.get("url").and_then(Value::as_str).unwrap_or("").to_string();
                let raw_thumb = media.get("thumb").and_then(Value::as_str).unwrap_or("").to_string();
                let media_key = media_key_string(media.get("key"));
                let (url_resolved, url_proxy) = self.resolve_media_url(proxy_base, &raw_url, media_key.as_deref(), inline).await;
                let (thumb_resolved, thumb_proxy) = self.resolve_media_url(proxy_base, &raw_thumb, media_key.as_deref(), inline).await;
                let mut item = media.as_object().cloned().unwrap_or_default();
                item.insert("rawUrl".into(), json!(raw_url));
                item.insert("rawThumb".into(), json!(raw_thumb));
                set_opt(&mut item, "resolvedUrl", url_resolved.clone());
                set_opt(&mut item, "resolvedThumbUrl", thumb_resolved.clone());
                set_opt(&mut item, "proxyUrl", url_proxy);
                set_opt(&mut item, "proxyThumbUrl", thumb_proxy);
                if replace {
                    item.insert("url".into(), json!(url_resolved.unwrap_or_else(|| raw_url.clone())));
                    item.insert("thumb".into(), json!(thumb_resolved.unwrap_or_else(|| raw_thumb.clone())));
                }
                if let Some(lp) = media.get("livePhoto").filter(|v| v.is_object()) {
                    let raw_l_url = lp.get("url").and_then(Value::as_str).unwrap_or("").to_string();
                    let raw_l_thumb = lp.get("thumb").and_then(Value::as_str).unwrap_or("").to_string();
                    let live_key = media_key_string(lp.get("key")).or_else(|| media_key.clone());
                    let (lu, lu_proxy) = self.resolve_media_url(proxy_base, &raw_l_url, live_key.as_deref(), inline).await;
                    let (lt, lt_proxy) = self.resolve_media_url(proxy_base, &raw_l_thumb, live_key.as_deref(), inline).await;
                    let mut live = lp.as_object().cloned().unwrap_or_default();
                    live.insert("rawUrl".into(), json!(raw_l_url));
                    live.insert("rawThumb".into(), json!(raw_l_thumb));
                    set_opt(&mut live, "resolvedUrl", lu.clone());
                    set_opt(&mut live, "resolvedThumbUrl", lt.clone());
                    set_opt(&mut live, "proxyUrl", lu_proxy);
                    set_opt(&mut live, "proxyThumbUrl", lt_proxy);
                    if replace {
                        live.insert("url".into(), json!(lu.unwrap_or_else(|| raw_l_url.clone())));
                        live.insert("thumb".into(), json!(lt.unwrap_or_else(|| raw_l_thumb.clone())));
                    }
                    item.insert("livePhoto".into(), Value::Object(live));
                }
                next_media.push(Value::Object(item));
            }
            let mut p = post.as_object().cloned().unwrap_or_default();
            p.insert("media".into(), Value::Array(next_media));
            out.push(Value::Object(p));
        }
        out
    }

    async fn resolve_media_url(&self, proxy_base: &str, raw_url: &str, key: Option<&str>, inline: bool) -> (Option<String>, Option<String>) {
        let target = raw_url.trim();
        if target.is_empty() {
            return (None, None);
        }
        let mut proxy = format!("{}/api/v1/sns/media/proxy?url={}", proxy_base.trim_end_matches('/'), url_encode(target));
        if let Some(k) = key {
            proxy.push_str(&format!("&key={}", url_encode(k)));
        }
        if !inline {
            return (Some(proxy.clone()), Some(proxy));
        }
        if let Ok(SnsProxyResult::DataUrl(d)) = self.sns_proxy_image(raw_url, key).await {
            return (Some(d), Some(proxy));
        }
        (Some(proxy.clone()), Some(proxy))
    }
}

fn truthy_json(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64().map_or(true, |f| f != 0.0),
        _ => true,
    }
}

fn set_opt(o: &mut Map<String, Value>, k: &str, v: Option<String>) {
    match v {
        Some(v) => {
            o.insert(k.into(), json!(v));
        }
        None => {
            o.shift_remove(k);
        }
    }
}

/// `toSnsMediaKey` rendered back to text (numbers and strings both become their string form).
pub fn media_key_string(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        _ => None,
    }
}

pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn legacy_comment(c: &Value) -> Value {
    let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut o = Map::new();
    o.insert("id".into(), json!(s("id")));
    o.insert("nickname".into(), json!(s("nickname")));
    o.insert("content".into(), json!(s("content")));
    o.insert("refCommentId".into(), json!(s("refCommentId")));
    if let Some(r) = c.get("refNickname").filter(|v| !v.is_null()) {
        o.insert("refNickname".into(), r.clone());
    }
    if let Some(e) = c.get("emojis").filter(|v| !v.is_null()) {
        o.insert("emojis".into(), e.clone());
    }
    Value::Object(o)
}

/// One post as written into the JSON / ArkmeJSON export (`undefined` fields dropped).
fn export_post_json(p: &Value, arkme: bool) -> Value {
    let get = |k: &str| p.get(k).filter(|v| !v.is_null()).cloned();
    let mut o = Map::new();
    for k in ["id", "username", "nickname"] {
        o.insert(k.into(), p.get(k).cloned().unwrap_or(Value::Null));
    }
    let create_time = p.get("createTime").and_then(Value::as_i64).unwrap_or(0);
    o.insert("createTime".into(), json!(create_time));
    o.insert("createTimeStr".into(), json!(sns::zh_locale_from_ts(create_time)));
    o.insert("contentDesc".into(), p.get("contentDesc").cloned().unwrap_or(Value::Null));
    if let Some(t) = get("type") {
        o.insert("type".into(), t);
    }
    let media: Vec<Value> = p
        .get("media")
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .map(|m| {
            let mut mo = Map::new();
            mo.insert("url".into(), m.get("url").cloned().unwrap_or(Value::Null));
            mo.insert("thumb".into(), m.get("thumb").cloned().unwrap_or(Value::Null));
            if let Some(l) = m.get("localPath").filter(|v| truthy_json(v)) {
                mo.insert("localPath".into(), l.clone());
            }
            if arkme {
                if let Some(lp) = m.get("livePhoto").filter(|v| v.is_object()) {
                    let mut lo = Map::new();
                    lo.insert("url".into(), lp.get("url").cloned().unwrap_or(Value::Null));
                    lo.insert("thumb".into(), lp.get("thumb").cloned().unwrap_or(Value::Null));
                    if let Some(l) = lp.get("localPath").filter(|v| truthy_json(v)) {
                        lo.insert("localPath".into(), l.clone());
                    }
                    mo.insert("livePhoto".into(), Value::Object(lo));
                }
            }
            Value::Object(mo)
        })
        .collect();
    o.insert("media".into(), Value::Array(media));
    o.insert("likes".into(), p.get("likes").cloned().unwrap_or_else(|| json!([])));
    o.insert("comments".into(), p.get("comments").cloned().unwrap_or_else(|| json!([])));
    if let Some(l) = get("location") {
        o.insert("location".into(), l);
    }
    for k in ["linkTitle", "linkUrl"] {
        if let Some(v) = get(k) {
            o.insert(k.into(), v);
        }
    }
    Value::Object(o)
}
