//! WeFlow's caches part by part (`weflow cache list` / `cache clear`), and the removal of the current account's
//! data (`weflow cache clear-account`).
//! Only WeFlow's own files are touched; WeChat's data is never written.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::*;
use crate::contract::CachePart;

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

/// A file or folder name belongs to the account when one of the (lowercase) ids appears
/// in it as a whole word, case-insensitively: `wxid_abc`, `wxid_abc_1a2b`, `wxid_abc-moments.json` and
/// `group_my_message_counts_wxid_abc.json` do, `wxid_abc123.json` (another account) does not.
fn account_name_matches(name: &str, ids: &[String]) -> bool {
    let lowered = name.trim().to_lowercase();
    ids.iter().any(|id| contains_word(&lowered, id))
}

/// Whether `id` appears in `name` with no ASCII letter or digit right before or after it. WeChat ids are ASCII
/// letters, digits, `_` and `-`, so a neighbouring letter or digit means a longer, different id; `_`, `-`, `.` and
/// any other character end the id.
fn contains_word(name: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    let (name, id) = (name.as_bytes(), id.as_bytes());
    let part_of_id = |at: Option<&u8>| at.is_some_and(u8::is_ascii_alphanumeric);
    name.windows(id.len()).enumerate().any(|(start, window)| {
        window == id
            && !part_of_id(start.checked_sub(1).and_then(|before| name.get(before)))
            && !part_of_id(name.get(start + id.len()))
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
    /// Files and folders holding `part`, whether they exist or not (unused programs, and the WebUI versions running
    /// servers do not show: those that exist).
    pub fn cache_part_paths(&self, part: CachePart) -> Vec<PathBuf> {
        let base = self.cache_base();
        match part {
            CachePart::Images => vec![base.join("Images")],
            CachePart::Voices => vec![base.join("Voices")],
            CachePart::Emojis => vec![base.join("Emojis")],
            CachePart::Sns => vec![base.join("sns_cache")],
            CachePart::Analytics => vec![base.join("analytics_cache.json")],
            CachePart::Api => vec![
                base.join("api-media"),
                base.join("push-avatar-files"),
                self.serve_files_dir(),
            ],
            // fingerprints of keys that worked, so later commands skip the slower check
            CachePart::Keys => vec![self.ctx.cache_dir().join("verified-keys")],
            CachePart::Runtime => self.unused_programs(),
            // the WebUI versions downloaded (issue #148), but the ones running servers show (#1139)
            CachePart::Web => self.web_store().clearable(),
        }
    }

    /// Programs that nothing runs: runtime directories earlier versions unpacked into `runtime/`, and the ffmpeg
    /// builds those versions installed in `ffmpeg/<release>/` (files of one's own in `ffmpeg/` stay).
    fn unused_programs(&self) -> Vec<PathBuf> {
        let subfolders = |dir: PathBuf| -> Vec<PathBuf> {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return Vec::new();
            };
            let mut dirs: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect();
            dirs.sort();
            dirs
        };
        let mut dirs = subfolders(self.ctx.home_dir.join("runtime"));
        dirs.extend(subfolders(self.ctx.home_dir.join("ffmpeg")));
        dirs
    }

    /// Folder of the files `weflow serve` writes for `?return=file` requests that name no `--out`, a folder of each
    /// running server's own in it (see [`crate::serve_files`]).
    pub fn serve_files_dir(&self) -> PathBuf {
        self.cache_base().join("serve-files")
    }

    /// `weflow cache list`: every part with its paths, size and number of files.
    pub fn cache_list(&self) -> Vec<crate::contract::CacheUsage> {
        CachePart::ALL
            .iter()
            .map(|&part| {
                let paths = self.cache_part_paths(part);
                let (bytes, files) = paths
                    .iter()
                    .map(|p| disk_usage(p))
                    .fold((0, 0), |(b, f), (pb, pf)| (b + pb, f + pf));
                crate::contract::CacheUsage {
                    part,
                    paths,
                    bytes,
                    files,
                }
            })
            .collect()
    }

    /// `weflow cache clear`: removes the files of `parts` and forgets what is held in memory about them.
    pub fn cache_clear(&self, parts: &[CachePart]) -> crate::contract::CacheCleared {
        let (mut removed, mut warnings) = (Vec::new(), Vec::new());
        let mut freed = 0u64;
        let serve_files = self.serve_files_dir();
        for &part in parts {
            if part == CachePart::Web {
                // the versions running servers show stay, as with `web prune` (#1139)
                let store = self.web_store();
                let before = disk_usage(store.root()).0;
                let (gone, failed) = store.clear();
                removed.extend(gone.iter().map(|p| p.to_string_lossy().into_owned()));
                warnings.extend(failed);
                freed += before.saturating_sub(disk_usage(store.root()).0);
                continue;
            }
            for path in self.cache_part_paths(part) {
                let (bytes, _) = disk_usage(&path);
                if path == serve_files {
                    // the folders of the servers running stay (#815)
                    let (gone, failed) = crate::serve_files::remove_unheld(&path);
                    removed.extend(gone.iter().map(|p| p.to_string_lossy().into_owned()));
                    warnings.extend(failed);
                    freed += bytes.saturating_sub(disk_usage(&path).0);
                } else if remove_path(&path, &mut removed, &mut warnings) {
                    freed += bytes;
                }
            }
            match part {
                CachePart::Images => self.image_forget_cached(),
                CachePart::Sns => self.sns_clear_memory_cache(),
                _ => {}
            }
        }
        crate::contract::CacheCleared {
            removed_paths: removed,
            freed_bytes: freed,
            warnings,
        }
    }

    /// What [`Self::clear_current_account_data`] removes (only what exists): WeFlow's caches of the current account
    /// (images, voices, stickers, Moments, analytics, group hints, key fingerprints), its insight records (with the
    /// copies of its records file set aside when it could not be read), and the entries of `export_dirs` named after
    /// the account.
    ///
    /// The file of every account's insight records that WeFlow 7.0.0 and earlier wrote is split into the accounts'
    /// files first, which changes no record, so that the account's own file is the one listed.
    pub fn account_data_paths(&self, export_dirs: &[PathBuf]) -> AppResult<Vec<PathBuf>> {
        let id = self.my_wxid_cleaned();
        if id.is_empty() {
            return Err(AppError::config(
                "no current account configured: wxid is not set, so there is nothing to clear",
            ));
        }
        let ids = [id.to_lowercase()];
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut add = |path: PathBuf| {
            if path.exists() && !paths.contains(&path) {
                paths.push(path);
            }
        };
        let base = self.cache_base();
        let roots = [
            base.clone(),
            base.join("Images"),
            base.join("Voices"),
            base.join("Emojis"),
        ];
        for root in &roots {
            add(root.join(&id));
        }
        for root in &roots {
            matched_entries(root, &ids).into_iter().for_each(&mut add);
        }
        add(base.join("analytics_cache.json"));
        // the group hints of the home folder's cache, there when `cache_path` names another folder (#1013)
        if let Some(path) = self.legacy_hint_path() {
            add(path);
        }
        // fingerprints of keys that worked (the profile's key goes with the account settings)
        add(self.ctx.cache_dir().join("verified-keys"));
        let home = &self.ctx.home_dir;
        crate::insight::split_legacy_records(home);
        let records = crate::insight::records_file(home, &self.insight_scope());
        let set_aside = crate::insight::set_aside_records_files(&records);
        add(records);
        set_aside.into_iter().for_each(&mut add);
        for dir in export_dirs {
            matched_entries(dir, &ids).into_iter().for_each(&mut add);
        }
        Ok(paths)
    }

    /// Removes [`Self::account_data_paths`]. The caller then resets the account
    /// settings of the profile, as WeFlow's desktop app does.
    pub fn clear_current_account_data(
        &self,
        export_dirs: &[PathBuf],
    ) -> AppResult<crate::contract::AccountCleared> {
        let paths = self.account_data_paths(export_dirs)?;
        let (mut removed, mut warnings) = (Vec::new(), Vec::new());
        for path in &paths {
            remove_path(path, &mut removed, &mut warnings);
        }
        self.sns_clear_memory_cache();
        self.image_forget_cached();
        Ok(crate::contract::AccountCleared {
            removed_paths: removed,
            profile_reset: true,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cache_part_is_written_by_the_name_cache_clear_takes() {
        for part in CachePart::ALL {
            assert_eq!(serde_json::to_value(part).unwrap(), part.name());
        }
    }

    #[test]
    fn names_match_the_account_ids() {
        let ids = vec!["wxid_abc".to_string()];
        assert!(account_name_matches("wxid_abc", &ids));
        assert!(account_name_matches("WXID_ABC_1a2b", &ids));
        assert!(account_name_matches("export-wxid_abc-2026", &ids));
        // the names WeFlow writes for the account: its folder, files named after it, the group hint file
        assert!(account_name_matches("wxid_abc_xxxx", &ids));
        assert!(account_name_matches("wxid_abc-moments.json", &ids));
        assert!(account_name_matches("wxid_abc.json", &ids));
        assert!(account_name_matches(
            "group_my_message_counts_wxid_abc.json",
            &ids
        ));
        assert!(!account_name_matches("wxid_other", &ids));
        assert!(!account_name_matches("", &ids));
    }

    #[test]
    fn a_longer_id_is_another_account() {
        let ids = vec!["wxid_abc".to_string()];
        for name in [
            "wxid_abc123.json",
            "xwxid_abc",
            "wxid_abcd_1a2b",
            "2wxid_abc.json",
        ] {
            assert!(!account_name_matches(name, &ids), "{name}");
        }
        // a short custom WeChat id of an older account
        let ids = vec!["john".to_string()];
        assert!(account_name_matches("john_1a2b", &ids));
        assert!(account_name_matches("chat-john.json", &ids));
        for name in ["johnny.json", "littlejohn", "john2"] {
            assert!(!account_name_matches(name, &ids), "{name}");
        }
    }

    #[test]
    fn an_empty_id_matches_nothing() {
        assert!(!account_name_matches("anything", &[String::new()]));
        assert!(!account_name_matches("wxid_abc", &[]));
    }

    #[test]
    fn an_id_is_found_after_a_partial_match() {
        // the first `wxid_abc` is part of a longer id, the second stands on its own
        let ids = vec!["wxid_abc".to_string()];
        assert!(account_name_matches("wxid_abcd-wxid_abc.json", &ids));
        assert!(account_name_matches("聊天记录wxid_abc", &ids));
    }
}
