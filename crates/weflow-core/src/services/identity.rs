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

    /// The account's folder from the profile alone, without the database key: where the media, the image keys and the
    /// key helpers read WeChat's files. A config error when the profile names no database path or account, or the
    /// folder is not there. [`Self::account_dir`] is the folder the databases are opened from.
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

    /// What the account calls each of `usernames`, as its databases give it (#1017): the remark, else the nickname,
    /// else the WeChat ID, else the username itself. Empty when the contacts cannot be read.
    pub(super) fn contact_names(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        usernames: &[String],
    ) -> HashMap<String, String> {
        if usernames.is_empty() {
            return HashMap::new();
        }
        texts(wcdb.display_names(usernames))
    }

    /// [`Self::contact_names`], and the address of the avatar of each that has one.
    pub(super) fn names_and_avatars(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        usernames: &[String],
    ) -> (HashMap<String, String>, HashMap<String, String>) {
        if usernames.is_empty() {
            return (HashMap::new(), HashMap::new());
        }
        (
            texts(wcdb.display_names(usernames)),
            texts(wcdb.avatar_urls(usernames)),
        )
    }

    /// [`Self::names_and_avatars`] as one entry for each username.
    pub(super) fn contact_book(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        usernames: &[String],
    ) -> HashMap<String, CachedContact> {
        let (names, avatars) = self.names_and_avatars(wcdb, usernames);
        let mut book: HashMap<String, CachedContact> = HashMap::new();
        for (username, name) in names {
            book.entry(username).or_default().display_name = Some(name);
        }
        for (username, url) in avatars {
            book.entry(username).or_default().avatar_url = Some(url);
        }
        book
    }

    /// Raw `id → group nickname` map of a chatroom.
    pub fn group_nicknames_pub(&self, chatroom_id: &str) -> Vec<(String, String)> {
        let Ok(wcdb) = self.open_wcdb() else {
            return Vec::new();
        };
        match wcdb.group_nicknames(chatroom_id) {
            Ok(Value::Object(m)) => m
                .into_iter()
                .map(|(k, v)| (k, v.as_str().unwrap_or("").to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// The `{ username: text }` the facade answered, without empty texts; empty when it could not read them.
fn texts(answer: anyhow::Result<Value>) -> HashMap<String, String> {
    let Ok(Value::Object(map)) = answer else {
        return HashMap::new();
    };
    map.into_iter()
        .filter_map(|(username, text)| match text {
            Value::String(text) if !text.is_empty() => Some((username, text)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn texts_keep_the_names_given_and_drop_the_empty_ones() {
        let answer = json!({ "wxid_a": "Alice", "wxid_b": "", "wxid_c": 3 });
        let got = texts(Ok(answer));
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got["wxid_a"], "Alice");
        assert!(texts(Err(anyhow::anyhow!("no contact database"))).is_empty());
        assert!(texts(Ok(json!([]))).is_empty());
    }
}
