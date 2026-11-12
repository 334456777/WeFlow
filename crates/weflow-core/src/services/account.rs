//! The account and its databases: where WeChat keeps them, which accounts there are, the configured profile, and
//! opening the databases with its key.

use std::path::{Path, PathBuf};

use super::ServiceHub;
use crate::config::{clean_account_dir_name, resolve_account_dir, ProfileConfig};
use crate::error::{AppError, AppResult};

impl ServiceHub {
    pub fn db_detect(&self) -> crate::contract::DbCandidates {
        let candidates = default_db_candidates()
            .into_iter()
            .map(|path| crate::contract::DbCandidate {
                exists: path.exists(),
                path,
            })
            .collect();
        crate::contract::DbCandidates { candidates }
    }

    pub fn db_scan(&self, root: &str) -> crate::contract::DbScan {
        let root = crate::config::expand_home(root);
        let mut wxids = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if crate::config::is_account_dir(&path) {
                    wxids.push(crate::contract::ScannedAccount { wxid: name, path });
                }
            }
        }
        crate::contract::DbScan {
            root,
            accounts: wxids,
        }
    }

    /// The wxid of every account found in the data directory (`root`, else the configured `db_path`, else the
    /// default locations), as `config set wxid` expects it (without the `_ab12` suffix of the folder name).
    pub fn db_wxid(&self, root: Option<&str>) -> AppResult<crate::contract::DbAccounts> {
        let configured = root
            .map(str::to_string)
            .or_else(|| self.configured_db_path());
        let roots: Vec<PathBuf> = match configured {
            Some(root) => vec![crate::config::expand_home(&root)],
            None => default_db_candidates()
                .into_iter()
                .filter(|p| p.exists())
                .collect(),
        };
        if roots.is_empty() {
            return Err(AppError::config(
                "no WeChat data directory found; pass the directory or run config set db_path",
            ));
        }
        let mut accounts = Vec::new();
        for root in &roots {
            for (wxid, path) in find_accounts(root) {
                accounts.push(crate::contract::DbAccount { wxid, path });
            }
        }
        if accounts.is_empty() {
            return Err(AppError::runtime(format!(
                "no WeChat account directory found in {}",
                roots[0].display()
            )));
        }
        Ok(crate::contract::DbAccounts { accounts })
    }

    pub fn db_test(&self) -> AppResult<crate::contract::DbConnection> {
        let (account_dir, key, _) = self.connection_inputs()?;
        let mut wcdb = weflow_native::wcdb::Wcdb::new();
        wcdb.test_connection(&account_dir, &key)
            .map_err(|err| AppError::native(err.to_string()))?;
        Ok(crate::contract::DbConnection {
            account_dir,
            connected: true,
        })
    }

    pub(super) fn open_wcdb(&self) -> AppResult<weflow_native::wcdb::Wcdb> {
        let (account_dir, key, wxid) = self.connection_inputs()?;
        let mut wcdb = weflow_native::wcdb::Wcdb::new();
        wcdb.open_unchecked(&account_dir, &key, wxid.as_deref())
            .map_err(|err| AppError::native(err.to_string()))?;
        // Proving the key costs a slow key derivation on session.db. Once it has worked, a fingerprint of
        // (key, session.db salt, account) is remembered, and later runs with the same key and database skip it.
        let fingerprint = wcdb
            .session_salt()
            .map(|salt| key_fingerprint(&account_dir, &key, &salt));
        let path = self.ctx.cache_dir().join("verified-keys");
        let known = fingerprint.as_ref().is_some_and(|fp| {
            std::fs::read_to_string(&path).is_ok_and(|s| s.lines().any(|l| l == fp))
        });
        if !known {
            wcdb.check_key()
                .map_err(|err| AppError::native(err.to_string()))?;
            if let Some(fp) = fingerprint {
                remember_fingerprint(&path, &fp);
            }
        }
        Ok(wcdb)
    }

    /// The configured account's folder (the one holding `db_storage`).
    pub fn account_dir(&self) -> AppResult<PathBuf> {
        self.connection_inputs().map(|(dir, _, _)| dir)
    }

    /// Opens the databases a chat page reads, so the first reads do not wait for their keys to be derived; returns
    /// how many opened. The databases stay open for the whole process.
    pub fn warm_chat_databases(&self) -> AppResult<usize> {
        self.open_wcdb()?
            .warm_chat_databases()
            .map_err(|e| AppError::native(e.to_string()))
    }

    pub(super) fn connection_inputs(&self) -> AppResult<(PathBuf, String, Option<String>)> {
        let profile = self.profile()?;
        let db_path = profile
            .db_path
            .clone()
            .ok_or_else(|| AppError::config("missing db_path; run config set db_path"))?;
        let key = profile
            .decrypt_key
            .clone()
            .ok_or_else(|| AppError::config("missing decrypt_key; run config set decrypt_key"))?;
        let wxid = profile.wxid.clone();
        let account_dir = match &wxid {
            Some(wxid) => resolve_account_dir(&db_path, wxid),
            None => PathBuf::from(db_path),
        };
        Ok((account_dir, key, wxid))
    }

    pub(super) fn profile(&self) -> AppResult<&ProfileConfig> {
        self.config
            .profile(Some(&self.profile_name))
            .ok_or_else(|| AppError::config(format!("profile not found: {}", self.profile_name)))
    }

    pub fn profile_name(&self) -> &str {
        &self.profile_name
    }

    /// The account's wxid as the profile has it: the id, or the account folder's name with its suffix.
    pub(super) fn configured_wxid(&self) -> Option<String> {
        self.profile().ok()?.wxid.clone()
    }

    /// The WeChat data directory (or account folder) of the profile.
    pub(super) fn configured_db_path(&self) -> Option<String> {
        self.profile().ok()?.db_path.clone()
    }
}

/// Account folders of a WeChat data directory (or the folder itself when it is one), with their wxid.
fn find_accounts(root: &Path) -> Vec<(String, PathBuf)> {
    let wxid_of = |path: &Path| {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string());
        clean_account_dir_name(&name.unwrap_or_default())
    };
    if crate::config::is_account_dir(root) {
        return vec![(wxid_of(root), root.to_path_buf())];
    }
    let mut accounts: Vec<(String, PathBuf)> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && crate::config::is_account_dir(p))
        .map(|p| (wxid_of(&p), p))
        .collect();
    accounts.sort();
    accounts
}

fn default_db_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = dirs::home_dir() {
        if cfg!(target_os = "macos") {
            candidates.push(home.join("Library/Containers/com.tencent.xinWeChat/Data/Library/Application Support/com.tencent.xinWeChat"));
            candidates.push(home.join("Library/Application Support/com.tencent.xinWeChat"));
            candidates.push(home.join("Library/Containers/com.tencent.WeChat/Data/Library/Application Support/com.tencent.WeChat"));
        } else if cfg!(target_os = "windows") {
            candidates.push(home.join("Documents").join("WeChat Files"));
            candidates.push(home.join("Documents").join("xwechat_files"));
        } else {
            candidates.push(home.join(".xwechat_files"));
            candidates.push(home.join("xwechat_files"));
        }
    }
    candidates
}

/// One-way fingerprint of a key that decrypted an account's `session.db` (the key cannot be recovered from it).
fn key_fingerprint(account_dir: &Path, key: &str, salt: &[u8; 16]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"weflow verified key v1\0");
    h.update(key.trim().to_ascii_lowercase().as_bytes());
    h.update([0]);
    h.update(salt);
    h.update([0]);
    h.update(account_dir.to_string_lossy().as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Add a fingerprint to the list of verified keys (a few recent ones are kept). Failing to write only means the
/// key is checked again next time.
fn remember_fingerprint(path: &Path, fingerprint: &str) {
    let mut lines: Vec<String> = std::fs::read_to_string(path)
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default();
    lines.retain(|l| l != fingerprint && l.len() == 64);
    lines.push(fingerprint.to_string());
    let keep = &lines[lines.len().saturating_sub(16)..];
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, keep.join("\n") + "\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_account_folders_and_strips_the_suffix() {
        let root = weflow_native::fixture::TestDir::new("weflow-wxid");
        std::fs::create_dir_all(root.join("wxid_abc123_ab12/db_storage")).unwrap();
        std::fs::create_dir_all(root.join("someone_a1b2/db_storage")).unwrap();
        std::fs::create_dir_all(root.join("all_users")).unwrap();
        let found: Vec<String> = find_accounts(&root).into_iter().map(|a| a.0).collect();
        assert_eq!(found, ["someone", "wxid_abc123"]);
        // the account folder itself works too
        let one = find_accounts(&root.join("wxid_abc123_ab12"));
        assert_eq!(one[0].0, "wxid_abc123");
    }
}
