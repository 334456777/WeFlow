//! 基于 [`Account`] 读取消息：查找分片和表、分页、按天统计和游标。行读成 [`MsgRow`](crate::msg_row::MsgRow)。

use std::path::PathBuf;

use anyhow::Result;
use md5::{Digest, Md5};
use serde_json::Value;

use crate::db::Account;

mod counts;
pub(crate) mod cursor;
mod lookup;
mod page;
mod tables;

/// 消息所在的一处位置：哪个分片的哪张表。
#[derive(Clone, Debug)]
pub struct MsgTable {
    /// 所在的消息库，`message_<n>.db` 或 `biz_message_<n>.db`。
    pub db: PathBuf,
    /// 表名 `Msg_<md5>`，其中 `<md5>` 是用户名的 md5。
    pub table: String,
}

/// 消息的排序键：`sort_seq`、`create_time`、`local_id` 和所在分片的序号。
type Key = (i64, i64, i64, usize);

/// 读取会话的消息时从哪一端开始。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// 最早的消息在前。
    OldestFirst,
    /// 最新的消息在前。
    NewestFirst,
}

/// 把 `keys` 按聊天窗口的顺序排列，最早或最新的在前；两个方向下不同分片的相同键都按分片号升序排列，
/// 与按偏移分页时列出的顺序一致。
fn sort_keys(keys: &mut [Key], order: Order) {
    match order {
        Order::OldestFirst => keys.sort_unstable(),
        Order::NewestFirst => keys.sort_unstable_by(newest_first),
    }
}

/// 最新在前时键的顺序：按排序键降序，相同的键按分片号升序。
fn newest_first(a: &Key, b: &Key) -> std::cmp::Ordering {
    (b.0, b.1, b.2).cmp(&(a.0, a.1, a.2)).then(a.3.cmp(&b.3))
}

/// 游标的发送者筛选：第一项判断一个用户名是否符合，第二项为真时同时保留账号所有者自己的消息。
pub type SenderFilter<'a> = (&'a dyn Fn(&str) -> bool, bool);

/// 导出游标的键需要重新读取、分页因此改变时，读一页返回的错误（见 [`Account::open_export_cursor`]）：
/// 导出要从头开始。
pub const KEYS_CHANGED: &str =
    "the order of the messages changed while they were read; the export has to start over";

/// 会话的消息表名 `Msg_<md5(username)>`。
pub fn table_name_for(session_id: &str) -> String {
    let digest = Md5::digest(session_id.as_bytes());
    format!(
        "Msg_{}",
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

/// `session.db` 中全部会话的用户名。
///
/// # Errors
///
/// 无法读取 `session.db`（文件不存在或解密失败），或者查询 `SessionTable` 失败。
pub fn session_usernames(acct: &Account) -> Result<Vec<String>> {
    let rows = acct.query(&acct.session_db(), "select username from SessionTable", &[])?;
    Ok(rows
        .iter()
        .filter_map(|r| r["username"].as_str().map(str::to_string))
        .filter(|u| !u.is_empty())
        .collect())
}

fn int(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// 消息在会话中按最新在前排列时的位置，用作翻页的起点。
///
/// 它由两部分组成：消息的排序键（`sort_seq`，没有时用 `create_time * 1000`；其次是 `create_time` 和 `local_id`），
/// 以及所在消息库的分片号（[`shard_name`]）。不同消息库中相同的键按分片号升序排列。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsgPos {
    /// `sort_seq`；行里没有时为 `create_time * 1000`。
    pub sort_seq: i64,
    /// 发送时间，Unix 时间戳（秒）。
    pub create_time: i64,
    /// 行在所在表中的 ID（本地 ID）。
    pub local_id: i64,
    /// 所在消息库的分片号。
    pub shard: u32,
}

// 游标为会话的每条消息保存一个：固定它的大小，只在有意时改变（#505）
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<MsgPos>() == 32);

impl MsgPos {
    /// 写成 `sort_seq.create_time.local_id.shard` 的游标，[`parse`](Self::parse) 可以读回。
    pub fn token(&self) -> String {
        format!(
            "{}.{}.{}.{}",
            self.sort_seq, self.create_time, self.local_id, self.shard
        )
    }

    /// [`token`](Self::token) 写出的游标表示的位置；其他文字为 `None`。
    pub fn parse(token: &str) -> Option<Self> {
        let mut parts = token.trim().split('.');
        let pos = Self {
            sort_seq: parts.next()?.parse().ok()?,
            create_time: parts.next()?.parse().ok()?,
            local_id: parts.next()?.parse().ok()?,
            shard: parts.next()?.parse().ok()?,
        };
        parts.next().is_none().then_some(pos)
    }

    /// 某一时刻（Unix 秒）在聊天窗口顺序中的位置，这个顺序就是微信的 `sort_seq`：毫秒的发送时间再加一点。
    /// 以它为界的 [`After`](PageFrom::After) 页从排在这一时刻或之后的第一条消息开始。
    /// 排序位置与发送时间相差一两秒的消息（真实账号上出现过）按排序位置落在这一时刻的某一侧，与聊天窗口一致，
    /// 所以从这里往后翻页既不跳过也不重复消息（#242）。
    pub fn at_time(seconds: i64) -> Self {
        Self {
            sort_seq: seconds.saturating_mul(1000),
            create_time: seconds,
            local_id: i64::MIN,
            shard: 0,
        }
    }

    fn key(&self) -> (i64, i64, i64) {
        (self.sort_seq, self.create_time, self.local_id)
    }
}

/// 在跨会话的列表里，与某个位置的排序位置相同的行是否排在它之后（#178）。
///
/// 在它自己的会话里（那里排序位置相同的行是同一条消息，保存在不止一个数据库中）和排在它前面的会话里都不在它之后，
/// 在排在它后面的会话里都在它之后。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ties {
    /// 排序位置相同的行都不在它之后。
    None,
    /// 排序位置相同的行都在它之后。
    All,
}

/// [`Account::messages_from`] 的一页从哪里开始。
#[derive(Clone, Copy, Debug)]
pub enum PageFrom {
    /// 列在这条消息之后的消息，即更早的消息。
    Before(MsgPos),
    /// 列在这条消息之前的消息，即更新的消息，先取离它最近的。
    After(MsgPos),
}

/// 由 `sort_seq`、`create_time` 和 `local_id` 三个单元格得出行的排序键（见 [`MsgPos`]）；有一个不是整数时为
/// `None`，但 `sort_seq` 为 null 时与 0 一样，算作没有 `sort_seq`。
fn typed_row_key(
    sort_seq: rusqlite::types::ValueRef<'_>,
    create_time: rusqlite::types::ValueRef<'_>,
    local_id: rusqlite::types::ValueRef<'_>,
) -> Option<(i64, i64, i64)> {
    use rusqlite::types::ValueRef;
    let (ValueRef::Integer(create), ValueRef::Integer(local_id)) = (create_time, local_id) else {
        return None;
    };
    let seq = match sort_seq {
        ValueRef::Integer(0) | ValueRef::Null => create * 1000,
        ValueRef::Integer(seq) => seq,
        _ => return None,
    };
    Some((seq, create, local_id))
}

/// `biz_message_<n>.db`（保存公众号和服务通知的消息）的编号从这里开始：只凭编号区分消息所在的数据库，
/// 所以它们不能与 `message_<n>.db` 的编号重叠。
const BIZ_SHARD_BASE: u32 = 1 << 24;

/// 编号为 `shard` 的数据库名：`message_<n>` 或 `biz_message_<n>`。
pub fn shard_name(shard: u32) -> String {
    match shard.checked_sub(BIZ_SHARD_BASE) {
        Some(n) => format!("biz_message_{n}"),
        None => format!("message_{shard}"),
    }
}

/// 表所在消息库的编号。
pub(crate) fn shard_of(t: &MsgTable) -> Option<u32> {
    t.db.file_name()
        .and_then(|n| n.to_str())
        .and_then(is_message_db)
}

fn is_message_db(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    let numbered = |prefix: &str| -> Option<u32> {
        lower
            .strip_prefix(prefix)?
            .strip_suffix(".db")?
            .parse()
            .ok()
    };
    numbered("message_")
        .filter(|n| *n < BIZ_SHARD_BASE)
        .or_else(|| BIZ_SHARD_BASE.checked_add(numbered("biz_message_")?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::hex;
    use crate::fixture::TestDir;
    use crate::msg_row::MsgRow;
    use crate::sqlcipher::testutil::{encrypt_db, plain_db_with, KEY, SALT};
    use crate::sqlcipher::PageCipher;
    use rusqlite::{params, Connection};

    pub(super) const T0: i64 = 1_700_000_000;
    pub(super) const DAY: i64 = 2 * 86_400; // 相隔两天，按本地时间分到哪一天都不会合并

    /// `msgs` 的每一项是（`local_id`、`day_offset`、`local_type`、`real_sender_id`、文字）；
    /// `server_id` 等于 `create_time` 相对 `T0` 的偏移。`zstd_local_id` 那一行的内容用 zstd 压缩。
    pub(super) fn shard(
        msgs: &[(i64, i64, i64, i64, &str)],
        zstd_local_id: Option<i64>,
    ) -> Vec<u8> {
        let table = table_name_for("wxid_bob");
        let msgs = msgs.to_vec();
        plain_db_with(move |c: &Connection| {
            c.execute_batch("create table Name2Id(user_name text primary key, is_session integer)")
                .unwrap();
            c.execute(
                "insert into Name2Id(rowid, user_name) values (1, ?1), (2, ?2)",
                params!["wxid_me", "wxid_bob"],
            )
            .unwrap();
            c.execute_batch(&format!(
                "create table \"{table}\"(local_id integer primary key autoincrement, server_id integer, local_type integer, \
                 sort_seq integer, real_sender_id integer, create_time integer, status integer, source text, message_content, compress_content, packed_info_data blob)"
            ))
            .unwrap();
            for (local_id, offset, local_type, sender, text) in &msgs {
                let create = T0 + offset;
                let content: Box<dyn rusqlite::ToSql> = if Some(*local_id) == zstd_local_id {
                    Box::new(zstd::encode_all(text.as_bytes(), 1).unwrap())
                } else {
                    Box::new(text.to_string())
                };
                c.execute(
                    &format!("insert into \"{table}\"(local_id, server_id, local_type, sort_seq, real_sender_id, create_time, message_content, packed_info_data, source) values (?1,?2,?3,?4,?5,?6,?7,?8,?9)"),
                    params![local_id, offset, local_type, create * 1000, sender, create, content, vec![0u8, 1, 2, 0xff], "<msgsource/>"],
                )
                .unwrap();
            }
        })
    }

    /// 两个分片：分片 0 有 ID 1..3（偏移为 0..2 个 `DAY`），分片 1 有 ID 1..2（偏移为 3..4 个 `DAY`）；
    /// `local_id` 特意重叠。
    pub(super) fn account(tag: &str) -> (TestDir, Account) {
        let dir = TestDir::new(&format!("weflow-msg-{tag}"));
        std::fs::create_dir_all(dir.join("message")).unwrap();
        let cipher = PageCipher::derive(&KEY, &SALT);
        let s0 = shard(
            &[
                (1, 0, 1, 2, "hello"),
                (2, DAY, 3, 1, "img"),
                (3, DAY * 2, 1, 2, "third"),
            ],
            Some(3),
        );
        let s1 = shard(
            &[(1, DAY * 3, 1, 1, "fourth"), (2, DAY * 4, 34, 2, "voice")],
            None,
        );
        std::fs::write(dir.join("message/message_0.db"), encrypt_db(&s0, &cipher)).unwrap();
        std::fs::write(dir.join("message/message_1.db"), encrypt_db(&s1, &cipher)).unwrap();
        std::fs::write(dir.join("message/message_fts.db"), b"not a message shard").unwrap();
        let acct = Account::new(dir.path(), &hex(&KEY)).unwrap();
        (dir, acct)
    }

    pub(super) fn texts(rows: &[MsgRow]) -> Vec<String> {
        rows.iter().map(|r| r.content.clone()).collect()
    }

    #[test]
    fn table_name_is_md5_of_session_id() {
        assert_eq!(
            table_name_for("abc"),
            "Msg_900150983cd24fb0d6963f7d28e17f72"
        );
    }

    #[test]
    fn only_message_databases_have_a_shard_number() {
        assert_eq!(is_message_db("message_3.db"), Some(3));
        assert_eq!(is_message_db("biz_message_0.db"), Some(BIZ_SHARD_BASE));
        for name in ["biz_message_0.kvdb", "message_fts.db", "biz_message.db"] {
            assert_eq!(is_message_db(name), None, "{name}");
        }
    }

    /// `wxid_bob` 的消息为 `shards` 的账号，每个分片一个 `message_<n>.db`，每行是（`local_id`、写成 SQL 字面量的
    /// `sort_seq`、同样写成字面量的 `create_time`）。这两列没有类型，所以一行可以存任意 SQLite 值。
    pub(super) fn keyed_account(
        tag: &str,
        shards: &[Vec<(i64, String, String)>],
    ) -> (TestDir, Account) {
        let dir = TestDir::new(&format!("weflow-msg-{tag}"));
        std::fs::create_dir_all(dir.join("message")).unwrap();
        let table = table_name_for("wxid_bob");
        let cipher = PageCipher::derive(&KEY, &SALT);
        for (n, rows) in shards.iter().enumerate() {
            let values = rows
                .iter()
                .map(|(id, seq, create)| format!("({id}, {seq}, {create}, 1, 1, '{n}:{id}')"))
                .collect::<Vec<_>>()
                .join(", ");
            let insert = if values.is_empty() {
                String::new()
            } else {
                format!(
                    "insert into \"{table}\"(local_id, sort_seq, create_time, real_sender_id, local_type,
                         message_content) values {values};"
                )
            };
            let table = table.clone();
            let plain = plain_db_with(move |c: &Connection| {
                c.execute_batch(&format!(
                    "create table Name2Id(user_name text primary key, is_session integer);
                     insert into Name2Id(rowid, user_name) values (1, 'wxid_bob');
                     create table \"{table}\"(local_id integer primary key, server_id integer, sort_seq,
                         create_time, status integer, real_sender_id integer, local_type integer, compress_content,
                         message_content text, source text, packed_info_data blob);
                     create index \"{table}_SORTSEQ\" on \"{table}\"(sort_seq);
                     {insert}"
                ))
                .unwrap();
            });
            std::fs::write(
                dir.join(format!("message/message_{n}.db")),
                encrypt_db(&plain, &cipher),
            )
            .unwrap();
        }
        let acct = Account::new(dir.path(), &hex(&KEY)).unwrap();
        (dir, acct)
    }
}

/// `cargo kani` 的证明（#501）。
#[cfg(kani)]
mod proofs {
    use super::*;

    /// 对任意的键 [`newest_first`] 都是全序，所以标准库的排序能把任意长度的切片排成这个顺序：只有相同的键才相等，
    /// 交换两边时结果相反，并且满足传递性。
    #[kani::proof]
    fn newest_first_is_a_total_order() {
        use std::cmp::Ordering::{Equal, Greater};
        let (a, b, c): (Key, Key, Key) = kani::any();
        assert!((newest_first(&a, &b) == Equal) == (a == b));
        assert!(newest_first(&a, &b) == newest_first(&b, &a).reverse());
        if newest_first(&a, &b) != Greater && newest_first(&b, &c) != Greater {
            assert!(newest_first(&a, &c) != Greater);
        }
    }

    /// 任意 3 个键经 [`sort_keys`] 排序后仍是这 3 个键，两个方向都按聊天窗口的顺序排列，
    /// 不同分片的相同键按分片号升序：排序用的是上面证明过的顺序。
    #[kani::proof]
    #[kani::unwind(5)]
    fn keys_are_sorted_either_way_with_ties_by_database() {
        let mut keys: [Key; 3] = kani::any();
        let before = keys;
        let ascending: bool = kani::any();
        let order = if ascending {
            Order::OldestFirst
        } else {
            Order::NewestFirst
        };
        sort_keys(&mut keys, order);
        for w in keys.windows(2) {
            let (a, b) = ((w[0].0, w[0].1, w[0].2), (w[1].0, w[1].1, w[1].2));
            assert!(if ascending { a <= b } else { a >= b });
            if a == b {
                assert!(w[0].3 <= w[1].3);
            }
        }
        for k in &before {
            let count = |keys: &[Key]| keys.iter().filter(|x| *x == k).count();
            assert!(count(&keys) == count(&before));
        }
    }
}
