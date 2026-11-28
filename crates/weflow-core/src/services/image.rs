//! 消息中的图片：在账号的 `msg/attach/<md5(session)>/<yyyy-mm>/Img` 文件夹中找到消息图片的 `.dat` 文件，
//! 解密（V1、V2、旧版的 XOR），用内置的解码器解开 WXGF，把结果保存在 `<cache>/Images/<session>/<yyyy-mm>/`。
//!
//! 桌面版里供页面边解密边显示图片的事件，以及它在后台检查「有更清晰的版本」的功能，在命令行中没有对应，
//! 所以没有实现。
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use base64::Engine;
use serde_json::{json, Value};

use super::*;
use crate::fs_atomic::{temporary_sibling, write_atomically};
use crate::image::{self as img};
use crate::sync::MutexExt;

/// 图片服务在调用之间保留的状态。
#[derive(Default)]
pub struct ImageState {
    resolved: HashMap<String, String>,
    /// 查找 `.dat` 文件时列出的 `Img` 文件夹的内容（见 [`ServiceHub::dir_listing`]）。
    dir_listings: HashMap<PathBuf, Listed>,
    /// 解密图片缓存各文件夹中的文件名（见 [`ServiceHub::possibly_cached`]）。
    cache_dirs: HashMap<PathBuf, CacheDirNames>,
    /// 微信的图片媒体索引（见 [`ServiceHub::linked_image_index`]）。
    linked_images: Option<LinkedImages>,
    /// 账号目录，以及最近一次确认它在磁盘上的时刻（见 [`ServiceHub::account_dir_since`]）。
    account_dir: Option<(PathBuf, Instant)>,
    /// 缓存目录下的 `Images`（见 [`ServiceHub::image_cache_root`]）。
    cache_root: Option<PathBuf>,
}

/// 查找时可以直接相信的内容：在这一时刻或之后对照磁盘检查过的（文件夹的内容、微信的媒体索引、账号目录、
/// 图片缓存的文件夹）直接使用，不再检查。
///
/// 每次检查都是一次系统调用，在 Windows 上打开文件的系统调用要几十微秒：不这样做时，
/// 带图片的导出每张图片约要五十次（#1099）。一次查找（[`ServiceHub::image_decrypt`]）从开始的时刻起每项最多检查一次；
/// 一次导出从开始的时刻起，为所有图片每项只检查一次，所以文件夹按那时的状态看待。
#[derive(Clone, Copy, Debug)]
pub(super) struct Since(Instant);

impl Since {
    /// 从现在起检查过的才可信。
    pub(super) fn now() -> Self {
        Self(Instant::now())
    }

    /// 在 `checked` 时刻做的检查是否可信。
    fn covers(self, checked: Instant) -> bool {
        checked >= self.0
    }
}

/// 一个文件夹的内容列表（没有这个文件夹时为 `None`），以及最近一次确认它仍是最新的时刻。
struct Listed {
    listing: Option<Arc<DirListing>>,
    checked: Instant,
}

/// 整个读出的微信媒体索引（`hardlink.db`）中的图片行，以及读取时它的数据库文件的状态：
/// 数据库或它的 WAL 有变化时重新读取。
struct LinkedImages {
    account_dir: PathBuf,
    stamp: [Option<(Option<SystemTime>, u64)>; 2],
    by_md5: Arc<HashMap<String, Vec<PathBuf>>>,
    /// 最近一次确认 `stamp` 仍是最新的时刻。
    checked: Instant,
}

/// 文件的大小和修改时间。
#[derive(Clone, Copy, Debug)]
struct FileMeta {
    len: u64,
    modified: Option<SystemTime>,
}

impl FileMeta {
    fn of(m: &std::fs::Metadata) -> Self {
        Self {
            len: m.len(),
            modified: m.modified().ok(),
        }
    }
}

/// 内置解码器转换不了的 WXGF 图片的 `failureKind`。
const UNSUPPORTED_IMAGE: &str = "unsupported_image";
/// 解密后不是图片或不是完整图片的 `.dat` 文件的 `failureKind`。
const DECRYPT_FAILED: &str = "decrypt_failed";
/// 文件不完整的图片给出的提示（#1112），在也没有较差的版本可以代替显示时使用。
const INCOMPLETE_IMAGE: &str =
    "the image file is incomplete, WeChat may still be downloading it: retry later";
/// 既没有 md5 也没有 `.dat` 文件名、指不出图片的请求的 `failureKind`：作为命令的回答是用法错误（#1076）。
const NO_IDENTIFIER: &str = "invalid_arguments";

/// 最多保留的目录列表（及缓存文件夹的文件名）个数；超过时缓存从头开始。
const MAX_DIR_LISTINGS: usize = 1024;
/// 在文件夹最后一次变化后这么短的时间内列出的内容不再复用：在粗粒度时间戳的同一个刻度内新增的文件不会改变修改时间。
const LISTING_SETTLE: Duration = Duration::from_secs(2);

/// 一个文件夹的条目。
///
/// 图片要在它所在月份的文件夹里找，这个文件夹可能有几千个文件，而带媒体的导出要找每一张图片：
/// 每次都读一遍文件夹会让耗时随图片数平方增长（#49）。
pub(super) struct DirListing {
    modified: Option<SystemTime>,
    listed_at: SystemTime,
    /// 文件名，按系统列出的顺序。
    files: Vec<String>,
    /// `files` 中每个文件的大小和时间，只在列出文件夹时不必逐个调用就能得到时才有（Windows）。
    meta: Vec<Option<FileMeta>>,
    /// （去掉首尾空白的小写文件名，在 `files` 中的下标），排好序，用来找以某个前缀开头的文件名。
    keys: Vec<(String, usize)>,
    /// 子文件夹的名字，按列出的顺序。
    dirs: Vec<String>,
}

impl DirListing {
    fn read(dir: &Path) -> Option<Self> {
        let modified = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        let listed_at = SystemTime::now();
        let (mut files, mut meta, mut dirs) = (Vec::new(), Vec::new(), Vec::new());
        for entry in std::fs::read_dir(dir).ok()?.filter_map(Result::ok) {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().to_string();
            if kind.is_file() {
                files.push(name);
                // Windows 列出每个文件时就带着大小和时间；在其他系统上每个文件都要单独调用一次，
                // 比一次查找要问的那几个文件的代价还大
                meta.push(
                    cfg!(windows)
                        .then(|| entry.metadata().ok())
                        .flatten()
                        .map(|m| FileMeta::of(&m)),
                );
            } else if kind.is_dir() {
                dirs.push(name);
            }
        }
        let mut keys: Vec<(String, usize)> = files
            .iter()
            .enumerate()
            .map(|(i, name)| (name.trim().to_lowercase(), i))
            .collect();
        keys.sort_unstable();
        Some(Self {
            modified,
            listed_at,
            files,
            meta,
            keys,
            dirs,
        })
    }

    /// 列表中关于文件 `name` 的信息：不论大小写都没有这个名字时为 `None`；
    /// 否则，列表中有完全相同的名字且带大小和时间时给出它们（只有大小写不同的名字，
    /// 在区分大小写的文件系统上可能是另一个文件）。
    fn file(&self, name: &str) -> Option<Option<FileMeta>> {
        let key = name.trim().to_lowercase();
        let start = self
            .keys
            .partition_point(|(k, _)| k.as_str() < key.as_str());
        let mut same = self.keys[start..]
            .iter()
            .take_while(|(k, _)| *k == key)
            .peekable();
        same.peek()?;
        Some(
            same.find(|(_, i)| self.files[*i] == name)
                .and_then(|(_, i)| self.meta[*i]),
        )
    }

    /// `dir` 是否仍是这些条目：列出之后没有修改过，而且列出的时刻离它最后一次变化足够远。
    fn current(&self, dir: &Path) -> bool {
        let Some(modified) = self.modified else {
            return false;
        };
        let now = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        now == Some(modified)
            && self
                .listed_at
                .duration_since(modified)
                .is_ok_and(|settled| settled >= LISTING_SETTLE)
    }

    /// 去掉首尾空白的小写文件名以 `prefix` 开头的文件，按列出的顺序。
    fn files_starting_with(&self, prefix: &str) -> Vec<&str> {
        let start = self.keys.partition_point(|(key, _)| key.as_str() < prefix);
        let mut found: Vec<usize> = self.keys[start..]
            .iter()
            .take_while(|(key, _)| key.starts_with(prefix))
            .map(|(_, i)| *i)
            .collect();
        found.sort_unstable();
        found.into_iter().map(|i| self.files[i].as_str()).collect()
    }
}

/// 解密图片缓存的一个文件夹中的小写文件名。
///
/// 查找一张图片要在三个文件夹里试多达 125 个文件名，带媒体的导出在填这些文件夹时要把每张图片查两次：
/// 逐个检查文件名，每个都要一次取元数据的调用（#16）。所以文件名只读一次，本进程写的文件随时加进去，
/// 文件夹的修改时间表明有别的改动时再重新读。已不在磁盘上的名字只多花检查那一个路径的代价；
/// 漏掉的名字会多做一次不必要的解密。
struct CacheDirNames {
    /// 文件夹的修改时间；没有这个文件夹时为 `None`。
    modified: Option<SystemTime>,
    names: HashSet<String>,
    /// 最近一次确认 `modified` 仍是最新的时刻。
    checked: Instant,
}

/// [`ServiceHub::index_cache_dir`] 的结果。
#[derive(Clone, Copy, PartialEq)]
enum CacheDir {
    /// 没有这个文件夹：其中的路径都不存在。
    Missing,
    /// 它的文件名在 `ImageState::cache_dirs` 中。
    Indexed,
    /// 列不出来：其中任何路径都可能存在。
    Unlisted,
}

fn modified_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn lower_file_name(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().to_lowercase())
}

/// 微信为一张图片保存的哪一个文件（#194）。
///
/// `Thumb` 是聊天中显示的缩略图（`<md5>_t.dat`）；`Normal` 是点开后的图片（`<md5>.dat`）；
/// `Hd` 是点「查看原图」时微信下载的原图（`<md5>_h.dat`）；`Best` 是本机上其中最好的一个。
/// `Normal` 没有时退而用现有最好的，`Hd` 没有时为空。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageVariant {
    /// 本机上最好的文件。
    Best,
    /// 缩略图 `<md5>_t.dat`。
    Thumb,
    /// 点开后的图片 `<md5>.dat`，没有时为现有最好的。
    Normal,
    /// 原图 `<md5>_h.dat`，没有时为空。
    Hd,
}

/// 一张图片各个版本的 `.dat` 文件（有的话）。
#[derive(Default, Debug)]
struct DatFiles {
    hd: Option<String>,
    base: Option<String>,
    thumb: Option<String>,
}

/// 图片的 `.dat` 文件是哪个版本，最好的在前。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rendition {
    Hd,
    Original,
    Thumbnail,
}

impl Rendition {
    fn of(dat_path: &str) -> Self {
        if img::is_hd_dat_path(dat_path) {
            Self::Hd
        } else if img::is_t_variant_dat(dat_path) {
            Self::Thumbnail
        } else {
            Self::Original
        }
    }

    /// 解密图片缓存中的文件是从哪个版本解密来的，按 [`img::cache_output_path`] 给它的后缀判断：
    /// `_hd`；缩略图的 `_t`、`.t` 或 `_thumb`；否则为原图。
    fn of_cached(path: &str) -> Self {
        let stem = Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if stem.ends_with("_hd") {
            Self::Hd
        } else if ["_t", ".t", "_thumb"].iter().any(|s| stem.ends_with(s)) {
            Self::Thumbnail
        } else {
            Self::Original
        }
    }
}

/// 要解密哪条图片消息的图片，以及怎样回答（`image decrypt`、`resolve-cache`、`resolve-batch`）。
#[derive(Default, Clone, Debug)]
pub struct ImagePayload {
    /// 会话的用户名。
    pub session_id: Option<String>,
    /// 图片的 MD5，即消息的 `imageMd5`。
    pub image_md5: Option<String>,
    /// 它的 `.dat` 文件名，即消息的 `imageDatName`。
    pub image_dat_name: Option<String>,
    /// 消息发送的时间，用来缩小查找文件的范围。
    pub create_time: Option<i64>,
    /// 回答解密后文件的路径，而不是它的 data URL。
    pub prefer_file_path: bool,
    /// `allowCacheIndex !== false`：找不到完全对应的文件时，能否采用以规范化的 `.dat` 文件名或 MD5 命名的文件。
    pub allow_cache_index: Option<bool>,
    /// 重新解密原图，而不是回答以前解密的缩略图。
    pub force: bool,
}

impl ImagePayload {
    /// 从带 WebUI 的 `camelCase` 键的 JSON 对象读出请求；缺少或为空的键视为没有设置。
    pub fn from_json(v: &Value) -> Self {
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|s| !s.is_empty())
        };
        let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        Self {
            session_id: s("sessionId"),
            image_md5: s("imageMd5"),
            image_dat_name: s("imageDatName"),
            create_time: v.get("createTime").and_then(|t| {
                t.as_i64()
                    .or_else(|| t.as_f64().map(crate::num::saturating_i64))
            }),
            prefer_file_path: b("preferFilePath"),
            allow_cache_index: v.get("allowCacheIndex").and_then(Value::as_bool),
            force: b("force"),
        }
    }
}

/// 消息图片解密后的位置（`image decrypt`、`resolve-cache`）；失败时改为说明原因：作为命令的错误，
/// 或者作为 `image resolve-batch` 的一行。
#[derive(Default, Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageResult {
    /// 解密后的图片：它的路径；没有 `preferFilePath` 时为它的 data URL。失败时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    /// 出了什么错；成功时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 失败的原因：`not_found`、`decrypt_failed`、`unsupported_image`；
    /// 既没有 md5 也没有 `.dat` 文件名的请求为 `invalid_arguments`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<&'static str>,
    /// 文件只是缩略图：原图不在磁盘上。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_thumb: Option<bool>,
}

/// [`ServiceHub::image_decrypt_data`] 找到的消息图片。
pub(super) struct DecryptedImage {
    /// [`ServiceHub::image_decrypt`] 的回答；刚解密而没有写进缓存的图片（[`CacheWrites::OnlyWxgf`]）
    /// 为空（既没有路径也没有错误）。
    pub(super) result: ImageResult,
    /// 刚解密时的图片（缓存里本来就有时没有）。
    pub(super) fresh: Option<FreshImage>,
}

impl From<ImageResult> for DecryptedImage {
    fn from(result: ImageResult) -> Self {
        Self {
            result,
            fresh: None,
        }
    }
}

/// 解密图片的一个版本的结果（[`ServiceHub::decrypt_rendition`]）。
enum Attempt {
    /// 图片，或对它的回答。
    Done(DecryptedImage),
    /// 文件不是完整图片时的回答：文件在它的文件头或图片结构所示的结尾之前就结束了，或者它的 WXGF 数据已损坏。
    /// 有较差的版本时改为显示它。
    Incomplete(DecryptedImage),
}

/// 刚解密的图片（[`DecryptedImage::fresh`]）。
pub(super) struct FreshImage {
    /// 它的字节。
    pub(super) data: Vec<u8>,
    /// 是否是缩略图，按它在缓存中的文件名判断（[`img::is_thumbnail_path`]）。
    pub(super) thumbnail: bool,
}

/// [`ServiceHub::image_decrypt_data`] 把哪些刚解密的图片写进图片缓存。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CacheWrites {
    /// 全部：回答的是它在缓存中的文件。
    All,
    /// 只写从 WXGF 转换来的图片，它们重新转换很慢。
    ///
    /// 其他图片重新解密和从缓存读回一样快，而导出会把它们的字节直接写到目标位置：再写进缓存会让每张图片有两个文件，
    /// 而在 Windows 上，建文件占了导出每张图片的大部分开销。
    OnlyWxgf,
}

/// `image resolve-batch` 的回答：每个请求一个结果，顺序与请求一致。
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
pub struct ImageBatch {
    /// 每个请求一个结果，顺序与请求一致：找到时有 `localPath`，找不到时有 `error`。
    pub rows: Vec<ImageResult>,
}

impl ImageResult {
    fn ok(local_path: String, is_thumb: Option<bool>) -> Self {
        Self {
            local_path: Some(local_path),
            is_thumb,
            ..Default::default()
        }
    }
    fn fail(error: impl Into<String>, kind: &'static str) -> Self {
        Self {
            error: Some(error.into()),
            failure_kind: Some(kind),
            ..Default::default()
        }
    }

    /// 既没有 md5 也没有 `.dat` 文件名的请求的失败结果。
    fn no_identifier() -> Self {
        Self::fail("missing image identifier", NO_IDENTIFIER)
    }

    /// 图片是否找到并解密了。
    pub fn is_ok(&self) -> bool {
        self.local_path.is_some()
    }

    /// 把结果作为命令的回答。
    ///
    /// # Errors
    /// 请求指不出图片时返回用法错误；其他失败返回运行时错误，`failureKind`（知道时还有 `isThumb`）放在详情中。
    pub fn into_answer(self) -> AppResult<Self> {
        if self.is_ok() {
            return Ok(self);
        }
        let message = self.error.unwrap_or_else(|| "image decrypt failed".into());
        if self.failure_kind == Some(NO_IDENTIFIER) {
            return Err(AppError::usage(message));
        }
        let mut details = json!({ "failureKind": self.failure_kind });
        if let Some(thumb) = self.is_thumb {
            details["isThumb"] = thumb.into();
        }
        Err(AppError::runtime(message).with_details(details))
    }
}

/// 文件不是完整图片时的错误（[`INCOMPLETE_IMAGE`]）。
fn incomplete_error() -> AppError {
    AppError::runtime(INCOMPLETE_IMAGE).with_details(json!({ "failureKind": DECRYPT_FAILED }))
}

/// [`ServiceHub::image_decrypt`] 找到的图片的字节；失败时把它的失败作为错误返回。
fn image_bytes(result: ImageResult) -> AppResult<Vec<u8>> {
    let path = result.local_path.ok_or_else(|| {
        let err = AppError::runtime(
            result
                .error
                .unwrap_or_else(|| "image decrypt failed".into()),
        );
        // 出错的原因与 `chat image` 的说法一致：`not_found` 是命令 API 的 404
        match result.failure_kind {
            Some(kind) => err.with_details(json!({ "failureKind": kind })),
            None => err,
        }
    })?;
    std::fs::read(&path).map_err(|e| AppError::runtime(format!("failed to read {path}: {e}")))
}

/// 内置解码器转换不了的 WXGF 图片的提示：知道消息时间时写出时间，便于在微信里找到它；
/// 并说明是数据损坏还是格式还不支持。
fn unsupported_image_message(create_time: Option<i64>, reason: crate::hevc::Unsupported) -> String {
    let what = match create_time
        .map(crate::message::time::format_timestamp)
        .filter(|t| !t.is_empty())
    {
        Some(time) => format!("WXGF image sent {time}"),
        None => "WXGF image".to_string(),
    };
    if reason.is_damaged() {
        format!("{what} cannot be converted because the data is damaged: {reason}")
    } else {
        format!("{what} cannot be converted because its format is not supported yet: {reason}")
    }
}

fn file_mtime_ms(p: &str) -> u128 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn file_path_to_url(path: &str) -> String {
    let url = crate::video::path_to_file_url(Path::new(path));
    if std::fs::metadata(path).is_ok() {
        format!("{url}?v={}", file_mtime_ms(path))
    } else {
        url
    }
}

fn file_to_data_url(path: &str) -> Option<String> {
    let ext = Path::new(path)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))?;
    let mime = img::mime_from_extension(&ext)?;
    let bytes = std::fs::read(path).ok()?;
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn resolve_emit_path(path: &str, prefer_file: bool) -> String {
    if prefer_file {
        file_path_to_url(path)
    } else {
        file_to_data_url(path).unwrap_or_else(|| file_path_to_url(path))
    }
}

fn local_path_for_payload(path: &str, prefer_file: bool) -> String {
    if prefer_file {
        path.to_string()
    } else {
        resolve_emit_path(path, false)
    }
}

/// 消息图片在缓存中可能用的键：原样的 md5 和 `.dat` 文件名、它们的小写形式，以及去掉版本后缀的形式。
fn cache_keys(p: &ImagePayload) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let mut add = |value: &str| {
        if value.is_empty() {
            return;
        }
        let lower = value.to_lowercase();
        if !keys.contains(&value.to_string()) {
            keys.push(value.to_string());
        }
        if !keys.contains(&lower) {
            keys.push(lower.clone());
        }
        let normalized = img::normalize_dat_base(&lower);
        if !normalized.is_empty() && !keys.contains(&normalized) {
            keys.push(normalized);
        }
    };
    if let Some(m) = &p.image_md5 {
        add(m);
    }
    if let Some(d) = &p.image_dat_name {
        if Some(d) != p.image_md5.as_ref() {
            add(d);
        }
    }
    keys
}

/// `resolve_dat_path` 查找请求的 `.dat` 文件时可以放宽到什么程度。
struct DatSearch {
    /// 没有高清文件时接受缩略图（`_t`）。
    allow_thumbnail: bool,
    /// 忽略以前找到的路径，重新在磁盘上找。
    skip_resolved_cache: bool,
    /// 也按规范化的 `.dat` 文件名（或 md5）得出的基础名查找，见 `lookup_bases`。
    allow_dat_name_fallback: bool,
}

/// 先是在微信的媒体索引中查找图片用的 md5，然后是要扫描的 `.dat` 文件名。
fn lookup_bases(
    md5: Option<&str>,
    dat_name: Option<&str>,
    allow_dat_name_fallback: bool,
) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let mut push = |v: &str| {
        let n = v.trim().to_lowercase();
        if !n.is_empty() && img::looks_like_md5(&n) && !keys.contains(&n) {
            keys.push(n);
        }
    };
    if let Some(m) = md5 {
        push(m);
    }
    let dat_raw = dat_name.unwrap_or("").trim().to_lowercase();
    if !dat_raw.is_empty() {
        push(&dat_raw);
        let no_ext = dat_raw.strip_suffix(".dat").unwrap_or(&dat_raw).to_string();
        push(&no_ext);
        push(&img::normalize_dat_base(&no_ext));
    }
    if !allow_dat_name_fallback {
        return keys;
    }
    let fallback_raw = dat_name
        .filter(|s| !s.is_empty())
        .or(md5)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if fallback_raw.is_empty() {
        return keys;
    }
    let no_ext = fallback_raw
        .strip_suffix(".dat")
        .unwrap_or(&fallback_raw)
        .to_string();
    let base = img::normalize_dat_base(&no_ext);
    if img::looks_like_md5(&base) && !keys.contains(&base) {
        keys.push(base);
    }
    keys
}

impl ServiceHub {
    /// [`Self::account_dir_only`]，在 `since` 之后最多到磁盘上确认一次。
    fn account_dir_since(&self, since: Since) -> AppResult<PathBuf> {
        if let Some((dir, checked)) = &self.image_state.lock_or_panic().account_dir {
            if since.covers(*checked) {
                return Ok(dir.clone());
            }
        }
        let checked = Instant::now();
        let dir = self.account_dir_only()?;
        self.image_state.lock_or_panic().account_dir = Some((dir.clone(), checked));
        Ok(dir)
    }

    /// 解密图片的文件夹：缓存目录下的 `Images`。第一次往里写解密的图片时才创建。
    pub fn image_cache_root(&self) -> PathBuf {
        if let Some(root) = &self.image_state.lock_or_panic().cache_root {
            return root.clone();
        }
        let root = self.cache_base().join("Images");
        self.image_state.lock_or_panic().cache_root = Some(root.clone());
        root
    }

    /// 建好图片缓存的文件夹 `dir`，以及它上面的缓存目录（按 [`Self::cache_base`] 的方式建）。
    fn make_image_cache_dir(&self, dir: &Path) -> io::Result<()> {
        self.cache_base();
        std::fs::create_dir_all(dir)
    }

    fn image_keys(&self) -> (Option<u8>, Option<[u8; 16]>) {
        match self.profile() {
            Ok(p) => (
                p.image_xor_key.and_then(|k| u8::try_from(k).ok()),
                p.image_aes_key
                    .as_deref()
                    .and_then(crate::decrypt::parse_aes_key),
            ),
            Err(_) => (None, None),
        }
    }

    fn image_state_get(&self, key: &str) -> Option<String> {
        self.image_state.lock_or_panic().resolved.get(key).cloned()
    }

    fn image_state_set(&self, key: String, path: String) {
        self.image_state.lock_or_panic().resolved.insert(key, path);
    }

    fn image_state_remove(&self, key: &str) {
        self.image_state.lock_or_panic().resolved.remove(key);
    }

    /// 缓存的图片能否使用：扩展名认得出、文件存在、不为空，也不是全为零或被截断的 JPEG。不能用的 JPEG 会被删掉。
    fn usable_image_cache_file(&self, path: &str) -> bool {
        // 空文件不会是图片（以前的缓存里可能有写了一半留下的）
        if !img::is_image_file(path) || std::fs::metadata(path).map_or(true, |m| m.len() == 0) {
            return false;
        }
        let ext = Path::new(path)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if ext == "jpg" || ext == "jpeg" {
            if let Ok(data) = std::fs::read(path) {
                // 被截断的文件：早期的版本会把微信还没写完的文件解密进缓存（#1112）
                if img::is_likely_corrupted_jpeg(&data) || img::ending::is_cut_short(&data) {
                    let _ = std::fs::remove_file(path);
                    return false;
                }
            }
        }
        true
    }

    fn cache_resolved_paths(
        &self,
        cache_key: &str,
        md5: Option<&str>,
        dat_name: Option<&str>,
        output: &str,
    ) {
        self.image_state_set(cache_key.to_string(), output.to_string());
        if let Some(m) = md5.filter(|m| *m != cache_key) {
            self.image_state_set(m.to_string(), output.to_string());
        }
        if let Some(d) = dat_name.filter(|d| *d != cache_key && Some(*d) != md5) {
            self.image_state_set(d.to_string(), output.to_string());
        }
    }

    fn cache_dat_path(&self, account_dir: &Path, name: &str, dat_path: &str) {
        let dir = account_dir.to_string_lossy();
        self.image_state_set(format!("{dir}|{name}"), dat_path.to_string());
        let normalized = img::normalize_dat_base(name);
        if !normalized.is_empty() && normalized != name.to_lowercase() {
            self.image_state_set(format!("{dir}|{normalized}"), dat_path.to_string());
        }
    }

    /// 一条消息在 `msg/attach/<md5(session)>/<yyyy-mm>/Img` 中的 `.dat` 文件，最多查 240 个目录；
    /// `<md5(session)>` 取 [`img::session_dirs_for_storage`] 中第一个有这个月份文件夹的。
    fn collect_dat_candidates(
        &self,
        account_dir: &Path,
        base_md5: &str,
        session_id: Option<&str>,
        create_time: Option<i64>,
        since: Since,
    ) -> Vec<String> {
        let session = session_id.unwrap_or("").trim();
        let month = img::year_month_from_create_time(create_time);
        if session.is_empty() || month.is_empty() {
            return Vec::new();
        }
        // 会话的各个文件夹中第一个有这个月份文件夹的
        let Some((root, listing)) = img::session_dirs_for_storage(session)
            .into_iter()
            .map(|session_dir| {
                account_dir
                    .join("msg")
                    .join("attach")
                    .join(session_dir)
                    .join(&month)
                    .join("Img")
            })
            .find_map(|root| self.dir_listing(&root, since).map(|l| (root, l)))
        else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        let mut budget = 240i32;
        let mut stack: Vec<(PathBuf, u32, Option<Arc<DirListing>>)> =
            vec![(root, 0, Some(listing))];
        while let Some((dir, depth, listed)) = stack.pop() {
            if budget <= 0 {
                break;
            }
            budget -= 1;
            let Some(listing) = listed.or_else(|| self.dir_listing(&dir, since)) else {
                continue;
            };
            // 每个候选名转成小写后都以 `base_md5` 开头（`normalize_dat_base` 只去掉后缀），所以只检查这些名字；
            // 列表在 `since` 之后确认过是最新的，所以它们都在磁盘上
            for name in listing.files_starting_with(base_md5) {
                if img::is_hardlink_candidate_name(name, base_md5) {
                    let s = dir.join(name).to_string_lossy().to_string();
                    if !out.contains(&s) {
                        out.push(s);
                    }
                }
            }
            if depth >= 1 {
                continue;
            }
            for name in &listing.dirs {
                if name.is_empty() || name.starts_with('.') {
                    continue;
                }
                stack.push((dir.join(name), depth + 1, None));
            }
        }
        out
    }

    /// `dir` 的条目（没有这个文件夹时为 `None`）；自上次读取后有变化时才重新读，
    /// 是否有变化在 `since` 之后最多检查一次。
    fn dir_listing(&self, dir: &Path, since: Since) -> Option<Arc<DirListing>> {
        let cached = self
            .image_state
            .lock_or_panic()
            .dir_listings
            .get(dir)
            .map(|l| (l.listing.clone(), l.checked));
        let checked = Instant::now();
        match cached {
            Some((listing, at)) if since.covers(at) => return listing,
            Some((Some(listing), _)) if listing.current(dir) => {
                if let Some(entry) = self.image_state.lock_or_panic().dir_listings.get_mut(dir) {
                    if entry
                        .listing
                        .as_ref()
                        .is_some_and(|l| Arc::ptr_eq(l, &listing))
                    {
                        entry.checked = checked;
                    }
                }
                return Some(listing);
            }
            _ => {}
        }
        // 读取时不持有锁：导出的媒体线程是同时查找的
        let listing = DirListing::read(dir).map(Arc::new);
        let mut state = self.image_state.lock_or_panic();
        if state.dir_listings.len() >= MAX_DIR_LISTINGS {
            state.dir_listings.clear();
        }
        state.dir_listings.insert(
            dir.to_path_buf(),
            Listed {
                listing: listing.clone(),
                checked,
            },
        );
        listing
    }

    /// 文件 `path` 的大小和时间，不在磁盘上时为 `None`：
    /// 它所在文件夹的列表在 `since` 之后确认过是最新的时取自列表（列表中没有的文件就不存在），否则取自磁盘。
    fn file_meta(&self, path: &Path, since: Since) -> Option<FileMeta> {
        let listed = path.parent().and_then(|dir| {
            self.image_state
                .lock_or_panic()
                .dir_listings
                .get(dir)
                .filter(|l| since.covers(l.checked))
                .map(|l| l.listing.clone())
        });
        if let (Some(listing), Some(name)) = (listed, path.file_name()) {
            // 没有这个文件夹，或者列表中没有这个名字：不在磁盘上
            if let Some(meta) = listing?.file(&name.to_string_lossy())? {
                return Some(meta);
            }
        }
        std::fs::metadata(path)
            .ok()
            .filter(std::fs::Metadata::is_file)
            .map(|m| FileMeta::of(&m))
    }

    /// 从 `dat` 解密出的图片所在的缓存文件夹（[`img::month_folder`]）。
    fn dat_month(&self, dat: &str, since: Since) -> String {
        img::month_folder(
            self.file_meta(Path::new(dat), since)
                .and_then(|m| m.modified),
        )
    }

    /// `paths` 中最大的：先比文件大小，再比新旧，最后比名字。
    fn pick_largest(&self, paths: &[String], since: Since) -> Option<String> {
        let mut list: Vec<(u64, u128, &String)> = Vec::new();
        for p in paths {
            if !p.is_empty() && !list.iter().any(|(_, _, q)| *q == p) {
                let meta = self.file_meta(Path::new(p), since);
                let modified_ms = meta
                    .and_then(|m| m.modified)
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_millis());
                list.push((meta.map_or(0, |m| m.len), modified_ms, p));
            }
        }
        list.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(b.2)));
        list.first().map(|(_, _, p)| (*p).clone())
    }

    /// 图片 `base_md5` 最好的 `.dat`：取它所在会话的文件夹中的（[`Self::select_listed_dat`]），
    /// 除非微信的媒体索引指出磁盘上有更好的版本，或者文件夹里一个也没有（#651）。
    ///
    /// 索引只是索引：许多图片在里面没有记录，所以总是查找文件夹，索引中没有更好的版本时保留文件夹中找到的。
    fn select_best_dat(
        &self,
        account_dir: &Path,
        base_md5: &str,
        session_id: Option<&str>,
        create_time: Option<i64>,
        allow_thumbnail: bool,
        since: Since,
    ) -> Option<String> {
        let listed = self.select_listed_dat(
            account_dir,
            base_md5,
            session_id,
            create_time,
            allow_thumbnail,
            since,
        );
        if listed
            .as_deref()
            .is_some_and(|p| Rendition::of(p) == Rendition::Hd)
        {
            return listed;
        }
        let linked = self.select_linked_dat(account_dir, base_md5, allow_thumbnail, since);
        match (listed, linked) {
            (Some(listed), Some(linked)) if Rendition::of(&linked) < Rendition::of(&listed) => {
                Some(linked)
            }
            (listed, linked) => listed.or(linked),
        }
    }

    /// 微信的媒体索引列出、且在磁盘上的 `base_md5` 的最好版本，同等的取最新的；
    /// 不是 `allow_thumbnail` 时只取高清版。
    fn select_linked_dat(
        &self,
        account_dir: &Path,
        base_md5: &str,
        allow_thumbnail: bool,
        since: Since,
    ) -> Option<String> {
        self.linked_dats(account_dir, base_md5, since)
            .into_iter()
            .filter(|p| allow_thumbnail || Rendition::of(p) == Rendition::Hd)
            // 同等的取第一个：索引把最新的列在前面
            .min_by_key(|p| Rendition::of(p))
    }

    /// 微信的媒体索引（`hardlink.db`）列出、且在磁盘上的 `base_md5` 的 `.dat` 文件，最新的在前。
    fn linked_dats(&self, account_dir: &Path, base_md5: &str, since: Since) -> Vec<String> {
        if !img::looks_like_md5(base_md5) {
            return Vec::new();
        }
        let index = self.linked_image_index(account_dir, since);
        index
            .get(base_md5)
            .into_iter()
            .flatten()
            .map(|rel| account_dir.join(rel))
            .filter(|p| {
                p.to_string_lossy().to_lowercase().ends_with(".dat")
                    && self.file_meta(p, since).is_some()
            })
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    /// 以 md5 为键的微信图片媒体索引；自上次读取后数据库有变化时才重新读，是否有变化在 `since` 之后最多检查一次。
    /// 没有数据库密钥时读不了，为空。
    fn linked_image_index(
        &self,
        account_dir: &Path,
        since: Since,
    ) -> Arc<HashMap<String, Vec<PathBuf>>> {
        if let Some(cached) = self
            .image_state
            .lock_or_panic()
            .linked_images
            .as_ref()
            .filter(|c| c.account_dir == account_dir && since.covers(c.checked))
        {
            return cached.by_md5.clone();
        }
        let checked = Instant::now();
        let db = account_dir.join("db_storage/hardlink/hardlink.db");
        let stamp = [db.clone(), db.with_extension("db-wal")].map(|path| {
            std::fs::metadata(path)
                .ok()
                .map(|m| (m.modified().ok(), m.len()))
        });
        if let Some(cached) = self
            .image_state
            .lock_or_panic()
            .linked_images
            .as_mut()
            .filter(|c| c.account_dir == account_dir && c.stamp == stamp)
        {
            cached.checked = checked;
            return cached.by_md5.clone();
        }
        // 与文件夹一样，读取时不持有锁
        let by_md5 = Arc::new(
            self.open_wcdb()
                .ok()
                .and_then(|wcdb| wcdb.image_hardlink_index().ok())
                .unwrap_or_default(),
        );
        self.image_state.lock_or_panic().linked_images = Some(LinkedImages {
            account_dir: account_dir.to_path_buf(),
            stamp,
            by_md5: by_md5.clone(),
            checked,
        });
        by_md5
    }

    /// 列表中最好的 `.dat` 文件：`Img` 文件夹中的高清版，没有时为 `<md5>.dat`，再没有时为 `_t` 缩略图。
    fn select_listed_dat(
        &self,
        account_dir: &Path,
        base_md5: &str,
        session_id: Option<&str>,
        create_time: Option<i64>,
        allow_thumbnail: bool,
        since: Since,
    ) -> Option<String> {
        let candidates: Vec<String> = self
            .collect_dat_candidates(account_dir, base_md5, session_id, create_time, since)
            .into_iter()
            .filter(|p| p.to_lowercase().ends_with(".dat"))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let in_img: Vec<String> = candidates
            .iter()
            .filter(|p| img::is_img_scoped_dat_path(p))
            .cloned()
            .collect();
        let hd: Vec<String> = in_img
            .iter()
            .filter(|p| img::is_hd_dat_path(p))
            .cloned()
            .collect();
        if let Some(p) = self.pick_largest(&hd, since) {
            return Some(p);
        }
        if !allow_thumbnail {
            return None;
        }
        let sel = |list: &[String], f: &dyn Fn(&str) -> bool| {
            self.pick_largest(
                &list.iter().filter(|p| f(p)).cloned().collect::<Vec<_>>(),
                since,
            )
        };
        let is_base = |p: &str| img::is_base_dat_path(p, base_md5);
        let is_t = |p: &str| img::is_t_variant_dat(p);
        sel(&in_img, &is_base)
            .or_else(|| sel(&candidates, &is_base))
            .or_else(|| sel(&in_img, &is_t))
            .or_else(|| sel(&candidates, &is_t))
    }

    fn resolve_dat_path(
        &self,
        account_dir: &Path,
        p: &ImagePayload,
        search: DatSearch,
        since: Since,
    ) -> Option<String> {
        let DatSearch {
            allow_thumbnail,
            skip_resolved_cache,
            allow_dat_name_fallback,
        } = search;
        let (md5, dat_name, session_id, create_time) = (
            p.image_md5.as_deref(),
            p.image_dat_name.as_deref(),
            p.session_id.as_deref(),
            p.create_time,
        );
        let bases = lookup_bases(md5, dat_name, allow_dat_name_fallback);
        if bases.is_empty() {
            return None;
        }
        let dir = account_dir.to_string_lossy().to_string();
        let mut thumbnail_found_before = None;
        if !skip_resolved_cache {
            let mut candidates: Vec<String> = bases.clone();
            for extra in [
                md5.unwrap_or("").trim().to_lowercase(),
                dat_name.unwrap_or("").trim().to_lowercase(),
            ] {
                if !extra.is_empty() && !candidates.contains(&extra) {
                    candidates.push(extra);
                }
            }
            for key in candidates {
                let Some(cached) = self.image_state_get(&format!("{dir}|{key}")) else {
                    continue;
                };
                if self.file_meta(Path::new(&cached), since).is_none() {
                    continue;
                }
                if !allow_thumbnail && !img::is_hd_dat_path(&cached) {
                    continue;
                }
                // 缩略图只是当时磁盘上有的：之后可能下载了完整的图（#858），所以先重新查看文件夹
                if Rendition::of(&cached) == Rendition::Thumbnail {
                    thumbnail_found_before.get_or_insert(cached);
                    continue;
                }
                return Some(cached);
            }
        }
        for base in &bases {
            let Some(selected) = self.select_best_dat(
                account_dir,
                base,
                session_id,
                create_time,
                allow_thumbnail,
                since,
            ) else {
                continue;
            };
            self.cache_dat_path(account_dir, base, &selected);
            if let Some(m) = md5 {
                self.cache_dat_path(account_dir, m, &selected);
            }
            if let Some(d) = dat_name {
                self.cache_dat_path(account_dir, d, &selected);
            }
            let name = Path::new(&selected)
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !name.is_empty() {
                self.cache_dat_path(account_dir, &name, &selected);
            }
            return Some(selected);
        }
        // 这条消息的文件夹里什么都没有：
        // 之前（通过另一条消息，在它的文件夹里）为同一张图片找到的缩略图仍然就是这张图片
        thumbnail_found_before
    }

    /// 缓存中从 `dat_path`（它的缓存文件夹为 `month`）解密出的图片，能用时才返回。
    fn find_cached_output(
        &self,
        dat_path: &str,
        month: &str,
        session_id: Option<&str>,
        prefer_hd: bool,
        since: Since,
    ) -> Option<String> {
        let root = self.image_cache_root();
        let candidates = img::CacheCandidates::new(&root, dat_path, month, session_id, prefer_hd);
        // 前面的文件夹里一个名字都没有时才检查下一个：多数图片在第一个文件夹里就能找到，
        // 而旧布局下每张图片各有自己的文件夹
        let found = candidates.folders().into_iter().find_map(|folder| {
            self.possibly_cached_in(folder, since)
                .into_iter()
                .map(|c| c.to_string_lossy().to_string())
                // 不存在的文件同样不能用
                .find(|c| self.usable_image_cache_file(c))
        });
        found
    }

    /// 一个文件夹中候选文件的 [`Self::possibly_cached`]，只为文件夹列出的名字拼出路径。
    fn possibly_cached_in(&self, folder: img::CacheFolder<'_>, since: Since) -> Vec<PathBuf> {
        let dir = folder.path();
        let found = self.index_cache_dir(dir, since);
        let mut kept = Vec::new();
        if found == CacheDir::Missing {
            return kept;
        }
        let state = self.image_state.lock_or_panic();
        // 此后被丢弃的索引（超过 `MAX_DIR_LISTINGS`）说明不了什么
        let names = match found {
            CacheDir::Indexed => state.cache_dirs.get(dir).map(|index| &index.names),
            CacheDir::Missing | CacheDir::Unlisted => None,
        };
        folder.each_name(|name| {
            if names.is_none_or(|names| names.contains(name)) {
                kept.push(dir.join(name));
            }
        });
        kept
    }

    /// `candidates` 中所在文件夹列出了其名字的那些，保持顺序：即可能存在的。
    ///
    /// 每个文件夹花一次取元数据的调用（在 `since` 之后最多一次），而不是每个路径一次；返回的路径仍要检查。
    fn possibly_cached(&self, candidates: Vec<PathBuf>, since: Since) -> Vec<PathBuf> {
        // 众多候选所在的少数几个文件夹，按路径的字节区分：路径的哈希要逐个遍历它的组成部分
        let mut dirs: Vec<(&Path, CacheDir)> = Vec::new();
        let dir_of: Vec<Option<usize>> = candidates
            .iter()
            .map(|c| {
                let dir = c.parent()?;
                Some(
                    match dirs
                        .iter()
                        .position(|(d, _)| d.as_os_str() == dir.as_os_str())
                    {
                        Some(i) => i,
                        None => {
                            dirs.push((dir, self.index_cache_dir(dir, since)));
                            dirs.len() - 1
                        }
                    },
                )
            })
            .collect();
        let state = self.image_state.lock_or_panic();
        // 每个文件夹的名字只查一次；此后被丢弃的索引（超过 `MAX_DIR_LISTINGS`）说明不了什么
        let names: Vec<Option<&HashSet<String>>> = dirs
            .iter()
            .map(|(dir, found)| match found {
                CacheDir::Indexed => state.cache_dirs.get(*dir).map(|index| &index.names),
                CacheDir::Missing | CacheDir::Unlisted => None,
            })
            .collect();
        let listed = |(c, dir): (&PathBuf, &Option<usize>)| {
            let Some(i) = *dir else {
                return true;
            };
            match dirs[i].1 {
                CacheDir::Missing => false,
                CacheDir::Indexed => match (names[i], lower_file_name(c)) {
                    (Some(index), Some(name)) => index.contains(&name),
                    _ => true,
                },
                CacheDir::Unlisted => true,
            }
        };
        let kept: Vec<bool> = candidates.iter().zip(&dir_of).map(listed).collect();
        drop(state);
        candidates
            .into_iter()
            .zip(kept)
            .filter_map(|(c, keep)| keep.then_some(c))
            .collect()
    }

    /// 更新缓存文件夹 `dir` 的文件名，在 `since` 之后最多检查一次。
    fn index_cache_dir(&self, dir: &Path, since: Since) -> CacheDir {
        if let Some(index) = self
            .image_state
            .lock_or_panic()
            .cache_dirs
            .get(dir)
            .filter(|index| since.covers(index.checked))
        {
            return match index.modified {
                Some(_) => CacheDir::Indexed,
                None => CacheDir::Missing,
            };
        }
        let checked = Instant::now();
        let Some(modified) = modified_time(dir) else {
            let mut state = self.image_state.lock_or_panic();
            if state.cache_dirs.len() >= MAX_DIR_LISTINGS && !state.cache_dirs.contains_key(dir) {
                state.cache_dirs.clear();
            }
            state.cache_dirs.insert(
                dir.to_path_buf(),
                CacheDirNames {
                    modified: None,
                    names: HashSet::new(),
                    checked,
                },
            );
            return CacheDir::Missing;
        };
        {
            let mut state = self.image_state.lock_or_panic();
            if let Some(index) = state.cache_dirs.get_mut(dir) {
                // 本进程在这期间改名放进文件夹的文件是在锁内记下的（`write_cache_file`）
                if index.modified.is_some()
                    && (index.modified == Some(modified) || index.modified == modified_time(dir))
                {
                    index.checked = checked;
                    return CacheDir::Indexed;
                }
            }
        }
        // 读取时不持有锁。先取了时间：期间写入的文件会再次改变它，所以下一次查找会重新读文件夹
        let Ok(entries) = std::fs::read_dir(dir) else {
            return CacheDir::Unlisted;
        };
        let names: Vec<String> = entries
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_lowercase())
            .collect();
        let mut state = self.image_state.lock_or_panic();
        if state.cache_dirs.len() >= MAX_DIR_LISTINGS && !state.cache_dirs.contains_key(dir) {
            state.cache_dirs.clear();
        }
        let index = state
            .cache_dirs
            .entry(dir.to_path_buf())
            .or_insert_with(|| CacheDirNames {
                modified: Some(modified),
                names: HashSet::new(),
                checked,
            });
        // 加到已知的名字中，期间别的线程记下的文件不会丢掉
        index.names.extend(names);
        index.modified = Some(modified);
        index.checked = checked;
        CacheDir::Indexed
    }

    /// 像 [`write_atomically`] 那样把解密的图片写进缓存，并把它加进所在文件夹的名字。
    ///
    /// 临时文件写在 `<cache>/Images/.partial`，改名在锁内记下：否则其他媒体线程会在写入时看到文件夹有变化，
    /// 重新读一遍它的所有名字。写入时发现文件夹不存在才创建。
    fn write_cache_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let (Some(dir), Some(name)) = (path.parent(), lower_file_name(path)) else {
            return write_atomically(path, bytes);
        };
        let partial = self.image_cache_root().join(".partial");
        let in_folder = |folder: &Path, op: &dyn Fn() -> io::Result<()>| match op() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.make_image_cache_dir(folder)?;
                op()
            }
            done => done,
        };
        let tmp = temporary_sibling(&partial.join(&name));
        let written = in_folder(&partial, &|| std::fs::write(&tmp, bytes)).and_then(|()| {
            let mut state = self.image_state.lock_or_panic();
            in_folder(dir, &|| std::fs::rename(&tmp, path))?;
            let modified = modified_time(dir);
            match (state.cache_dirs.get_mut(dir), modified) {
                // 已知不存在的文件夹，此后可能已被别的进程建好并写入了文件
                (Some(index), _) if index.modified.is_none() => {
                    state.cache_dirs.remove(dir);
                }
                (Some(index), Some(modified)) => {
                    index.names.insert(name);
                    index.modified = Some(modified);
                }
                _ => {}
            }
            Ok(())
        });
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written
    }

    fn remove_duplicate_cache_candidates(
        &self,
        dat_path: &str,
        month: &str,
        session_id: Option<&str>,
        keep: &str,
        since: Since,
    ) {
        let root = self.image_cache_root();
        let mut all: Vec<PathBuf> =
            img::cache_output_candidates(&root, dat_path, month, session_id, false);
        all.extend(img::cache_output_candidates(
            &root, dat_path, month, session_id, true,
        ));
        let mut seen = HashSet::new();
        all.retain(|c| seen.insert(c.clone()));
        for c in self.possibly_cached(all, since) {
            let s = c.to_string_lossy().to_string();
            if s == keep || !img::is_image_file(&s) {
                continue;
            }
            // 文件不存在时删不掉也无妨
            let _ = std::fs::remove_file(&c);
        }
    }

    /// 有高清版时，用它替换能用的缓存缩略图 `cached`。
    fn try_promote_thumbnail(
        &self,
        p: &ImagePayload,
        cache_key: &str,
        cached: &str,
        since: Since,
    ) -> Option<String> {
        if !img::is_image_file(cached) || img::is_hd_path(cached) {
            return None;
        }
        let account_dir = self.account_dir_since(since).ok()?;
        let hd_dat = self.resolve_dat_path(
            &account_dir,
            p,
            DatSearch {
                allow_thumbnail: false,
                skip_resolved_cache: true,
                allow_dat_name_fallback: false,
            },
            since,
        )?;
        let remove_old = |keep: &str| {
            if cached != keep && Path::new(cached).exists() && !img::is_hd_path(cached) {
                let _ = std::fs::remove_file(cached);
            }
        };
        let month = self.dat_month(&hd_dat, since);
        if let Some(existing) =
            self.find_cached_output(&hd_dat, &month, p.session_id.as_deref(), true, since)
        {
            // 能用的缓存文件，所以在磁盘上
            if img::is_image_file(&existing) && img::is_hd_path(&existing) {
                self.cache_resolved_paths(
                    cache_key,
                    p.image_md5.as_deref(),
                    p.image_dat_name.as_deref(),
                    &existing,
                );
                remove_old(&existing);
                return Some(existing);
            }
        }
        let upgraded = self.image_decrypt_since(
            &ImagePayload {
                prefer_file_path: true,
                force: true,
                ..p.clone()
            },
            since,
        );
        if !upgraded.is_ok() {
            return None;
        }
        let path = self
            .image_state_get(cache_key)
            .filter(|c| Path::new(c).exists())
            .or(upgraded.local_path)?;
        if !Path::new(&path).exists() || !img::is_image_file(&path) || !img::is_hd_path(&path) {
            return None;
        }
        self.cache_resolved_paths(
            cache_key,
            p.image_md5.as_deref(),
            p.image_dat_name.as_deref(),
            &path,
        );
        remove_old(&path);
        Some(path)
    }

    /// 只从缓存回答（`image resolve-cache`），从不解密缺少的文件。
    pub fn image_resolve_cache(&self, p: &ImagePayload) -> ImageResult {
        self.image_resolve_cache_since(p, Since::now())
    }

    /// 直接相信 `since` 之后检查过的内容的 [`Self::image_resolve_cache`]。
    pub(super) fn image_resolve_cache_since(&self, p: &ImagePayload, since: Since) -> ImageResult {
        let keys = cache_keys(p);
        let Some(cache_key) = keys.first().cloned() else {
            return ImageResult::no_identifier();
        };
        let finish = |final_path: &str| ImageResult {
            local_path: Some(local_path_for_payload(final_path, p.prefer_file_path)),
            ..Default::default()
        };
        for key in &keys {
            let Some(cached) = self.image_state_get(key) else {
                continue;
            };
            // 不存在的文件同样不能用
            if !self.usable_image_cache_file(&cached) {
                self.image_state_remove(key);
                continue;
            }
            let upgraded = if !img::is_hd_path(&cached) {
                self.try_promote_thumbnail(p, key, &cached, since)
            } else {
                None
            };
            return finish(upgraded.as_deref().unwrap_or(&cached));
        }
        if let Ok(account_dir) = self.account_dir_since(since) {
            if let Some(dat) = self.resolve_dat_path(
                &account_dir,
                p,
                DatSearch {
                    allow_thumbnail: true,
                    skip_resolved_cache: false,
                    allow_dat_name_fallback: p.allow_cache_index != Some(false),
                },
                since,
            ) {
                let month = self.dat_month(&dat, since);
                if let Some(existing) =
                    self.find_cached_output(&dat, &month, p.session_id.as_deref(), false, since)
                {
                    let upgraded = if !img::is_hd_path(&existing) {
                        self.try_promote_thumbnail(p, &cache_key, &existing, since)
                    } else {
                        None
                    };
                    let final_path = upgraded.unwrap_or(existing);
                    self.cache_resolved_paths(
                        &cache_key,
                        p.image_md5.as_deref(),
                        p.image_dat_name.as_deref(),
                        &final_path,
                    );
                    return finish(&final_path);
                }
            }
        }
        ImageResult::fail("cached image not found", "not_found")
    }

    /// 把消息图片解密进缓存，或者在缓存中找到它（`image decrypt`）。
    pub fn image_decrypt(&self, p: &ImagePayload) -> ImageResult {
        self.image_decrypt_since(p, Since::now())
    }

    /// 直接相信 `since` 之后检查过的内容的 [`Self::image_decrypt`]。
    pub(super) fn image_decrypt_since(&self, p: &ImagePayload, since: Since) -> ImageResult {
        self.image_decrypt_data(p, since, CacheWrites::All).result
    }

    /// [`Self::image_decrypt_since`]，刚解密时同时给出图片：导出直接写出它的字节，而不是再从缓存文件读回。
    ///
    /// `cache` 说明哪些刚解密的图片写进缓存。
    pub(super) fn image_decrypt_data(
        &self,
        p: &ImagePayload,
        since: Since,
        cache: CacheWrites,
    ) -> DecryptedImage {
        let keys = cache_keys(p);
        let Some(cache_key) = keys.first().cloned() else {
            return ImageResult::no_identifier().into();
        };
        if p.force {
            for key in &keys {
                let Some(cached) = self.image_state_get(key) else {
                    continue;
                };
                // 不存在的文件同样不能用
                if !self.usable_image_cache_file(&cached) {
                    self.image_state_remove(key);
                    continue;
                }
                if img::is_hd_path(&cached) {
                    self.cache_resolved_paths(
                        &cache_key,
                        p.image_md5.as_deref(),
                        p.image_dat_name.as_deref(),
                        &cached,
                    );
                    return ImageResult::ok(
                        local_path_for_payload(&cached, p.prefer_file_path),
                        None,
                    )
                    .into();
                }
            }
        } else if let Some(cached) = self.image_state_get(&cache_key) {
            if self.usable_image_cache_file(&cached) {
                let upgraded = if !img::is_hd_path(&cached) {
                    self.try_promote_thumbnail(p, &cache_key, &cached, since)
                } else {
                    None
                };
                return ImageResult::ok(
                    local_path_for_payload(
                        upgraded.as_deref().unwrap_or(&cached),
                        p.prefer_file_path,
                    ),
                    None,
                )
                .into();
            }
            self.image_state_remove(&cache_key);
        }
        self.image_decrypt_internal(p, &cache_key, since, cache)
    }

    fn image_decrypt_internal(
        &self,
        p: &ImagePayload,
        cache_key: &str,
        since: Since,
        cache: CacheWrites,
    ) -> DecryptedImage {
        let Ok(account_dir) = self.account_dir_since(since) else {
            return ImageResult::fail("no account or database path configured", "not_found").into();
        };
        let allow_fallback = p.allow_cache_index != Some(false);
        let search = |allow_thumbnail: bool| {
            self.resolve_dat_path(
                &account_dir,
                p,
                DatSearch {
                    allow_thumbnail,
                    skip_resolved_cache: false,
                    allow_dat_name_fallback: allow_fallback,
                },
                since,
            )
        };
        let dat = if p.force {
            search(false).or_else(|| search(true))
        } else {
            search(true)
        };
        let Some(dat) = dat else {
            return if p.force {
                ImageResult::fail(
                    "image file not found, open the image in WeChat and retry",
                    "not_found",
                )
            } else {
                ImageResult::fail("image file not found", "not_found")
            }
            .into();
        };
        let incomplete = match self.decrypt_rendition(p, cache_key, &dat, since, cache) {
            Attempt::Done(image) => return image,
            Attempt::Incomplete(answer) => answer,
        };
        // 微信还在写的文件：改用较差的版本，与更好的版本还不存在时一样
        for lesser in self.lesser_renditions(p, &dat, since) {
            if let Attempt::Done(image) =
                self.decrypt_rendition(p, cache_key, &lesser, since, cache)
            {
                if image.result.error.is_none() {
                    return image;
                }
            }
        }
        incomplete
    }

    /// `p` 的图片中比 `dat` 差的各版本的 `.dat` 文件，最好的在前（[`Self::dat_files`]）。
    fn lesser_renditions(&self, p: &ImagePayload, dat: &str, since: Since) -> Vec<String> {
        let rendition = Rendition::of(dat);
        let files = self.dat_files(p, since).unwrap_or_default();
        [files.base, files.thumb]
            .into_iter()
            .flatten()
            .filter(|f| Rendition::of(f) > rendition)
            .collect()
    }

    /// 把 `p` 的图片的一个文件 `dat` 解密进缓存（按 `cache` 的规定），或者在缓存中找到它。
    fn decrypt_rendition(
        &self,
        p: &ImagePayload,
        cache_key: &str,
        dat: &str,
        since: Since,
        cache: CacheWrites,
    ) -> Attempt {
        let incomplete = || {
            Attempt::Incomplete(
                ImageResult {
                    error: Some(INCOMPLETE_IMAGE.into()),
                    failure_kind: Some(DECRYPT_FAILED),
                    is_thumb: Some(img::is_thumbnail_path(dat)),
                    ..Default::default()
                }
                .into(),
            )
        };
        let done = |result: ImageResult| Attempt::Done(result.into());
        if !Path::new(dat)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase().contains("dat"))
            .unwrap_or(false)
        {
            self.cache_resolved_paths(
                cache_key,
                p.image_md5.as_deref(),
                p.image_dat_name.as_deref(),
                dat,
            );
            return done(ImageResult::ok(
                local_path_for_payload(dat, p.prefer_file_path),
                Some(img::is_thumbnail_path(dat)),
            ));
        }

        let prefer_hd = p.force && Rendition::of(dat) == Rendition::Hd;
        let month = self.dat_month(dat, since);
        if let Some(existing) =
            self.find_cached_output(dat, &month, p.session_id.as_deref(), prefer_hd, since)
        {
            // 从比 `dat` 差的版本解密出的缓存（微信下载这张图之前缓存的缩略图，#858）改为解密 `dat` 来替换
            if Rendition::of_cached(&existing) <= Rendition::of(dat) {
                self.cache_resolved_paths(
                    cache_key,
                    p.image_md5.as_deref(),
                    p.image_dat_name.as_deref(),
                    &existing,
                );
                return done(ImageResult::ok(
                    local_path_for_payload(&existing, p.prefer_file_path),
                    Some(img::is_thumbnail_path(&existing)),
                ));
            }
        }

        let (xor_key, aes_key) = self.image_keys();
        let Some(xor_key) = xor_key else {
            return done(ImageResult::fail(
                "image decrypt key is not configured",
                "not_found",
            ));
        };
        let decrypted =
            match crate::decrypt::decrypt_file(Path::new(dat), xor_key, aes_key.as_ref()) {
                Ok(r) => r,
                Err(e) if e.is::<crate::decrypt::CutShort>() => return incomplete(),
                Err(e) => {
                    return done(ImageResult::fail(
                        format!("native decrypt failed, check the image keys: {e:#}"),
                        "not_found",
                    ))
                }
            };
        let wxgf = img::is_wxgf(&decrypted.data);
        let (data, wxgf_failure) = img::unwrap_wxgf(decrypted.data);
        if let Some(reason) = wxgf_failure {
            let answer = ImageResult {
                error: Some(unsupported_image_message(p.create_time, reason)),
                failure_kind: Some(UNSUPPORTED_IMAGE),
                is_thumb: Some(img::is_thumbnail_path(dat)),
                ..Default::default()
            };
            // 损坏的数据可能是微信还没写完的；它的提示说明数据已损坏
            return if reason.is_damaged() {
                Attempt::Incomplete(answer.into())
            } else {
                done(answer)
            };
        }
        let Some(ext) = img::detect_image_extension(&data) else {
            return done(ImageResult {
                error: Some("decrypted data is not a valid image".into()),
                failure_kind: Some(DECRYPT_FAILED),
                is_thumb: Some(img::is_thumbnail_path(dat)),
                ..Default::default()
            });
        };
        if img::ending::is_cut_short(&data) {
            return incomplete();
        }
        let output = img::cache_output_path(
            &self.image_cache_root(),
            dat,
            &month,
            ext,
            p.session_id.as_deref(),
        );
        let out = output.to_string_lossy().to_string();
        let thumbnail = img::is_thumbnail_path(&out);
        if !wxgf && cache == CacheWrites::OnlyWxgf {
            return Attempt::Done(DecryptedImage {
                result: ImageResult::default(),
                fresh: Some(FreshImage { data, thumbnail }),
            });
        }
        // 写好后改名到位：导出的其他媒体线程一看到这个缓存文件就可能去读它
        if let Err(e) = self.write_cache_file(&output, &data) {
            return done(ImageResult::fail(e.to_string(), "not_found"));
        }
        self.remove_duplicate_cache_candidates(dat, &month, p.session_id.as_deref(), &out, since);
        self.cache_resolved_paths(
            cache_key,
            p.image_md5.as_deref(),
            p.image_dat_name.as_deref(),
            &out,
        );
        let local = if p.prefer_file_path {
            out
        } else {
            img::mime_from_extension(ext)
                .map(|m| {
                    format!(
                        "data:{m};base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(&data)
                    )
                })
                .unwrap_or_else(|| file_path_to_url(&out))
        };
        Attempt::Done(DecryptedImage {
            result: ImageResult::ok(local, Some(img::is_thumbnail_path(dat))),
            fresh: Some(FreshImage { data, thumbnail }),
        })
    }

    /// 每个请求一行结果（重复的只查一次），整批只检查一次文件夹。
    pub fn image_resolve_cache_batch(&self, payloads: &[ImagePayload]) -> ImageBatch {
        let since = Since::now();
        let mut memo: HashMap<String, ImageResult> = HashMap::new();
        let rows: Vec<ImageResult> = payloads
            .iter()
            .map(|p| {
                let key = format!(
                    "{}|{}|{}|{}|{}",
                    p.session_id.as_deref().unwrap_or("").trim().to_lowercase(),
                    p.image_md5.as_deref().unwrap_or("").trim().to_lowercase(),
                    p.image_dat_name
                        .as_deref()
                        .unwrap_or("")
                        .trim()
                        .to_lowercase(),
                    p.create_time.unwrap_or(0),
                    p.prefer_file_path
                );
                memo.entry(key)
                    .or_insert_with(|| self.image_resolve_cache_since(p, since))
                    .clone()
            })
            .collect();
        ImageBatch { rows }
    }

    /// 一条消息解密后的图片的字节。
    ///
    /// `create_time` 用来区分不同消息库中本地 ID 相同的消息（其中会优先选图片）。
    ///
    /// # Errors
    ///
    /// 消息 ID 无效或消息不是图片时返回用法错误，找不到消息时返回 `not_found`；
    /// 图片没有 md5 和 `.dat` 文件名、找不到或解密失败时返回运行时错误（`failureKind` 写在详情中）；
    /// 账号没有配置好时返回配置错误。
    pub fn image_data_for_message(
        &self,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
    ) -> AppResult<Vec<u8>> {
        let payload = self.image_payload_for_message(session_id, msg_id, create_time)?;
        self.image_data_for_payload(&payload)
    }

    /// 微信为消息图片保存的缩略图（`<md5>_t.dat`，几 KB），在内存中解密；
    /// 没有缩略图时为图片本身（[`Self::image_data_for_message`]）。
    fn image_thumbnail_for_message(
        &self,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
    ) -> AppResult<Vec<u8>> {
        let payload = self.image_payload_for_message(session_id, msg_id, create_time)?;
        match self.thumbnail_bytes(&payload) {
            Some(bytes) => Ok(bytes),
            None => self.image_data_for_payload(&payload),
        }
    }

    /// 消息图片在微信保存的某个版本下的内容，见 [`ImageVariant`]。
    ///
    /// # Errors
    ///
    /// 同 [`Self::image_data_for_message`]；要 [`ImageVariant::Hd`] 而本机没有原图时返回 `not_found`，
    /// 原图还没写完时返回运行时错误。
    pub fn image_variant_for_message(
        &self,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
        variant: ImageVariant,
    ) -> AppResult<Vec<u8>> {
        match variant {
            ImageVariant::Best => self.image_data_for_message(session_id, msg_id, create_time),
            ImageVariant::Thumb => {
                self.image_thumbnail_for_message(session_id, msg_id, create_time)
            }
            ImageVariant::Normal | ImageVariant::Hd => {
                let payload = self.image_payload_for_message(session_id, msg_id, create_time)?;
                let files = self.dat_files(&payload, Since::now()).unwrap_or_default();
                let hd_payload = ImagePayload {
                    force: true,
                    ..payload.clone()
                };
                match (variant, &files.hd, &files.base) {
                    // 高清文件走通常的路径，高清版会留在缓存中；对微信还没写完的高清文件，
                    // 通常的路径会改为显示较差的版本，而这个选项不退而求其次
                    (ImageVariant::Hd, Some(_), _) => {
                        let result = self.image_decrypt(&hd_payload);
                        match result.local_path.as_deref() {
                            Some(path) if !img::is_hd_path(path) => Err(incomplete_error()),
                            _ => image_bytes(result),
                        }
                    }
                    (ImageVariant::Hd, None, _) => Err(AppError::not_found(
                        "the HD original of this image is not on this computer: open the image in WeChat and choose View original",
                    )),
                    // 没有高清文件时，通常的路径给出的就是图片本身（没有它时为缩略图）；
                    // 有高清文件时通常的路径会优先用高清文件，所以在这里解密图片本身
                    (_, None, _) => self.image_data_for_payload(&payload),
                    (_, Some(_), Some(base)) => self.decrypt_dat_bytes(Path::new(base), &payload),
                    (_, Some(_), None) => self.image_data_for_payload(&hd_payload),
                }
            }
        }
    }

    /// 消息图片的哪些版本在本机上（#194），以及微信给出的图片和高清原图的大小（字节；原图没下载时也有它的大小）。
    ///
    /// # Errors
    ///
    /// 消息 ID 无效或消息不是图片时返回用法错误，找不到消息时返回 `not_found`，
    /// 图片没有 md5 和 `.dat` 文件名时返回运行时错误；账号没有配置好时返回配置错误。
    pub fn image_info_for_message(
        &self,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
    ) -> AppResult<crate::contract::ImageInfo> {
        let (payload, content) = self.image_payload_and_content(session_id, msg_id, create_time)?;
        let files = self.dat_files(&payload, Since::now()).unwrap_or_default();
        let attr = |name: &str| -> Option<u64> {
            let re = crate::message::xml::rx(&format!(r#"\s{name}\s*=\s*"(\d+)""#));
            re.captures(&content)?
                .get(1)?
                .as_str()
                .parse()
                .ok()
                .filter(|n| *n > 0)
        };
        let size = |path: &Option<String>| {
            path.as_ref()
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
        };
        Ok(crate::contract::ImageInfo {
            thumb: files.thumb.is_some(),
            normal: files.base.is_some(),
            hd: files.hd.is_some(),
            normal_bytes: attr("length").or_else(|| size(&files.base)),
            hd_bytes: attr("hdlength").or_else(|| size(&files.hd)),
        })
    }

    /// `p` 的图片每个版本的 `.dat` 文件，按 [`Self::select_best_dat`] 的方式挑选：
    /// 文件夹中有的，以及文件夹中缺少、而微信的媒体索引给出位置的版本（#651）。
    fn dat_files(&self, p: &ImagePayload, since: Since) -> Option<DatFiles> {
        let account_dir = self.account_dir_since(since).ok()?;
        let bases = lookup_bases(
            p.image_md5.as_deref(),
            p.image_dat_name.as_deref(),
            p.allow_cache_index != Some(false),
        );
        let mut files = self
            .listed_dat_files(&account_dir, p, &bases, since)
            .unwrap_or_default();
        for base in &bases {
            for linked in self.linked_dats(&account_dir, base, since) {
                let slot = match Rendition::of(&linked) {
                    Rendition::Hd => &mut files.hd,
                    Rendition::Original => &mut files.base,
                    Rendition::Thumbnail => &mut files.thumb,
                };
                // 索引把最新的列在前面
                slot.get_or_insert(linked);
            }
        }
        (files.hd.is_some() || files.base.is_some() || files.thumb.is_some()).then_some(files)
    }

    /// 只在会话的文件夹中查找的 [`Self::dat_files`]。
    fn listed_dat_files(
        &self,
        account_dir: &Path,
        p: &ImagePayload,
        bases: &[String],
        since: Since,
    ) -> Option<DatFiles> {
        for base in bases {
            let candidates: Vec<String> = self
                .collect_dat_candidates(
                    account_dir,
                    base,
                    p.session_id.as_deref(),
                    p.create_time,
                    since,
                )
                .into_iter()
                .filter(|c| c.to_lowercase().ends_with(".dat"))
                .collect();
            if candidates.is_empty() {
                continue;
            }
            let in_img: Vec<String> = candidates
                .iter()
                .filter(|c| img::is_img_scoped_dat_path(c))
                .cloned()
                .collect();
            let of = |list: &[String], f: &dyn Fn(&str) -> bool| {
                self.pick_largest(
                    &list.iter().filter(|c| f(c)).cloned().collect::<Vec<_>>(),
                    since,
                )
            };
            let is_base = |c: &str| img::is_base_dat_path(c, base);
            let is_t = |c: &str| img::is_t_variant_dat(c);
            return Some(DatFiles {
                hd: of(&in_img, &|c| img::is_hd_dat_path(c)),
                base: of(&in_img, &is_base).or_else(|| of(&candidates, &is_base)),
                thumb: of(&in_img, &is_t).or_else(|| of(&candidates, &is_t)),
            });
        }
        None
    }

    /// 在内存中解密一个 `.dat`（HEVC 图片 WXGF 会转换），不动图片缓存。
    fn decrypt_dat_bytes(&self, dat: &Path, p: &ImagePayload) -> AppResult<Vec<u8>> {
        let (xor_key, aes_key) = self.image_keys();
        let xor_key =
            xor_key.ok_or_else(|| AppError::runtime("image decrypt key is not configured"))?;
        let decrypted =
            crate::decrypt::decrypt_file(dat, xor_key, aes_key.as_ref()).map_err(|e| {
                if e.is::<crate::decrypt::CutShort>() {
                    incomplete_error()
                } else {
                    AppError::runtime(format!(
                        "native decrypt failed, check the image keys: {e:#}"
                    ))
                }
            })?;
        let (data, failure) = img::unwrap_wxgf(decrypted.data);
        if let Some(reason) = failure {
            return Err(
                AppError::runtime(unsupported_image_message(p.create_time, reason))
                    .with_details(json!({ "failureKind": UNSUPPORTED_IMAGE })),
            );
        }
        if img::detect_image_extension(&data).is_none() {
            return Err(AppError::runtime("decrypted data is not a valid image"));
        }
        if img::ending::is_cut_short(&data) {
            return Err(incomplete_error());
        }
        Ok(data)
    }

    /// `p` 的图片文件旁边的 `_t.dat`，解密后是页面能显示的图片时才返回。
    fn thumbnail_bytes(&self, p: &ImagePayload) -> Option<Vec<u8>> {
        let since = Since::now();
        let account_dir = self.account_dir_since(since).ok()?;
        let dat = self.resolve_dat_path(
            &account_dir,
            p,
            DatSearch {
                allow_thumbnail: true,
                skip_resolved_cache: false,
                allow_dat_name_fallback: p.allow_cache_index != Some(false),
            },
            since,
        )?;
        let path = Path::new(&dat);
        let thumb = if img::is_t_variant_dat(&dat) {
            path.to_path_buf()
        } else {
            let base = img::normalize_dat_base(&path.file_name()?.to_string_lossy());
            path.with_file_name(format!("{base}_t.dat"))
        };
        self.file_meta(&thumb, since)?;
        let (xor_key, aes_key) = self.image_keys();
        let decrypted = crate::decrypt::decrypt_file(&thumb, xor_key?, aes_key.as_ref()).ok()?;
        let (data, _) = img::unwrap_wxgf(decrypted.data);
        // 转换不了的 WXGF 或被截断的图片交给完整的流程处理，由它说明原因
        let ext = crate::decrypt::detect_image_extension(&data);
        (ext != ".bin" && ext != ".wxgf" && !img::ending::is_cut_short(&data)).then_some(data)
    }

    /// [`Self::image_decrypt`] 找到一条消息的图片所需的请求。
    fn image_payload_for_message(
        &self,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
    ) -> AppResult<ImagePayload> {
        Ok(self
            .image_payload_and_content(session_id, msg_id, create_time)?
            .0)
    }

    /// [`Self::image_payload_for_message`]，以及微信保存的消息原文（它的 XML）。
    fn image_payload_and_content(
        &self,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
    ) -> AppResult<(ImagePayload, String)> {
        let local_id = crate::api::js_parse_int(msg_id)
            .ok_or_else(|| AppError::usage("invalid message id"))?;
        let wcdb = self.open_wcdb()?;
        let row = chat::message_by_local_id(&wcdb, session_id, local_id, create_time, Some(3))?
            .ok_or_else(|| AppError::not_found("message not found"))?;
        // 查找时只在这个 ID 对应多条消息时才优先选图片：找到的那条可能是任何类型
        if row.local_type & 0xFFFF_FFFF != 3 {
            return Err(AppError::usage("the message is not an image"));
        }
        let my = self.my_wxid_cleaned();
        let msg =
            crate::chat_msg::map_msg_row(row, &weflow_native::msg::table_name_for(session_id), &my);
        let raw_md5 = msg
            .image_md5
            .clone()
            .filter(|m| !m.is_empty())
            .or_else(|| crate::message::media::extract_image_md5(&msg.raw_content));
        let dat_name = msg.image_dat_name.clone().filter(|m| !m.is_empty());
        if raw_md5.is_none() && dat_name.is_none() {
            return Err(AppError::runtime(
                "image has no md5 / datName, cannot locate the original file",
            ));
        }
        let payload = ImagePayload {
            session_id: Some(session_id.to_string()),
            image_md5: raw_md5,
            image_dat_name: dat_name,
            create_time: Some(msg.create_time).filter(|t| *t != 0),
            prefer_file_path: true,
            ..Default::default()
        };
        Ok((payload, msg.raw_content))
    }

    fn image_data_for_payload(&self, payload: &ImagePayload) -> AppResult<Vec<u8>> {
        image_bytes(self.image_decrypt(payload))
    }

    /// 忘掉关于解密图片缓存的已知信息，在缓存文件被删除时调用。
    pub(super) fn image_forget_cached(&self) {
        let mut state = self.image_state.lock_or_panic();
        state.resolved.clear();
        state.cache_dirs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsupported_wxgf_image_says_when_it_was_sent_and_whether_it_is_damaged() {
        use crate::hevc::Unsupported;
        let date = crate::message::time::format_timestamp(1_700_000_000);
        assert_eq!(
            unsupported_image_message(Some(1_700_000_000), Unsupported::BitDepth(10)),
            format!("WXGF image sent {date} cannot be converted because its format is not supported yet: 10-bit samples")
        );
        assert_eq!(
            unsupported_image_message(None, Unsupported::NoPicture),
            "WXGF image cannot be converted because the data is damaged: no picture"
        );
    }

    fn listing_dir(tag: &str) -> weflow_native::fixture::TestDir {
        let dir = weflow_native::fixture::TestDir::new(&format!("weflow-listing-{tag}"));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        dir
    }

    /// 把文件夹的修改时间往回调一小时，就像不再有人写入的月份文件夹。
    fn age(dir: &Path) {
        let past = SystemTime::now() - Duration::from_secs(3600);
        open_dir_to_set_times(dir).set_modified(past).unwrap();
    }

    /// 为设置时间而打开的文件夹。
    ///
    /// Windows 只能以备份语义打开文件夹，设置时间需要的是写属性的权限，而不是写入权限。
    fn open_dir_to_set_times(dir: &Path) -> std::fs::File {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            std::fs::File::options()
                .access_mode(FILE_WRITE_ATTRIBUTES)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(dir)
                .unwrap()
        }
        #[cfg(not(windows))]
        {
            std::fs::File::open(dir).unwrap()
        }
    }

    #[test]
    fn a_listing_finds_the_names_with_a_prefix_in_listing_order() {
        let md5 = "0123456789abcdef0123456789abcdef";
        let dir = listing_dir("prefix");
        for name in [
            format!("{md5}_h.dat"),
            format!("{}.dat", md5.to_uppercase()),
            format!("{md5}_t.dat"),
            "ffffffffffffffffffffffffffffffff.dat".to_string(),
            format!("x{md5}.dat"),
        ] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let listing = DirListing::read(&dir).unwrap();
        let found = listing.files_starting_with(md5);
        let in_listing_order: Vec<&str> = listing
            .files
            .iter()
            .map(String::as_str)
            .filter(|n| found.contains(n))
            .collect();
        assert_eq!(found, in_listing_order);
        let mut sorted = found.clone();
        sorted.sort_unstable();
        let mut expected = vec![
            format!("{md5}_h.dat"),
            format!("{}.dat", md5.to_uppercase()),
            format!("{md5}_t.dat"),
        ];
        expected.sort_unstable();
        assert_eq!(sorted, expected);
        assert_eq!(listing.dirs, vec!["sub".to_string()]);
    }

    #[test]
    fn a_listing_tells_a_file_it_lists_from_one_it_does_not() {
        let dir = listing_dir("file");
        std::fs::write(dir.join("a.dat"), b"abc").unwrap();
        let listing = DirListing::read(&dir).unwrap();
        let listed = listing.file("a.dat").expect("listed");
        if cfg!(windows) {
            assert_eq!(listed.map(|m| m.len), Some(3), "Windows lists the size");
        } else {
            assert!(listed.is_none(), "asked of the disk elsewhere");
        }
        assert!(listing.file("b.dat").is_none());
        assert!(listing.file("sub").is_none(), "a folder is not a file");
        assert!(
            listing
                .file("A.DAT")
                .expect("listed in another case")
                .is_none(),
            "another case may be another file: asked of the disk"
        );
    }

    #[test]
    fn a_listing_is_reused_until_the_folder_changes() {
        let dir = listing_dir("current");
        std::fs::write(dir.join("a.dat"), b"").unwrap();
        // 刚改过：在同一个时间戳刻度内新增的文件看不出来，所以要重新读
        let fresh = DirListing::read(&dir).unwrap();
        assert!(!fresh.current(&dir));
        age(&dir);
        let settled = DirListing::read(&dir).unwrap();
        assert!(settled.current(&dir));
        std::fs::write(dir.join("b.dat"), b"").unwrap();
        assert!(
            !settled.current(&dir),
            "a new file changes the folder's time"
        );
    }

    #[test]
    fn lookup_bases_expand_dat_names() {
        let md5 = "0123456789abcdef0123456789abcdef";
        assert_eq!(lookup_bases(Some(md5), None, true), vec![md5.to_string()]);
        assert_eq!(
            lookup_bases(None, Some(&format!("{md5}_h.dat")), true),
            vec![md5.to_string()]
        );
        assert!(lookup_bases(None, Some("notmd5.dat"), true).is_empty());
        assert_eq!(
            lookup_bases(
                Some("zzz"),
                Some(&format!("{}_t", md5.to_uppercase())),
                false
            ),
            vec![md5.to_string()]
        );
    }

    #[test]
    fn cache_keys_follow_the_desktop_order() {
        let p = ImagePayload {
            image_md5: Some("AbC_h".into()),
            image_dat_name: Some("ABC_t".into()),
            ..Default::default()
        };
        assert_eq!(
            cache_keys(&p),
            vec!["AbC_h", "abc_h", "abc", "ABC_t", "abc_t"]
        );
        assert!(cache_keys(&ImagePayload::default()).is_empty());
    }

    #[test]
    fn result_json_omits_unset_fields() {
        let ok = ImageResult::ok("/x.jpg".into(), Some(false));
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            json!({ "localPath": "/x.jpg", "isThumb": false })
        );
        assert_eq!(
            serde_json::to_value(ImageResult::fail("nope", "not_found")).unwrap(),
            json!({ "error": "nope", "failureKind": "not_found" })
        );
        let err = ImageResult::fail("nope", "not_found")
            .into_answer()
            .unwrap_err();
        assert_eq!(
            (err.message.as_str(), err.details.clone()),
            ("nope", Some(json!({ "failureKind": "not_found" })))
        );
        assert_eq!(ok.clone().into_answer().unwrap().local_path, ok.local_path);
    }
}
