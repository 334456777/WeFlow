//! 账号和它的数据库：微信把数据放在哪里、有哪些账号、配置的档案，以及用密钥打开数据库。

use std::path::{Path, PathBuf};

use super::ServiceHub;
use crate::config::{clean_account_dir_name, resolve_account_dir, ProfileConfig};
use crate::error::{AppError, AppResult};

impl ServiceHub {
    /// 本系统上微信默认存放数据目录的位置（`db detect`）。
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

    /// `root` 下一层的账号目录（`db scan`），即含 `db_storage` 文件夹的目录。
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

    /// 数据目录中找到的每个账号的 wxid，写成 `config set wxid` 要求的形式（去掉目录名的 `_ab12` 后缀）。
    ///
    /// 数据目录取 `root`；没有给 `root` 时取配置的 `db_path`，也没有配置时取存在的各个默认位置。
    ///
    /// # Errors
    ///
    /// 找不到数据目录时返回配置错误；数据目录中没有账号目录时返回运行时错误。
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

    /// 检查配置的账号的数据库能否用它的密钥打开（`db test`）。
    ///
    /// # Errors
    /// 档案不存在、没有设置数据目录或密钥时返回配置错误；找不到数据库或密钥打不开数据库时返回数据库层的错误。
    pub fn db_test(&self) -> AppResult<crate::contract::DbConnection> {
        let (account_dir, key, _) = self.connection_inputs()?;
        let mut wcdb = weflow_native::wcdb::Wcdb::new();
        wcdb.test_connection(&account_dir, &key)
            .map_err(|err| AppError::native(format!("{err:#}")))?;
        Ok(crate::contract::DbConnection { account_dir })
    }

    pub(super) fn open_wcdb(&self) -> AppResult<weflow_native::wcdb::Wcdb> {
        let (account_dir, key, wxid) = self.connection_inputs()?;
        let mut wcdb = weflow_native::wcdb::Wcdb::new();
        wcdb.open_unchecked(&account_dir, &key, wxid.as_deref())
            .map_err(|err| AppError::native(format!("{err:#}")))?;
        // 校验密钥要在 `session.db` 上做一次很慢的密钥派生。校验通过后记下（密钥、`session.db` 的盐、账号）的指纹，
        // 之后用同样的密钥和数据库运行时跳过校验。
        let fingerprint = wcdb
            .session_salt()
            .map(|salt| key_fingerprint(&account_dir, &key, &salt));
        let path = self.ctx.cache_dir().join("verified-keys");
        let known = fingerprint.as_ref().is_some_and(|fp| {
            std::fs::read_to_string(&path).is_ok_and(|s| s.lines().any(|l| l == fp))
        });
        if !known {
            wcdb.check_key()
                .map_err(|err| AppError::native(format!("{err:#}")))?;
            if let Some(fp) = fingerprint {
                remember_fingerprint(&path, &fp);
            }
        }
        Ok(wcdb)
    }

    /// 配置的账号目录（含 `db_storage` 的目录），即打开数据库所用的目录。
    ///
    /// 要求设置了数据库密钥；档案没有指定账号时就是 `db_path` 本身。不打开数据库、只读微信文件的服务用
    /// `account_dir_only` 找账号目录。
    ///
    /// # Errors
    ///
    /// 档案不存在，或没有 `db_path`、`decrypt_key` 时返回配置错误。
    pub fn account_dir(&self) -> AppResult<PathBuf> {
        self.connection_inputs().map(|(dir, _, _)| dir)
    }

    /// 预先打开聊天页面要读的数据库，返回打开的个数，最初几次读取就不必等待派生密钥。
    ///
    /// 这些数据库在整个进程中保持打开。
    ///
    /// # Errors
    ///
    /// 档案不存在、没有设置数据目录或密钥时返回配置错误；数据库打不开或密钥不对时返回数据库层的错误。
    pub fn warm_chat_databases(&self) -> AppResult<usize> {
        self.open_wcdb()?
            .warm_chat_databases()
            .map_err(|e| AppError::native(format!("{e:#}")))
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

    /// 读取的档案的名字。
    pub fn profile_name(&self) -> &str {
        &self.profile_name
    }

    /// 档案中写的账号 wxid：裸 wxid，或者带后缀的账号目录名。
    pub(super) fn configured_wxid(&self) -> Option<String> {
        self.profile().ok()?.wxid.clone()
    }

    /// 档案中的微信数据目录（或账号目录）。
    pub(super) fn configured_db_path(&self) -> Option<String> {
        self.profile().ok()?.db_path.clone()
    }
}

/// 微信数据目录中的账号目录及其 wxid；`root` 本身就是账号目录时只有它自己。
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
    if let Some(home) = crate::sudo_caller::home_dir() {
        if cfg!(target_os = "macos") {
            // 微信 4 把账号放在它的容器的 Documents 下（#869）。
            candidates.push(
                home.join("Library/Containers/com.tencent.xinWeChat/Data/Documents/xwechat_files"),
            );
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

/// 解密过账号的 `session.db` 的密钥的单向指纹，无法从中反推出密钥。
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

/// 把指纹加进已校验的密钥列表，只保留最近的 16 个。写入失败只会让下次重新校验密钥。
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
        // 直接给账号目录也可以
        let one = find_accounts(&root.join("wxid_abc123_ab12"));
        assert_eq!(one[0].0, "wxid_abc123");
    }
}
