//! Cache and account-data cleanup: ports of the `cache:clearAll` and `chat:clearCurrentAccountData` handlers in
//! `electron/main.ts`. Only WeFlow's own files are touched; WeChat's data is never written.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::*;

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

fn remove_path(path: &Path, removed: &mut Vec<String>, warnings: &mut Vec<String>) {
    if !path.exists() {
        return;
    }
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match result {
        Ok(()) => removed.push(path.to_string_lossy().to_string()),
        Err(e) => warnings.push(format!("{}: {e}", path.display())),
    }
}

fn remove_matched_entries(
    root: &Path,
    ids: &[String],
    removed: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if account_name_matches(&entry.file_name().to_string_lossy(), ids) {
            remove_path(&entry.path(), removed, warnings);
        }
    }
}

impl ServiceHub {
    /// `cache:clearAll`: the analytics cache, the decrypted-image cache and the in-memory caches.
    pub fn cache_clear_all(&self) -> Value {
        let mut errors: Vec<String> = Vec::new();
        if let Err(e) = self.analytics_clear_cache() {
            errors.push(e.into_message());
        }
        let image = self.image_clear_cache();
        if image["success"] != true {
            errors.push(image["error"].as_str().unwrap_or("image cache").to_string());
        }
        *self.group_state.lock().unwrap() = Default::default();
        self.sns_clear_memory_cache();
        if errors.is_empty() {
            json!({ "success": true })
        } else {
            json!({ "success": false, "error": errors.join("; ") })
        }
    }

    /// `chat:clearCurrentAccountData`. `clear_cache` removes WeFlow's caches of the current account (images, voices,
    /// stickers, Moments, analytics); `export_dirs` are folders whose entries named after the account are removed.
    /// The caller resets the account settings of the profile when `clear_cache` is set, as the desktop app does.
    pub fn clear_current_account_data(
        &self,
        clear_cache: bool,
        export_dirs: &[PathBuf],
    ) -> AppResult<Value> {
        if !clear_cache && export_dirs.is_empty() {
            return Err(AppError::usage(
                "choose at least one thing to clear: --cache and/or --exports-dir <dir>",
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
        let (mut removed, mut warnings): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());

        if clear_cache {
            if let Err(e) = self.analytics_clear_cache() {
                warnings.push(e.into_message());
            }
            let image = self.image_clear_cache();
            if image["success"] != true {
                warnings.push(image["error"].as_str().unwrap_or("image cache").to_string());
            }
            *self.group_state.lock().unwrap() = Default::default();
            self.sns_clear_memory_cache();
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
                    remove_path(&root.join(&id), &mut removed, &mut warnings);
                }
            }
            for root in &roots {
                remove_matched_entries(root, &ids, &mut removed, &mut warnings);
            }
            // fingerprints of keys that worked (the profile's key goes with the account settings)
            let verified = self.ctx.cache_dir().join("verified-keys");
            if verified.exists() {
                remove_path(&verified, &mut removed, &mut warnings);
            }
        }
        for dir in export_dirs {
            remove_matched_entries(dir, &ids, &mut removed, &mut warnings);
        }
        removed.dedup();
        let mut out =
            json!({ "success": true, "removedPaths": removed, "profileReset": clear_cache });
        if !warnings.is_empty() {
            out["warning"] = json!(warnings.join("; "));
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
