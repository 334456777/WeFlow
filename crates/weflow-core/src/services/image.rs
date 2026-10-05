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

use base64::Engine;
use serde_json::{json, Map, Value};

use super::*;
use crate::image::{self as img};

#[derive(Default)]
pub struct ImageState {
    resolved: HashMap<String, String>,
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

    /// `isUsableImageCacheFile`: a recognised extension, present, and not a zeroed-out JPEG.
    fn usable_image_cache_file(&self, path: &str) -> bool {
        if !img::is_image_file(path) || !Path::new(path).exists() {
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
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            let entries: Vec<_> = rd.filter_map(Result::ok).collect();
            for e in &entries {
                if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    continue;
                }
                let name = e.file_name().to_string_lossy().to_string();
                if img::is_hardlink_candidate_name(&name, base_md5) {
                    let full = dir.join(&name);
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
            for e in &entries {
                if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let name = e.file_name().to_string_lossy().to_string();
                if name.is_empty() || name.starts_with('.') {
                    continue;
                }
                stack.push((dir.join(&name), depth + 1));
            }
        }
        out
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
        img::cache_output_candidates(&root, dat_path, session_id, prefer_hd)
            .into_iter()
            .filter(|c| c.exists())
            .map(|c| c.to_string_lossy().to_string())
            .find(|c| self.usable_image_cache_file(c))
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
        for c in all {
            let s = c.to_string_lossy().to_string();
            if s == keep || !seen.insert(s.clone()) || !c.exists() || !img::is_image_file(&s) {
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
        let (data, _still_wxgf) = img::unwrap_wxgf(decrypted.data);
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
        if let Err(e) = std::fs::write(&output, &data) {
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

    /// `image:clearCache`: empties the decrypted-image cache (keeps the folder layout).
    pub fn image_clear_cache(&self) -> Value {
        self.image_state.lock().unwrap().resolved.clear();
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
