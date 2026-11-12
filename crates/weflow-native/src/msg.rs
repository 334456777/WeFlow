//! Message reading on top of [`Account`]: shard/table lookup, paging, per-day statistics
//! and cursors. Rows are read as [`MsgRow`](crate::msg_row::MsgRow)s.

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

/// One message location: which shard database, which table.
#[derive(Clone, Debug)]
pub struct MsgTable {
    pub db: PathBuf,
    pub table: String,
}

/// Sort key of a message: (sort_seq, create_time, local_id, shard index).
type Key = (i64, i64, i64, usize);

/// `keys` in the chat's order, oldest or newest first; equal keys of different databases by ascending database
/// number either way, as the pages by offset list them.
fn sort_keys(keys: &mut [Key], ascending: bool) {
    if ascending {
        keys.sort_unstable();
    } else {
        keys.sort_unstable_by(newest_first);
    }
}

/// The newest-first order of keys: by sort key descending, equal ones by ascending database number.
fn newest_first(a: &Key, b: &Key) -> std::cmp::Ordering {
    (b.0, b.1, b.2).cmp(&(a.0, a.1, a.2)).then(a.3.cmp(&b.3))
}

/// Sender filter of a cursor: (does this username match, also keep the account owner's own messages).
pub type SenderFilter<'a> = (&'a dyn Fn(&str) -> bool, bool);

/// What a page read of an export cursor fails with when its keys had to be read again and the pages are no longer
/// the same (see [`Account::open_export_cursor`]): the export has to start over.
pub const KEYS_CHANGED: &str =
    "the order of the messages changed while they were read; the export has to start over";

/// `Msg_<md5(username)>`, the per-conversation table name.
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

/// Usernames of every conversation in `session.db`.
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

/// A message's place in its conversation's newest-first order, to page from: its sort key (`sort_seq`, or
/// `create_time * 1000` without one, then `create_time`, `local_id`) and the number of its `message_<n>.db`. Equal
/// keys of different databases are listed by ascending database number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsgPos {
    pub sort_seq: i64,
    pub create_time: i64,
    pub local_id: i64,
    pub shard: u32,
}

// one per message of a conversation in a cursor: its size changes only on purpose (#505)
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<MsgPos>() == 32);

impl MsgPos {
    /// `sort_seq.create_time.local_id.shard`, what [`parse`](Self::parse) reads back.
    pub fn token(&self) -> String {
        format!(
            "{}.{}.{}.{}",
            self.sort_seq, self.create_time, self.local_id, self.shard
        )
    }

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

    /// The place of a moment (Unix seconds) in the chat's order, which is WeChat's: `sort_seq`, the send time in
    /// milliseconds plus a little. A page [`After`](PageFrom::After) it starts at the first message ordered at that
    /// moment or later. A message ordered a second or two off its own send time (seen on a real account) falls on
    /// the side its order puts it, as in the chat window, so pages on from there neither skip nor repeat one
    /// (issue #242).
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

/// Whether rows at a position's own place come after it, in a listing across conversations (issue #178): not in its
/// own conversation (a row there at the same place is the same message, kept by more than one database) nor in
/// one listed before it, all in one listed after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ties {
    None,
    All,
}

/// Where a page of [`Account::messages_from`] starts.
#[derive(Clone, Copy, Debug)]
pub enum PageFrom {
    /// The messages listed after this one: older ones.
    Before(MsgPos),
    /// The messages listed before this one: newer ones, the closest first taken.
    After(MsgPos),
}

/// The sort key of a row (see [`MsgPos`]) from its cells `sort_seq`, `create_time` and `local_id`; `None` when one is not an integer, but for a
/// null `sort_seq`, which is one without, as a zero.
fn typed_row_key(
    sort_seq: rusqlite::types::ValueRef,
    create_time: rusqlite::types::ValueRef,
    local_id: rusqlite::types::ValueRef,
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

/// Numbers of the `biz_message_<n>.db` databases, where official accounts and service notices are kept, start here:
/// a message's database is told by its number alone, so they must not meet those of `message_<n>.db`.
const BIZ_SHARD_BASE: u32 = 1 << 24;

/// The name of the database with this number: `message_<n>`, or `biz_message_<n>`.
pub fn shard_name(shard: u32) -> String {
    match shard.checked_sub(BIZ_SHARD_BASE) {
        Some(n) => format!("biz_message_{n}"),
        None => format!("message_{shard}"),
    }
}

/// The number of the message database holding a table.
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
    pub(super) const DAY: i64 = 2 * 86_400; // two days apart so local-time bucketing cannot merge them

    /// `msgs`: (local_id, day_offset, local_type, real_sender_id, text). `server_id` = create_time offset.
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

    /// Two shards: ids 1..3 in shard 0 (days 0..2), ids 1..2 in shard 1 (days 3..4); local_id overlaps on purpose.
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

    /// An account whose `wxid_bob` messages are `shards`, one `message_<n>.db` each: rows of (local_id, sort_seq as
    /// an SQL literal, create_time as one). Neither column has a type, so a row can store any SQLite value.
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

/// Proofs of `cargo kani` (#501).
#[cfg(kani)]
mod proofs {
    use super::*;

    /// [`newest_first`] is a total order for any keys, so the standard library's sort puts slices of any length in
    /// it: equal only for equal keys, the reverse when swapped, and transitive.
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

    /// Any 3 keys come out of [`sort_keys`] as a reordering of them, in the chat's order in either direction, equal
    /// keys of different databases by ascending database number: the sort uses the order proved above.
    #[kani::proof]
    #[kani::unwind(5)]
    fn keys_are_sorted_either_way_with_ties_by_database() {
        let mut keys: [Key; 3] = kani::any();
        let before = keys;
        let ascending: bool = kani::any();
        sort_keys(&mut keys, ascending);
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
