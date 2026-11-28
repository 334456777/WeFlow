//! 账号是谁、文件在哪里、联系人叫什么：各领域的服务都要用，放在这里，各领域不必为此互相调用。

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use super::*;
use crate::sns::CachedContact;

impl ServiceHub {
    // ── 账号 ──

    pub(super) fn my_wxid_cleaned(&self) -> String {
        clean_account_dir_name(&self.configured_wxid().unwrap_or_default())
    }

    pub(super) fn wmy(&self) -> String {
        self.my_wxid_cleaned()
    }

    /// 只按档案找到的账号目录，不需要数据库密钥：媒体、图片密钥和取密钥的功能从这里读微信的文件。
    ///
    /// 档案没有数据库路径或账号，或者目录不存在时返回配置错误。打开数据库所用的目录见 [`Self::account_dir`]。
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

    // ── 路径 ──

    /// 缓存目录；不存在时在这里创建，只有所有者能访问：放进去的解密后的图片、语音等文件沿用目录的保护（#812）。
    pub(super) fn cache_base(&self) -> PathBuf {
        let base = self
            .profile()
            .ok()
            .and_then(|p| p.cache_path.clone())
            .filter(|p| !p.trim().is_empty())
            .map(|p| crate::config::expand_home(&p))
            .unwrap_or_else(|| self.ctx.cache_dir());
        // 在 `sudo` 下运行时，调用者的缓存目录仍归调用者所有（#855）
        crate::sudo_caller::adopt(&base);
        if !base.exists() {
            // 建不了目录时不在这里报错，由往里写文件的地方报告
            let _ = crate::private_files::create_dir_all(&base);
        }
        base
    }

    // ── 联系人 ──

    /// `usernames` 中每个人在账号里的显示名，取自数据库（#1017）：备注，没有时依次用昵称、微信号和用户名本身。
    /// 读不出联系人时为空。
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

    /// [`Self::contact_names`]，以及有头像的人的头像地址。
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

    /// 把 [`Self::names_and_avatars`] 合成每个用户名一项。
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

    /// 群聊里 `id → 群昵称` 的原始对应表；数据库打不开或读不出时为空。
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

/// 数据库门面（facade）回答的 `{ username: text }`，去掉空的文字；读不出时为空。
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
