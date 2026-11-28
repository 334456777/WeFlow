//! Who the account is, where its files are, and what its contacts are called: what every domain's service asks, kept
//! here so that the domains do not call each other for it.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use super::*;
use crate::sns::CachedContact;

impl ServiceHub {
    // ── the account ──

    pub(super) fn my_wxid_cleaned(&self) -> String {
        clean_account_dir_name(&self.configured_wxid().unwrap_or_default())
    }

    pub(super) fn wmy(&self) -> String {
        self.my_wxid_cleaned()
    }

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

    // ── paths ──

    /// The cache folder, made here when it is not there yet so that it is its owner's only: what goes in it, such as
    /// decrypted images and voices, takes its folder's protection (#812).
    pub(super) fn cache_base(&self) -> PathBuf {
        let base = self
            .profile()
            .ok()
            .and_then(|p| p.cache_path.clone())
            .filter(|p| !p.trim().is_empty())
            .map(|p| crate::config::expand_home(&p))
            .unwrap_or_else(|| self.ctx.cache_dir());
        // under `sudo`, a cache folder of the caller's goes on being the caller's (#855)
        crate::sudo_caller::adopt(&base);
        if !base.exists() {
            // the parts that write there report a folder they cannot make
            let _ = crate::private_files::create_dir_all(&base);
        }
        base
    }

    // ── contacts ──

    /// The names and avatars of `usernames`, as the account's databases give them (#1017).
    pub(super) fn contact_book(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        usernames: &[String],
    ) -> HashMap<String, CachedContact> {
        let mut book: HashMap<String, CachedContact> = HashMap::new();
        if usernames.is_empty() {
            return book;
        }
        if let Ok(Value::Object(names)) = wcdb.display_names(usernames) {
            for (k, v) in names {
                if let Some(n) = v.as_str().filter(|n| !n.is_empty()) {
                    book.entry(k).or_default().display_name = Some(n.to_string());
                }
            }
        }
        if let Ok(Value::Object(urls)) = wcdb.avatar_urls(usernames) {
            for (k, v) in urls {
                if let Some(u) = v.as_str().filter(|u| !u.is_empty()) {
                    book.entry(k).or_default().avatar_url = Some(u.to_string());
                }
            }
        }
        book
    }

    /// Display names and avatar URLs of `usernames`, each map holding the ones the account knows.
    pub(super) fn names_and_avatars_pub(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        usernames: &[String],
    ) -> (HashMap<String, String>, HashMap<String, String>) {
        let to_map = |v: anyhow::Result<Value>| -> HashMap<String, String> {
            v.ok()
                .and_then(|v| v.as_object().cloned())
                .map(|m| {
                    m.into_iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                        .collect()
                })
                .unwrap_or_default()
        };
        if usernames.is_empty() {
            return (HashMap::new(), HashMap::new());
        }
        (
            to_map(wcdb.display_names(usernames)),
            to_map(wcdb.avatar_urls(usernames)),
        )
    }

    pub(super) fn names_and_avatars(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        usernames: &[String],
    ) -> (HashMap<String, String>, HashMap<String, String>) {
        let to_map = |v: anyhow::Result<Value>| -> HashMap<String, String> {
            v.ok()
                .and_then(|v| v.as_object().cloned())
                .map(|m| {
                    m.into_iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                        .collect()
                })
                .unwrap_or_default()
        };
        (
            to_map(wcdb.display_names(usernames)),
            to_map(wcdb.avatar_urls(usernames)),
        )
    }

    pub(super) fn session_display_names(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        ids: &[String],
    ) -> HashMap<String, String> {
        let mut out = HashMap::new();
        if ids.is_empty() {
            return out;
        }
        let map = wcdb.display_names(ids).ok();
        if let Some(map) = map {
            for (k, v) in map.as_object().into_iter().flatten() {
                if let Some(name) = v.as_str().filter(|s| !s.is_empty()) {
                    out.insert(k.clone(), name.to_string());
                }
            }
        }
        out
    }
}
