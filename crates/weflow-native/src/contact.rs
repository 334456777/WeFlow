//! 基于 `contact.db` 的联系人、显示名、头像和群成员。
//!
//! 表的结构（在真实账号上核对过）：`contact.id == name2id.rowid`，`chat_room.id` 是群的联系人 ID，
//! `chatroom_member(room_id, member_id)` 把 `chat_room.id` 和成员的联系人 ID 关联起来，`chat_room.owner`
//! 是群主的用户名。每个成员的群昵称保存在类似 protobuf 的 `chat_room.ext_buffer` 里。

use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::types::ValueRef;
use rusqlite::ToSql;
use serde_json::{json, Map, Value};

use crate::db::Account;

/// 不是个人的账号，不计入 `private` 的数量。
const NOT_PRIVATE: &[&str] = &["medianote", "floatbottle", "qmessage", "qqmail", "fmessage"];
/// 每次 `IN (...)` 查询的行数（远低于 SQLite 的变量个数上限）。
const CHUNK: usize = 400;

fn text(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn int(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn first_non_empty(row: &Value, keys: &[&str]) -> String {
    keys.iter()
        .map(|k| text(row, k))
        .find(|s| !s.is_empty())
        .unwrap_or_default()
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// `names` 中去掉首尾空白后互不相同、不为空的用户名，保持原来的顺序。
pub fn distinct_usernames(names: &[impl AsRef<str>]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    names
        .iter()
        .map(|name| name.as_ref().trim())
        .filter(|name| !name.is_empty() && seen.insert(*name))
        .map(str::to_string)
        .collect()
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
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@' | '-'))
}

/// 从 `chat_room.ext_buffer` 中取出「用户名 → 群昵称」的对应。
///
/// 这段数据是类似 protobuf 的字节流，其中成员的条目形如 `0x0a <len> <username> 0x12 <len> <nickname> ...`；
/// 与桌面版一样按这个形状扫描，不依赖（没有文档的）外层消息结构。
fn parse_group_nicknames(buf: &[u8]) -> BTreeMap<String, String> {
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
        let id = String::from_utf8_lossy(&buf[id_start..id_end])
            .trim()
            .to_string();
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

impl Account {
    fn contact_db(&self) -> std::path::PathBuf {
        self.db_path("contact/contact.db")
    }

    /// 给定用户名的联系人行（`contact` 里没有的再从 `stranger` 里找）。
    fn contact_rows(&self, usernames: &[String]) -> Result<Vec<Value>> {
        let mut rows = Vec::new();
        let mut found = std::collections::HashSet::new();
        for table in ["contact", "stranger"] {
            let wanted: Vec<&String> = usernames.iter().filter(|u| !found.contains(*u)).collect();
            for chunk in wanted.chunks(CHUNK) {
                let sql = format!(
                    "select * from {table} where username in ({})",
                    placeholders(chunk.len())
                );
                let params: Vec<&dyn ToSql> = chunk.iter().map(|u| *u as &dyn ToSql).collect();
                for r in self.query(&self.contact_db(), &sql, &params)? {
                    found.insert(text(&r, "username"));
                    rows.push(r);
                }
            }
        }
        Ok(rows)
    }

    /// `contact` 的全部行。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
    pub fn contacts(&self) -> Result<Value> {
        Ok(Value::Array(self.query(
            &self.contact_db(),
            "select * from contact",
            &[],
        )?))
    }

    /// `contact` 中 `username` 含有 `gh_`、含有不区分 ASCII 大小写的 `weixin` 或 `@openim`，或者不是文字的行，
    /// 按表中的顺序：包括全部公众号、`weixin…` 和 `@openim` 联系人，以及其他由调用方区分的行。
    /// 只把这些行转换成结果，不像 [`contacts`](Self::contacts) 那样转换每个联系人。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
    pub fn contacts_maybe_special(&self) -> Result<Value> {
        Ok(Value::Array(self.query(
            &self.contact_db(),
            "select * from contact where typeof(username) <> 'text' or instr(username, 'gh_') > 0 \
             or instr(lower(username), 'weixin') > 0 or instr(lower(username), '@openim') > 0",
            &[],
        )?))
    }

    /// 给定用户名的联系人行；列表为空时返回全部联系人。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
    pub fn contacts_compact(&self, usernames: &[String]) -> Result<Value> {
        if usernames.is_empty() {
            return self.contacts();
        }
        Ok(Value::Array(self.contact_rows(usernames)?))
    }

    /// 一个联系人的行；不认识这个用户名时为 `{}`。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
    pub fn contact(&self, username: &str) -> Result<Value> {
        Ok(self
            .contact_rows(&[username.to_string()])?
            .into_iter()
            .next()
            .unwrap_or_else(|| json!({})))
    }

    /// 各类联系人的数量 `{ "private": n, "group": n, "official": n, "former_friend": n }`。
    /// 账号自己的行按好友保存（`local_type` 为 1），不计入（#1079）。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
    pub fn contact_type_counts(&self) -> Result<Value> {
        let (mut private, mut group, mut official, mut former) = (0, 0, 0, 0);
        let rows = self.query(
            &self.contact_db(),
            "select username, local_type, quan_pin from contact",
            &[],
        )?;
        for r in &rows {
            let username = text(r, "username");
            if username.is_empty() || self.is_me(&username) {
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
        Ok(
            json!({ "private": private, "group": group, "official": official, "former_friend": former }),
        )
    }

    /// `{ username: name }`：依次取备注、昵称、微信号，都没有时为用户名本身。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
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

    /// 微信在本机为 `username` 保存的头像（`head_image/head_image.db`）：
    /// 图片的字节；没有头像或没有这个数据库时为 `None`。
    ///
    /// # Errors
    ///
    /// 无法读取 `head_image.db`（打开、解密或查询失败）。
    pub fn head_image(&self, username: &str) -> Result<Option<Vec<u8>>> {
        let db = self.db_path("head_image/head_image.db");
        if username.is_empty() || !db.is_file() {
            return Ok(None);
        }
        self.with_db(&db, |conn| {
            let mut stmt =
                conn.prepare("select image_buffer from head_image where username = ?1")?;
            let mut rows = stmt.query([username])?;
            while let Some(row) = rows.next()? {
                // 按存储的字节取图片（zstd 帧先解压）；文字单元格按其中的十六进制解码
                let bytes = match row.get_ref(0)? {
                    ValueRef::Blob(b) => crate::db::inflate_blob(b).into_owned(),
                    ValueRef::Text(t) => std::str::from_utf8(t).map(decode_hex).unwrap_or_default(),
                    ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) => Vec::new(),
                };
                if !bytes.is_empty() {
                    return Ok(Some(bytes));
                }
            }
            Ok(None)
        })
    }

    /// 有头像地址的联系人的 `{ username: url }`（优先取大头像）。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
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

    /// 设了微信号的联系人的 `{ username: alias }`。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
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

    /// 各联系人的折叠、免打扰和置顶状态 `{ username: { isFolded, isMuted, isPinned } }`。
    ///
    /// 微信把它们保存在 `contact.flag` 的位里（折叠的群聊：第 28 位；免打扰的单聊：第 9 位；任何会话的置顶：
    /// 第 11 位，#885），群聊的免打扰还保存在 `chat_room_notify` 里（1 表示关闭通知）。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
    pub fn contact_status(&self, usernames: &[String]) -> Result<Value> {
        let mut map = Map::new();
        for r in self.contact_rows(usernames)? {
            let flag = int(&r, "flag");
            let muted = int(&r, "chat_room_notify") == 1 || flag & (1 << 9) != 0;
            let folded = flag & (1 << 28) != 0;
            let pinned = flag & (1 << 11) != 0;
            map.insert(
                text(&r, "username"),
                json!({ "isFolded": folded, "isMuted": muted, "isPinned": pinned }),
            );
        }
        Ok(Value::Object(map))
    }

    fn room(&self, chatroom_id: &str) -> Result<Option<Value>> {
        let rows = self.query(
            &self.contact_db(),
            "select id, username, owner, ext_buffer from chat_room where username = ?1",
            &[&chatroom_id],
        )?;
        Ok(rows.into_iter().next())
    }

    /// 群的成员，格式与桌面版相同：`[{ username, nickName, remark, alias, avatarUrl?, isOwner? }]`。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
    pub fn group_members(&self, chatroom_id: &str) -> Result<Value> {
        let Some(room) = self.room(chatroom_id)? else {
            return Ok(json!([]));
        };
        let owner = text(&room, "owner");
        let rows = self.query(
            &self.contact_db(),
            "select n.username as username, c.nick_name as nick_name, c.remark as remark, c.alias as alias, \
             coalesce(nullif(c.big_head_url, ''), c.small_head_url) as avatar \
             from chatroom_member m join name2id n on n.rowid = m.member_id \
             left join contact c on c.id = m.member_id where m.room_id = ?1 order by m.rowid",
            &[&int(&room, "id")],
        )?;
        let members: Vec<Value> = rows
            .into_iter()
            .map(|r| {
                let u = text(&r, "username");
                let mut o = json!({ "username": u, "nickName": text(&r, "nick_name"), "remark": text(&r, "remark"), "alias": text(&r, "alias") });
                let avatar = text(&r, "avatar");
                if !avatar.is_empty() {
                    o["avatarUrl"] = json!(avatar);
                }
                if !owner.is_empty() && u == owner {
                    o["isOwner"] = json!(true);
                }
                o
            })
            .collect();
        Ok(Value::Array(members))
    }

    /// `{ "count": n }`：`chatroom_member` 中这个群的成员数；不认识的群为 0。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
    pub fn group_member_count(&self, chatroom_id: &str) -> Result<Value> {
        let Some(room) = self.room(chatroom_id)? else {
            return Ok(json!({ "count": 0 }));
        };
        let rows = self.query(
            &self.contact_db(),
            "select count(*) as n from chatroom_member where room_id = ?1",
            &[&int(&room, "id")],
        )?;
        Ok(json!({ "count": int(&rows[0], "n") }))
    }

    /// 每个给定的群的 `{ chatroom: count }`。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
    pub fn group_member_counts(&self, chatroom_ids: &[String]) -> Result<Value> {
        let mut map = Map::new();
        for id in chatroom_ids {
            let n = self.group_member_count(id)?["count"].as_i64().unwrap_or(0);
            map.insert(id.clone(), json!(n));
        }
        Ok(Value::Object(map))
    }

    /// 从群的 `ext_buffer` 解析出的 `{ username: group nickname }`。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
    pub fn group_nicknames(&self, chatroom_id: &str) -> Result<Value> {
        let Some(room) = self.room(chatroom_id)? else {
            return Ok(json!({}));
        };
        let buf = decode_hex(&text(&room, "ext_buffer"));
        Ok(Value::Object(
            parse_group_nicknames(&buf)
                .into_iter()
                .map(|(k, v)| (k, json!(v)))
                .collect(),
        ))
    }

    /// `{ "ext_buffer": "<hex>" }`；不认识这个群时为空对象。
    ///
    /// # Errors
    ///
    /// 无法读取 `contact.db`（打开、解密或查询失败）。
    pub fn chat_room_ext_buffer(&self, chatroom_id: &str) -> Result<Value> {
        Ok(match self.room(chatroom_id)? {
            Some(room) => json!({ "ext_buffer": text(&room, "ext_buffer") }),
            None => json!({}),
        })
    }
}

/// 十六进制字符串表示的字节，不是十六进制的字符对略过；字符串不是 ASCII 或长度为奇数时为空。
fn decode_hex(s: &str) -> Vec<u8> {
    // 先检查 ASCII：每次取两个字节的切片不能切开一个字符
    if !s.is_ascii() || !s.len().is_multiple_of(2) {
        return Vec::new();
    }
    (0..s.len() / 2)
        .filter_map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

#[cfg(test)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "测试用的值已知放得下，或者本来就要回绕"
)]
mod tests {
    use super::*;
    use crate::db::hex;
    use crate::fixture::TestDir;
    use crate::sqlcipher::testutil::{encrypt_db, plain_db_with, KEY, SALT};
    use crate::sqlcipher::PageCipher;
    use rusqlite::{params, Connection};

    /// `ext_buffer` 中保存的一个成员条目：字段 1 包着 `0x0a id 0x12 nick 0x18 0`。
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

    fn account(tag: &str) -> (TestDir, Account) {
        let dir = TestDir::new(&format!("weflow-contact-{tag}"));
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
            // 各列依次为 (id, username, local_type, alias, flag, remark, nick, quan_pin, big, small, notify)
            type Row = (
                i64,
                &'static str,
                i64,
                &'static str,
                i64,
                &'static str,
                &'static str,
                &'static str,
                &'static str,
                &'static str,
                i64,
            );
            let rows: [Row; 7] = [
                (1, "wxid_me", 1, "", 0, "", "Me", "me", "", "", 0),
                (
                    2,
                    "wxid_bob",
                    1,
                    "bb",
                    0,
                    "Bobby",
                    "Bob",
                    "bob",
                    "http://big",
                    "http://small",
                    0,
                ),
                (
                    3,
                    "wxid_eve",
                    3,
                    "",
                    // 置顶，并且在通讯录里（第 0、1 位），与微信 4.x 保存置顶好友的方式一致
                    (1 << 11) | 0b11,
                    "",
                    "Eve",
                    "eve",
                    "",
                    "http://eve-small",
                    0,
                ),
                (4, "gh_news", 5, "", 0, "", "News", "news", "", "", 0),
                (
                    5,
                    "room1@chatroom",
                    2,
                    "",
                    1 << 28,
                    "",
                    "Room",
                    "room",
                    "",
                    "",
                    1,
                ),
                (6, "medianote", 1, "", 0, "", "Note", "note", "", "", 0),
                (7, "wxid_old", 0, "", 0, "", "Old", "old", "", "", 0),
            ];
            for r in rows {
                c.execute(
                    "insert into contact(id, username, local_type, alias, flag, remark, nick_name, quan_pin, big_head_url, small_head_url, chat_room_notify) values (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                    params![r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, r.9, r.10],
                )
                .unwrap();
                c.execute(
                    "insert into name2id(rowid, username) values (?1, ?2)",
                    params![r.0, r.1],
                )
                .unwrap();
            }
            c.execute("insert into chat_room(id, username, owner, ext_buffer) values (5, 'room1@chatroom', 'wxid_bob', ?1)", params![ext]).unwrap();
            c.execute_batch(
                "insert into chatroom_member(room_id, member_id) values (5,1),(5,2),(5,3)",
            )
            .unwrap();
        });
        let cipher = PageCipher::derive(&KEY, &SALT);
        std::fs::write(dir.join("contact/contact.db"), encrypt_db(&db, &cipher)).unwrap();
        let acct = Account::new(dir.path(), &hex(&KEY)).unwrap();
        (dir, acct)
    }

    /// 可能特殊的联系人，就是全部联系人中用户名含有 `gh_`、含有不区分 ASCII 大小写的 `weixin` 或 `@openim`，
    /// 或者不是文字的那些，按表中的顺序。
    #[test]
    fn contacts_that_may_be_special_are_found_among_all() {
        let dir = TestDir::new("weflow-contact-special");
        std::fs::create_dir_all(dir.join("contact")).unwrap();
        let db = plain_db_with(|c: &Connection| {
            c.execute_batch(
                "create table contact(id integer primary key, username, local_type integer, remark text);
                 insert into contact(username, local_type) values ('wxid_a', 1), ('gh_news', 1), ('GH_upper', 1),
                     (' gh_spaced', 1), ('weixin', 1), ('WeiXinTeam', 1), ('a@openim', 5), ('b@kefu.OpenIM', 5),
                     ('room@chatroom', 2), (null, 0), (42, 1), ('wxid_gh_x', 1),
                     ('中文weixin', 1), ('filehelper', 1);",
            )
            .unwrap();
        });
        let cipher = PageCipher::derive(&KEY, &SALT);
        std::fs::write(dir.join("contact/contact.db"), encrypt_db(&db, &cipher)).unwrap();
        let a = Account::new(dir.path(), &hex(&KEY)).unwrap();
        let all = a.contacts().unwrap();
        let want: Vec<&Value> = all
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| match &r["username"] {
                Value::String(u) => {
                    let l = u.to_ascii_lowercase();
                    u.contains("gh_") || l.contains("weixin") || l.contains("@openim")
                }
                _ => true,
            })
            .collect();
        let found = a.contacts_maybe_special().unwrap();
        assert_eq!(found.as_array().unwrap().iter().collect::<Vec<_>>(), want);
        // `b@kefu.OpenIM` 不含 `@openim`
        assert_eq!(want.len(), 9, "{want:?}");
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn contact_lookups() {
        let (_d, acct) = account("lookup");
        assert_eq!(acct.contact("wxid_bob").unwrap()["nick_name"], "Bob");
        assert_eq!(acct.contact("nobody").unwrap(), json!({}));
        assert_eq!(acct.contacts().unwrap().as_array().unwrap().len(), 7);
        assert_eq!(
            acct.contacts_compact(&[])
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            7
        );
        let some = acct
            .contacts_compact(&ids(&["wxid_bob", "gh_news", "nobody"]))
            .unwrap();
        assert_eq!(some.as_array().unwrap().len(), 2);
    }

    #[test]
    fn type_counts_follow_the_desktop_rules() {
        let (_d, acct) = account("counts");
        // wxid_me 和 wxid_bob（不算 medianote）、一个群、一个公众号、一个曾经的好友（local_type 为 0 且有拼音）
        assert_eq!(
            acct.contact_type_counts().unwrap(),
            json!({"private": 2, "group": 1, "official": 1, "former_friend": 1})
        );
    }

    #[test]
    fn names_avatars_aliases_flags_status() {
        let (_d, acct) = account("names");
        let who = ids(&["wxid_bob", "wxid_eve", "nobody"]);
        assert_eq!(
            acct.display_names(&who).unwrap(),
            json!({"wxid_bob": "Bobby", "wxid_eve": "Eve", "nobody": "nobody"})
        );
        // 优先取大头像，没有时取小头像，都没有时不列出
        assert_eq!(
            acct.avatar_urls(&who).unwrap(),
            json!({"wxid_bob": "http://big", "wxid_eve": "http://eve-small"})
        );
        assert_eq!(
            acct.contact_alias_map(&who).unwrap(),
            json!({"wxid_bob": "bb"})
        );
        let st = acct
            .contact_status(&ids(&["room1@chatroom", "wxid_bob", "wxid_eve"]))
            .unwrap();
        assert_eq!(
            st["room1@chatroom"],
            json!({"isFolded": true, "isMuted": true, "isPinned": false})
        );
        assert_eq!(
            st["wxid_bob"],
            json!({"isFolded": false, "isMuted": false, "isPinned": false})
        );
        // 置顶：flag 的第 11 位，不论旁边还设了哪些位
        assert_eq!(
            st["wxid_eve"],
            json!({"isFolded": false, "isMuted": false, "isPinned": true})
        );
    }

    #[test]
    fn group_membership_owner_and_nicknames() {
        let (_d, acct) = account("group");
        let m = acct.group_members("room1@chatroom").unwrap();
        let members = m.as_array().unwrap();
        let names: Vec<&str> = members
            .iter()
            .map(|x| x["username"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["wxid_me", "wxid_bob", "wxid_eve"]);
        assert_eq!(members[1]["isOwner"], true);
        assert!(members[0].get("isOwner").is_none());
        assert_eq!(members[1]["remark"], "Bobby");
        assert_eq!(
            acct.group_member_count("room1@chatroom").unwrap(),
            json!({"count": 3})
        );
        assert_eq!(
            acct.group_member_counts(&ids(&["room1@chatroom", "x@chatroom"]))
                .unwrap(),
            json!({"room1@chatroom": 3, "x@chatroom": 0})
        );
        assert_eq!(
            acct.group_nicknames("room1@chatroom").unwrap(),
            json!({"wxid_bob": "Bobby-in-room", "wxid_eve": "Eve!"})
        );
        assert!(
            !acct.chat_room_ext_buffer("room1@chatroom").unwrap()["ext_buffer"]
                .as_str()
                .unwrap()
                .is_empty()
        );
        assert_eq!(acct.group_members("x@chatroom").unwrap(), json!([]));
        assert_eq!(acct.group_nicknames("x@chatroom").unwrap(), json!({}));
        assert_eq!(acct.chat_room_ext_buffer("x@chatroom").unwrap(), json!({}));
    }

    #[test]
    fn nickname_parser_is_robust() {
        assert!(parse_group_nicknames(&[]).is_empty());
        assert!(parse_group_nicknames(&[0x0a, 0xff, 0xff, 0xff, 0xff, 0x0f]).is_empty());
        // 先是截断的条目，然后是完整的；同一个 ID 取第一次出现的
        let mut buf = vec![0x0a, 0x08, b'w', b'x'];
        buf.extend(entry("wxid_a", "first"));
        buf.extend(entry("wxid_a", "second"));
        let got = parse_group_nicknames(&buf);
        assert_eq!(got.get("wxid_a").map(String::as_str), Some("first"));
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn head_images_come_back_byte_for_byte() {
        use crate::fixture::Fixture;
        let root = TestDir::new("weflow-contact-head-image");
        let f = Fixture::new(&root, "wxid_me_ab12");
        let png: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        // 恰好是 UTF-8 的字节：纯 ASCII 文字，以及含双字节字符的文字
        let ascii: &[u8] = b"hello!";
        let wide: &[u8] = "a\u{e9}b".as_bytes();
        let packed = zstd::stream::encode_all(png, 3).unwrap();
        f.head_image_db(&[
            ("wxid_png", png),
            ("wxid_ascii", ascii),
            ("wxid_wide", wide),
            ("wxid_packed", &packed),
            ("wxid_empty", b""),
        ]);
        let acct = Account::new(f.db_storage(), &f.key_hex()).unwrap();
        let got = |u: &str| acct.head_image(u).unwrap();
        assert_eq!(got("wxid_png").as_deref(), Some(png));
        assert_eq!(got("wxid_ascii").as_deref(), Some(ascii));
        assert_eq!(got("wxid_wide").as_deref(), Some(wide));
        assert_eq!(
            got("wxid_packed").as_deref(),
            Some(png),
            "a zstd frame inflated"
        );
        assert_eq!(got("wxid_empty"), None);
        assert_eq!(got("nobody"), None);
        assert_eq!(got(""), None);
    }

    #[test]
    fn hex_decoding_leaves_out_what_is_not_hex() {
        assert_eq!(decode_hex("00ff7a"), [0x00, 0xff, 0x7a]);
        assert_eq!(decode_hex("zz01"), [0x01]);
        assert!(decode_hex("abc").is_empty(), "odd length");
        // 含一个双字节字符：共四个字节，但无法按两个字节切开
        assert!(decode_hex("a\u{e9}b").is_empty());
        assert!(decode_hex("\u{e9}\u{e9}").is_empty());
    }

    #[test]
    fn usernames_are_trimmed_and_given_once() {
        assert_eq!(distinct_usernames(&[" a ", "b", "a", ""]), ["a", "b"]);
        assert!(distinct_usernames(&[" ", ""]).is_empty());
    }
}
