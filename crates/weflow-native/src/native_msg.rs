//! Message reading on top of [`NativeAccount`]: shard/table lookup, paging, per-day statistics
//! and cursors. Row objects use WeChat's own column names (`local_id`, `create_time`, `sort_seq`,
//! `message_content`, ...) plus `sender_username`, `_db_path`, `table_name` and `db_name`, which is
//! what the service layer expects from a raw message row.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use md5::{Digest, Md5};
use serde_json::{json, Map, Value};

use crate::native_db::NativeAccount;

/// Columns dropped from "lite" rows (bulk scans that only need type, time, sender and text).
const LITE_DROP: &[&str] = &[
    "compress_content",
    "packed_info_data",
    "source",
    "WCDB_CT_source",
    "WCDB_CT_message_content",
];

/// One message location: which shard database, which table.
#[derive(Clone, Debug)]
pub struct MsgTable {
    pub db: PathBuf,
    pub table: String,
}

/// Sort key of a message: (sort_seq, create_time, local_id, shard index).
type Key = (i64, i64, i64, usize);

pub struct MessageCursor {
    tables: Vec<MsgTable>,
    keys: Vec<Key>,
    pos: usize,
    batch: usize,
    lite: bool,
}

#[derive(Default)]
pub struct Cursors {
    next: i64,
    open: HashMap<i64, MessageCursor>,
}

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
pub fn session_usernames(acct: &NativeAccount) -> Result<Vec<String>> {
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

fn row_key(row: &Value) -> (i64, i64, i64) {
    let create = int(row, "create_time");
    let seq = match int(row, "sort_seq") {
        0 => create * 1000,
        s => s,
    };
    (seq, create, int(row, "local_id"))
}

fn is_message_db(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    lower
        .strip_prefix("message_")?
        .strip_suffix(".db")?
        .parse()
        .ok()
}

impl NativeAccount {
    /// `message/message_<n>.db` shards, ascending by number.
    pub fn message_dbs(&self) -> Vec<PathBuf> {
        let dir = self.db_path("message");
        let mut found: Vec<(u32, PathBuf)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = is_message_db(e.file_name().to_str()?)?;
                Some((n, e.path()))
            })
            .collect();
        found.sort();
        found.into_iter().map(|(_, p)| p).collect()
    }

    /// Every shard that holds a table for `session_id`, oldest shard first.
    pub fn message_tables(&self, session_id: &str) -> Result<Vec<MsgTable>> {
        let table = table_name_for(session_id);
        let mut out = Vec::new();
        let dbs = self.message_dbs();
        self.prefetch(&dbs);
        for db in dbs {
            let hit = self.with_db(&db, |c| {
                Ok(c.query_row(
                    "select 1 from sqlite_master where type='table' and name=?1",
                    [&table],
                    |_| Ok(()),
                )
                .is_ok())
            })?;
            if hit {
                out.push(MsgTable {
                    db,
                    table: table.clone(),
                });
            }
        }
        Ok(out)
    }

    /// `select` over one message table with the sender resolved from `Name2Id`.
    fn select_sql(t: &MsgTable, tail: &str) -> String {
        format!(
            "select m.*, n.user_name as sender_username from \"{}\" m left join Name2Id n on n.rowid = m.real_sender_id {tail}",
            t.table
        )
    }

    pub(crate) fn finish_row(&self, row: &mut Value, t: &MsgTable, lite: bool) {
        let is_send = self.is_me(
            row.get("sender_username")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        let Some(obj) = row.as_object_mut() else {
            return;
        };
        obj.insert("is_send".into(), json!(if is_send { 1 } else { 0 }));
        // Text was already inflated by the reader, so the WCDB "compressed" flags no longer apply.
        obj.insert("WCDB_CT_message_content".into(), Value::Null);
        obj.insert("_db_path".into(), json!(t.db.to_string_lossy()));
        obj.insert(
            "db_name".into(),
            json!(t.db.file_stem().and_then(|s| s.to_str()).unwrap_or("")),
        );
        obj.insert("table_name".into(), json!(t.table));
        if lite {
            for k in LITE_DROP {
                obj.remove(*k);
            }
        }
    }

    fn cmp_desc(a: &Value, b: &Value) -> Ordering {
        row_key(b).cmp(&row_key(a))
    }

    /// Newest-first page of messages across all shards.
    pub fn messages(&self, session_id: &str, limit: i32, offset: i32) -> Result<Value> {
        let (offset, limit) = (offset.max(0) as usize, limit.max(0) as usize);
        let want = if limit == 0 {
            -1
        } else {
            (offset + limit) as i64
        };
        let mut rows: Vec<Value> = Vec::new();
        for t in self.message_tables(session_id)? {
            let order = "order by m.sort_seq desc, m.create_time desc, m.local_id desc limit ?1";
            // Asked as is, SQLite sorts the whole table (it walks the sender index for the join). The `want`-th
            // newest sort_seq, read from the sort_seq index, bounds the same rows to a range that index serves.
            let floor = if want > 0 {
                self.sort_seq_floor(&t, want)?
            } else {
                None
            };
            let page = match floor {
                Some(floor) => self.query(
                    &t.db,
                    &Self::select_sql(&t, &format!("where m.sort_seq >= ?2 {order}")),
                    &[&want, &floor],
                )?,
                None => self.query(&t.db, &Self::select_sql(&t, order), &[&want])?,
            };
            for mut r in page {
                self.finish_row(&mut r, &t, false);
                rows.push(r);
            }
        }
        rows.sort_by(Self::cmp_desc);
        let it = rows.into_iter().skip(offset);
        Ok(Value::Array(if limit == 0 {
            it.collect()
        } else {
            it.take(limit).collect()
        }))
    }

    /// The `n`-th largest `sort_seq` of a table: every one of its `n` newest rows has a `sort_seq` at least this big.
    /// `None` when the table has fewer rows, or that row has no `sort_seq` (then only a full sort is exact).
    fn sort_seq_floor(&self, t: &MsgTable, n: i64) -> Result<Option<i64>> {
        let sql = format!(
            "select sort_seq from \"{}\" order by sort_seq desc limit 1 offset ?1",
            t.table
        );
        let rows = self.query(&t.db, &sql, &[&(n - 1)])?;
        Ok(rows.first().and_then(|r| r["sort_seq"].as_i64()))
    }

    pub fn message_count(&self, session_id: &str) -> Result<i32> {
        let mut total = 0i64;
        for t in self.message_tables(session_id)? {
            let sql = format!("select count(*) as n from \"{}\"", t.table);
            total += int(&self.query(&t.db, &sql, &[])?[0], "n");
        }
        Ok(total.min(i32::MAX as i64) as i32)
    }

    pub fn session_message_counts(&self, session_ids: &[String]) -> Result<Value> {
        let mut out = Map::new();
        for id in session_ids {
            out.insert(id.clone(), json!(self.message_count(id)?));
        }
        Ok(Value::Object(out))
    }

    /// `{ "YYYY-MM-DD": count }` in local time, ascending.
    pub fn session_message_date_counts(&self, session_id: &str) -> Result<Value> {
        let mut counts: std::collections::BTreeMap<String, i64> = Default::default();
        for t in self.message_tables(session_id)? {
            let sql = format!(
                "select date(create_time, 'unixepoch', 'localtime') as d, count(*) as n from \"{}\" where create_time > 0 group by d",
                t.table
            );
            for r in self.query(&t.db, &sql, &[])? {
                if let Some(d) = r["d"].as_str() {
                    *counts.entry(d.to_string()).or_default() += int(&r, "n");
                }
            }
        }
        Ok(Value::Object(
            counts.into_iter().map(|(d, n)| (d, json!(n))).collect(),
        ))
    }

    /// `["YYYY-MM-DD", ...]` days that have at least one message, ascending.
    pub fn message_dates(&self, session_id: &str) -> Result<Value> {
        let counts = self.session_message_date_counts(session_id)?;
        Ok(Value::Array(
            counts
                .as_object()
                .map(|o| o.keys().map(|k| json!(k)).collect())
                .unwrap_or_default(),
        ))
    }

    /// Messages of one `local_type`; `limit == 0` means all.
    pub fn messages_by_type(
        &self,
        session_id: &str,
        local_type: i64,
        ascending: bool,
        limit: i32,
        offset: i32,
    ) -> Result<Value> {
        let (offset, limit) = (offset.max(0) as usize, limit.max(0) as usize);
        let want = if limit == 0 {
            -1
        } else {
            (offset + limit) as i64
        };
        let dir = if ascending { "asc" } else { "desc" };
        let mut rows: Vec<Value> = Vec::new();
        for t in self.message_tables(session_id)? {
            let sql = Self::select_sql(
                &t,
                &format!("where m.local_type = ?1 order by m.sort_seq {dir}, m.create_time {dir}, m.local_id {dir} limit ?2"),
            );
            for mut r in self.query(&t.db, &sql, &[&local_type, &want])? {
                self.finish_row(&mut r, &t, false);
                rows.push(r);
            }
        }
        rows.sort_by(|a, b| {
            if ascending {
                Self::cmp_desc(b, a)
            } else {
                Self::cmp_desc(a, b)
            }
        });
        let it = rows.into_iter().skip(offset);
        Ok(Value::Array(if limit == 0 {
            it.collect()
        } else {
            it.take(limit).collect()
        }))
    }

    /// One message by `local_id`; `{}` when absent. If several shards hold the id, the newest shard wins.
    pub fn message_by_id(&self, session_id: &str, local_id: i64) -> Result<Value> {
        self.find_one(session_id, "m.local_id = ?1", &local_id)
    }

    /// One message by server id (a decimal string, may exceed 2^53); `{}` when absent.
    pub fn message_by_server_id(&self, session_id: &str, server_id: &str) -> Result<Value> {
        let id: i64 = server_id
            .trim()
            .parse()
            .map_err(|_| anyhow!("invalid server id: {server_id}"))?;
        self.find_one(session_id, "m.server_id = ?1", &id)
    }

    fn find_one(&self, session_id: &str, cond: &str, arg: &i64) -> Result<Value> {
        for t in self.message_tables(session_id)?.into_iter().rev() {
            let sql = Self::select_sql(&t, &format!("where {cond} limit 1"));
            if let Some(mut r) = self.query(&t.db, &sql, &[arg])?.into_iter().next() {
                self.finish_row(&mut r, &t, false);
                return Ok(r);
            }
        }
        Ok(json!({}))
    }

    // ── cursors ──

    /// Open a cursor over `[begin, end]` (seconds; `0` = unbounded). Only sort keys are held in memory.
    pub fn open_message_cursor(
        &self,
        session_id: &str,
        batch: i32,
        ascending: bool,
        begin: i32,
        end: i32,
        lite: bool,
    ) -> Result<i64> {
        self.open_cursor(session_id, batch, ascending, begin, end, lite, None)
    }

    /// Like [`open_message_cursor`](Self::open_message_cursor), but only over messages whose sender `sender_ok`
    /// accepts (plus the account owner's own messages when `include_mine`). Messages whose sender cannot be
    /// resolved from `Name2Id` are always kept, so callers that look at the text prefix still see them.
    /// This lets a per-member scan of a huge group read only that member's rows.
    #[allow(clippy::too_many_arguments)]
    pub fn open_message_cursor_for_senders(
        &self,
        session_id: &str,
        batch: i32,
        ascending: bool,
        begin: i32,
        end: i32,
        lite: bool,
        sender_ok: &dyn Fn(&str) -> bool,
        include_mine: bool,
    ) -> Result<i64> {
        self.open_cursor(
            session_id,
            batch,
            ascending,
            begin,
            end,
            lite,
            Some((sender_ok, include_mine)),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn open_cursor(
        &self,
        session_id: &str,
        batch: i32,
        ascending: bool,
        begin: i32,
        end: i32,
        lite: bool,
        senders: Option<(&dyn Fn(&str) -> bool, bool)>,
    ) -> Result<i64> {
        let tables = self.message_tables(session_id)?;
        let (begin, end) = (begin.max(0) as i64, end.max(0) as i64);
        let mut keys: Vec<Key> = Vec::new();
        for (idx, t) in tables.iter().enumerate() {
            let who = match senders {
                None => String::new(),
                Some((ok, include_mine)) => {
                    let names =
                        self.query(&t.db, "select rowid as id, user_name from Name2Id", &[])?;
                    let wanted: Vec<String> = names
                        .iter()
                        .filter(|r| {
                            let name = r["user_name"].as_str().unwrap_or("");
                            ok(name) || (include_mine && self.is_me(name))
                        })
                        .map(|r| int(r, "id").to_string())
                        .collect();
                    let known = if wanted.is_empty() {
                        String::new()
                    } else {
                        format!("real_sender_id in ({}) or ", wanted.join(","))
                    };
                    format!(" and ({known}real_sender_id is null or real_sender_id not in (select rowid from Name2Id))")
                }
            };
            let sql = format!(
                "select sort_seq, create_time, local_id from \"{}\" where create_time >= ?1 and (?2 = 0 or create_time <= ?2){who}",
                t.table
            );
            let base = keys.len();
            let cell = std::cell::RefCell::new(&mut keys);
            self.query_each(
                &t.db,
                &sql,
                &[&begin, &end],
                || cell.borrow_mut().truncate(base),
                |r| {
                    let (seq, create, local) = row_key(&r);
                    cell.borrow_mut().push((seq, create, local, idx));
                },
            )?;
        }
        keys.sort();
        if !ascending {
            keys.reverse();
        }
        let mut cursors = self
            .cursors
            .lock()
            .map_err(|_| anyhow!("cursor lock poisoned"))?;
        cursors.next += 1;
        let id = cursors.next;
        cursors.open.insert(
            id,
            MessageCursor {
                tables,
                keys,
                pos: 0,
                batch: batch.max(1) as usize,
                lite,
            },
        );
        Ok(id)
    }

    /// Next batch of rows and whether more remain.
    pub fn fetch_message_batch(&self, cursor: i64) -> Result<(Value, bool)> {
        let (tables, chunk, lite, more) = {
            let mut cursors = self
                .cursors
                .lock()
                .map_err(|_| anyhow!("cursor lock poisoned"))?;
            let c = cursors
                .open
                .get_mut(&cursor)
                .ok_or_else(|| anyhow!("unknown message cursor {cursor}"))?;
            let end = (c.pos + c.batch).min(c.keys.len());
            let chunk = c.keys[c.pos..end].to_vec();
            c.pos = end;
            (c.tables.clone(), chunk, c.lite, c.pos < c.keys.len())
        };
        let mut by_shard: HashMap<usize, Vec<i64>> = HashMap::new();
        for (_, _, local, idx) in &chunk {
            by_shard.entry(*idx).or_default().push(*local);
        }
        let mut fetched: HashMap<(usize, i64), Value> = HashMap::new();
        for (idx, ids) in by_shard {
            let t = &tables[idx];
            let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            let sql = Self::select_sql(t, &format!("where m.local_id in ({list})"));
            for mut r in self.query(&t.db, &sql, &[])? {
                self.finish_row(&mut r, t, lite);
                fetched.insert((idx, int(&r, "local_id")), r);
            }
        }
        let rows = chunk
            .iter()
            .filter_map(|(_, _, local, idx)| fetched.remove(&(*idx, *local)))
            .collect();
        Ok((Value::Array(rows), more))
    }

    pub fn close_message_cursor(&self, cursor: i64) -> Result<()> {
        self.cursors
            .lock()
            .map_err(|_| anyhow!("cursor lock poisoned"))?
            .open
            .remove(&cursor);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_db::hex;
    use crate::sqlcipher::testutil::{encrypt_db, plain_db_with, KEY, SALT};
    use crate::sqlcipher::PageCipher;
    use rusqlite::{params, Connection};

    const T0: i64 = 1_700_000_000;
    const DAY: i64 = 2 * 86_400; // two days apart so local-time bucketing cannot merge them

    /// `msgs`: (local_id, day_offset, local_type, real_sender_id, text). `server_id` = create_time offset.
    fn shard(msgs: &[(i64, i64, i64, i64, &str)], zstd_local_id: Option<i64>) -> Vec<u8> {
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
                 sort_seq integer, real_sender_id integer, create_time integer, status integer, message_content, compress_content, packed_info_data blob)"
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
                    &format!("insert into \"{table}\"(local_id, server_id, local_type, sort_seq, real_sender_id, create_time, message_content, packed_info_data) values (?1,?2,?3,?4,?5,?6,?7,?8)"),
                    params![local_id, offset, local_type, create * 1000, sender, create, content, vec![0u8, 1, 2, 0xff]],
                )
                .unwrap();
            }
        })
    }

    /// Two shards: ids 1..3 in shard 0 (days 0..2), ids 1..2 in shard 1 (days 3..4); local_id overlaps on purpose.
    fn account(tag: &str) -> (NativeAccount, PathBuf) {
        let dir = std::env::temp_dir().join(format!("weflow-msg-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
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
        (NativeAccount::new(&dir, &hex(&KEY)).unwrap(), dir)
    }

    fn texts(v: &Value) -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|r| r["message_content"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn table_name_is_md5_of_session_id() {
        assert_eq!(
            table_name_for("abc"),
            "Msg_900150983cd24fb0d6963f7d28e17f72"
        );
    }

    #[test]
    fn shards_are_discovered_and_merged_newest_first() {
        let (acct, _dir) = account("merge");
        assert_eq!(
            acct.message_dbs().len(),
            2,
            "message_fts.db must not count as a shard"
        );
        assert_eq!(acct.message_tables("wxid_bob").unwrap().len(), 2);
        assert!(acct.message_tables("wxid_nobody").unwrap().is_empty());
        assert_eq!(acct.message_count("wxid_bob").unwrap(), 5);
        assert_eq!(
            texts(&acct.messages("wxid_bob", 3, 0).unwrap()),
            ["voice", "fourth", "third"]
        );
        assert_eq!(
            texts(&acct.messages("wxid_bob", 10, 3).unwrap()),
            ["img", "hello"]
        );
        assert_eq!(acct.messages("wxid_bob", 10, 5).unwrap(), json!([]));
    }

    #[test]
    fn newest_pages_are_exact_when_sort_seq_ties_cross_the_page_boundary() {
        let dir = std::env::temp_dir().join(format!("weflow-msg-{}-ties", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("message")).unwrap();
        let table = table_name_for("wxid_bob");
        // 60 rows: sort_seq takes only 6 distinct values (10 rows each), some rows have none; create_time varies
        let plain = plain_db_with(move |c: &Connection| {
            c.execute_batch("create table Name2Id(user_name text primary key, is_session integer)")
                .unwrap();
            c.execute_batch(&format!(
                "create table \"{table}\"(local_id integer primary key autoincrement, local_type integer, sort_seq integer, \
                 real_sender_id integer, create_time integer, message_content)"
            ))
            .unwrap();
            c.execute_batch(&format!(
                "create index \"{table}_SORTSEQ\" on \"{table}\"(sort_seq)"
            ))
            .unwrap();
            for i in 0..60i64 {
                let seq: Option<i64> = if i % 13 == 0 {
                    None
                } else {
                    Some((i % 6) * 1000)
                };
                c.execute(
                    &format!("insert into \"{table}\"(local_id, local_type, sort_seq, real_sender_id, create_time, message_content) values (?1, 1, ?2, 1, ?3, ?4)"),
                    params![i + 1, seq, T0 + (i * 7) % 11, format!("m{i}")],
                )
                .unwrap();
            }
        });
        let cipher = PageCipher::derive(&KEY, &SALT);
        std::fs::write(
            dir.join("message/message_0.db"),
            encrypt_db(&plain, &cipher),
        )
        .unwrap();
        let acct = NativeAccount::new(&dir, &hex(&KEY)).unwrap();
        // what the plain "sort everything, take the first rows" query gives
        let t = acct.message_tables("wxid_bob").unwrap().remove(0);
        let reference = |limit: i32, offset: i32| -> Vec<String> {
            let want = (offset + limit) as i64;
            let sql = NativeAccount::select_sql(
                &t,
                "order by m.sort_seq desc, m.create_time desc, m.local_id desc limit ?1",
            );
            let mut rows = acct.query(&t.db, &sql, &[&want]).unwrap();
            rows.sort_by(NativeAccount::cmp_desc);
            texts(&Value::Array(
                rows.into_iter()
                    .skip(offset as usize)
                    .take(limit as usize)
                    .collect(),
            ))
        };
        assert_eq!(
            acct.messages("wxid_bob", 0, 0)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            60
        );
        for (limit, offset) in [
            (1, 0),
            (5, 0),
            (10, 0),
            (7, 3),
            (10, 10),
            (25, 30),
            (40, 15),
            (50, 5),
            (100, 0),
        ] {
            let page = texts(&acct.messages("wxid_bob", limit, offset).unwrap());
            assert_eq!(
                page,
                reference(limit, offset),
                "limit {limit} offset {offset}"
            );
            assert_eq!(
                page.len(),
                (limit as usize).min(60 - (offset as usize).min(60))
            );
        }
    }

    #[test]
    fn rows_carry_sender_source_and_inflated_content() {
        let (acct, _dir) = account("rows");
        let rows = acct.messages("wxid_bob", 0, 0).unwrap();
        let rows = rows.as_array().unwrap();
        let third = rows
            .iter()
            .find(|r| r["message_content"] == "third")
            .unwrap(); // stored as a zstd blob
        assert_eq!(third["sender_username"], "wxid_bob");
        assert_eq!(third["table_name"], table_name_for("wxid_bob"));
        assert_eq!(third["db_name"], "message_0");
        assert!(third["_db_path"]
            .as_str()
            .unwrap()
            .ends_with("message_0.db"));
        assert_eq!(
            third["packed_info_data"], "000102ff",
            "binary columns are hex"
        );
        let img = rows.iter().find(|r| r["message_content"] == "img").unwrap();
        assert_eq!(img["sender_username"], "wxid_me");
    }

    #[test]
    fn dates_and_counts() {
        let (acct, _dir) = account("dates");
        let counts = acct.session_message_date_counts("wxid_bob").unwrap();
        assert_eq!(counts.as_object().unwrap().len(), 5);
        assert!(counts.as_object().unwrap().values().all(|n| *n == 1));
        let dates = acct.message_dates("wxid_bob").unwrap();
        let dates: Vec<&str> = dates
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d.as_str().unwrap())
            .collect();
        assert_eq!(dates.len(), 5);
        assert!(
            dates.windows(2).all(|w| w[0] < w[1]),
            "ascending: {dates:?}"
        );
        assert_eq!(
            acct.session_message_counts(&["wxid_bob".into(), "x".into()])
                .unwrap(),
            json!({"wxid_bob": 5, "x": 0})
        );
    }

    #[test]
    fn lookups_by_type_and_id() {
        let (acct, _dir) = account("lookup");
        assert_eq!(
            texts(&acct.messages_by_type("wxid_bob", 1, true, 0, 0).unwrap()),
            ["hello", "third", "fourth"]
        );
        assert_eq!(
            texts(&acct.messages_by_type("wxid_bob", 1, false, 2, 0).unwrap()),
            ["fourth", "third"]
        );
        assert_eq!(
            acct.messages_by_type("wxid_bob", 999, false, 0, 0).unwrap(),
            json!([])
        );
        // local_id 1 exists in both shards: the newest shard wins.
        assert_eq!(
            acct.message_by_id("wxid_bob", 1).unwrap()["message_content"],
            "fourth"
        );
        assert_eq!(
            acct.message_by_id("wxid_bob", 3).unwrap()["message_content"],
            "third"
        );
        assert_eq!(acct.message_by_id("wxid_bob", 77).unwrap(), json!({}));
        assert_eq!(
            acct.message_by_server_id("wxid_bob", &(DAY * 4).to_string())
                .unwrap()["message_content"],
            "voice"
        );
        assert!(acct.message_by_server_id("wxid_bob", "abc").is_err());
    }

    #[test]
    fn cursor_walks_all_messages_in_order_across_shards() {
        let (acct, _dir) = account("cursor");
        let c = acct
            .open_message_cursor("wxid_bob", 2, true, 0, 0, false)
            .unwrap();
        let mut all = Vec::new();
        let mut batches = 0;
        loop {
            let (rows, more) = acct.fetch_message_batch(c).unwrap();
            batches += 1;
            all.extend(texts(&rows));
            if !more {
                break;
            }
        }
        assert_eq!(batches, 3);
        assert_eq!(all, ["hello", "img", "third", "fourth", "voice"]);
        acct.close_message_cursor(c).unwrap();
        assert!(acct.fetch_message_batch(c).is_err());
    }

    #[test]
    fn cursor_honours_time_range_direction_and_lite() {
        let (acct, _dir) = account("range");
        // [day1, day3] inclusive, newest first, lite rows
        let (begin, end) = ((T0 + DAY) as i32, (T0 + DAY * 3) as i32);
        let c = acct
            .open_message_cursor("wxid_bob", 10, false, begin, end, true)
            .unwrap();
        let (rows, more) = acct.fetch_message_batch(c).unwrap();
        assert!(!more);
        assert_eq!(texts(&rows), ["fourth", "third", "img"]);
        let first = &rows.as_array().unwrap()[0];
        assert!(first.get("packed_info_data").is_none() && first.get("compress_content").is_none());
        assert!(first.get("message_content").is_some() && first.get("sender_username").is_some());
    }

    #[test]
    fn sender_filtered_cursor_reads_only_the_wanted_senders() {
        let (acct, _dir) = account("senders");
        let acct = acct.with_my_wxid(Some("wxid_me".into()));
        let read = |ok: &dyn Fn(&str) -> bool, mine: bool| -> Vec<String> {
            let c = acct
                .open_message_cursor_for_senders("wxid_bob", 100, true, 0, 0, true, ok, mine)
                .unwrap();
            let (rows, _) = acct.fetch_message_batch(c).unwrap();
            acct.close_message_cursor(c).unwrap();
            texts(&rows)
        };
        // wxid_bob wrote messages 1, 3 and 5 (in time order), wxid_me the others
        assert_eq!(
            read(&|n| n == "wxid_bob", false),
            ["hello", "third", "voice"]
        );
        assert_eq!(read(&|n| n == "wxid_me", false), ["img", "fourth"]);
        assert_eq!(
            read(&|_| false, true),
            ["img", "fourth"],
            "the owner's own messages can be asked for separately"
        );
        assert_eq!(read(&|n| n == "wxid_bob", true).len(), 5);
        assert!(
            read(&|_| false, false).is_empty(),
            "nobody asked for: nothing read"
        );
        assert_eq!(read(&|_| true, false).len(), 5);
    }
}
