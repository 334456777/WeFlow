//! Port of `imageDecryptService.ts`: locates a message image's `.dat` file in the account's
//! `msg/attach/<md5(session)>/<yyyy-mm>/Img` folder, decrypts it (V1 / V2 / legacy XOR), unwraps
//! WXGF through ffmpeg and keeps the result in `<cache>/Images/<session>/<yyyy-mm>/`.
//!
//! The desktop app's render-process events (`image:cacheResolved`, `image:decryptProgress`,
//! `image:updateAvailable`) and the background "better quality available" check have no
//! counterpart in a CLI; like the desktop app's headless worker mode they are disabled, so
//! `hasUpdate` is always `false`.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use base64::Engine;
use serde_json::{json, Map, Value};

use super::*;
use crate::image::{self as img};

#[derive(Default)]
pub struct ImageState {
    resolved: HashMap<String, String>,
    /// Listings of the `Img` folders searched for `.dat` files (see [`ServiceHub::dir_listing`]).
    dir_listings: HashMap<PathBuf, Arc<DirListing>>,
    /// Names in the folders of the decrypted-image cache (see [`ServiceHub::possibly_cached`]).
    cache_dirs: HashMap<PathBuf, CacheDirNames>,
    /// Images that could not be converted since [`ServiceHub::start_counting_missing_ffmpeg`] because ffmpeg was
    /// not found (their cache keys), for the summary of an export.
    ffmpeg_missing: HashSet<String>,
}

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
    /// (trimmed lower-case name, index into `files`), sorted, to find the names starting with a prefix.
    keys: Vec<(String, usize)>,
    /// Subfolder names, in listing order.
    dirs: Vec<String>,
}

impl DirListing {
    fn read(dir: &Path) -> Option<Self> {
        let modified = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        let listed_at = SystemTime::now();
        let (mut files, mut dirs) = (Vec::new(), Vec::new());
        for entry in std::fs::read_dir(dir).ok()?.filter_map(Result::ok) {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().to_string();
            if kind.is_file() {
                files.push(name);
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
            keys,
            dirs,
        })
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
    modified: SystemTime,
    names: HashSet<String>,
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

#[derive(Default, Clone, Debug)]
pub struct ImagePayload {
    pub session_id: Option<String>,
    pub image_md5: Option<String>,
    pub image_dat_name: Option<String>,
    pub create_time: Option<i64>,
    pub prefer_file_path: bool,
    pub hardlink_only: bool,
    /// `allowCacheIndex !== false`
    pub allow_cache_index: Option<bool>,
    pub force: bool,
}

impl ImagePayload {
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
            create_time: v
                .get("createTime")
                .and_then(|t| t.as_i64().or_else(|| t.as_f64().map(|f| f as i64))),
            prefer_file_path: b("preferFilePath"),
            hardlink_only: b("hardlinkOnly"),
            allow_cache_index: v.get("allowCacheIndex").and_then(Value::as_bool),
            force: b("force"),
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct ImageResult {
    pub success: bool,
    pub local_path: Option<String>,
    pub error: Option<String>,
    pub failure_kind: Option<&'static str>,
    pub is_thumb: Option<bool>,
    pub has_update: Option<bool>,
}

impl ImageResult {
    fn ok(local_path: String, is_thumb: Option<bool>) -> Self {
        Self {
            success: true,
            local_path: Some(local_path),
            is_thumb,
            ..Default::default()
        }
    }
    fn fail(error: impl Into<String>, kind: &'static str) -> Self {
        Self {
            success: false,
            error: Some(error.into()),
            failure_kind: Some(kind),
            ..Default::default()
        }
    }
    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("success".into(), json!(self.success));
        if let Some(v) = &self.local_path {
            o.insert("localPath".into(), json!(v));
        }
        if let Some(v) = &self.error {
            o.insert("error".into(), json!(v));
        }
        if let Some(v) = self.failure_kind {
            o.insert("failureKind".into(), json!(v));
        }
        if let Some(v) = self.is_thumb {
            o.insert("isThumb".into(), json!(v));
        }
        if let Some(v) = self.has_update {
            o.insert("hasUpdate".into(), json!(v));
        }
        Value::Object(o)
    }
}

/// What a WXGF image that cannot be converted for lack of ffmpeg says, and how to fix it.
fn ffmpeg_missing_message() -> String {
    let (path, source) = crate::ffmpeg::locate();
    if source == "FFMPEG_PATH" {
        format!(
            "WXGF image needs ffmpeg: FFMPEG_PATH ({}) cannot be started",
            path.display()
        )
    } else {
        "WXGF image needs ffmpeg: not found on PATH or installed; run `weflow ffmpeg install` or set FFMPEG_PATH"
            .to_string()
    }
}

/// The summary line of an export that skipped `n` WXGF images for lack of ffmpeg.
pub(super) fn ffmpeg_missing_hint(n: usize) -> String {
    format!("{n} WXGF images were not exported because ffmpeg was not found; run `weflow ffmpeg install` or set FFMPEG_PATH, then export again")
}

fn file_size(p: &str) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn file_mtime_ms(p: &str) -> u128 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// `pickLargestDatPath`: biggest file, then newest, then name.
fn pick_largest(paths: &[String]) -> Option<String> {
    let mut list: Vec<String> = Vec::new();
    for p in paths {
        if !p.is_empty() && !list.contains(p) {
            list.push(p.clone());
        }
    }
    list.sort_by(|a, b| {
        file_size(b)
            .cmp(&file_size(a))
            .then(file_mtime_ms(b).cmp(&file_mtime_ms(a)))
            .then(a.cmp(b))
    });
    list.into_iter().next()
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

/// `getCacheKeys`
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

/// `collectHardlinkLookupMd5s` + the dat-name scan fallback of `collectLookupBasesForScan`.
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
        let db_path = self
            .db_path_override
            .clone()
            .or_else(|| profile.db_path.clone())
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| AppError::config("no account or database path configured"))?;
        let wxid = self
            .wxid_override
            .clone()
            .or_else(|| profile.wxid.clone())
            .filter(|w| !w.trim().is_empty())
            .ok_or_else(|| AppError::config("no account or database path configured"))?;
        let dir = crate::config::resolve_account_dir(&db_path, &wxid);
        if dir.exists() {
            Ok(dir)
        } else {
            Err(AppError::config("account directory not found"))
        }
    }

    pub fn image_cache_root(&self) -> PathBuf {
        let root = self.cache_base().join("Images");
        let _ = std::fs::create_dir_all(&root);
        root
    }

    fn image_keys(&self) -> (Option<u8>, Option<[u8; 16]>) {
        match self.profile() {
            Ok(p) => (
                p.image_xor_key.map(|k| k as u8),
                p.image_aes_key
                    .as_deref()
                    .and_then(crate::decrypt::parse_aes_key),
            ),
            Err(_) => (None, None),
        }
    }

    fn image_state_get(&self, key: &str) -> Option<String> {
        self.image_state.lock().unwrap().resolved.get(key).cloned()
    }

    fn image_state_set(&self, key: String, path: String) {
        self.image_state.lock().unwrap().resolved.insert(key, path);
    }

    fn image_state_remove(&self, key: &str) {
        self.image_state.lock().unwrap().resolved.remove(key);
    }

    /// `isUsableImageCacheFile`: a recognised extension, present, not empty and not a zeroed-out JPEG.
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

    /// `collectDatCandidatesFromSessionMonth`: `msg/attach/<md5(session)>/<yyyy-mm>/Img`, at most 240 directories.
    fn collect_dat_candidates(
        &self,
        account_dir: &Path,
        base_md5: &str,
        session_id: Option<&str>,
        create_time: Option<i64>,
    ) -> Vec<String> {
        let session = session_id.unwrap_or("").trim();
        let month = img::year_month_from_create_time(create_time);
        if session.is_empty() || month.is_empty() {
            return Vec::new();
        }
        let session_dir = img::session_dir_for_storage(session, clean_account_dir_name);
        if session_dir.is_empty() {
            return Vec::new();
        }
        let root = account_dir
            .join("msg")
            .join("attach")
            .join(&session_dir)
            .join(&month)
            .join("Img");
        let mut out: Vec<String> = Vec::new();
        if !root.is_dir() {
            return out;
        }
        let mut budget = 240i32;
        let mut stack: Vec<(PathBuf, u32)> = vec![(root, 0)];
        while let Some((dir, depth)) = stack.pop() {
            if budget <= 0 {
                break;
            }
            budget -= 1;
            let Some(listing) = self.dir_listing(&dir) else {
                continue;
            };
            // every candidate name, lower-cased, starts with `base_md5` (`normalize_dat_base` only drops suffixes),
            // so only those names are checked
            for name in listing.files_starting_with(base_md5) {
                if img::is_hardlink_candidate_name(name, base_md5) {
                    let full = dir.join(name);
                    if full.exists() {
                        let s = full.to_string_lossy().to_string();
                        if !out.contains(&s) {
                            out.push(s);
                        }
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
                stack.push((dir.join(name), depth + 1));
            }
        }
        out
    }

    /// The entries of `dir`, read again only when it changed since it was last read.
    fn dir_listing(&self, dir: &Path) -> Option<Arc<DirListing>> {
        let cached = self
            .image_state
            .lock()
            .unwrap()
            .dir_listings
            .get(dir)
            .cloned();
        if let Some(listing) = cached.filter(|l| l.current(dir)) {
            return Some(listing);
        }
        // read without holding the lock: the export's media threads search side by side
        let Some(listing) = DirListing::read(dir).map(Arc::new) else {
            self.image_state.lock().unwrap().dir_listings.remove(dir);
            return None;
        };
        let mut state = self.image_state.lock().unwrap();
        if state.dir_listings.len() >= MAX_DIR_LISTINGS {
            state.dir_listings.clear();
        }
        state
            .dir_listings
            .insert(dir.to_path_buf(), listing.clone());
        Some(listing)
    }

    /// `selectBestDatPathByBase`: HD variant in an Img folder, else `<md5>.dat`, else a `_t` thumbnail.
    fn select_best_dat(
        &self,
        account_dir: &Path,
        base_md5: &str,
        session_id: Option<&str>,
        create_time: Option<i64>,
        allow_thumbnail: bool,
    ) -> Option<String> {
        let candidates: Vec<String> = self
            .collect_dat_candidates(account_dir, base_md5, session_id, create_time)
            .into_iter()
            .filter(|p| Path::new(p).exists() && p.to_lowercase().ends_with(".dat"))
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
        if let Some(p) = pick_largest(&hd) {
            return Some(p);
        }
        if !allow_thumbnail {
            return None;
        }
        let sel = |list: &[String], f: &dyn Fn(&str) -> bool| {
            pick_largest(&list.iter().filter(|p| f(p)).cloned().collect::<Vec<_>>())
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
                if !Path::new(&cached).exists() {
                    continue;
                }
                if !allow_thumbnail && !img::is_hd_dat_path(&cached) {
                    continue;
                }
                return Some(cached);
            }
        }
        for base in &bases {
            let Some(selected) =
                self.select_best_dat(account_dir, base, session_id, create_time, allow_thumbnail)
            else {
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
        None
    }

    fn find_cached_output(
        &self,
        dat_path: &str,
        session_id: Option<&str>,
        prefer_hd: bool,
    ) -> Option<String> {
        let root = self.image_cache_root();
        self.possibly_cached(img::cache_output_candidates(
            &root, dat_path, session_id, prefer_hd,
        ))
        .into_iter()
        .filter(|c| c.exists())
        .map(|c| c.to_string_lossy().to_string())
        .find(|c| self.usable_image_cache_file(c))
    }

    /// The `candidates` whose folder lists their name, in order: those that may exist. Costs one metadata call per
    /// folder instead of one per path; the paths returned still have to be checked.
    fn possibly_cached(&self, candidates: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut dirs: HashMap<&Path, CacheDir> = HashMap::new();
        for dir in candidates.iter().filter_map(|c| c.parent()) {
            if !dirs.contains_key(dir) {
                dirs.insert(dir, self.index_cache_dir(dir));
            }
        }
        let state = self.image_state.lock().unwrap();
        let listed = |c: &PathBuf| {
            let Some(dir) = c.parent() else {
                return true;
            };
            match dirs.get(dir) {
                Some(CacheDir::Missing) => false,
                // an index dropped since (over `MAX_DIR_LISTINGS`) proves nothing
                Some(CacheDir::Indexed) => match (state.cache_dirs.get(dir), lower_file_name(c)) {
                    (Some(index), Some(name)) => index.names.contains(&name),
                    _ => true,
                },
                _ => true,
            }
        };
        let kept: Vec<bool> = candidates.iter().map(listed).collect();
        drop(state);
        candidates
            .into_iter()
            .zip(kept)
            .filter_map(|(c, keep)| keep.then_some(c))
            .collect()
    }

    /// Brings the names of the cache folder `dir` up to date.
    fn index_cache_dir(&self, dir: &Path) -> CacheDir {
        let Some(modified) = modified_time(dir) else {
            self.image_state.lock().unwrap().cache_dirs.remove(dir);
            return CacheDir::Missing;
        };
        {
            let state = self.image_state.lock().unwrap();
            let recorded = state.cache_dirs.get(dir).map(|index| index.modified);
            // a file this process renamed into the folder in between is recorded under the lock (`write_cache_file`)
            if recorded.is_some() && (recorded == Some(modified) || recorded == modified_time(dir))
            {
                return CacheDir::Indexed;
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
        let mut state = self.image_state.lock().unwrap();
        if state.cache_dirs.len() >= MAX_DIR_LISTINGS && !state.cache_dirs.contains_key(dir) {
            state.cache_dirs.clear();
        }
        let index = state
            .cache_dirs
            .entry(dir.to_path_buf())
            .or_insert_with(|| CacheDirNames {
                modified,
                names: HashSet::new(),
            });
        // added to the names already known, so a file another thread noted in between is not dropped
        index.names.extend(names);
        index.modified = modified;
        CacheDir::Indexed
    }

    /// Writes a decrypted image into the cache like [`write_atomically`], and adds it to its folder's names. The
    /// temporary file is written in `<cache>/Images/.partial`, and the rename is recorded under the lock: otherwise
    /// the other media threads would see the folder change while it is written and read all its names again.
    fn write_cache_file(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let (Some(dir), Some(name)) = (path.parent(), lower_file_name(path)) else {
            return write_atomically(path, bytes);
        };
        let partial = self.image_cache_root().join(".partial");
        std::fs::create_dir_all(&partial)?;
        let tmp = temporary_sibling(&partial.join(&name));
        let written = std::fs::write(&tmp, bytes).and_then(|()| {
            let mut state = self.image_state.lock().unwrap();
            std::fs::rename(&tmp, path)?;
            let modified = modified_time(dir);
            if let (Some(index), Some(modified)) = (state.cache_dirs.get_mut(dir), modified) {
                index.names.insert(name);
                index.modified = modified;
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
        session_id: Option<&str>,
        keep: &str,
    ) {
        let root = self.image_cache_root();
        let mut all: Vec<PathBuf> =
            img::cache_output_candidates(&root, dat_path, session_id, false);
        all.extend(img::cache_output_candidates(
            &root, dat_path, session_id, true,
        ));
        let mut seen = HashSet::new();
        all.retain(|c| seen.insert(c.clone()));
        for c in self.possibly_cached(all) {
            let s = c.to_string_lossy().to_string();
            if s == keep || !c.exists() || !img::is_image_file(&s) {
                continue;
            }
            let _ = std::fs::remove_file(&c);
        }
    }

    /// `tryPromoteThumbnailCache`: swaps a cached thumbnail for the HD rendition when one exists.
    fn try_promote_thumbnail(
        &self,
        p: &ImagePayload,
        cache_key: &str,
        cached: &str,
    ) -> Option<String> {
        if !Path::new(cached).exists() || !img::is_image_file(cached) || img::is_hd_path(cached) {
            return None;
        }
        let account_dir = self.account_dir_only().ok()?;
        let hd_dat = self.resolve_dat_path(
            &account_dir,
            p,
            DatSearch {
                allow_thumbnail: false,
                skip_resolved_cache: true,
                allow_dat_name_fallback: false,
            },
        )?;
        let remove_old = |keep: &str| {
            if cached != keep && Path::new(cached).exists() && !img::is_hd_path(cached) {
                let _ = std::fs::remove_file(cached);
            }
        };
        if let Some(existing) = self.find_cached_output(&hd_dat, p.session_id.as_deref(), true) {
            if Path::new(&existing).exists()
                && img::is_image_file(&existing)
                && img::is_hd_path(&existing)
            {
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
        let upgraded = self.image_decrypt(&ImagePayload {
            prefer_file_path: true,
            force: true,
            hardlink_only: true,
            ..p.clone()
        });
        if !upgraded.success {
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

    /// `image:resolveCache`: answers from the cache only; never decrypts a missing file.
    pub fn image_resolve_cache(&self, p: &ImagePayload) -> ImageResult {
        let keys = cache_keys(p);
        let Some(cache_key) = keys.first().cloned() else {
            return ImageResult::fail("missing image identifier", "not_found");
        };
        let finish = |final_path: &str| ImageResult {
            success: true,
            local_path: Some(local_path_for_payload(final_path, p.prefer_file_path)),
            has_update: Some(false),
            ..Default::default()
        };
        for key in &keys {
            let Some(cached) = self.image_state_get(key) else {
                continue;
            };
            if Path::new(&cached).exists() && self.usable_image_cache_file(&cached) {
                let upgraded = if !img::is_hd_path(&cached) {
                    self.try_promote_thumbnail(p, key, &cached)
                } else {
                    None
                };
                return finish(upgraded.as_deref().unwrap_or(&cached));
            }
            if !self.usable_image_cache_file(&cached) {
                self.image_state_remove(key);
            }
        }
        if let Ok(account_dir) = self.account_dir_only() {
            if let Some(dat) = self.resolve_dat_path(
                &account_dir,
                p,
                DatSearch {
                    allow_thumbnail: true,
                    skip_resolved_cache: false,
                    allow_dat_name_fallback: p.allow_cache_index != Some(false),
                },
            ) {
                if let Some(existing) =
                    self.find_cached_output(&dat, p.session_id.as_deref(), false)
                {
                    let upgraded = if !img::is_hd_path(&existing) {
                        self.try_promote_thumbnail(p, &cache_key, &existing)
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

    /// `image:decrypt`
    pub fn image_decrypt(&self, p: &ImagePayload) -> ImageResult {
        let keys = cache_keys(p);
        let Some(cache_key) = keys.first().cloned() else {
            return ImageResult::fail("missing image identifier", "not_found");
        };
        if p.force {
            for key in &keys {
                let Some(cached) = self.image_state_get(key) else {
                    continue;
                };
                if Path::new(&cached).exists()
                    && self.usable_image_cache_file(&cached)
                    && img::is_hd_path(&cached)
                {
                    self.cache_resolved_paths(
                        &cache_key,
                        p.image_md5.as_deref(),
                        p.image_dat_name.as_deref(),
                        &cached,
                    );
                    return ImageResult::ok(
                        local_path_for_payload(&cached, p.prefer_file_path),
                        None,
                    );
                }
                if !self.usable_image_cache_file(&cached) {
                    self.image_state_remove(key);
                }
            }
        } else if let Some(cached) = self.image_state_get(&cache_key) {
            if Path::new(&cached).exists() && self.usable_image_cache_file(&cached) {
                let upgraded = if !img::is_hd_path(&cached) {
                    self.try_promote_thumbnail(p, &cache_key, &cached)
                } else {
                    None
                };
                return ImageResult::ok(
                    local_path_for_payload(
                        upgraded.as_deref().unwrap_or(&cached),
                        p.prefer_file_path,
                    ),
                    None,
                );
            }
            if !self.usable_image_cache_file(&cached) {
                self.image_state_remove(&cache_key);
            }
        }
        self.image_decrypt_internal(p, &cache_key)
    }

    fn image_decrypt_internal(&self, p: &ImagePayload, cache_key: &str) -> ImageResult {
        let Ok(account_dir) = self.account_dir_only() else {
            return ImageResult::fail("no account or database path configured", "not_found");
        };
        let allow_fallback = p.allow_cache_index != Some(false);
        let mut fallback_to_thumbnail = false;
        let dat = if p.force {
            match self.resolve_dat_path(
                &account_dir,
                p,
                DatSearch {
                    allow_thumbnail: false,
                    skip_resolved_cache: false,
                    allow_dat_name_fallback: allow_fallback,
                },
            ) {
                Some(d) => Some(d),
                None => {
                    let d = self.resolve_dat_path(
                        &account_dir,
                        p,
                        DatSearch {
                            allow_thumbnail: true,
                            skip_resolved_cache: false,
                            allow_dat_name_fallback: allow_fallback,
                        },
                    );
                    fallback_to_thumbnail = d.is_some();
                    d
                }
            }
        } else {
            self.resolve_dat_path(
                &account_dir,
                p,
                DatSearch {
                    allow_thumbnail: true,
                    skip_resolved_cache: false,
                    allow_dat_name_fallback: allow_fallback,
                },
            )
        };
        let Some(dat) = dat else {
            return if p.force {
                ImageResult::fail(
                    "image file not found, open the image in WeChat and retry",
                    "not_found",
                )
            } else {
                ImageResult::fail("image file not found", "not_found")
            };
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
            );
        }

        let prefer_hd = p.force && !fallback_to_thumbnail;
        if let Some(existing) = self.find_cached_output(&dat, p.session_id.as_deref(), prefer_hd) {
            let is_hd = img::is_hd_path(&existing);
            // With --force a cached thumbnail is re-decrypted in case the HD file has appeared since; when `dat` is
            // already the best file on disk (`fallback_to_thumbnail`) the cached output is that very rendition.
            if !(p.force && !is_hd && !fallback_to_thumbnail) {
                self.cache_resolved_paths(
                    cache_key,
                    p.image_md5.as_deref(),
                    p.image_dat_name.as_deref(),
                    &existing,
                );
                return ImageResult::ok(
                    local_path_for_payload(&existing, p.prefer_file_path),
                    Some(img::is_thumbnail_path(&existing)),
                );
            }
        }

        let (xor_key, aes_key) = self.image_keys();
        let Some(xor_key) = xor_key else {
            return ImageResult::fail("image decrypt key is not configured", "not_found");
        };
        let decrypted =
            match crate::decrypt::decrypt_file(Path::new(&dat), xor_key, aes_key.as_ref()) {
                Ok(r) => r,
                Err(e) => {
                    return ImageResult::fail(
                        format!("native decrypt failed, check the image keys: {}", e.message),
                        "not_found",
                    )
                }
            };
        let (data, wxgf_failure) = img::unwrap_wxgf(decrypted.data);
        if wxgf_failure == Some(img::HevcError::FfmpegMissing) {
            self.image_state
                .lock()
                .unwrap()
                .ffmpeg_missing
                .insert(cache_key.to_string());
            return ImageResult {
                success: false,
                error: Some(ffmpeg_missing_message()),
                failure_kind: Some("ffmpeg_missing"),
                is_thumb: Some(img::is_thumbnail_path(&dat)),
                ..Default::default()
            };
        }
        let Some(ext) = img::detect_image_extension(&data) else {
            return ImageResult {
                success: false,
                error: Some("decrypted data is not a valid image".into()),
                failure_kind: Some("decrypt_failed"),
                is_thumb: Some(img::is_thumbnail_path(&dat)),
                ..Default::default()
            };
        };
        let output =
            img::cache_output_path(&self.image_cache_root(), &dat, ext, p.session_id.as_deref());
        if let Some(parent) = output.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return ImageResult::fail(e.to_string(), "not_found");
            }
        }
        // renamed into place: an export's other media threads may read this cache file as soon as it exists
        if let Err(e) = self.write_cache_file(&output, &data) {
            return ImageResult::fail(e.to_string(), "not_found");
        }
        let out = output.to_string_lossy().to_string();
        self.remove_duplicate_cache_candidates(&dat, p.session_id.as_deref(), &out);
        self.cache_resolved_paths(
            cache_key,
            p.image_md5.as_deref(),
            p.image_dat_name.as_deref(),
            &out,
        );
        let local = if p.prefer_file_path {
            out.clone()
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
        ImageResult::ok(local, Some(img::is_thumbnail_path(&dat)))
    }

    /// `image:resolveCacheBatch`: one result row per payload (duplicates are resolved once).
    pub fn image_resolve_cache_batch(&self, payloads: &[ImagePayload]) -> Value {
        let mut memo: HashMap<String, Value> = HashMap::new();
        let rows: Vec<Value> = payloads
            .iter()
            .map(|p| {
                let key = format!(
                    "{}|{}|{}|{}|{}|{}",
                    p.session_id.as_deref().unwrap_or("").trim().to_lowercase(),
                    p.image_md5.as_deref().unwrap_or("").trim().to_lowercase(),
                    p.image_dat_name
                        .as_deref()
                        .unwrap_or("")
                        .trim()
                        .to_lowercase(),
                    p.create_time.unwrap_or(0),
                    p.prefer_file_path,
                    p.hardlink_only
                );
                memo.entry(key)
                    .or_insert_with(|| self.image_resolve_cache(p).to_json())
                    .clone()
            })
            .collect();
        json!({ "success": true, "rows": rows })
    }

    /// `chat:getImageData`: the decrypted image of a message as bytes.
    pub fn image_data_for_message(&self, session_id: &str, msg_id: &str) -> AppResult<Vec<u8>> {
        let local_id = crate::api::js_parse_int(msg_id)
            .ok_or_else(|| AppError::usage("invalid message id"))?;
        let wcdb = self.open_wcdb()?;
        let row = wcdb
            .message_by_id(
                session_id,
                local_id.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            )
            .map_err(|e| AppError::native(e.to_string()))?;
        if row.as_object().is_none_or(|o| o.is_empty()) {
            return Err(AppError::runtime("message not found"));
        }
        let my = self.my_wxid_cleaned();
        let msg = crate::chat_msg::map_rows(std::slice::from_ref(&row), &my)
            .into_iter()
            .next()
            .ok_or_else(|| AppError::runtime("message not found"))?;
        let raw_md5 = msg
            .image_md5
            .clone()
            .filter(|m| !m.is_empty())
            .or_else(|| crate::message::extract_image_md5(&msg.raw_content));
        let dat_name = msg.image_dat_name.clone().filter(|m| !m.is_empty());
        if raw_md5.is_none() && dat_name.is_none() {
            return Err(AppError::runtime(
                "image has no md5 / datName, cannot locate the original file",
            ));
        }
        let result = self.image_decrypt(&ImagePayload {
            session_id: Some(session_id.to_string()),
            image_md5: raw_md5,
            image_dat_name: dat_name,
            create_time: Some(msg.create_time).filter(|t| *t != 0),
            prefer_file_path: true,
            hardlink_only: true,
            ..Default::default()
        });
        let path = result
            .local_path
            .filter(|_| result.success)
            .ok_or_else(|| {
                AppError::runtime(
                    result
                        .error
                        .unwrap_or_else(|| "image decrypt failed".into()),
                )
            })?;
        std::fs::read(&path).map_err(|e| AppError::runtime(format!("failed to read {path}: {e}")))
    }

    /// Starts counting the images that cannot be converted for lack of ffmpeg (an export's summary).
    pub(super) fn start_counting_missing_ffmpeg(&self) {
        self.image_state.lock().unwrap().ffmpeg_missing.clear();
    }

    /// Images (not messages) that could not be converted for lack of ffmpeg since the count was started.
    pub(super) fn images_missing_ffmpeg(&self) -> usize {
        self.image_state.lock().unwrap().ffmpeg_missing.len()
    }

    /// `image:clearCache`: empties the decrypted-image cache (keeps the folder layout).
    pub fn image_clear_cache(&self) -> Value {
        {
            let mut state = self.image_state.lock().unwrap();
            state.resolved.clear();
            state.cache_dirs.clear();
        }
        let root = self.cache_base().join("Images");
        if !root.exists() {
            return json!({ "success": true });
        }
        fn clear(dir: &Path) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            for e in rd.filter_map(Result::ok) {
                let p = e.path();
                if p.is_dir() {
                    clear(&p);
                } else {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
        clear(&root);
        json!({ "success": true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("weflow-listing-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        dir
    }

    /// Sets the folder's modification time an hour back, as for a month folder nothing writes to any more.
    fn age(dir: &Path) {
        let past = SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::open(dir)
            .unwrap()
            .set_modified(past)
            .unwrap();
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
        let _ = std::fs::remove_dir_all(&dir);
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
        let _ = std::fs::remove_dir_all(&dir);
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
        let ok = ImageResult {
            success: true,
            local_path: Some("/x.jpg".into()),
            is_thumb: Some(false),
            has_update: Some(false),
            ..Default::default()
        };
        assert_eq!(
            ok.to_json(),
            json!({ "success": true, "localPath": "/x.jpg", "isThumb": false, "hasUpdate": false })
        );
        assert_eq!(
            ImageResult::fail("nope", "not_found").to_json(),
            json!({ "success": false, "error": "nope", "failureKind": "not_found" })
        );
    }
}
