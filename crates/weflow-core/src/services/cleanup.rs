//! WeFlow's caches part by part (`weflow cache list` / `cache clear`), and the removal of the current account's
//! data (`weflow cache clear-account`, a port of the `chat:clearCurrentAccountData` handler in `electron/main.ts`).
//! Only WeFlow's own files are touched; WeChat's data is never written.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use walkdir::WalkDir;

use super::*;

/// A part of WeFlow's cache that is listed and cleared on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePart {
    Images,
    Voices,
    Emojis,
    Sns,
    Analytics,
    Api,
    Keys,
    Runtime,
}

impl CachePart {
    /// Every part, in the order they are listed; `cache clear --all` clears them all.
    pub const ALL: [CachePart; 8] = [
        CachePart::Images,
        CachePart::Voices,
        CachePart::Emojis,
        CachePart::Sns,
        CachePart::Analytics,
        CachePart::Api,
        CachePart::Keys,
        CachePart::Runtime,
    ];

    /// Name in the JSON output; also the option of `weflow cache clear` (`--images`, …).
    pub fn name(self) -> &'static str {
        match self {
            CachePart::Images => "images",
            CachePart::Voices => "voices",
            CachePart::Emojis => "emojis",
            CachePart::Sns => "sns",
            CachePart::Analytics => "analytics",
            CachePart::Api => "api",
            CachePart::Keys => "keys",
            CachePart::Runtime => "runtime",
        }
    }
}

/// Bytes and number of the files under `path` (the file itself when it is one); 0 when it is missing.
pub fn disk_usage(path: &Path) -> (u64, u64) {
    WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .fold((0, 0), |(bytes, files), e| {
            (bytes + e.metadata().map_or(0, |m| m.len()), files + 1)
        })
}

/// `normalizeAccountId`: `wxid_abc_1a2b` → `wxid_abc`, `name_1a2b` → `name`.
pub fn normalize_account_id(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.to_lowercase().starts_with("wxid_") {
        // `wxid_` plus everything up to the next underscore
        let rest = &trimmed[5..];
        let end = rest.find('_').unwrap_or(rest.len());
        return if end == 0 {
            trimmed.to_string()
        } else {
            trimmed[..5 + end].to_string()
        };
    }
    match trimmed.rsplit_once('_') {
        Some((head, tail))
            if !head.is_empty()
                && tail.len() == 4
                && tail.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            head.to_string()
        }
        _ => trimmed.to_string(),
    }
}

/// `buildAccountNameMatcher`: a file or folder name belongs to the account when it equals, starts with
/// `<id>_` or contains one of the ids (case-insensitive).
fn account_name_matches(name: &str, ids: &[String]) -> bool {
    let lowered = name.trim().to_lowercase();
    !lowered.is_empty()
        && ids.iter().any(|id| {
            lowered == *id
                || lowered.starts_with(&format!("{id}_"))
                || lowered.contains(id.as_str())
        })
}

/// Removes a file or folder; true when it was there and is gone.
fn remove_path(path: &Path, removed: &mut Vec<String>, warnings: &mut Vec<String>) -> bool {
    if !path.exists() {
        return false;
    }
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match result {
        Ok(()) => {
            removed.push(path.to_string_lossy().to_string());
            true
        }
        Err(e) => {
            warnings.push(format!("{}: {e}", path.display()));
            false
        }
    }
}

/// Entries of `root` named after the account.
fn matched_entries(root: &Path, ids: &[String]) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| account_name_matches(&e.file_name().to_string_lossy(), ids))
        .map(|e| e.path())
        .collect();
    paths.sort();
    paths
}

impl ServiceHub {
    /// Files and folders holding `part`, whether they exist or not (older runtimes: those that exist).
    pub fn cache_part_paths(&self, part: CachePart) -> Vec<PathBuf> {
        let base = self.cache_base();
        match part {
            CachePart::Images => vec![base.join("Images")],
            CachePart::Voices => vec![base.join("Voices")],
            CachePart::Emojis => vec![base.join("Emojis")],
            CachePart::Sns => vec![base.join("sns_cache")],
            CachePart::Analytics => vec![base.join("analytics_cache.json")],
            CachePart::Api => vec![base.join("api-media"), base.join("push-avatar-files")],
            // fingerprints of keys that worked, so later commands skip the slower check
            CachePart::Keys => vec![self.ctx.cache_dir().join("verified-keys")],
            CachePart::Runtime => self.older_runtimes(),
        }
    }

    /// Folder the embedded runtimes are unpacked into, one subfolder per version.
    pub fn runtime_root(&self) -> PathBuf {
        self.ctx.home_dir.join("runtime")
    }

    /// `<home>/runtime/<version>` folders left by other versions; the running version's is unpacked again on every
    /// start, so it stays.
    fn older_runtimes(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(self.runtime_root()) else {
            return Vec::new();
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_type().is_ok_and(|t| t.is_dir())
                    && e.file_name() != self.ctx.version.as_str()
            })
            .map(|e| e.path())
            .collect();
        dirs.sort();
        dirs
    }

    /// `weflow cache list`: every part with its paths, size and number of files.
    pub fn cache_list(&self) -> Value {
        Value::Array(
            CachePart::ALL
                .iter()
                .map(|&part| {
                    let paths = self.cache_part_paths(part);
                    let (bytes, files) = paths
                        .iter()
                        .map(|p| disk_usage(p))
                        .fold((0, 0), |(b, f), (pb, pf)| (b + pb, f + pf));
                    json!({ "part": part.name(), "paths": paths, "bytes": bytes, "files": files })
                })
                .collect(),
        )
    }

    /// `weflow cache clear`: removes the files of `parts` and forgets what is held in memory about them.
    pub fn cache_clear(&self, parts: &[CachePart]) -> Value {
        let (mut removed, mut warnings) = (Vec::new(), Vec::new());
        let mut freed = 0u64;
        for &part in parts {
            for path in self.cache_part_paths(part) {
                let (bytes, _) = disk_usage(&path);
                if remove_path(&path, &mut removed, &mut warnings) {
                    freed += bytes;
                }
            }
            match part {
                CachePart::Images => self.image_forget_cached(),
                CachePart::Analytics => {
                    let _ = self.analytics_clear_cache();
                }
                CachePart::Sns => self.sns_clear_memory_cache(),
                _ => {}
            }
        }
        let mut out = json!({ "removedPaths": removed, "freedBytes": freed });
        if !warnings.is_empty() {
            out["warnings"] = json!(warnings);
        }
        out
    }

    /// What [`Self::clear_current_account_data`] removes (only what exists). `clear_cache` takes WeFlow's caches of
    /// the current account (images, voices, stickers, Moments, analytics, key fingerprints); `export_dirs` are
    /// folders whose entries named after the account are taken.
    pub fn account_data_paths(
        &self,
        clear_cache: bool,
        export_dirs: &[PathBuf],
    ) -> AppResult<Vec<PathBuf>> {
        if !clear_cache && export_dirs.is_empty() {
            return Err(AppError::usage(
                "nothing chosen to clear: neither the caches nor an export folder",
            ));
        }
        let raw = self.my_wxid_cleaned();
        if raw.trim().is_empty() {
            return Err(AppError::config(
                "no current account (wxid) configured, nothing to clear",
            ));
        }
        let normalized = normalize_account_id(&raw);
        let mut ids: Vec<String> = vec![raw.trim().to_lowercase()];
        if !normalized.is_empty() && !ids.contains(&normalized.to_lowercase()) {
            ids.push(normalized.to_lowercase());
        }
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut add = |path: PathBuf| {
            if path.exists() && !paths.contains(&path) {
                paths.push(path);
            }
        };
        if clear_cache {
            let base = self.cache_base();
            let roots = [
                base.clone(),
                base.join("Images"),
                base.join("Voices"),
                base.join("Emojis"),
            ];
            for id in [raw.trim().to_string(), normalized.clone()]
                .into_iter()
                .filter(|s| !s.is_empty())
            {
                for root in &roots {
                    add(root.join(&id));
                }
            }
            for root in &roots {
                matched_entries(root, &ids).into_iter().for_each(&mut add);
            }
            add(base.join("analytics_cache.json"));
            // fingerprints of keys that worked (the profile's key goes with the account settings)
            add(self.ctx.cache_dir().join("verified-keys"));
        }
        for dir in export_dirs {
            matched_entries(dir, &ids).into_iter().for_each(&mut add);
        }
        Ok(paths)
    }

    /// `chat:clearCurrentAccountData`: removes [`Self::account_data_paths`]. The caller resets the account settings
    /// of the profile when `clear_cache` is set, as the desktop app does.
    pub fn clear_current_account_data(
        &self,
        clear_cache: bool,
        export_dirs: &[PathBuf],
    ) -> AppResult<Value> {
        let paths = self.account_data_paths(clear_cache, export_dirs)?;
        let (mut removed, mut warnings) = (Vec::new(), Vec::new());
        for path in &paths {
            remove_path(path, &mut removed, &mut warnings);
        }
        if clear_cache {
            *self.group_state.lock().unwrap() = Default::default();
            self.sns_clear_memory_cache();
            self.image_forget_cached();
            let _ = self.analytics_clear_cache();
        }
        let mut out = json!({ "removedPaths": removed, "profileReset": clear_cache });
        if !warnings.is_empty() {
            out["warnings"] = json!(warnings);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_ids_lose_their_directory_suffix() {
        assert_eq!(normalize_account_id("wxid_abc123_1a2b"), "wxid_abc123");
        assert_eq!(normalize_account_id("WXID_abc"), "WXID_abc");
        assert_eq!(normalize_account_id("alice_9x9z"), "alice");
        assert_eq!(normalize_account_id("alice_toolong"), "alice_toolong");
        assert_eq!(normalize_account_id("  "), "");
    }

    #[test]
    fn names_match_the_account_ids() {
        let ids = vec!["wxid_abc".to_string()];
        assert!(account_name_matches("wxid_abc", &ids));
        assert!(account_name_matches("WXID_ABC_1a2b", &ids));
        assert!(account_name_matches("export-wxid_abc-2026", &ids));
        assert!(!account_name_matches("wxid_other", &ids));
        assert!(!account_name_matches("", &ids));
    }
}
