//! Contacts, display names, avatars and group membership on top of `contact.db`.
//!
//! Layout (verified on a real account): `contact.id == name2id.rowid`, `chat_room.id` is the room's
//! contact id, `chatroom_member(room_id, member_id)` links `chat_room.id` to member contact ids, and
//! `chat_room.owner` holds the owner's username. Per-member group nicknames live in the protobuf-like
//! `chat_room.ext_buffer`.

use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::ToSql;
use serde_json::{json, Map, Value};

use crate::native_db::NativeAccount;

/// Accounts that are not people, excluded from "private" counts.
const NOT_PRIVATE: &[&str] = &["medianote", "floatbottle", "qmessage", "qqmail", "fmessage"];
/// Rows per `IN (...)` query (well below SQLite's variable limit).
const CHUNK: usize = 400;

fn text(row: &Value, key: &str) -> String {
    row.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn int(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn first_non_empty(row: &Value, keys: &[&str]) -> String {
    keys.iter().map(|k| text(row, k)).find(|s| !s.is_empty()).unwrap_or_default()
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// Distinct, non-empty usernames from a JSON array (or a bare string).
pub fn usernames_from_json(payload: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let items: Vec<String> = match serde_json::from_str::<Value>(payload) {
        // ids can also arrive as numbers (numeric session ids)
        Ok(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        Ok(Value::String(s)) => vec![s.trim().to_string()],
        _ => Vec::new(),
    };
    items.into_iter().filter(|u| !u.is_empty() && seen.insert(u.clone())).collect()
}

fn read_varint(buf: &[u8], mut i: usize) -> Option<(usize, usize)> {
    let (mut value, mut shift) = (0usize, 0u32);
    while i < buf.len() && shift <= 28 {
        let b = buf[i];
        value |= ((b & 0x7f) as usize) << shift;
        i += 1;
        if b & 0x80 == 0 {
            return Some((value, i));
        }
        shift += 7;
    }
    None
}

fn is_member_id(id: &str) -> bool {
    let n = id.chars().count();
    if !(4..=80).contains(&n) || id.contains("@chatroom") {
        return false;
    }
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@' | '-'))
}

/// Extract `username -> group nickname` pairs from a `chat_room.ext_buffer`.
///
/// The buffer is a protobuf-style stream whose member entries look like
/// `0x0a <len> <username> 0x12 <len> <nickname> ...`; like the desktop app we scan for that shape
/// instead of relying on the (undocumented) outer message layout.
pub fn parse_group_nicknames(buf: &[u8]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut i = 0;
    while i + 2 < buf.len() {
        if buf[i] != 0x0a {
            i += 1;
            continue;
        }
        let Some((id_len, id_start)) = read_varint(buf, i + 1) else {
            i += 1;
            continue;
        };
        let id_end = id_start + id_len;
        if id_len == 0 || id_len > 96 || id_end > buf.len() {
            i += 1;
            continue;
        }
        let id = String::from_utf8_lossy(&buf[id_start..id_end]).trim().to_string();
        if !is_member_id(&id) {
            i += 1;
            continue;
        }
        if id_end >= buf.len() || buf[id_end] != 0x12 {
            i = id_end;
            continue;
        }
        let Some((nick_len, nick_start)) = read_varint(buf, id_end + 1) else {
            i = id_end;
            continue;
        };
        let nick_end = nick_start + nick_len;
        if nick_len == 0 || nick_len > 128 || nick_end > buf.len() {
            i = id_end;
            continue;
        }
        let nick: String = String::from_utf8_lossy(&buf[nick_start..nick_end])
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .trim()
            .to_string();
        if !nick.is_empty() {
            out.entry(id).or_insert(nick);
        }
        i = nick_end;
    }
    out
}

impl NativeAccount {
    fn contact_db(&self) -> std::path::PathBuf {
        self.db_path("contact/contact.db")
    }

    /// Contact rows (plus `stranger` rows for ids missing from `contact`) for the given usernames.
    fn contact_rows(&self, usernames: &[String]) -> Result<Vec<Value>> {
        let mut rows = Vec::new();
        let mut found = std::collections::HashSet::new();
        for table in ["contact", "stranger"] {
            let wanted: Vec<&String> = usernames.iter().filter(|u| !found.contains(*u)).collect();
            for chunk in wanted.chunks(CHUNK) {
                let sql = format!("select * from {table} where username in ({})", placeholders(chunk.len()));
                let params: Vec<&dyn ToSql> = chunk.iter().map(|u| *u as &dyn ToSql).collect();
                for r in self.query(&self.contact_db(), &sql, &params)? {
                    found.insert(text(&r, "username"));
                    rows.push(r);
                }
            }
        }
        Ok(rows)
    }

    /// Every row of `contact`.
    pub fn contacts(&self) -> Result<Value> {
        Ok(Value::Array(self.query(&self.contact_db(), "select * from contact", &[])?))
    }

    /// Rows for the given usernames; an empty list means every contact.
    pub fn contacts_compact(&self, usernames: &[String]) -> Result<Value> {
        if usernames.is_empty() {
            return self.contacts();
        }
        Ok(Value::Array(self.contact_rows(usernames)?))
    }

    /// One contact row, or `{}` when unknown.
    pub fn contact(&self, username: &str) -> Result<Value> {
        Ok(self.contact_rows(&[username.to_string()])?.into_iter().next().unwrap_or_else(|| json!({})))
    }

    /// `{ "private": n, "group": n, "official": n, "former_friend": n }`.
    pub fn contact_type_counts(&self) -> Result<Value> {
        let (mut private, mut group, mut official, mut former) = (0, 0, 0, 0);
        let rows = self.query(&self.contact_db(), "select username, local_type, quan_pin from contact", &[])?;
        for r in &rows {
            let username = text(r, "username");
            if username.is_empty() {
                continue;
            }
            if username.ends_with("@chatroom") {
                group += 1;
            } else if username.starts_with("gh_") {
                official += 1;
            } else if int(r, "local_type") == 1 && !NOT_PRIVATE.contains(&username.as_str()) {
                private += 1;
            } else if int(r, "local_type") == 0 && !text(r, "quan_pin").is_empty() {
                former += 1;
            }
        }
        Ok(json!({ "private": private, "group": group, "official": official, "former_friend": former }))
    }

    /// `{ username: name }`: remark, else nickname, else alias, else the username itself.
    pub fn display_names(&self, usernames: &[String]) -> Result<Value> {
        let mut map: Map<String, Value> = Map::new();
        for r in self.contact_rows(usernames)? {
            let u = text(&r, "username");
            let name = first_non_empty(&r, &["remark", "nick_name", "alias"]);
            map.insert(u.clone(), json!(if name.is_empty() { u } else { name }));
        }
        for u in usernames {
            map.entry(u.clone()).or_insert_with(|| json!(u));
        }
        Ok(Value::Object(map))
    }

    /// `{ username: url }` for contacts that have one (big avatar preferred).
    pub fn avatar_urls(&self, usernames: &[String]) -> Result<Value> {
        let mut map = Map::new();
        for r in self.contact_rows(usernames)? {
            let url = first_non_empty(&r, &["big_head_url", "small_head_url"]);
            if !url.is_empty() {
                map.insert(text(&r, "username"), json!(url));
            }
        }
        Ok(Value::Object(map))
    }

    /// `{ username: alias }` for contacts with a WeChat id set.
    pub fn contact_alias_map(&self, usernames: &[String]) -> Result<Value> {
        let mut map = Map::new();
        for r in self.contact_rows(usernames)? {
            let alias = text(&r, "alias");
            if !alias.is_empty() {
                map.insert(text(&r, "username"), json!(alias));
            }
        }
        Ok(Value::Object(map))
    }

    /// `{ username: bool }`: whether the contact is currently a friend.
    pub fn contact_friend_flags(&self, usernames: &[String]) -> Result<Value> {
        let mut map = Map::new();
        for r in self.contact_rows(usernames)? {
            let u = text(&r, "username");
            let friend = int(&r, "local_type") == 1 && !u.ends_with("@chatroom") && !u.starts_with("gh_") && !NOT_PRIVATE.contains(&u.as_str());
            map.insert(u, json!(friend));
        }
        Ok(Value::Object(map))
    }

    /// `{ username: { isFolded, isMuted } }`.
    ///
    /// WeChat keeps these in `contact.flag` bits (folded group chats: bit 28; muted single chats: bit 9)
    /// and, for group chats, in `chat_room_notify` (1 = notifications off).
    pub fn contact_status(&self, usernames: &[String]) -> Result<Value> {
        let mut map = Map::new();
        for r in self.contact_rows(usernames)? {
            let flag = int(&r, "flag");
            let muted = int(&r, "chat_room_notify") == 1 || flag & (1 << 9) != 0;
            let folded = flag & (1 << 28) != 0;
            map.insert(text(&r, "username"), json!({ "isFolded": folded, "isMuted": muted }));
        }
        Ok(Value::Object(map))
    }

    fn room(&self, chatroom_id: &str) -> Result<Option<Value>> {
        let rows = self.query(&self.contact_db(), "select id, username, owner, ext_buffer from chat_room where username = ?1", &[&chatroom_id])?;
        Ok(rows.into_iter().next())
    }

    /// Members of a group: `{ "members": [{ username, nickName, remark, alias, isOwner? }] }`.
    pub fn group_members(&self, chatroom_id: &str) -> Result<Value> {
        let Some(room) = self.room(chatroom_id)? else {
            return Ok(json!({ "members": [] }));
        };
        let owner = text(&room, "owner");
        let rows = self.query(
            &self.contact_db(),
            "select n.username as username, c.nick_name as nick_name, c.remark as remark, c.alias as alias \
             from chatroom_member m join name2id n on n.rowid = m.member_id \
             left join contact c on c.id = m.member_id where m.room_id = ?1 order by m.rowid",
            &[&int(&room, "id")],
        )?;
        let members: Vec<Value> = rows
            .into_iter()
            .map(|r| {
                let u = text(&r, "username");
                let mut o = json!({ "username": u, "nickName": text(&r, "nick_name"), "remark": text(&r, "remark"), "alias": text(&r, "alias") });
                if !owner.is_empty() && u == owner {
                    o["isOwner"] = json!(true);
                }
                o
            })
            .collect();
        Ok(json!({ "members": members }))
    }

    pub fn group_member_count(&self, chatroom_id: &str) -> Result<Value> {
        let Some(room) = self.room(chatroom_id)? else {
            return Ok(json!({ "count": 0 }));
        };
        let rows = self.query(&self.contact_db(), "select count(*) as n from chatroom_member where room_id = ?1", &[&int(&room, "id")])?;
        Ok(json!({ "count": int(&rows[0], "n") }))
    }

    /// `{ chatroom: count }` for each requested group.
    pub fn group_member_counts(&self, chatroom_ids: &[String]) -> Result<Value> {
        let mut map = Map::new();
        for id in chatroom_ids {
            let n = self.group_member_count(id)?["count"].as_i64().unwrap_or(0);
            map.insert(id.clone(), json!(n));
        }
        Ok(Value::Object(map))
    }

    /// `{ username: group nickname }` parsed from the room's `ext_buffer`.
    pub fn group_nicknames(&self, chatroom_id: &str) -> Result<Value> {
        let Some(room) = self.room(chatroom_id)? else {
            return Ok(json!({}));
        };
        let buf = decode_hex(&text(&room, "ext_buffer"));
        Ok(Value::Object(parse_group_nicknames(&buf).into_iter().map(|(k, v)| (k, json!(v))).collect()))
    }

    /// `{ "ext_buffer": "<hex>" }`, empty object when the room is unknown.
    pub fn chat_room_ext_buffer(&self, chatroom_id: &str) -> Result<Value> {
        Ok(match self.room(chatroom_id)? {
            Some(room) => json!({ "ext_buffer": text(&room, "ext_buffer") }),
            None => json!({}),
        })
    }
}

fn decode_hex(s: &str) -> Vec<u8> {
    if s.len() % 2 != 0 {
        return Vec::new();
    }
    (0..s.len() / 2).filter_map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_db::hex;
    use crate::sqlcipher::testutil::{encrypt_db, plain_db_with, KEY, SALT};
    use crate::sqlcipher::PageCipher;
    use rusqlite::{params, Connection};
    use std::path::PathBuf;

    /// One member entry as stored in `ext_buffer`: field 1 wrapping `0x0a id 0x12 nick 0x18 0`.
    fn entry(id: &str, nick: &str) -> Vec<u8> {
        let mut inner = vec![0x0a, id.len() as u8];
        inner.extend(id.as_bytes());
        inner.extend([0x12, nick.len() as u8]);
        inner.extend(nick.as_bytes());
        inner.extend([0x18, 0x00]);
        let mut out = vec![0x0a, inner.len() as u8];
        out.extend(inner);
        out
    }

    fn account(tag: &str) -> (NativeAccount, PathBuf) {
        let dir = std::env::temp_dir().join(format!("weflow-contact-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("contact")).unwrap();
        let mut ext = vec![0x08, 0x01];
        ext.extend(entry("wxid_bob", "Bobby-in-room"));
        ext.extend(entry("wxid_eve", "Eve!"));
        let db = plain_db_with(move |c: &Connection| {
            c.execute_batch(
                "create table contact(id integer primary key, username text, local_type integer, alias text, flag integer, remark text, \
                 nick_name text, quan_pin text, big_head_url text, small_head_url text, chat_room_notify integer, extra_buffer blob);
                 create table stranger(id integer primary key, username text, local_type integer, alias text, flag integer, remark text, \
                 nick_name text, quan_pin text, big_head_url text, small_head_url text, chat_room_notify integer, extra_buffer blob);
                 create table name2id(username text primary key);
                 create table chat_room(id integer primary key, username text, owner text, ext_buffer blob);
                 create table chatroom_member(room_id integer, member_id integer);",
            )
            .unwrap();
            // (id, username, local_type, alias, flag, remark, nick, quan_pin, big, small, notify)
            type Row = (i64, &'static str, i64, &'static str, i64, &'static str, &'static str, &'static str, &'static str, &'static str, i64);
            let rows: [Row; 7] = [
                (1, "wxid_me", 1, "", 0, "", "Me", "me", "", "", 0),
                (2, "wxid_bob", 1, "bb", 0, "Bobby", "Bob", "bob", "http://big", "http://small", 0),
                (3, "wxid_eve", 3, "", 0, "", "Eve", "eve", "", "http://eve-small", 0),
                (4, "gh_news", 5, "", 0, "", "News", "news", "", "", 0),
                (5, "room1@chatroom", 2, "", 1 << 28, "", "Room", "room", "", "", 1),
                (6, "medianote", 1, "", 0, "", "Note", "note", "", "", 0),
                (7, "wxid_old", 0, "", 0, "", "Old", "old", "", "", 0),
            ];
            for r in rows {
                c.execute(
                    "insert into contact(id, username, local_type, alias, flag, remark, nick_name, quan_pin, big_head_url, small_head_url, chat_room_notify) values (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                    params![r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, r.9, r.10],
                )
                .unwrap();
                c.execute("insert into name2id(rowid, username) values (?1, ?2)", params![r.0, r.1]).unwrap();
            }
            c.execute("insert into chat_room(id, username, owner, ext_buffer) values (5, 'room1@chatroom', 'wxid_bob', ?1)", params![ext]).unwrap();
            c.execute_batch("insert into chatroom_member(room_id, member_id) values (5,1),(5,2),(5,3)").unwrap();
        });
        let cipher = PageCipher::derive(&KEY, &SALT);
        std::fs::write(dir.join("contact/contact.db"), encrypt_db(&db, &cipher)).unwrap();
        (NativeAccount::new(&dir, &hex(&KEY)).unwrap(), dir)
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn contact_lookups() {
        let (acct, _d) = account("lookup");
        assert_eq!(acct.contact("wxid_bob").unwrap()["nick_name"], "Bob");
        assert_eq!(acct.contact("nobody").unwrap(), json!({}));
        assert_eq!(acct.contacts().unwrap().as_array().unwrap().len(), 7);
        assert_eq!(acct.contacts_compact(&[]).unwrap().as_array().unwrap().len(), 7);
        let some = acct.contacts_compact(&ids(&["wxid_bob", "gh_news", "nobody"])).unwrap();
        assert_eq!(some.as_array().unwrap().len(), 2);
    }

    #[test]
    fn type_counts_follow_the_desktop_rules() {
        let (acct, _d) = account("counts");
        // wxid_me + wxid_bob (medianote excluded), one group, one official, one former friend (local_type 0 with a pinyin)
        assert_eq!(acct.contact_type_counts().unwrap(), json!({"private": 2, "group": 1, "official": 1, "former_friend": 1}));
    }

    #[test]
    fn names_avatars_aliases_flags_status() {
        let (acct, _d) = account("names");
        let who = ids(&["wxid_bob", "wxid_eve", "nobody"]);
        assert_eq!(acct.display_names(&who).unwrap(), json!({"wxid_bob": "Bobby", "wxid_eve": "Eve", "nobody": "nobody"}));
        // big avatar wins, small is the fallback, none means no entry
        assert_eq!(acct.avatar_urls(&who).unwrap(), json!({"wxid_bob": "http://big", "wxid_eve": "http://eve-small"}));
        assert_eq!(acct.contact_alias_map(&who).unwrap(), json!({"wxid_bob": "bb"}));
        assert_eq!(
            acct.contact_friend_flags(&ids(&["wxid_bob", "wxid_eve", "room1@chatroom"])).unwrap(),
            json!({"wxid_bob": true, "wxid_eve": false, "room1@chatroom": false})
        );
        let st = acct.contact_status(&ids(&["room1@chatroom", "wxid_bob"])).unwrap();
        assert_eq!(st["room1@chatroom"], json!({"isFolded": true, "isMuted": true}));
        assert_eq!(st["wxid_bob"], json!({"isFolded": false, "isMuted": false}));
    }

    #[test]
    fn group_membership_owner_and_nicknames() {
        let (acct, _d) = account("group");
        let m = acct.group_members("room1@chatroom").unwrap();
        let members = m["members"].as_array().unwrap();
        let names: Vec<&str> = members.iter().map(|x| x["username"].as_str().unwrap()).collect();
        assert_eq!(names, ["wxid_me", "wxid_bob", "wxid_eve"]);
        assert_eq!(members[1]["isOwner"], true);
        assert!(members[0].get("isOwner").is_none());
        assert_eq!(members[1]["remark"], "Bobby");
        assert_eq!(acct.group_member_count("room1@chatroom").unwrap(), json!({"count": 3}));
        assert_eq!(acct.group_member_counts(&ids(&["room1@chatroom", "x@chatroom"])).unwrap(), json!({"room1@chatroom": 3, "x@chatroom": 0}));
        assert_eq!(acct.group_nicknames("room1@chatroom").unwrap(), json!({"wxid_bob": "Bobby-in-room", "wxid_eve": "Eve!"}));
        assert!(!acct.chat_room_ext_buffer("room1@chatroom").unwrap()["ext_buffer"].as_str().unwrap().is_empty());
        assert_eq!(acct.group_members("x@chatroom").unwrap(), json!({"members": []}));
        assert_eq!(acct.group_nicknames("x@chatroom").unwrap(), json!({}));
        assert_eq!(acct.chat_room_ext_buffer("x@chatroom").unwrap(), json!({}));
    }

    #[test]
    fn nickname_parser_is_robust() {
        assert!(parse_group_nicknames(&[]).is_empty());
        assert!(parse_group_nicknames(&[0x0a, 0xff, 0xff, 0xff, 0xff, 0x0f]).is_empty());
        // truncated entry, then a good one; the first occurrence of an id wins
        let mut buf = vec![0x0a, 0x08, b'w', b'x'];
        buf.extend(entry("wxid_a", "first"));
        buf.extend(entry("wxid_a", "second"));
        let got = parse_group_nicknames(&buf);
        assert_eq!(got.get("wxid_a").map(String::as_str), Some("first"));
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn username_payloads() {
        assert_eq!(usernames_from_json(r#"[" a ","b","a",""]"#), ["a", "b"]);
        assert_eq!(usernames_from_json(r#""solo""#), ["solo"]);
        assert!(usernames_from_json("null").is_empty());
        assert!(usernames_from_json("not json").is_empty());
    }
}
