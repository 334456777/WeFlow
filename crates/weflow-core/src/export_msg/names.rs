//! Who sent a message and what to call them: contacts, the name book, the display preference and group
//! nicknames.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct ContactInfo {
    pub username: String,
    pub nickname: String,
    pub remark: String,
    pub alias: String,
}

impl ContactInfo {
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

    /// What WCDB's display-name lookup yields: remark, else nickname.
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

pub struct NameBook<'a> {
    cache: HashMap<String, Option<ContactInfo>>,
    loader: Loader<'a>,
}

impl<'a> NameBook<'a> {
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

    pub fn get(&mut self, username: &str) -> Option<ContactInfo> {
        if let Some(found) = self.cache.get(username) {
            return found.clone();
        }
        let loaded = (self.loader)(username);
        self.cache.insert(username.to_string(), loaded.clone());
        loaded
    }

    pub fn display_name(&mut self, username: &str) -> String {
        match self.get(username) {
            Some(c) => c.display_name(),
            None => username.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayPref {
    GroupNickname,
    Remark,
    Nickname,
}

impl DisplayPref {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "group-nickname" => Some(Self::GroupNickname),
            "remark" => Some(Self::Remark),
            "nickname" => Some(Self::Nickname),
            _ => None,
        }
    }

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

pub fn normalize_group_nickname(value: &str) -> String {
    let cleaned: String = value
        .trim()
        .chars()
        .filter(|c| !(*c <= '\u{1f}' || *c == '\u{7f}'))
        .collect();
    if cleaned.is_empty() {
        return String::new();
    }
    if cleaned.chars().all(|c| ",\"'“”‘’，、".contains(c)) {
        return String::new();
    }
    cleaned
}

/// Keep only identities whose nickname is unambiguous (`buildTrustedGroupNicknameMap`).
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

#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub wxid: String,
    pub nickname: String,
    pub remark: String,
    pub alias: String,
    pub group_nickname: String,
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
}
