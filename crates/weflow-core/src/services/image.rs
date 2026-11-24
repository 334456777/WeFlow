//! Message images: locates a message image's `.dat` file in the account's
//! `msg/attach/<md5(session)>/<yyyy-mm>/Img` folder, decrypts it (V1 / V2 / legacy XOR), unwraps
//! WXGF with the built-in decoder and keeps the result in `<cache>/Images/<session>/<yyyy-mm>/`.
//!
//! The desktop app's events for a page that shows images as they are decrypted, and its
//! background "better quality available" check, have no counterpart in a CLI, so they are
//! left out.
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

#[derive(Default)]
pub struct ImageState {
    resolved: HashMap<String, String>,
    /// Listings of the `Img` folders searched for `.dat` files (see [`ServiceHub::dir_listing`]).
    dir_listings: HashMap<PathBuf, Listed>,
    /// Names in the folders of the decrypted-image cache (see [`ServiceHub::possibly_cached`]).
    cache_dirs: HashMap<PathBuf, CacheDirNames>,
    /// WeChat's media index of images (see [`ServiceHub::linked_image_index`]).
    linked_images: Option<LinkedImages>,
    /// The account folder, and when it was last found on disk (see [`ServiceHub::account_dir_since`]).
    account_dir: Option<(PathBuf, Instant)>,
    /// `Images` in the cache folder (see [`ServiceHub::image_cache_root`]).
    cache_root: Option<PathBuf>,
}

/// What a lookup takes on trust: what was checked against the disk at or after this instant (a folder's listing,
/// WeChat's media index, the account folder, a folder of the image cache) is used without checking it again.
///
/// Each check is a system call, and on Windows a system call that opens a file costs tens of microseconds: an
/// export with images made some fifty per image (issue #1099). A lookup ([`ServiceHub::image_decrypt`]) checks each
/// at most once, from the instant it starts; an export checks each once for all its images, from the instant it
/// starts, so the folders are taken as they were then.
#[derive(Clone, Copy, Debug)]
pub(super) struct Since(Instant);

impl Since {
    /// What is checked from now on.
    pub(super) fn now() -> Self {
        Self(Instant::now())
    }

    /// Whether a check made at `checked` can be taken on trust.
    fn covers(self, checked: Instant) -> bool {
        checked >= self.0
    }
}

/// A listing of a folder (`None` when there was no such folder) and when it was last found current.
struct Listed {
    listing: Option<Arc<DirListing>>,
    checked: Instant,
}

/// The image rows of WeChat's media index (`hardlink.db`), read whole, and the state of its database files when
/// they were read: a change to the database or to its WAL reads them again.
struct LinkedImages {
    account_dir: PathBuf,
    stamp: [Option<(Option<SystemTime>, u64)>; 2],
    by_md5: Arc<HashMap<String, Vec<PathBuf>>>,
    /// When `stamp` was last found current.
    checked: Instant,
}

/// The size and modification time of a file.
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

/// `failureKind` of a WXGF image the built-in decoder cannot convert.
const UNSUPPORTED_IMAGE: &str = "unsupported_image";
/// `failureKind` of a payload that names no image, with neither an md5 nor a `.dat` name: a usage error as a
/// command's answer (#1076).
const NO_IDENTIFIER: &str = "invalid_arguments";

/// Directory listings (and cache folders' names) kept at most; past that the cache starts over.
const MAX_DIR_LISTINGS: usize = 1024;
/// A listing taken this soon after the folder's last change is not reused: a file added in the same tick of a coarse
/// timestamp would leave the modification time as it was.
const LISTING_SETTLE: Duration = Duration::from_secs(2);

/// One folder's entries. An image is searched for in its month's folder, which can hold thousands of files, and an
/// export with media searches for every image: reading the folder each time made that quadratic (issue #49).
pub(super) struct DirListing {
    modified: Option<SystemTime>,
    listed_at: SystemTime,
    /// File names in the order the system listed them.
    files: Vec<String>,
    /// The size and time of each of `files` where listing the folder gave them without a call per file (Windows).
    meta: Vec<Option<FileMeta>>,
    /// (trimmed lower-case name, index into `files`), sorted, to find the names starting with a prefix.
    keys: Vec<(String, usize)>,
    /// Subfolder names, in listing order.
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
                // Windows lists each file with its size and times; elsewhere they would cost a call per file, more
                // than the few files a lookup asks about
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

    /// What the listing says of the file `name`: `None` when it lists no file of that name in any case, else its size
    /// and time when it lists that very name with them (a name listed in another case only may be another file, on a
    /// file system that tells cases apart).
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

    /// Whether `dir` still has these entries: not modified since it was listed, and listed well after its change.
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

    /// Files whose trimmed lower-case name starts with `prefix`, in listing order.
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

/// The lower-case names in a folder of the decrypted-image cache. Looking an image up tries up to 125 file names in
/// three folders, and an export with media looks every image up twice while it fills those folders: checking each
/// name took one metadata call apiece (issue #16). The names are read once, the files this process writes are added,
/// and the folder is read again when its modification time shows a change made otherwise. A name that is no longer
/// on disk only costs the check of that one path; a missing name costs a decrypt that was not needed.
struct CacheDirNames {
    /// The folder's modification time; `None` when there was no such folder.
    modified: Option<SystemTime>,
    names: HashSet<String>,
    /// When `modified` was last found current.
    checked: Instant,
}

/// What [`ServiceHub::index_cache_dir`] found.
#[derive(Clone, Copy, PartialEq)]
enum CacheDir {
    /// No such folder: none of its paths exist.
    Missing,
    /// Its names are in `ImageState::cache_dirs`.
    Indexed,
    /// It could not be listed: any of its paths may exist.
    Unlisted,
}

fn modified_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn lower_file_name(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().to_lowercase())
}

/// Which of the files WeChat keeps of an image (#194).
///
/// `Thumb` is the thumbnail a chat shows (`<md5>_t.dat`); `Normal`, the image opened (`<md5>.dat`); `Hd`, the
/// original WeChat downloads on View original (`<md5>_h.dat`); `Best`, the best of them on this computer. `Normal`
/// falls back to the best there is, `Hd` to nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageVariant {
    /// The best file on this computer.
    Best,
    /// The thumbnail, `<md5>_t.dat`.
    Thumb,
    /// The image opened, `<md5>.dat`, else the best there is.
    Normal,
    /// The original, `<md5>_h.dat`, else nothing.
    Hd,
}

/// The `.dat` file of each rendition of an image, where there is one.
#[derive(Default, Debug)]
struct DatFiles {
    hd: Option<String>,
    base: Option<String>,
    thumb: Option<String>,
}

/// What a `.dat` file of an image holds, best first.
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

    /// The rendition a file of the decrypted-image cache was decrypted from, by the suffix
    /// [`img::cache_output_path`] gives it: `_hd`, a thumbnail's `_t`, `.t` or `_thumb`, else the original.
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

/// Which image message's image to decrypt, and how to answer (`image decrypt`, `resolve-cache`, `resolve-batch`).
#[derive(Default, Clone, Debug)]
pub struct ImagePayload {
    /// The conversation's username.
    pub session_id: Option<String>,
    /// The image's MD5, as a message's `imageMd5`.
    pub image_md5: Option<String>,
    /// The name of its `.dat` file, as a message's `imageDatName`.
    pub image_dat_name: Option<String>,
    /// When the message was sent, which narrows the search for the file.
    pub create_time: Option<i64>,
    /// Answer the decrypted file's path rather than a data URL of it.
    pub prefer_file_path: bool,
    /// `allowCacheIndex !== false`: whether a file named after the normalized `.dat` name or the MD5 may be taken
    /// when the exact one is not found.
    pub allow_cache_index: Option<bool>,
    /// Decrypt the original again rather than answer a thumbnail decrypted before.
    pub force: bool,
}

impl ImagePayload {
    /// The payload of a JSON object with the WebUI's `camelCase` keys; a key that is missing or empty is unset.
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

/// Where a message's image is decrypted (`image decrypt`, `resolve-cache`); a failure says why instead: the
/// command's error, or the row of `image resolve-batch`.
#[derive(Default, Clone, Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageResult {
    /// The decrypted image: its path, or a data URL of it without `preferFilePath`; left out on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    /// What went wrong; left out on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Why it failed: `not_found`, `decrypt_failed`, `unsupported_image`, or `invalid_arguments` for a payload with
    /// neither an md5 nor a `.dat` name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<&'static str>,
    /// The file is only the thumbnail: the original is not on disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_thumb: Option<bool>,
}

/// A message image as [`ServiceHub::image_decrypt_data`] finds it.
pub(super) struct DecryptedImage {
    /// The answer of [`ServiceHub::image_decrypt`]; empty (neither a path nor an error) for an image decrypted just
    /// now that was not cached ([`CacheWrites::OnlyWxgf`]).
    pub(super) result: ImageResult,
    /// The image, when it was decrypted just now (not when the cache already held it).
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

/// An image decrypted just now ([`DecryptedImage::fresh`]).
pub(super) struct FreshImage {
    /// Its bytes.
    pub(super) data: Vec<u8>,
    /// Whether it is a thumbnail, by the name of its file in the cache ([`img::is_thumbnail_path`]).
    pub(super) thumbnail: bool,
}

/// Which images decrypted just now [`ServiceHub::image_decrypt_data`] writes into the image cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CacheWrites {
    /// Every one: the answer is its file in the cache.
    All,
    /// Those converted from WXGF, slow to convert again. The others are as quick to decrypt again as to read back
    /// from the cache, and an export writes their bytes where they go: writing them into the cache too would make
    /// two files of each, and on Windows making the files is most of what an image costs an export.
    OnlyWxgf,
}

/// What `image resolve-batch` answers: one result per payload, in their order.
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
pub struct ImageBatch {
    /// One result for each payload, in their order: `localPath` when it was found, `error` when not.
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

    /// The failure of a payload with neither an md5 nor a `.dat` name.
    fn no_identifier() -> Self {
        Self::fail("missing image identifier", NO_IDENTIFIER)
    }

    /// Whether the image was found and decrypted.
    pub fn is_ok(&self) -> bool {
        self.local_path.is_some()
    }

    /// The result as a command's answer.
    ///
    /// # Errors
    /// A usage error for a payload that names no image; any other failure as a runtime error, with `failureKind`
    /// (and `isThumb` when known) as its details.
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

/// What a WXGF image the built-in decoder cannot convert says: the time of its message, when known, so that it can be
/// found in WeChat, and whether the data is damaged or in a format not supported yet.
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

/// The keys a message image may be cached under: its md5 and `.dat` name as given, lower-cased, and without the variant suffix.
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

/// How far `resolve_dat_path` may relax its search for a payload's `.dat` file.
struct DatSearch {
    /// Accept a thumbnail (`_t`) when no HD file exists.
    allow_thumbnail: bool,
    /// Ignore previously resolved paths and look on disk again.
    skip_resolved_cache: bool,
    /// Also look for a base derived from the normalized `.dat` name (or the md5), see `lookup_bases`.
    allow_dat_name_fallback: bool,
}

/// The md5s to look an image up by in WeChat's media index, then the `.dat` names to scan for.
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
    /// The account directory without needing the database key.
    pub(super) fn account_dir_only(&self) -> AppResult<PathBuf> {
        let profile = self.profile()?;
        let db_path = profile
            .db_path
            .clone()
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| AppError::config("no account or database path configured"))?;
        let wxid = profile
            .wxid
            .clone()
            .filter(|w| !w.trim().is_empty())
            .ok_or_else(|| AppError::config("no account or database path configured"))?;
        let dir = crate::config::resolve_account_dir(&db_path, &wxid);
        if dir.exists() {
            Ok(dir)
        } else {
            Err(AppError::config("account directory not found"))
        }
    }

    /// [`Self::account_dir_only`], found on disk at most once for `since`.
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

    /// The folder of the decrypted images, `Images` in the cache folder. It is made when a decrypted image is first
    /// written into it.
    pub fn image_cache_root(&self) -> PathBuf {
        if let Some(root) = &self.image_state.lock_or_panic().cache_root {
            return root.clone();
        }
        let root = self.cache_base().join("Images");
        self.image_state.lock_or_panic().cache_root = Some(root.clone());
        root
    }

    /// Makes `dir`, a folder of the image cache, and the cache folder above it as [`Self::cache_base`] makes it.
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

    /// Whether a cached image can be used: a recognised extension, present, not empty and not a zeroed-out JPEG.
    fn usable_image_cache_file(&self, path: &str) -> bool {
        // an empty file is never an image (an older cache may hold one left half written)
        if !img::is_image_file(path) || std::fs::metadata(path).map_or(true, |m| m.len() == 0) {
            return false;
        }
        let ext = Path::new(path)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if ext == "jpg" || ext == "jpeg" {
            if let Ok(data) = std::fs::read(path) {
                if img::is_likely_corrupted_jpeg(&data) {
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

    /// The `.dat` files of a message in `msg/attach/<md5(session)>/<yyyy-mm>/Img`, at most 240 directories;
    /// `<md5(session)>` is the first of [`img::session_dirs_for_storage`] with that month's folder.
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
        // the month's folder in the first of the conversation's folders that has one
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
            // every candidate name, lower-cased, starts with `base_md5` (`normalize_dat_base` only drops suffixes),
            // so only those names are checked; the listing was found current for `since`, so they are on disk
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

    /// The entries of `dir` (`None` when there is no such folder), read again only when it changed since it was last
    /// read, and checked for that at most once for `since`.
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
        // read without holding the lock: the export's media threads search side by side
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

    /// The size and time of the file `path`, `None` when it is not on disk: from the listing of its folder when that
    /// was found current for `since` (a file it does not list is not there), else from the disk.
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
            // no such folder, or a name it does not list: not on disk
            if let Some(meta) = listing?.file(&name.to_string_lossy())? {
                return Some(meta);
            }
        }
        std::fs::metadata(path)
            .ok()
            .filter(std::fs::Metadata::is_file)
            .map(|m| FileMeta::of(&m))
    }

    /// The cache folder of the image decrypted from `dat` ([`img::month_folder`]).
    fn dat_month(&self, dat: &str, since: Since) -> String {
        img::month_folder(
            self.file_meta(Path::new(dat), since)
                .and_then(|m| m.modified),
        )
    }

    /// The largest of `paths`: biggest file, then newest, then name.
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

    /// The best `.dat` of the image `base_md5`: what the folders of its conversation hold
    /// ([`Self::select_listed_dat`]), unless WeChat's media index names a better rendition on disk, or the folders
    /// hold none (#651). The index is only an index: many images have no row in it, so the folders are always
    /// searched, and what they find is kept when the index has nothing better.
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

    /// The best rendition of `base_md5` that WeChat's media index lists and that is on disk, the newest of equals;
    /// only an HD one unless `allow_thumbnail`.
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
            // the first of equals: the index lists the newest first
            .min_by_key(|p| Rendition::of(p))
    }

    /// The `.dat` files of `base_md5` that WeChat's media index (`hardlink.db`) lists and that are on disk, newest
    /// first.
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

    /// WeChat's media index of images by md5, read again only when its database changed since it was last read,
    /// which is checked at most once for `since`. Without the database key it cannot be read, and is empty.
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
        // read without holding the lock, as the folders are
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

    /// The best `.dat` file of a listing: HD variant in an Img folder, else `<md5>.dat`, else a `_t` thumbnail.
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
                // a thumbnail is what was on disk then: the full image may have been downloaded since (#858), so the
                // folders are looked at again first
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
        // nothing in this message's folders: the thumbnail found before for the same image (through another message,
        // whose folder holds it) is still the image
        thumbnail_found_before
    }

    /// The image decrypted from `dat_path` (whose cache folder is `month`) that the cache holds, when it is usable.
    fn find_cached_output(
        &self,
        dat_path: &str,
        month: &str,
        session_id: Option<&str>,
        prefer_hd: bool,
        since: Since,
    ) -> Option<String> {
        let root = self.image_cache_root();
        self.possibly_cached(
            img::cache_output_candidates(&root, dat_path, month, session_id, prefer_hd),
            since,
        )
        .into_iter()
        .map(|c| c.to_string_lossy().to_string())
        // a file that is not there is not usable either
        .find(|c| self.usable_image_cache_file(c))
    }

    /// The `candidates` whose folder lists their name, in order: those that may exist. Costs one metadata call per
    /// folder, at most once for `since`, instead of one per path; the paths returned still have to be checked.
    fn possibly_cached(&self, candidates: Vec<PathBuf>, since: Since) -> Vec<PathBuf> {
        // the few folders of the many candidates, told apart by their bytes: a path's hash walks its components
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
        // each folder's names, looked up once; an index dropped since (over `MAX_DIR_LISTINGS`) proves nothing
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

    /// Brings the names of the cache folder `dir` up to date, checking it at most once for `since`.
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
                // a file this process renamed into the folder in between is recorded under the lock
                // (`write_cache_file`)
                if index.modified.is_some()
                    && (index.modified == Some(modified) || index.modified == modified_time(dir))
                {
                    index.checked = checked;
                    return CacheDir::Indexed;
                }
            }
        }
        // read without holding the lock. The time was taken first: a file written meanwhile changes it again, so the
        // next lookup reads the folder anew
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
        // added to the names already known, so a file another thread noted in between is not dropped
        index.names.extend(names);
        index.modified = Some(modified);
        index.checked = checked;
        CacheDir::Indexed
    }

    /// Writes a decrypted image into the cache like [`write_atomically`], and adds it to its folder's names. The
    /// temporary file is written in `<cache>/Images/.partial`, and the rename is recorded under the lock: otherwise
    /// the other media threads would see the folder change while it is written and read all its names again. The
    /// folders are made when the write finds one missing.
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
                // a folder known to be missing may have been made and filled by another process since
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
            // a file that is not there stays not there
            let _ = std::fs::remove_file(&c);
        }
    }

    /// Swaps `cached`, a usable cached thumbnail, for the HD rendition when one exists.
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
            // a usable file of the cache, so one on disk
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

    /// Answers from the cache only; never decrypts a missing file.
    pub fn image_resolve_cache(&self, p: &ImagePayload) -> ImageResult {
        self.image_resolve_cache_since(p, Since::now())
    }

    /// [`Self::image_resolve_cache`] that takes on trust what was checked for `since`.
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
            // a file that is not there is not usable either
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

    /// Decrypts a message image into the cache, or finds it there (`image decrypt`).
    pub fn image_decrypt(&self, p: &ImagePayload) -> ImageResult {
        self.image_decrypt_since(p, Since::now())
    }

    /// [`Self::image_decrypt`] that takes on trust what was checked for `since`.
    pub(super) fn image_decrypt_since(&self, p: &ImagePayload, since: Since) -> ImageResult {
        self.image_decrypt_data(p, since, CacheWrites::All).result
    }

    /// [`Self::image_decrypt_since`], with the image when it was decrypted just now: an export writes its bytes
    /// rather than read the cache file back. `cache` says which images decrypted just now go into the cache.
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
                // a file that is not there is not usable either
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

        if !Path::new(&dat)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase().contains("dat"))
            .unwrap_or(false)
        {
            self.cache_resolved_paths(
                cache_key,
                p.image_md5.as_deref(),
                p.image_dat_name.as_deref(),
                &dat,
            );
            return ImageResult::ok(
                local_path_for_payload(&dat, p.prefer_file_path),
                Some(img::is_thumbnail_path(&dat)),
            )
            .into();
        }

        let prefer_hd = p.force && Rendition::of(&dat) == Rendition::Hd;
        let month = self.dat_month(&dat, since);
        if let Some(existing) =
            self.find_cached_output(&dat, &month, p.session_id.as_deref(), prefer_hd, since)
        {
            // a cached output decrypted from a lesser rendition than `dat` (a thumbnail cached before WeChat
            // downloaded the image, #858) is replaced by decrypting `dat`
            if Rendition::of_cached(&existing) <= Rendition::of(&dat) {
                self.cache_resolved_paths(
                    cache_key,
                    p.image_md5.as_deref(),
                    p.image_dat_name.as_deref(),
                    &existing,
                );
                return ImageResult::ok(
                    local_path_for_payload(&existing, p.prefer_file_path),
                    Some(img::is_thumbnail_path(&existing)),
                )
                .into();
            }
        }

        let (xor_key, aes_key) = self.image_keys();
        let Some(xor_key) = xor_key else {
            return ImageResult::fail("image decrypt key is not configured", "not_found").into();
        };
        let decrypted =
            match crate::decrypt::decrypt_file(Path::new(&dat), xor_key, aes_key.as_ref()) {
                Ok(r) => r,
                Err(e) => {
                    return ImageResult::fail(
                        format!("native decrypt failed, check the image keys: {}", e.message),
                        "not_found",
                    )
                    .into()
                }
            };
        let wxgf = img::is_wxgf(&decrypted.data);
        let (data, wxgf_failure) = img::unwrap_wxgf(decrypted.data);
        if let Some(reason) = wxgf_failure {
            return ImageResult {
                error: Some(unsupported_image_message(p.create_time, reason)),
                failure_kind: Some(UNSUPPORTED_IMAGE),
                is_thumb: Some(img::is_thumbnail_path(&dat)),
                ..Default::default()
            }
            .into();
        }
        let Some(ext) = img::detect_image_extension(&data) else {
            return ImageResult {
                error: Some("decrypted data is not a valid image".into()),
                failure_kind: Some("decrypt_failed"),
                is_thumb: Some(img::is_thumbnail_path(&dat)),
                ..Default::default()
            }
            .into();
        };
        let output = img::cache_output_path(
            &self.image_cache_root(),
            &dat,
            &month,
            ext,
            p.session_id.as_deref(),
        );
        let out = output.to_string_lossy().to_string();
        let thumbnail = img::is_thumbnail_path(&out);
        if !wxgf && cache == CacheWrites::OnlyWxgf {
            return DecryptedImage {
                result: ImageResult::default(),
                fresh: Some(FreshImage { data, thumbnail }),
            };
        }
        // renamed into place: an export's other media threads may read this cache file as soon as it exists
        if let Err(e) = self.write_cache_file(&output, &data) {
            return ImageResult::fail(e.to_string(), "not_found").into();
        }
        self.remove_duplicate_cache_candidates(&dat, &month, p.session_id.as_deref(), &out, since);
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
        DecryptedImage {
            result: ImageResult::ok(local, Some(img::is_thumbnail_path(&dat))),
            fresh: Some(FreshImage { data, thumbnail }),
        }
    }

    /// One result row per payload (duplicates are resolved once), the folders checked once for the whole batch.
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

    /// The decrypted image of a message as bytes.
    /// The decrypted image of a message; `create_time` tells apart messages of different message databases that
    /// share the local id (an image is preferred among them anyway).
    pub fn image_data_for_message(
        &self,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
    ) -> AppResult<Vec<u8>> {
        let payload = self.image_payload_for_message(session_id, msg_id, create_time)?;
        self.image_data_for_payload(&payload)
    }

    /// The thumbnail WeChat keeps of a message's image (`<md5>_t.dat`, a few kilobytes), decrypted in memory; the
    /// image itself ([`Self::image_data_for_message`]) when there is none.
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

    /// The image of a message in one of the renditions WeChat keeps: [`ImageVariant`].
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
                let files = self.dat_files(&payload).unwrap_or_default();
                let hd_payload = ImagePayload {
                    force: true,
                    ..payload.clone()
                };
                match (variant, &files.hd, &files.base) {
                    // the HD file through the usual path, which keeps the HD rendition in the cache
                    (ImageVariant::Hd, Some(_), _) => self.image_data_for_payload(&hd_payload),
                    (ImageVariant::Hd, None, _) => Err(AppError::not_found(
                        "the HD original of this image is not on this computer: open the image in WeChat and choose View original",
                    )),
                    // with no HD file the usual path already gives the image itself (or the thumbnail without it);
                    // beside an HD file, which the usual path prefers, the image itself is decrypted here
                    (_, None, _) => self.image_data_for_payload(&payload),
                    (_, Some(_), Some(base)) => self.decrypt_dat_bytes(Path::new(base), &payload),
                    (_, Some(_), None) => self.image_data_for_payload(&hd_payload),
                }
            }
        }
    }

    /// Which renditions of a message's image are on this computer (#194), and the sizes WeChat gives for the image
    /// and its HD original (bytes; the HD one is there even when it has not been downloaded).
    pub fn image_info_for_message(
        &self,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
    ) -> AppResult<crate::contract::ImageInfo> {
        let (payload, content) = self.image_payload_and_content(session_id, msg_id, create_time)?;
        let files = self.dat_files(&payload).unwrap_or_default();
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

    /// The `.dat` files of each rendition of `p`'s image, chosen as [`Self::select_best_dat`] chooses among them:
    /// what the folders hold, and a rendition they lack where WeChat's media index puts one (#651).
    fn dat_files(&self, p: &ImagePayload) -> Option<DatFiles> {
        let since = Since::now();
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
                // the index lists the newest first
                slot.get_or_insert(linked);
            }
        }
        (files.hd.is_some() || files.base.is_some() || files.thumb.is_some()).then_some(files)
    }

    /// [`Self::dat_files`] in the folders of the conversation only.
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

    /// One `.dat` decrypted in memory, an HEVC image (WXGF) converted, without touching the image cache.
    fn decrypt_dat_bytes(&self, dat: &Path, p: &ImagePayload) -> AppResult<Vec<u8>> {
        let (xor_key, aes_key) = self.image_keys();
        let xor_key =
            xor_key.ok_or_else(|| AppError::runtime("image decrypt key is not configured"))?;
        let decrypted =
            crate::decrypt::decrypt_file(dat, xor_key, aes_key.as_ref()).map_err(|e| {
                AppError::runtime(format!(
                    "native decrypt failed, check the image keys: {}",
                    e.message
                ))
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
        Ok(data)
    }

    /// The `_t.dat` beside the image file of `p`, decrypted, when it is an image a page can show.
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
        // a WXGF that could not be converted is left to the full path, which says why
        let ext = crate::decrypt::detect_image_extension(&data);
        (ext != ".bin" && ext != ".wxgf").then_some(data)
    }

    /// What [`Self::image_decrypt`] needs to find the image of a message.
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

    /// [`Self::image_payload_for_message`], and the message as WeChat keeps it (its XML).
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
        // the lookup prefers an image only among several messages of the id: the one it found may be of any kind
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
        let result = self.image_decrypt(payload);
        let path = result.local_path.ok_or_else(|| {
            let err = AppError::runtime(
                result
                    .error
                    .unwrap_or_else(|| "image decrypt failed".into()),
            );
            // what went wrong, as `chat image` says it: `not_found` is a 404 of the command API
            match result.failure_kind {
                Some(kind) => err.with_details(json!({ "failureKind": kind })),
                None => err,
            }
        })?;
        std::fs::read(&path).map_err(|e| AppError::runtime(format!("failed to read {path}: {e}")))
    }

    /// Forgets what is known about the decrypted-image cache, for when its files are removed.
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

    /// Sets the folder's modification time an hour back, as for a month folder nothing writes to any more.
    fn age(dir: &Path) {
        let past = SystemTime::now() - Duration::from_secs(3600);
        open_dir_to_set_times(dir).set_modified(past).unwrap();
    }

    /// A folder opened so that its times can be set. Windows opens a folder only with backup semantics, and setting
    /// times needs the write-attributes right rather than write access.
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
        // just changed: a file added in the same timestamp tick would not show, so it is read again
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
