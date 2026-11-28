//! 消息是谁发的、该怎么称呼：联系人、名字簿、显示偏好和群昵称。

use std::collections::{HashMap, HashSet};

use serde_json::Value;

/// 联系人的各个名字；联系人没有的字段为空。
#[derive(Clone, Debug, Default)]
pub struct ContactInfo {
    /// 用户名（wxid）。
    pub username: String,
    /// 联系人给自己起的昵称。
    pub nickname: String,
    /// 账号所有者给联系人的备注。
    pub remark: String,
    /// 微信号。
    pub alias: String,
}

impl ContactInfo {
    /// 从联系人的行 `v` 读出各个名字，键可以是 `snake_case` 或 `camelCase`；行里没有用户名时用 `username`。
    pub fn from_value(username: &str, v: &Value) -> Self {
        let s = |keys: &[&str]| {
            keys.iter()
                .filter_map(|k| v.get(*k).and_then(Value::as_str))
                .find(|x| !x.is_empty())
                .unwrap_or("")
                .to_string()
        };
        ContactInfo {
            username: {
                let u = s(&["username", "userName"]);
                if u.is_empty() {
                    username.to_string()
                } else {
                    u
                }
            },
            nickname: s(&["nickName", "nick_name", "nickname"]),
            remark: s(&["remark"]),
            alias: s(&["alias"]),
        }
    }

    /// 与 WCDB 取显示名的结果相同：依次取备注、昵称、微信号，都没有时为用户名。
    pub fn display_name(&self) -> String {
        if !self.remark.is_empty() {
            self.remark.clone()
        } else if !self.nickname.is_empty() {
            self.nickname.clone()
        } else if !self.alias.is_empty() {
            self.alias.clone()
        } else {
            self.username.clone()
        }
    }
}

/// 按用户名查联系人的加载函数。
type Loader<'a> = Box<dyn FnMut(&str) -> Option<ContactInfo> + 'a>;

/// 导出时要取名字的联系人：每个只用加载函数查一次，查到的结果留下来。
pub struct NameBook<'a> {
    cache: HashMap<String, Option<ContactInfo>>,
    loader: Loader<'a>,
}

/// 只显示已经查过的联系人；加载函数是闭包，不显示。
impl std::fmt::Debug for NameBook<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NameBook")
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

impl<'a> NameBook<'a> {
    /// 名字簿：联系人第一次被问到时用 `loader` 查找。
    pub fn new(loader: impl FnMut(&str) -> Option<ContactInfo> + 'a) -> Self {
        NameBook {
            cache: HashMap::new(),
            loader: Box::new(loader),
        }
    }

    /// 测试用：从现成的表里查联系人的名字簿。
    #[cfg(test)]
    pub(super) fn from_map(map: HashMap<String, ContactInfo>) -> NameBook<'static> {
        NameBook::new(move |u| map.get(u).cloned())
    }

    /// 联系人 `username`；加载函数不知道时为 `None`，这个结果也会留下来。
    pub fn get(&mut self, username: &str) -> Option<ContactInfo> {
        if let Some(found) = self.cache.get(username) {
            return found.clone();
        }
        let loaded = (self.loader)(username);
        self.cache.insert(username.to_string(), loaded.clone());
        loaded
    }

    /// `username` 的 [`ContactInfo::display_name`]；加载函数不知道这个联系人时为 `username` 本身。
    pub fn display_name(&mut self, username: &str) -> String {
        match self.get(username) {
            Some(c) => c.display_name(),
            None => username.to_string(),
        }
    }
}

/// 导出时发送者显示哪个名字（`export messages --display-name`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayPref {
    /// 依次取群昵称、备注、昵称、wxid。
    GroupNickname,
    /// 依次取备注、昵称、wxid。
    Remark,
    /// 先取昵称，没有时取 wxid。
    Nickname,
}

impl DisplayPref {
    /// 读取 `group-nickname`、`remark` 或 `nickname`；其他值为 `None`。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "group-nickname" => Some(Self::GroupNickname),
            "remark" => Some(Self::Remark),
            "nickname" => Some(Self::Nickname),
            _ => None,
        }
    }

    /// 按偏好的顺序取第一个不为空的名字。
    pub fn pick(self, wxid: &str, nickname: &str, remark: &str, group_nickname: &str) -> String {
        let first = |vals: &[&str]| {
            vals.iter()
                .find(|v| !v.is_empty())
                .map(|v| v.to_string())
                .unwrap_or_default()
        };
        match self {
            Self::GroupNickname => first(&[group_nickname, remark, nickname, wxid]),
            Self::Remark => first(&[remark, nickname, wxid]),
            Self::Nickname => first(&[nickname, wxid]),
        }
    }
}

/// 去掉首尾空白和控制字符后的群昵称；只剩引号、逗号、顿号和 `@` 时为空。
/// 导出、聊天窗口和群面板都按这一条规则读取成员的群昵称（#1266）。
pub fn normalize_group_nickname(value: &str) -> String {
    let cleaned: String = value
        .trim()
        .chars()
        .filter(|c| !(*c <= '\u{1f}' || *c == '\u{7f}'))
        .collect();
    if cleaned.is_empty() {
        return String::new();
    }
    if cleaned.chars().all(|c| ",\"'“”‘’，、@".contains(c)) {
        return String::new();
    }
    cleaned
}

/// 只保留群昵称没有歧义的身份：身份不分大小写，规范化（见 [`normalize_group_nickname`]）后只有一种群昵称。
pub fn build_trusted_group_nicknames(
    entries: impl IntoIterator<Item = (String, String)>,
) -> HashMap<String, String> {
    let mut buckets: HashMap<String, HashSet<String>> = HashMap::new();
    for (id, nick) in entries {
        let identity = id.trim().to_lowercase();
        if identity.is_empty() {
            continue;
        }
        let nickname = normalize_group_nickname(&nick);
        if nickname.is_empty() {
            continue;
        }
        buckets.entry(identity).or_default().insert(nickname);
    }
    buckets
        .into_iter()
        .filter_map(|(k, v)| {
            if v.len() == 1 {
                v.into_iter().next().map(|n| (k, n))
            } else {
                None
            }
        })
        .collect()
}

/// `candidates` 这些身份在 `map` 中的群昵称：都没有，或者有两个不同的群昵称时为空。
pub fn resolve_group_nickname(map: &HashMap<String, String>, candidates: &[&str]) -> String {
    let mut resolved = String::new();
    let mut seen: Vec<&str> = Vec::new();
    for c in candidates {
        let t = c.trim();
        if t.is_empty() || seen.contains(&t) {
            continue;
        }
        seen.push(t);
        let id = t.to_lowercase();
        let nick = normalize_group_nickname(map.get(&id).map(String::as_str).unwrap_or(""));
        if nick.is_empty() {
            continue;
        }
        if resolved.is_empty() {
            resolved = nick;
        } else if resolved != nick {
            return String::new();
        }
    }
    resolved
}

/// 导出写出的消息发送者的各个名字。
#[derive(Clone, Debug, Default)]
pub struct Profile {
    /// 发送者的 wxid；消息里没有写时为 `unknown`。
    pub wxid: String,
    /// 发送者的昵称；没有时取消息给出的名字，再没有时为 wxid。
    pub nickname: String,
    /// 账号所有者给发送者的备注；没有时为空。
    pub remark: String,
    /// 发送者的微信号；没有时为空。
    pub alias: String,
    /// 发送者的群昵称；没有时或在私聊中为空。
    pub group_nickname: String,
    /// 按导出的 [`DisplayPref`] 显示的名字。
    pub display_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_nickname_rules() {
        let map = build_trusted_group_nicknames(vec![
            ("WXID_A".into(), "Alpha".into()),
            ("wxid_a".into(), "Alpha".into()),
            ("wxid_b".into(), "X".into()),
            ("wxid_b".into(), "Y".into()),
            ("wxid_c".into(), "，，".into()),
        ]);
        assert_eq!(resolve_group_nickname(&map, &["wxid_a"]), "Alpha");
        assert_eq!(resolve_group_nickname(&map, &["wxid_b"]), "");
        assert_eq!(resolve_group_nickname(&map, &["wxid_c"]), "");
        assert_eq!(DisplayPref::GroupNickname.pick("w", "n", "r", "g"), "g");
        assert_eq!(DisplayPref::Remark.pick("w", "n", "", "g"), "n");
        assert_eq!(DisplayPref::parse("nickname"), Some(DisplayPref::Nickname));
    }

    #[test]
    fn a_nickname_of_marks_only_is_none() {
        for none in ["", "  ", "@", "@@", "\"'", "“”", "，、", "\u{7}"] {
            assert_eq!(normalize_group_nickname(none), "", "{none:?}");
        }
        for (raw, kept) in [
            (" 小明 ", "小明"),
            ("a\u{7}b", "ab"),
            ("@小明", "@小明"),
            ("'s", "'s"),
        ] {
            assert_eq!(normalize_group_nickname(raw), kept, "{raw:?}");
        }
    }
}
