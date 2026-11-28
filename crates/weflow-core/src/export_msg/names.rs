//! Who sent a message and what to call them: contacts, the name book, the display preference and group
//! nicknames.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

/// The names of a contact; a field is empty when the contact has none.
#[derive(Clone, Debug, Default)]
pub struct ContactInfo {
    /// The wxid.
    pub username: String,
    /// The name they gave themselves.
    pub nickname: String,
    /// My remark for them.
    pub remark: String,
    /// The WeChat ID.
    pub alias: String,
}

impl ContactInfo {
    /// The names of the contact row `v`, by its `snake_case` or `camelCase` keys; `username` when the row names none.
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

    /// What WCDB's display-name lookup yields: the remark, else the nickname, else the alias, else the username.
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

type Loader<'a> = Box<dyn FnMut(&str) -> Option<ContactInfo> + 'a>;

/// The contacts an export names, each looked up once by its loader and kept.
pub struct NameBook<'a> {
    cache: HashMap<String, Option<ContactInfo>>,
    loader: Loader<'a>,
}

/// The contacts looked up so far; the loader is a closure.
impl std::fmt::Debug for NameBook<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NameBook")
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

impl<'a> NameBook<'a> {
    /// A book that looks a contact up with `loader` the first time it is asked for.
    pub fn new(loader: impl FnMut(&str) -> Option<ContactInfo> + 'a) -> Self {
        NameBook {
            cache: HashMap::new(),
            loader: Box::new(loader),
        }
    }

    #[cfg(test)]
    pub(super) fn from_map(map: HashMap<String, ContactInfo>) -> NameBook<'static> {
        NameBook::new(move |u| map.get(u).cloned())
    }

    /// The contact `username`; `None` when the loader knows none, which is kept too.
    pub fn get(&mut self, username: &str) -> Option<ContactInfo> {
        if let Some(found) = self.cache.get(username) {
            return found.clone();
        }
        let loaded = (self.loader)(username);
        self.cache.insert(username.to_string(), loaded.clone());
        loaded
    }

    /// [`ContactInfo::display_name`] of `username`; `username` itself for one the loader does not know.
    pub fn display_name(&mut self, username: &str) -> String {
        match self.get(username) {
            Some(c) => c.display_name(),
            None => username.to_string(),
        }
    }
}

/// Which name an export shows for a sender (`export messages --display-name`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayPref {
    /// The group nickname, else the remark, else the nickname, else the wxid.
    GroupNickname,
    /// The remark, else the nickname, else the wxid.
    Remark,
    /// The nickname, else the wxid.
    Nickname,
}

impl DisplayPref {
    /// `group-nickname`, `remark` or `nickname`; `None` for anything else.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "group-nickname" => Some(Self::GroupNickname),
            "remark" => Some(Self::Remark),
            "nickname" => Some(Self::Nickname),
            _ => None,
        }
    }

    /// The first name that is not empty, in the preference's order.
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

/// A group nickname trimmed and without control characters; empty when nothing but quotes, commas and `@` is left.
/// The exports, the chat window and the group panels read members' nicknames by this one rule (#1266).
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

/// Keep only identities whose nickname is unambiguous.
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

/// The group nickname of the identities `candidates` in `map`: empty when none has one, or when two have different
/// ones.
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

/// The names of a message's sender, as an export writes them.
#[derive(Clone, Debug, Default)]
pub struct Profile {
    /// The sender's wxid; `unknown` when the message does not say.
    pub wxid: String,
    /// Their nickname, else the name the message gives, else the wxid.
    pub nickname: String,
    /// My remark for them; empty when none.
    pub remark: String,
    /// Their WeChat ID; empty when none.
    pub alias: String,
    /// Their nickname in the group; empty when none, or in a private chat.
    pub group_nickname: String,
    /// The name the export shows, by its [`DisplayPref`].
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
