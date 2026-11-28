//! 朋友圈中要用到网络和数据库的部分：时间线查询、统计的缓存、媒体的下载和解密、导出，以及评论表情的下载。

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use md5::{Digest, Md5};
use serde_json::{json, Map, Value};

use super::*;
use crate::contract::{
    SnsComment, SnsEmojiSaved, SnsExported, SnsPost, SnsResourceHeaders, SnsResourceProbe,
    SnsStats, UserPosts,
};
use crate::fs_atomic::{copy_atomically, write_atomically};
use crate::isaac64;
use crate::sns::{self, CachedContact, SavedMedia};
use crate::sync::MutexExt;
use weflow_native::wcdb::Databases;

const EXPORT_STATS_TTL: Duration = Duration::from_secs(5 * 60);
const USER_COUNTS_TTL: Duration = Duration::from_secs(5 * 60);
const TIMELINE_FALLBACK_COOLDOWN: Duration = Duration::from_secs(3 * 60);
const IMAGE_CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const IMAGE_CACHE_MAX: usize = 120;
const VIDEO_HEADER_BYTES: usize = 131_072;

#[derive(Default)]
pub(crate) struct SnsState {
    export_stats: Option<(SnsStats, Instant)>,
    user_counts: Option<(BTreeMap<String, i64>, Instant)>,
    last_timeline_fallback: Option<Instant>,
    image_cache: HashMap<String, (String, Instant)>,
    image_order: VecDeque<String>,
    /// 访问微信媒体服务器的客户端，保留下来让请求复用连接（#202）：一个页面一次要一屏的图片，
    /// 每张图片新建一个客户端就要每次重新做 DNS 解析和 TLS 握手。
    client: Option<reqwest::Client>,
    /// 访问证书上写着别的主机名的表情主机的客户端（#814），与 `client` 一样保留下来。
    unchecked_client: Option<reqwest::Client>,
}

/// `sns timeline` 列出哪些动态。
#[derive(Clone, Debug, Default)]
pub struct SnsTimelineQuery {
    /// 最多的动态数。
    pub limit: i32,
    /// 从新到旧跳过的动态数。
    pub offset: i32,
    /// 只列这些 wxid 的动态；为空时列出所有人的。
    pub usernames: Vec<String>,
    /// 只列含有这段文字的动态。
    pub keyword: Option<String>,
    /// Unix 秒：只列从这时起的动态；0 表示不限。
    pub start: i64,
    /// Unix 秒：只列到这时为止的动态；0 表示不限。
    pub end: i64,
}

/// `sns export` 写出什么。
#[derive(Clone, Debug, Default)]
pub struct SnsExportOptions {
    /// 导出到的文件夹。
    pub output_dir: PathBuf,
    /// `json`、`html` 或 `arkmejson`，不区分大小写；其他值被拒绝。
    pub format: String,
    /// 只导出这些 wxid 的动态；为空时导出所有人的。
    pub usernames: Vec<String>,
    /// 只导出含有这段文字的动态。
    pub keyword: Option<String>,
    /// 同时把图片、实况照片和视频保存在导出文件旁边。
    pub export_media: bool,
    /// Unix 秒：只导出从这时起的动态；0 表示不限。
    pub start: i64,
    /// Unix 秒：只导出到这时为止的动态；0 表示不限。
    pub end: i64,
}

/// `sns export` 能写的格式（[`SnsExportOptions::format`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SnsExportFormat {
    Json,
    Html,
    ArkmeJson,
}

impl SnsExportFormat {
    /// `name` 指的格式，不区分大小写。
    fn parse(name: &str) -> AppResult<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "html" => Ok(Self::Html),
            "arkmejson" => Ok(Self::ArkmeJson),
            other => Err(AppError::usage(format!(
                "unsupported sns export format: {other}; supported: json, html, arkmejson"
            ))),
        }
    }

    /// 写出的文件的扩展名，不带点。
    fn extension(self) -> &'static str {
        match self {
            Self::Json | Self::ArkmeJson => "json",
            Self::Html => "html",
        }
    }
}

/// `sns export` 保存的媒体可能的内容：内容类型，以及保存时用的扩展名。
const MEDIA_EXTENSIONS: [(&str, &str); 8] = [
    ("image/jpeg", "jpg"),
    ("image/png", "png"),
    ("image/gif", "gif"),
    ("image/webp", "webp"),
    ("image/bmp", "bmp"),
    ("image/avif", "avif"),
    ("image/heic", "heic"),
    ("video/mp4", "mp4"),
];

/// 内容类型为 `content_type` 的媒体保存时用的扩展名；不在 [`MEDIA_EXTENSIONS`] 中时，图片用 `jpg`，视频用 `mp4`。
fn media_extension(content_type: &str, is_video: bool) -> &'static str {
    MEDIA_EXTENSIONS
        .iter()
        .find(|(mime, _)| *mime == content_type)
        .map_or(if is_video { "mp4" } else { "jpg" }, |(_, ext)| *ext)
}

/// 以前的导出在 `dir` 中以 `stem` 为名保存的文件，不论扩展名：媒体的扩展名要取到它之后才知道。
fn saved_media(dir: &Path, stem: &str) -> Option<String> {
    MEDIA_EXTENSIONS
        .iter()
        .map(|(_, ext)| format!("{stem}.{ext}"))
        .find(|name| dir.join(name).exists())
}

/// 把取到的媒体保存到 `dir`，文件名为 `stem` 加上按内容决定的扩展名；返回文件名，写不进去时为 `None`。
fn save_fetched(dir: &Path, stem: &str, fetched: SnsMediaFetch, is_video: bool) -> Option<String> {
    let name = format!(
        "{stem}.{}",
        media_extension(&fetched.content_type, is_video)
    );
    let path = dir.join(&name);
    let written = match (fetched.data, fetched.cache_path) {
        (Some(data), _) => write_atomically(&path, &data).is_ok(),
        (None, Some(cached)) => copy_atomically(&cached, &path).is_ok(),
        (None, None) => false,
    };
    written.then_some(name)
}

/// 取到的一张朋友圈图片或一段视频（[`ServiceHub::sns_fetch_media`]）。
#[derive(Clone, Debug, Default)]
pub struct SnsMediaFetch {
    /// 文件内容，有密钥时已解密；视频为 `None`，只保存在磁盘上。
    pub data: Option<Vec<u8>>,
    /// 它的 MIME 类型。
    pub content_type: String,
    /// WeFlow 保存它的位置；没有保存时为 `None`。
    pub cache_path: Option<PathBuf>,
}

/// 朋友圈媒体代理对一个地址的回答。
#[derive(Clone, Debug)]
pub enum SnsProxyResult {
    /// 图片，写成 data URL。
    DataUrl(String),
    /// 视频，即 WeFlow 保存它的文件；没有保存时为 `None`。
    VideoPath(Option<PathBuf>),
}

fn md5_hex(data: &[u8]) -> String {
    let mut h = Md5::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn err_native(e: anyhow::Error) -> AppError {
    AppError::from_native(e)
}

fn truthy_key(key: Option<&str>) -> Option<&str> {
    key.map(str::trim).filter(|k| !k.is_empty())
}

/// 按开头的几个字节判断缓存的媒体是不是图片，不必为此读完整个视频。
fn cached_is_image(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 16];
    let read = std::fs::File::open(path).and_then(|mut f| f.read(&mut head));
    read.is_ok_and(|n| sns::detect_image_mime(&head[..n], "").starts_with("image/"))
}

fn export_stats_value(total_posts: i64, total_friends: i64, my_posts: Option<i64>) -> SnsStats {
    SnsStats {
        total_posts,
        total_friends,
        my_posts,
    }
}

impl ServiceHub {
    // ── 路径 ──

    fn sns_cache_dir(&self) -> PathBuf {
        let dir = self.cache_base().join("sns_cache");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    pub(super) fn emoji_cache_dir(&self) -> PathBuf {
        let dir = self.cache_base().join("Emojis");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn sns_cache_file(&self, url: &str) -> PathBuf {
        let ext = if sns::is_video_url(url) {
            ".mp4"
        } else {
            ".jpg"
        };
        self.sns_cache_dir()
            .join(format!("{}{}", md5_hex(url.as_bytes()), ext))
    }

    // ── 联系人 ──

    fn contact_identity(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        book: &HashMap<String, CachedContact>,
        cache: &mut HashMap<String, Option<Value>>,
        username: &str,
    ) -> Option<Value> {
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
            .or_else(|| {
                book.get(username)
                    .and_then(|c| c.display_name.clone())
                    .filter(|d| !d.is_empty())
            })
            .unwrap_or_else(|| username.to_string());
        let mut o = Map::new();
        o.insert("username".into(), json!(username));
        o.insert("wxid".into(), json!(username));
        for (k, v) in [
            ("alias", alias.clone()),
            ("wechatId", alias),
            ("remark", remark),
            ("nickName", nick),
        ] {
            if let Some(v) = v {
                o.insert(k.into(), json!(v));
            }
        }
        o.insert("displayName".into(), json!(display));
        let v = Some(Value::Object(o));
        cache.insert(username.to_string(), v.clone());
        v
    }

    // ── 时间线 ──

    /// 对应桌面版的 `snsService.getTimeline`：补全了修正后的地址、评论、位置和头像的动态。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn sns_timeline_query(&self, q: &SnsTimelineQuery) -> AppResult<Vec<SnsPost>> {
        let wcdb = self.open_wcdb()?;
        self.sns_timeline_with(&wcdb, q)
    }

    fn sns_timeline_with(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        q: &SnsTimelineQuery,
    ) -> AppResult<Vec<SnsPost>> {
        let mut asked = std::collections::HashSet::new();
        let usernames: Vec<String> = q
            .usernames
            .iter()
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty() && asked.insert(u.clone()))
            .collect();
        let posts = wcdb
            .sns_timeline(
                q.limit,
                q.offset,
                &usernames,
                q.keyword.as_deref(),
                crate::num::saturating_i32(q.start),
                crate::num::saturating_i32(q.end),
            )
            .map_err(err_native)?;
        if posts.is_empty() {
            return Ok(Vec::new());
        }
        let users: Vec<String> = {
            let mut seen = std::collections::BTreeSet::new();
            posts
                .iter()
                .map(|p| p.username.as_str())
                .filter(|u| seen.insert(u.to_string()))
                .map(str::to_string)
                .collect()
        };
        let book = self.contact_book(wcdb, &users);
        Ok(posts
            .into_iter()
            .map(|p| {
                let contact = book.get(p.username.as_str());
                sns::enrich_post(p, contact)
            })
            .collect())
    }

    /// 在朋友圈发过动态的人，最近发的在前（`sns users`）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn sns_usernames_list(&self) -> AppResult<Vec<String>> {
        let wcdb = self.open_wcdb()?;
        let raw = wcdb.sns_usernames().map_err(err_native)?;
        let direct: Vec<String> = raw
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|u| u.as_str())
                    .map(|u| u.trim().to_string())
                    .filter(|u| !u.is_empty())
                    .collect()
            })
            .unwrap_or_default();
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

    fn scan_timeline_rows(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        max_rounds: usize,
        mut visit: impl FnMut(&weflow_native::sns::SnsPost),
    ) -> AppResult<()> {
        let page = 500;
        let mut offset = 0;
        for _ in 0..max_rounds {
            let rows = wcdb
                .sns_timeline(page, offset, &[], None, 0, 0)
                .map_err(err_native)?;
            if rows.is_empty() {
                break;
            }
            rows.iter().for_each(&mut visit);
            let got = crate::num::saturating_i32(rows.len());
            if got < page {
                break;
            }
            offset = offset.saturating_add(got);
        }
        Ok(())
    }

    fn collect_timeline_usernames(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        max_rounds: usize,
    ) -> AppResult<Vec<String>> {
        let mut seen = std::collections::BTreeSet::new();
        let mut order = Vec::new();
        self.scan_timeline_rows(wcdb, max_rounds, |row| {
            let u = row.username.trim().to_string();
            if !u.is_empty() && seen.insert(u.clone()) {
                order.push(u);
            }
        })?;
        Ok(order)
    }

    // ── 统计 ──

    fn export_stats_from_table(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        my_wxid: Option<&str>,
    ) -> (i64, i64, Option<i64>) {
        match wcdb.sns_export_stats(my_wxid) {
            Ok(raw) if raw.is_object() => {
                let n = |k: &str| {
                    raw.get(k)
                        .and_then(|v| {
                            v.as_i64()
                                .or_else(|| v.as_f64().map(crate::num::saturating_i64))
                                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                        })
                        .unwrap_or(0)
                };
                let my = match raw.get("my_posts") {
                    None | Some(Value::Null) => None,
                    Some(_) => Some(n("my_posts")),
                };
                (n("total_posts"), n("total_friends"), my)
            }
            _ => (0, 0, my_wxid.map(|_| 0)),
        }
    }

    fn export_stats_from_timeline(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        my_wxid: Option<&str>,
    ) -> AppResult<(i64, i64, Option<i64>)> {
        let (mut total, mut mine) = (0i64, 0i64);
        let mut users = std::collections::HashSet::new();
        self.scan_timeline_rows(wcdb, 2000, |row| {
            total += 1;
            let u = row.username.trim().to_string();
            if !u.is_empty() {
                if my_wxid == Some(u.as_str()) {
                    mine += 1;
                }
                users.insert(u);
            }
        })?;
        Ok((
            total,
            crate::num::count_i64(users.len()),
            my_wxid.map(|_| mine),
        ))
    }

    /// 朋友圈的数字：返回 `{totalPosts, totalFriends, myPosts}`。
    ///
    /// [`Freshness::Cached`]（`--prefer-cache`）时，保存的结果没过期就直接用，也不走较慢的遍历时间线的后备。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，数据库打不开时返回数据库层的错误；
    /// 表里没有数据、遍历时间线也失败时返回那次失败的错误。
    pub fn sns_export_stats(&self, freshness: Freshness) -> AppResult<SnsStats> {
        let prefer_cache = freshness == Freshness::Cached;
        let allow_fallback = !prefer_cache;
        let my_wxid = Some(self.my_wxid_cleaned()).filter(|w| !w.trim().is_empty());
        let cached = self.sns_state.lock_or_panic().export_stats;
        if prefer_cache {
            if let Some((v, at)) = &cached {
                if at.elapsed() <= EXPORT_STATS_TTL {
                    return Ok(*v);
                }
            }
        }
        let wcdb = self.open_wcdb()?;
        let (mut total, mut friends, mut mine) =
            self.export_stats_from_table(&wcdb, my_wxid.as_deref());
        let mut fallback_error: Option<String> = None;
        let mut fallback_attempted = false;
        let cooled_down = self
            .sns_state
            .lock_or_panic()
            .last_timeline_fallback
            .is_none_or(|t| t.elapsed() >= TIMELINE_FALLBACK_COOLDOWN);
        if allow_fallback && (total <= 0 || friends <= 0) && cooled_down {
            fallback_attempted = true;
            match self.export_stats_from_timeline(&wcdb, my_wxid.as_deref()) {
                Ok((t, f, m)) => {
                    self.sns_state.lock_or_panic().last_timeline_fallback = Some(Instant::now());
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
                Err(e) => fallback_error = Some(e.into_message()),
            }
        }
        let normalized = export_stats_value(
            total.max(0),
            friends.max(0),
            if my_wxid.is_some() {
                mine.map(|m| m.max(0))
            } else {
                None
            },
        );
        let has_data = total > 0 || friends > 0;
        let cached_with_data = cached.filter(|(v, _)| v.total_posts > 0 || v.total_friends > 0);
        if let (false, Some((v, _))) = (has_data, cached_with_data) {
            return Ok(v);
        }
        if !has_data && fallback_attempted {
            if let Some(e) = fallback_error {
                return Err(AppError::native(e));
            }
        }
        self.sns_state.lock_or_panic().export_stats = Some((normalized, Instant::now()));
        Ok(normalized)
    }

    /// 每个用户名的动态数，按用户名排序。
    ///
    /// [`Freshness::Cached`] 时用没过期的保存结果；读数据库失败时用上次保存的结果。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误；读取失败且没有保存过结果时返回数据库层的错误。
    pub fn sns_user_post_counts(&self, freshness: Freshness) -> AppResult<BTreeMap<String, i64>> {
        if freshness == Freshness::Cached {
            if let Some((c, at)) = &self.sns_state.lock_or_panic().user_counts {
                if at.elapsed() <= USER_COUNTS_TTL {
                    return Ok(c.clone());
                }
            }
        }
        let wcdb = self.open_wcdb()?;
        let res = wcdb.sns_post_counts().map_err(err_native).map(|v| {
            v.as_object()
                .into_iter()
                .flatten()
                .map(|(u, n)| (u.clone(), n.as_i64().unwrap_or(0)))
                .collect::<BTreeMap<String, i64>>()
        });
        match res {
            Ok(counts) => {
                self.sns_state.lock_or_panic().user_counts = Some((counts.clone(), Instant::now()));
                Ok(counts)
            }
            Err(e) => match &self.sns_state.lock_or_panic().user_counts {
                Some((c, _)) => Ok(c.clone()),
                None => Err(e),
            },
        }
    }

    /// 一个人有多少条动态（`sns post-counts --user`），保存的计数没过期时直接用。
    ///
    /// # Errors
    /// 用户名为空时返回用法错误；数不出动态时返回数据库层的错误。
    pub fn sns_user_post_stats(&self, username: &str) -> AppResult<UserPosts> {
        let username = username.trim();
        if username.is_empty() {
            return Err(AppError::usage("username must not be empty"));
        }
        let counts = self.sns_user_post_counts(Freshness::Cached)?;
        Ok(UserPosts {
            username: username.to_string(),
            total_posts: counts.get(username).copied().unwrap_or(0).max(0),
        })
    }

    // ── 缓存与调试 ──

    /// 丢弃内存中的图片缓存、统计缓存和保留的 HTTP 客户端。
    pub fn sns_clear_memory_cache(&self) {
        *self.sns_state.lock_or_panic() = SnsState::default();
    }

    /// 像微信那样向服务器请求朋友圈资源的前几个字节，
    /// 报告状态码和与解密有关的响应头（`x-enc`、`x-time`、长度、类型）。
    ///
    /// # Errors
    ///
    /// `url` 为空、建不了 HTTP 客户端或请求失败时返回运行时错误。
    pub async fn sns_debug_resource(&self, url: &str) -> AppResult<SnsResourceProbe> {
        if url.trim().is_empty() {
            return Err(AppError::runtime("url must not be empty"));
        }
        let client = self.sns_http_client(false)?;
        let req = client
            .get(url.trim())
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/107.0.0.0 Safari/537.36 MicroMessenger/7.0.20.1781(0x6700143B) WindowsWechat(0x63090719) XWEB/8351")
            .header("Accept", "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .header("Range", "bytes=0-10");
        let res = req
            .send()
            .await
            .map_err(|e| AppError::runtime(crate::http_client::failure(e)))?;
        let header = |k: &str| {
            res.headers()
                .get(k)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        Ok(SnsResourceProbe {
            status: res.status().as_u16(),
            headers: SnsResourceHeaders {
                x_enc: header("x-enc"),
                x_time: header("x-time"),
                content_length: header("content-length"),
                content_type: header("content-type"),
            },
        })
    }

    // ── 媒体的下载和解密 ──

    /// 访问微信媒体服务器的客户端，校验证书；`unchecked` 时为访问证书上写着别的主机名的表情主机的客户端，
    /// 见 `http_client::sticker_certificate_unchecked`（#814）。每个只建一次并保留下来。
    fn sns_http_client(&self, unchecked: bool) -> AppResult<reqwest::Client> {
        let kept = |state: &mut SnsState| {
            if unchecked {
                state.unchecked_client.clone()
            } else {
                state.client.clone()
            }
        };
        if let Some(client) = kept(&mut self.sns_state.lock_or_panic()) {
            return Ok(client);
        }
        let client = crate::http_client::builder()
            .timeout(Duration::from_secs(15))
            .danger_accept_invalid_certs(unchecked)
            .build()
            .map_err(|e| AppError::runtime(format!("failed to create HTTP client: {e}")))?;
        let mut state = self.sns_state.lock_or_panic();
        let slot = if unchecked {
            &mut state.unchecked_client
        } else {
            &mut state.client
        };
        Ok(slot.get_or_insert(client).clone())
    }

    /// 下载朋友圈的图片或视频，用 ISAAC-64 密钥解密；已缓存的直接读缓存。
    ///
    /// # Errors
    ///
    /// `url` 为空时返回用法错误；请求失败、服务器回答的状态不是 200 或 206（404、410 时 `failureKind` 为
    /// `not_found`）、视频写不进缓存或解密后不是图片时返回运行时错误。
    pub async fn sns_fetch_media(&self, url: &str, key: Option<&str>) -> AppResult<SnsMediaFetch> {
        if url.is_empty() {
            return Err(AppError::usage("url must not be empty"));
        }
        let is_video = sns::is_video_url(url);
        let cache_path = self.sns_cache_file(url);

        if cache_path.exists() {
            // 视频的封面也放在视频服务器上：看缓存的内容才知道是哪一种
            if is_video && !cached_is_image(&cache_path) {
                return Ok(SnsMediaFetch {
                    data: None,
                    content_type: "video/mp4".into(),
                    cache_path: Some(cache_path),
                });
            }
            match std::fs::read(&cache_path) {
                Ok(data) if sns::detect_image_mime(&data, "").starts_with("image/") => {
                    let ct = sns::detect_image_mime(&data, "image/jpeg");
                    return Ok(SnsMediaFetch {
                        data: Some(data),
                        content_type: ct,
                        cache_path: Some(cache_path),
                    });
                }
                Ok(_) => {
                    let _ = std::fs::remove_file(&cache_path);
                }
                Err(_) => {}
            }
        }

        let client = self.sns_http_client(false)?;
        let mut req = client
            .get(url)
            .header("User-Agent", "MicroMessenger Client")
            .header("Accept", "*/*")
            .header("Connection", "keep-alive");
        if !is_video {
            req = req.header("Accept-Language", "zh-CN,zh;q=0.9");
        }
        let resp = req.send().await.map_err(|e| {
            AppError::runtime(if e.is_timeout() {
                "request timed out".to_string()
            } else {
                // 地址里带着媒体的 token：报错里不写出地址
                crate::http_client::failure(e)
            })
        })?;
        let status = resp.status();
        if status.as_u16() != 200 && status.as_u16() != 206 {
            let err = AppError::runtime(format!("HTTP {}", status.as_u16()));
            // 已从微信的服务器上删除，如已删除动态的媒体：对页面来说是 404，而不是失败
            return Err(if matches!(status.as_u16(), 404 | 410) {
                err.with_details(json!({ "failureKind": "not_found" }))
            } else {
                err
            });
        }
        let header_ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg")
            .to_string();
        let mut raw = resp
            .bytes()
            .await
            .map_err(|e| AppError::runtime(crate::http_client::failure(e)))?
            .to_vec();
        let key = truthy_key(key);

        // 视频的封面从视频服务器以普通图片的形式下载：把它当作视频开头的字节解密会弄坏它，所以按图片处理。
        if is_video && !sns::detect_image_mime(&raw, "").starts_with("image/") {
            if let Some(k) = key {
                isaac64::xor_in_place(&mut raw, k, Some(VIDEO_HEADER_BYTES));
            }
            write_atomically(&cache_path, &raw).map_err(|e| {
                AppError::runtime(format!("failed to write {}: {e}", cache_path.display()))
            })?;
            // 用视频的密钥加密的视频封面，解密后是图片
            let content_type = match sns::detect_image_mime(&raw, "") {
                image if image.starts_with("image/") => image,
                _ => "video/mp4".into(),
            };
            return Ok(SnsMediaFetch {
                data: Some(raw),
                content_type,
                cache_path: Some(cache_path),
            });
        }

        let mut decoded = raw.clone();
        let raw_is_image = sns::detect_image_mime(&raw, "").starts_with("image/");
        // 只要有密钥就解密，不看 `x-enc` 响应头（与桌面版相同）
        if let Some(key) = key {
            if key.chars().all(|c| c.is_ascii_digit()) {
                let mut decrypted = raw.clone();
                isaac64::xor_in_place(&mut decrypted, key, None);
                if sns::detect_image_mime(&decrypted, "").starts_with("image/") || !raw_is_image {
                    decoded = decrypted;
                }
            }
        }
        if !sns::detect_image_mime(&decoded, "").starts_with("image/") {
            return Err(AppError::runtime(
                "image decryption failed: unrecognized image format",
            ));
        }
        let _ = write_atomically(&cache_path, &decoded);
        let ct = sns::detect_image_mime(&decoded, &header_ct);
        Ok(SnsMediaFetch {
            data: Some(decoded),
            content_type: ct,
            cache_path: Some(cache_path),
        })
    }

    /// 给页面用的朋友圈资源：图片给出 `data:` 地址（在内存中保留 15 分钟），视频给出本地路径。
    ///
    /// # Errors
    ///
    /// 同 [`Self::sns_fetch_media`]；取到的不是图片（密钥不对或缓存损坏）时返回运行时错误。
    pub async fn sns_proxy_image(&self, url: &str, key: Option<&str>) -> AppResult<SnsProxyResult> {
        if url.is_empty() {
            return Err(AppError::usage("url must not be empty"));
        }
        let cache_key = format!("{url}|{}", key.unwrap_or(""));
        {
            let mut st = self.sns_state.lock_or_panic();
            if let Some((data_url, at)) = st.image_cache.get(&cache_key).cloned() {
                let valid = at.elapsed() <= IMAGE_CACHE_TTL
                    && data_url.split(',').nth(1).is_some_and(|b64| {
                        let bytes = sns::lenient_base64(b64);
                        sns::detect_image_mime(&bytes, "").starts_with("image/")
                    });
                if valid {
                    st.image_order.retain(|k| k != &cache_key);
                    st.image_order.push_back(cache_key.clone());
                    st.image_cache
                        .insert(cache_key, (data_url.clone(), Instant::now()));
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
        let data = fetched
            .data
            .ok_or_else(|| AppError::runtime("empty image data"))?;
        if !sns::detect_image_mime(&data, "").starts_with("image/") {
            return Err(AppError::runtime(
                "invalid image data: wrong key or corrupt cache",
            ));
        }
        use base64::Engine;
        let data_url = format!(
            "data:{};base64,{}",
            fetched.content_type,
            base64::engine::general_purpose::STANDARD.encode(&data)
        );
        let mut st = self.sns_state.lock_or_panic();
        st.image_order.retain(|k| k != &cache_key);
        st.image_order.push_back(cache_key.clone());
        st.image_cache
            .insert(cache_key, (data_url.clone(), Instant::now()));
        let now = Instant::now();
        let expired: Vec<String> = st
            .image_cache
            .iter()
            .filter(|(_, (_, at))| now.duration_since(*at) > IMAGE_CACHE_TTL)
            .map(|(k, _)| k.clone())
            .collect();
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

    // ── 评论表情 ──

    /// 下载 `target_url` 的文件，以 `<cache_key>.<its kind>` 为名保存到 `dir`。
    ///
    /// 校验证书，只有证书上写着别的主机名的表情主机除外（#814）；客户端自己跟随重定向。
    async fn download_raw(&self, target_url: &str, cache_key: &str, dir: &Path) -> Option<PathBuf> {
        let url = target_url.replace("&amp;", "&");
        let unchecked = crate::http_client::sticker_certificate_unchecked(&url);
        let resp = self
            .sns_http_client(unchecked)
            .ok()?
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 MicroMessenger/7.0.20.1781(0x67001431)")
            .header("Accept", "*/*")
            .send()
            .await
            .ok()?;
        if resp.status() != reqwest::StatusCode::OK {
            return None;
        }
        let bytes = resp.bytes().await.ok()?;
        if bytes.is_empty() {
            return None;
        }
        let ext = if sns::is_valid_image_buffer(&bytes) {
            sns::image_ext_from_buffer(&bytes)
        } else {
            ".bin"
        };
        let path = dir.join(format!("{cache_key}{ext}"));
        write_atomically(&path, &bytes).ok()?;
        Some(path)
    }

    /// 下载评论中的表情，需要时用 AES 密钥解密；已下载过的直接取缓存。
    ///
    /// # Errors
    ///
    /// `url` 和 `encrypt_url` 都为空时返回用法错误；下载或保存失败时返回运行时错误。
    pub async fn sns_download_emoji(
        &self,
        url: &str,
        encrypt_url: Option<&str>,
        aes_key: Option<&str>,
    ) -> AppResult<SnsEmojiSaved> {
        let encrypt_url = encrypt_url.filter(|u| !u.is_empty());
        let aes_key = aes_key.filter(|k| !k.is_empty());
        let Some(source) = (if url.is_empty() {
            encrypt_url
        } else {
            Some(url)
        }) else {
            return Err(AppError::usage("url must not be empty"));
        };
        let cache_key = md5_hex(source.as_bytes());
        let dir = self.emoji_cache_dir();
        for ext in [".gif", ".png", ".webp", ".jpg", ".jpeg"] {
            let p = dir.join(format!("{cache_key}{ext}"));
            if p.exists() {
                return Ok(SnsEmojiSaved::at(p));
            }
        }
        let save_decrypted = |buf: &[u8]| -> Option<PathBuf> {
            let ext = if sns::is_valid_image_buffer(buf) {
                sns::image_ext_from_buffer(buf)
            } else {
                ".gif"
            };
            let p = dir.join(format!("{cache_key}{ext}"));
            write_atomically(&p, buf).ok().map(|()| p)
        };

        if let (Some(enc_url), Some(key)) = (encrypt_url, aes_key) {
            if let Some(enc_path) = self
                .download_raw(enc_url, &format!("{cache_key}_enc"), &dir)
                .await
            {
                if let Ok(enc) = std::fs::read(&enc_path) {
                    if sns::is_valid_image_buffer(&enc) {
                        let p =
                            dir.join(format!("{cache_key}{}", sns::image_ext_from_buffer(&enc)));
                        if write_atomically(&p, &enc).is_ok() {
                            let _ = std::fs::remove_file(&enc_path);
                            return Ok(SnsEmojiSaved::at(p));
                        }
                    }
                    if let Some(dec) = sns::decrypt_emoji_aes(&enc, key) {
                        let _ = std::fs::remove_file(&enc_path);
                        if let Some(p) = save_decrypted(&dec) {
                            return Ok(SnsEmojiSaved::at(p));
                        }
                        return Err(AppError::runtime("failed to save the decrypted emoji"));
                    }
                }
                let _ = std::fs::remove_file(&enc_path);
            }
        }
        if !url.is_empty() {
            if let Some(path) = self.download_raw(url, &cache_key, &dir).await {
                if let Ok(buf) = std::fs::read(&path) {
                    if sns::is_valid_image_buffer(&buf) {
                        return Ok(SnsEmojiSaved::at(path));
                    }
                    if let Some(key) = aes_key {
                        if let Some(dec) = sns::decrypt_emoji_aes(&buf, key) {
                            let _ = std::fs::remove_file(&path);
                            if let Some(p) = save_decrypted(&dec) {
                                return Ok(SnsEmojiSaved::at(p));
                            }
                            return Err(AppError::runtime("failed to save the decrypted emoji"));
                        }
                    }
                }
                let _ = std::fs::remove_file(&path);
            }
        }
        Err(AppError::runtime("failed to download the emoji"))
    }

    // ── 导出 ──

    /// 把时间线写成 json、html 或 arkmejson，可选同时下载媒体。
    ///
    /// # Errors
    /// 格式不是这三种之一时返回错误（这时什么都不创建）；朋友圈数据库读不出时返回错误（已读到的动态不写出）；
    /// 文件夹或文件写不进去时返回错误。
    pub async fn sns_export_timeline(&self, opts: &SnsExportOptions) -> AppResult<SnsExported> {
        use futures::StreamExt;
        let want_media = opts.export_media;
        let format = SnsExportFormat::parse(&opts.format)?;

        std::fs::create_dir_all(&opts.output_dir).map_err(|e| {
            AppError::runtime(format!(
                "failed to create {}: {e}",
                opts.output_dir.display()
            ))
        })?;
        let usernames = &opts.usernames;
        let keyword = opts.keyword.as_deref().filter(|k| !k.is_empty());

        // 1. 逐页读出所有动态；各页按时间线的全序排列（先按时间、再按行 ID，从新到旧），
        // 所以按偏移能读到每条动态，包括同一秒发的和没有时间的。导出中途新发的动态会让后面的页都后移一条，
        // 结果是重复一条而不是漏掉一条：按行 ID（`tid`）去掉重复的。
        let wcdb = self.open_wcdb()?;
        // 导出先读动态，再从联系人中读作者的名字和头像
        wcdb.prefetch(&[Databases::Moments, Databases::Contacts])
            .map_err(err_native)?;
        let mut posts: Vec<SnsPost> = Vec::new();
        let mut loaded = std::collections::HashSet::new();
        let page = 50;
        let mut offset = 0i32;
        self.emit_progress("sns", "loading Moments…", 0, 0);
        loop {
            let q = SnsTimelineQuery {
                limit: page,
                offset,
                usernames: usernames.clone(),
                keyword: keyword.map(str::to_string),
                start: opts.start,
                end: opts.end,
            };
            // 读不出的一页让导出以它的错误结束：已读到的几页不是完整的时间线
            let batch = self.sns_timeline_with(&wcdb, &q)?;
            if batch.is_empty() {
                break;
            }
            let n = crate::num::saturating_i32(batch.len());
            posts.extend(batch.into_iter().filter(|p| loaded.insert(p.tid.clone())));
            self.emit_progress("sns", "loading Moments…", posts.len(), 0);
            match offset.checked_add(n) {
                Some(next) if n == page => offset = next,
                _ => break,
            }
        }
        if posts.is_empty() {
            return Ok(SnsExported {
                file_path: PathBuf::new(),
                post_count: 0,
                media_count: 0,
            });
        }

        // 2. 媒体：每条动态的每个媒体保存下来的文件
        let media_dir = opts.output_dir.join("media");
        let mut media_count = 0usize;
        let mut saved: Vec<Vec<SavedMedia>> = posts
            .iter()
            .map(|p| vec![SavedMedia::default(); p.media.len()])
            .collect();
        if want_media {
            std::fs::create_dir_all(&media_dir).map_err(|e| {
                AppError::runtime(format!("failed to create {}: {e}", media_dir.display()))
            })?;
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
                let post_id = post.id.clone();
                for (mi, m) in post.media.iter().enumerate() {
                    let url = m.url.clone();
                    let key = m.key.clone();
                    if !url.is_empty() {
                        tasks.push(Task {
                            kind: if sns::is_video_url(&url) {
                                "video"
                            } else {
                                "image"
                            },
                            url: url.clone(),
                            key: key.clone(),
                            post_idx: pi,
                            media_idx: mi,
                            post_id: post_id.clone(),
                        });
                    }
                    if let Some(lp) = m.live_photo.as_ref() {
                        if !lp.url.is_empty() {
                            tasks.push(Task {
                                kind: "livephoto",
                                url: lp.url.clone(),
                                key: lp.key.clone().filter(|k| !k.is_empty()).or(key.clone()),
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
                    let is_video = task.kind == "video"
                        || task.kind == "livephoto"
                        || sns::is_video_url(&task.url);
                    let suffix = if task.kind == "livephoto" {
                        "_live"
                    } else {
                        ""
                    };
                    let stem = format!("{}_{}{}", task.post_id, task.media_idx, suffix);
                    if let Some(name) = saved_media(media_dir_ref, &stem) {
                        return (task, Some(format!("media/{name}")));
                    }
                    let saved = match self.sns_fetch_media(&task.url, task.key.as_deref()).await {
                        Ok(fetched) => save_fetched(media_dir_ref, &stem, fetched, is_video),
                        Err(_) => None,
                    };
                    (task, saved.map(|name| format!("media/{name}")))
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
                if let Some(s) = saved
                    .get_mut(task.post_idx)
                    .and_then(|s| s.get_mut(task.media_idx))
                {
                    if task.kind == "livephoto" {
                        s.live = Some(local);
                    } else {
                        s.file = Some(local);
                    }
                }
            }
        }

        // 2.5 头像（只用于 html）
        let mut avatar_map: HashMap<String, String> = HashMap::new();
        if format == SnsExportFormat::Html {
            std::fs::create_dir_all(&media_dir).map_err(|e| {
                AppError::runtime(format!("failed to create {}: {e}", media_dir.display()))
            })?;
            let mut unique: Vec<(String, String)> = Vec::new();
            for p in &posts {
                if let (u, Some(a)) = (
                    p.username.as_str(),
                    p.avatar_url.as_deref().filter(|a| !a.is_empty()),
                ) {
                    if !unique.iter().any(|(x, _)| x == u) {
                        unique.push((u.to_string(), a.to_string()));
                    }
                }
            }
            let total = unique.len();
            let media_dir_ref = &media_dir;
            let fetched: Vec<(String, Option<String>)> = futures::stream::iter(unique)
                .map(|(user, avatar)| async move {
                    let stem = format!("avatar_{}", &md5_hex(user.as_bytes())[..8]);
                    if let Some(name) = saved_media(media_dir_ref, &stem) {
                        return (user, Some(format!("media/{name}")));
                    }
                    let saved = match self.sns_fetch_media(&avatar, None).await {
                        Ok(fetched) => save_fetched(media_dir_ref, &stem, fetched, false),
                        Err(_) => None,
                    };
                    (user, saved.map(|name| format!("media/{name}")))
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

        // 3. 输出文件：文件名用本地时间，与其他导出的页面一样；数据中的导出时间用 UTC
        let now_utc = chrono::Utc::now();
        let stamp = now_utc
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%dT%H-%M-%S")
            .to_string();
        let export_time = now_utc.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        let filters = json!({ "usernames": usernames, "keyword": keyword.unwrap_or("") });
        let out_path = opts
            .output_dir
            .join(format!("朋友圈导出_{stamp}.{}", format.extension()));
        match format {
            SnsExportFormat::Json => {
                let posts_json: Vec<Value> = posts
                    .iter()
                    .zip(&saved)
                    .map(|(p, s)| export_post_json(p, s, false))
                    .collect();
                let data = json!({ "exportTime": export_time, "totalPosts": posts.len(), "filters": filters, "posts": posts_json });
                let text = serde_json::to_string_pretty(&data)
                    .map_err(|e| AppError::runtime(format!("{e:#}")))?;
                std::fs::write(&out_path, text).map_err(|e| {
                    AppError::runtime(format!("failed to write {}: {e}", out_path.display()))
                })?;
            }
            SnsExportFormat::ArkmeJson => {
                let mut identity_cache: HashMap<String, Option<Value>> = HashMap::new();
                let uniq: Vec<String> = {
                    let mut seen = std::collections::BTreeSet::new();
                    posts
                        .iter()
                        .map(|p| p.username.as_str())
                        .filter(|u| seen.insert(u.to_string()))
                        .map(str::to_string)
                        .collect()
                };
                let book = self.contact_book(&wcdb, &uniq);
                let mut built = Vec::with_capacity(posts.len());
                for (i, (post, saved)) in posts.iter().zip(&saved).enumerate() {
                    let username = post.username.as_str();
                    let author = self.contact_identity(&wcdb, &book, &mut identity_cache, username).unwrap_or_else(|| {
                        json!({ "username": username, "wxid": username, "displayName": Some(post.nickname.as_str()).filter(|n| !n.is_empty()).unwrap_or(username) })
                    });
                    let (likes_detail, comments_detail) =
                        self.arkme_interaction_details(&wcdb, &book, &mut identity_cache, post);
                    let base = export_post_json(post, saved, true);
                    let b = base.as_object().cloned().unwrap_or_default();
                    let mut o = Map::new();
                    for k in ["id", "username", "nickname"] {
                        o.insert(k.into(), b.get(k).cloned().unwrap_or(Value::Null));
                    }
                    o.insert("author".into(), author);
                    for k in [
                        "createTime",
                        "createTimeStr",
                        "contentDesc",
                        "type",
                        "media",
                        "likes",
                        "comments",
                        "location",
                    ] {
                        if let Some(v) = b.get(k) {
                            o.insert(k.into(), v.clone());
                        }
                    }
                    o.insert("likesDetail".into(), Value::Array(likes_detail));
                    o.insert("commentsDetail".into(), Value::Array(comments_detail));
                    built.push(Value::Object(o));
                    if (i + 1) % 20 == 0 || i + 1 == posts.len() {
                        self.emit_progress("sns-arkme", "building ArkmeJSON", i + 1, posts.len());
                    }
                }
                let owner_wxid = Some(self.my_wxid_cleaned()).filter(|w| !w.is_empty());
                let record_owner = match owner_wxid.as_deref() {
                    Some(w) => self
                        .contact_identity(&wcdb, &book, &mut identity_cache, w)
                        .unwrap_or_else(|| json!({ "username": w, "wxid": w, "displayName": w })),
                    None => json!({ "username": "", "wxid": "", "displayName": "" }),
                };
                let data = json!({
                    "exportTime": export_time,
                    "format": "arkmejson",
                    "schemaVersion": "1.0.0",
                    "recordOwner": record_owner,
                    "mediaSelection": { "images": want_media, "livePhotos": want_media, "videos": want_media },
                    "totalPosts": posts.len(),
                    "filters": filters,
                    "posts": built
                });
                let text = serde_json::to_string_pretty(&data)
                    .map_err(|e| AppError::runtime(format!("{e:#}")))?;
                std::fs::write(&out_path, text).map_err(|e| {
                    AppError::runtime(format!("failed to write {}: {e}", out_path.display()))
                })?;
            }
            SnsExportFormat::Html => {
                let html = sns::generate_html(&posts, &saved, keyword, &avatar_map)
                    .map_err(|e| AppError::runtime(format!("{e:#}")))?;
                std::fs::write(&out_path, html).map_err(|e| {
                    AppError::runtime(format!("failed to write {}: {e}", out_path.display()))
                })?;
            }
        }
        self.emit_progress("sns", "export finished", posts.len(), posts.len());
        Ok(SnsExported {
            file_path: out_path,
            post_count: posts.len(),
            media_count,
        })
    }

    /// Arkme 导出用的一条动态的点赞和评论，每一项带着留下它的人的名字和头像。
    fn arkme_interaction_details(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        book: &HashMap<String, CachedContact>,
        cache: &mut HashMap<String, Option<Value>>,
        post: &SnsPost,
    ) -> (Vec<Value>, Vec<Value>) {
        let raw_xml = post.raw_xml.as_str();
        let xml_likes = sns::parse_like_users_from_xml(raw_xml);
        let legacy_likes: Vec<sns::LikeUser> = post
            .likes
            .iter()
            .map(|n| sns::LikeUser {
                username: None,
                nickname: Some(n.clone()),
            })
            .collect();
        let (like_candidates, like_source) = if xml_likes.is_empty() {
            (legacy_likes, "legacy")
        } else {
            (xml_likes, "xml")
        };
        let mut likes_detail = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for like in like_candidates {
            let identity = like
                .username
                .as_deref()
                .and_then(|u| self.contact_identity(wcdb, book, cache, u));
            let idv = |k: &str| {
                identity
                    .as_ref()
                    .and_then(|i| i.get(k))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            };
            let nickname = like
                .nickname
                .clone()
                .filter(|n| !n.is_empty())
                .or_else(|| idv("displayName"))
                .or_else(|| like.username.clone())
                .unwrap_or_default();
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
            o.insert(
                "displayName".into(),
                json!(idv("displayName")
                    .filter(|d| !d.is_empty())
                    .or_else(|| Some(nickname.clone()).filter(|n| !n.is_empty()))
                    .or(username)
                    .unwrap_or_default()),
            );
            o.insert("source".into(), json!(like_source));
            likes_detail.push(Value::Object(o));
        }

        let xml_comments = sns::parse_comments_from_xml(raw_xml);
        let post_comments = &post.comments;
        let by_id: HashMap<&str, &SnsComment> = post_comments
            .iter()
            .filter(|c| !c.id.is_empty())
            .map(|c| (c.id.as_str(), c))
            .collect();

        let mut base: Vec<SnsComment> = Vec::new();
        if !xml_comments.is_empty() {
            for c in &xml_comments {
                let fb = by_id.get(c.id.as_str()).copied();
                let pick = |own: &str, of: fn(&SnsComment) -> &str| -> String {
                    if !own.is_empty() {
                        own.to_string()
                    } else {
                        fb.map(|f| of(f).to_string()).unwrap_or_default()
                    }
                };
                base.push(SnsComment {
                    id: pick(&c.id, |f| &f.id),
                    nickname: pick(&c.nickname, |f| &f.nickname),
                    username: c.username.clone(),
                    content: pick(&c.content, |f| &f.content),
                    ref_comment_id: pick(&c.ref_comment_id, |f| &f.ref_comment_id),
                    ref_username: c.ref_username.clone(),
                    ref_nickname: c
                        .ref_nickname
                        .clone()
                        .filter(|n| !n.is_empty())
                        .or_else(|| fb.and_then(|f| f.ref_nickname.clone())),
                    emojis: if c.emojis.is_empty() {
                        fb.map(|f| f.emojis.clone()).unwrap_or_default()
                    } else {
                        c.emojis.clone()
                    },
                });
            }
            let mapped: std::collections::HashSet<String> = base
                .iter()
                .filter(|c| !c.id.is_empty())
                .map(|c| c.id.clone())
                .collect();
            for c in post_comments {
                if !c.id.is_empty() && mapped.contains(&c.id) {
                    continue;
                }
                base.push(legacy_comment(c));
            }
        } else {
            base.extend(post_comments.iter().map(legacy_comment));
        }

        let comment_source = if xml_comments.is_empty() {
            "legacy"
        } else {
            "xml"
        };
        let mut comments_detail = Vec::new();
        for c in &base {
            let username = c.username.clone().filter(|u| !u.is_empty());
            let ref_username = c.ref_username.clone().filter(|u| !u.is_empty());
            let actor = username
                .as_deref()
                .and_then(|u| self.contact_identity(wcdb, book, cache, u));
            let ref_actor = ref_username
                .as_deref()
                .and_then(|u| self.contact_identity(wcdb, book, cache, u));
            let av = |a: &Option<Value>, k: &str| {
                a.as_ref()
                    .and_then(|i| i.get(k))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            };
            let nickname = Some(c.nickname.clone())
                .filter(|n| !n.is_empty())
                .or_else(|| av(&actor, "displayName"))
                .or_else(|| username.clone())
                .unwrap_or_default();
            let eff_username = av(&actor, "username").or_else(|| username.clone());
            let eff_ref_username = av(&ref_actor, "username").or_else(|| ref_username.clone());
            let mut o = Map::new();
            o.insert("id".into(), json!(c.id));
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
            o.insert(
                "displayName".into(),
                json!(av(&actor, "displayName")
                    .filter(|d| !d.is_empty())
                    .or_else(|| Some(nickname.clone()).filter(|n| !n.is_empty()))
                    .or(eff_username)
                    .unwrap_or_default()),
            );
            o.insert("content".into(), json!(c.content));
            o.insert("refCommentId".into(), json!(c.ref_comment_id));
            let ref_nickname = c
                .ref_nickname
                .clone()
                .filter(|n| !n.is_empty())
                .or_else(|| av(&ref_actor, "displayName"));
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
            if !c.emojis.is_empty() {
                o.insert("emojis".into(), json!(c.emojis));
            }
            o.insert("source".into(), json!(comment_source));
            comments_detail.push(Value::Object(o));
        }
        (likes_detail, comments_detail)
    }

    /// 取到媒体后的时间线（桌面版的 `enrichSnsTimelineMedia`，用于 `sns timeline --with-media`）。
    ///
    /// `url` 和 `thumb` 变成 data URL 形式的文件，取不到时变成代理地址；服务器的地址移到 `rawUrl` 和 `rawThumb`。
    pub async fn sns_enrich_timeline_media(&self, mut posts: Vec<SnsPost>) -> Vec<SnsPost> {
        for media in posts.iter_mut().flat_map(|p| p.media.iter_mut()) {
            let raw_url = std::mem::take(&mut media.url);
            let raw_thumb = std::mem::take(&mut media.thumb);
            let key = media_key(media.key.as_deref());
            let (url_resolved, url_proxy) = self.resolve_media_url(&raw_url, key.as_deref()).await;
            let (thumb_resolved, thumb_proxy) =
                self.resolve_media_url(&raw_thumb, key.as_deref()).await;
            media.url = url_resolved.clone().unwrap_or_else(|| raw_url.clone());
            media.thumb = thumb_resolved.clone().unwrap_or_else(|| raw_thumb.clone());
            media.raw_url = Some(raw_url);
            media.raw_thumb = Some(raw_thumb);
            media.resolved_url = url_resolved;
            media.resolved_thumb_url = thumb_resolved;
            media.proxy_url = url_proxy;
            media.proxy_thumb_url = thumb_proxy;
            if let Some(live) = media.live_photo.as_mut() {
                let raw_l_url = std::mem::take(&mut live.url);
                let raw_l_thumb = std::mem::take(&mut live.thumb);
                let live_key = media_key(live.key.as_deref()).or_else(|| key.clone());
                let (lu, lu_proxy) = self
                    .resolve_media_url(&raw_l_url, live_key.as_deref())
                    .await;
                let (lt, lt_proxy) = self
                    .resolve_media_url(&raw_l_thumb, live_key.as_deref())
                    .await;
                live.url = lu.clone().unwrap_or_else(|| raw_l_url.clone());
                live.thumb = lt.clone().unwrap_or_else(|| raw_l_thumb.clone());
                live.raw_url = Some(raw_l_url);
                live.raw_thumb = Some(raw_l_thumb);
                live.resolved_url = lu;
                live.resolved_thumb_url = lt;
                live.proxy_url = lu_proxy;
                live.proxy_thumb_url = lt_proxy;
            }
        }
        posts
    }

    /// `raw_url` 显示成什么，以及它的代理地址：能取到文件时为 data URL，否则两者都是代理地址。
    ///
    /// 代理地址是 `weflow serve` 上 `sns media` 的路由（#663）。
    async fn resolve_media_url(
        &self,
        raw_url: &str,
        key: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let target = raw_url.trim();
        if target.is_empty() {
            return (None, None);
        }
        let mut proxy = format!("{SNS_MEDIA_ROUTE}?url={}", url_encode(target));
        if let Some(k) = key {
            proxy.push_str(&format!("&key={}", url_encode(k)));
        }
        if let Ok(SnsProxyResult::DataUrl(d)) = self.sns_proxy_image(raw_url, key).await {
            return (Some(d), Some(proxy));
        }
        (Some(proxy.clone()), Some(proxy))
    }
}

/// `weflow serve` 在自己的进程内回答 `sns media` 的路由。
const SNS_MEDIA_ROUTE: &str = "/api/sns/media";

/// 媒体密钥的文本：去掉首尾空白，为空时为 `None`。
fn media_key(key: Option<&str>) -> Option<String> {
    let t = key?.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Arkme JSON 导出中列出的动态自带的一条评论：去掉用户名，因为它的 XML 里没有。
fn legacy_comment(c: &SnsComment) -> SnsComment {
    SnsComment {
        username: None,
        ref_username: None,
        ..c.clone()
    }
}

/// 写进 JSON 和 Arkme JSON 导出的一条动态，带上它的媒体保存下来的文件 `saved`。
fn export_post_json(p: &SnsPost, saved: &[SavedMedia], arkme: bool) -> Value {
    let local = |path: &Option<String>| path.clone().filter(|l| !l.is_empty());
    let mut o = Map::new();
    o.insert("id".into(), json!(p.id));
    o.insert("username".into(), json!(p.username));
    o.insert("nickname".into(), json!(p.nickname));
    o.insert("createTime".into(), json!(p.create_time));
    o.insert(
        "createTimeStr".into(),
        json!(sns::zh_locale_from_ts(p.create_time)),
    );
    o.insert("contentDesc".into(), json!(p.content_desc));
    o.insert("type".into(), json!(p.kind));
    let media: Vec<Value> = p
        .media
        .iter()
        .enumerate()
        .map(|(mi, m)| {
            let files = saved.get(mi).cloned().unwrap_or_default();
            let mut mo = Map::new();
            mo.insert("url".into(), json!(m.url));
            mo.insert("thumb".into(), json!(m.thumb));
            if let Some(l) = local(&files.file) {
                mo.insert("localPath".into(), json!(l));
            }
            if arkme {
                if let Some(lp) = &m.live_photo {
                    let mut lo = Map::new();
                    lo.insert("url".into(), json!(lp.url));
                    lo.insert("thumb".into(), json!(lp.thumb));
                    if let Some(l) = local(&files.live) {
                        lo.insert("localPath".into(), json!(l));
                    }
                    mo.insert("livePhoto".into(), Value::Object(lo));
                }
            }
            Value::Object(mo)
        })
        .collect();
    o.insert("media".into(), Value::Array(media));
    o.insert("likes".into(), json!(p.likes));
    o.insert("comments".into(), json!(p.comments));
    if let Some(l) = &p.location {
        o.insert("location".into(), json!(l));
    }
    Value::Object(o)
}

impl ServiceHub {
    /// 下载 `url` 处的图片（`sns download-image`），是 `.dat` 数据时用图片密钥解密；有 `out` 时写进它，
    /// 否则以 base64 回答。
    ///
    /// # Errors
    /// 下载失败、解密失败或 `out` 写不进去时返回运行时错误；档案不存在时返回配置错误。
    pub async fn sns_download_image(
        &self,
        url: &str,
        out: Option<&Path>,
    ) -> AppResult<crate::contract::SnsImageDownload> {
        use crate::contract::{SnsImageDownload, SnsImageInline, SnsImageSaved};
        let client = crate::http_client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|err| AppError::runtime(format!("failed to create HTTP client: {err}")))?;
        let response = client.get(url).send().await.map_err(|err| {
            AppError::runtime(format!(
                "failed to download image: {}",
                crate::http_client::failure(err)
            ))
        })?;
        if !response.status().is_success() {
            return Err(AppError::runtime(format!(
                "image download failed with status {}",
                response.status()
            )));
        }
        let bytes = response.bytes().await.map_err(|err| {
            AppError::runtime(format!(
                "failed to read image response: {}",
                crate::http_client::failure(err)
            ))
        })?;
        let data = bytes.to_vec();

        let profile = self.profile()?;
        let xor_key = profile.image_xor_key.and_then(|k| u8::try_from(k).ok());
        let aes_key_bytes = profile
            .image_aes_key
            .as_deref()
            .and_then(crate::decrypt::parse_aes_key);

        let version = crate::decrypt::detect_dat_version(&data);
        let (final_data, ext) = match xor_key {
            Some(xor_key) if version > 0 => {
                let result = crate::decrypt::decrypt_dat(&data, xor_key, aes_key_bytes.as_ref())
                    .map_err(|err| AppError::runtime(format!("{err:#}")))?;
                (result.data, result.ext)
            }
            _ => {
                let ext = crate::decrypt::detect_image_extension(&data).to_string();
                (data, ext)
            }
        };

        if let Some(out_path) = out {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|err| {
                    AppError::runtime(format!("failed to create {}: {err}", parent.display()))
                })?;
            }
            std::fs::write(out_path, &final_data).map_err(|err| {
                AppError::runtime(format!("failed to write {}: {err}", out_path.display()))
            })?;
            Ok(SnsImageDownload::Saved(SnsImageSaved {
                url: url.to_string(),
                out: out_path.to_path_buf(),
                ext,
                size: final_data.len(),
            }))
        } else {
            use base64::Engine as _;
            let data = format!(
                "data:image/{};base64,{}",
                ext.trim_start_matches('.'),
                base64::engine::general_purpose::STANDARD.encode(&final_data)
            );
            Ok(SnsImageDownload::Inline(SnsImageInline {
                url: url.to_string(),
                ext,
                size: final_data.len(),
                data,
            }))
        }
    }

    /// 关注的公众号（`biz accounts`），每个带最后一次推送的时间。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn biz_accounts(&self) -> AppResult<Value> {
        let contacts = self.contacts()?;
        let sessions = self.sessions()?;
        let accounts = crate::biz::filter_official_contacts(&contacts, &sessions);
        Ok(json!({ "accounts": accounts }))
    }

    /// 公众号推送中的一页，从新到旧（`biz messages`）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn biz_messages(
        &self,
        username: &str,
        limit: i32,
        offset: i32,
    ) -> AppResult<crate::contract::BizMessages> {
        let messages = self.messages(username, limit, offset)?;
        Ok(crate::contract::BizMessages {
            username: username.to_string(),
            messages: crate::biz::parse_biz_messages(&messages),
        })
    }

    /// 从第 `offset` 条起的 `limit` 条消息中的微信支付通知，从新到旧。
    ///
    /// 消息来自微信支付账号的会话，以及微信存放支付卡片的服务通知。其他消息不列出，所以一页可能少于 `limit` 条：
    /// `hasMore` 说明这一页之后是否还有消息。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn biz_pay_records(
        &self,
        limit: i32,
        offset: i32,
    ) -> AppResult<crate::contract::PayRecords> {
        const PAY_ACCOUNT: &str = "gh_3dfda90e39d6";
        // 两个会话的消息合在一起在这里分页，所以每个会话都要读到这一页的末尾
        let upto = if limit > 0 {
            limit.saturating_add(offset.max(0))
        } else {
            limit
        };
        let own = self.messages(PAY_ACCOUNT, upto, 0)?;
        let cards = self.messages("notifymessage", upto, 0)?;
        let mut rows: Vec<(MsgRow, bool)> = own.into_iter().map(|m| (m, true)).collect();
        rows.extend(cards.into_iter().map(|m| {
            let from_pay =
                crate::message::xml::extract_xml_value(&m.content, "fromusername") == PAY_ACCOUNT;
            (m, from_pay)
        }));
        rows.sort_by_key(|(m, _)| {
            let at = m.pos();
            std::cmp::Reverse((at.sort_seq, at.create_time, at.local_id))
        });
        let page: Vec<(MsgRow, bool)> = rows
            .into_iter()
            .skip(usize::try_from(offset.max(0)).unwrap_or(usize::MAX))
            .take(
                usize::try_from(limit)
                    .ok()
                    .filter(|n| *n > 0)
                    .unwrap_or(usize::MAX),
            )
            .collect();
        let read = page.len();
        let notices: Vec<MsgRow> = page
            .into_iter()
            .filter_map(|(m, pay)| pay.then_some(m))
            .collect();
        let records = crate::biz::parse_pay_records(&notices);
        Ok(crate::contract::PayRecords {
            records,
            has_more: usize::try_from(limit).is_ok_and(|limit| limit > 0 && read >= limit),
        })
    }
}
