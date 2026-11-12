//! Cursors over a conversation's messages: their keys, read from the rows or the `sort_seq` index, and their
//! pages.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{anyhow, Context, Result};

use super::SenderFilter;
use super::KEYS_CHANGED;
use super::{int, sort_keys, Key, MsgTable};
use crate::db::Account;
use crate::msg_row::{Columns, MsgRow};

/// Tables whose `create_time` turned out not to be `sort_seq / 1000`: export cursors read their keys from the table.
static EXACT_KEY_TABLES: LazyLock<Mutex<HashSet<(PathBuf, String)>>> =
    LazyLock::new(Mutex::default);

/// Which messages a cursor lists, and how it reads their keys.
enum CursorKeys<'a> {
    /// Every message, keys read from the rows.
    All,
    /// The messages of some senders (see [`Account::open_message_cursor_for_senders`]).
    Senders(SenderFilter<'a>),
    /// Every message, keys from the `sort_seq` index (see [`Account::open_export_cursor`]).
    Export,
}

#[cfg(test)]
thread_local! {
    /// Tables this thread read every key of ([`Account::table_keys`]).
    static TABLE_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn exact_key_table(t: &MsgTable) -> bool {
    EXACT_KEY_TABLES
        .lock()
        .is_ok_and(|set| set.contains(&(t.db.clone(), t.table.clone())))
}

struct MessageCursor {
    tables: Vec<MsgTable>,
    keys: Vec<Key>,
    pos: usize,
    batch: usize,
    /// The columns its pages read.
    columns: Columns,
    /// Per table: whether its keys came from the `sort_seq` index ([`Account::open_export_cursor`]) and are
    /// still to be checked against the rows.
    derived: Vec<bool>,
    range: (i64, i64),
    ascending: bool,
    /// Held while keys are read again from the rows ([`Account::rekey`]): workers whose pages find the same
    /// table's keys wrong at once wait for one scan of it instead of each scanning the whole table.
    rekeying: Arc<Mutex<()>>,
}

#[derive(Default)]
pub struct Cursors {
    next: i64,
    open: HashMap<i64, MessageCursor>,
}

impl Account {
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
        self.open_cursor(
            session_id,
            batch,
            ascending,
            (begin, end),
            if lite { Columns::Lite } else { Columns::All },
            CursorKeys::All,
        )
    }

    /// The cursor of an export: oldest first, full rows but the `source` column, which exports do not use, like
    /// [`open_message_cursor`](Self::open_message_cursor).
    /// Without time bounds, collecting its keys reads only the `sort_seq` index instead of every row of the table, taking `create_time`
    /// as `sort_seq / 1000`, which is how WeChat makes `sort_seq` (`create_time` in milliseconds plus a counter). The
    /// pages check that against the rows they read: when a table does not follow it, the keys are read from the table
    /// as before, the table is remembered for the next exports, and when that changes the pages,
    /// [`KEYS_CHANGED`] tells the export to start over (issue #39). Bounded cursors read exact `create_time` keys:
    /// index-derived bounds could exclude an in-range row entirely, so no fetched page could detect its wrong key.
    pub fn open_export_cursor(
        &self,
        session_id: &str,
        batch: i32,
        begin: i32,
        end: i32,
    ) -> Result<i64> {
        self.open_cursor(
            session_id,
            batch,
            true,
            (begin, end),
            Columns::NoSource,
            CursorKeys::Export,
        )
    }

    /// Like [`open_message_cursor`](Self::open_message_cursor) over `range` = `(begin, end)`, but only over messages
    /// whose sender `senders.0` accepts (plus the account owner's own messages when `senders.1` is true). Messages
    /// whose sender cannot be resolved from `Name2Id` are always kept, so callers that look at the text prefix still
    /// see them. This lets a per-member scan of a huge group read only that member's rows.
    pub fn open_message_cursor_for_senders(
        &self,
        session_id: &str,
        batch: i32,
        ascending: bool,
        range: (i32, i32),
        lite: bool,
        senders: SenderFilter<'_>,
    ) -> Result<i64> {
        self.open_cursor(
            session_id,
            batch,
            ascending,
            range,
            if lite { Columns::Lite } else { Columns::All },
            CursorKeys::Senders(senders),
        )
    }

    fn open_cursor(
        &self,
        session_id: &str,
        batch: i32,
        ascending: bool,
        (begin, end): (i32, i32),
        columns: Columns,
        listed: CursorKeys<'_>,
    ) -> Result<i64> {
        let (senders, derive) = match listed {
            CursorKeys::All => (None, false),
            CursorKeys::Senders(s) => (Some(s), false),
            CursorKeys::Export => (None, true),
        };
        let tables = self.message_tables(session_id)?;
        let (begin, end) = (begin.max(0) as i64, end.max(0) as i64);
        let mut keys: Vec<Key> = Vec::new();
        let mut derived: Vec<bool> = Vec::with_capacity(tables.len());
        for (idx, t) in tables.iter().enumerate() {
            if derive && begin == 0 && end == 0 && !exact_key_table(t) {
                keys.extend(self.index_keys(t, idx)?);
                derived.push(true);
                continue;
            }
            derived.push(false);
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
                    // senders `Name2Id` does not know are placed by the content later, so they are kept too. Each
                    // term is one SQLite answers from the sender index (WeChat's `<table>_SENDERID`): the unknown
                    // ones are the distinct senders of that index, not rows of the table (#576)
                    format!(
                        " and ({known}real_sender_id is null or real_sender_id in (select distinct real_sender_id \
                         from \"{table}\" where real_sender_id not in (select rowid from Name2Id)))",
                        table = t.table
                    )
                }
            };
            keys.extend(self.table_keys(t, idx, (begin, end), &who)?);
        }
        sort_keys(&mut keys, ascending);
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
                columns,
                derived,
                range: (begin, end),
                ascending,
                rekeying: Arc::default(),
            },
        );
        Ok(id)
    }

    /// The keys of `t` in `create_time` range `(begin, end)` read from its rows (`where` clause extra: `who`).
    fn table_keys(
        &self,
        t: &MsgTable,
        idx: usize,
        (begin, end): (i64, i64),
        who: &str,
    ) -> Result<Vec<Key>> {
        #[cfg(test)]
        TABLE_SCANS.with(|n| n.set(n.get() + 1));
        let sql = format!(
            "select sort_seq, create_time, local_id from \"{}\" where create_time >= ?1 and (?2 = 0 or create_time <= ?2){who}",
            t.table
        );
        self.collect_keys(t, idx, &sql, &[&begin, &end], false)
    }

    /// The keys of every message of `t` (as [`table_keys`](Self::table_keys) without a range lists them) from the
    /// `sort_seq` index alone (`local_id` is the rowid, so the index covers the query): `create_time` is taken as
    /// `sort_seq / 1000`. Rows whose `sort_seq` says nothing about the time (0, negative or none) are read from the
    /// table, as [`typed_row_key`](super::typed_row_key) does.
    fn index_keys(&self, t: &MsgTable, idx: usize) -> Result<Vec<Key>> {
        let sql = format!(
            "select sort_seq, local_id from \"{}\" where sort_seq > 0",
            t.table
        );
        let mut keys = self.collect_keys(t, idx, &sql, &[], true)?;
        let rest = format!(
            "select sort_seq, create_time, local_id from \"{}\" where (sort_seq is null or sort_seq <= 0) and create_time >= 0",
            t.table
        );
        keys.extend(self.collect_keys(t, idx, &rest, &[], false)?);
        Ok(keys)
    }

    fn collect_keys(
        &self,
        t: &MsgTable,
        idx: usize,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
        from_index: bool,
    ) -> Result<Vec<Key>> {
        self.with_db(&t.db, |conn| {
            // Each retry owns a fresh vector: a stale snapshot cannot leave partial or duplicate keys behind.
            let mut keys = Vec::new();
            let mut stmt = conn
                .prepare(sql)
                .with_context(|| format!("bad query: {sql}"))?;
            let mut rows = stmt.query(params)?;
            while let Some(row) = rows.next()? {
                // Match int(): only SQLite integers count; nulls and other storage types become zero.
                let integer = |column: usize| -> rusqlite::Result<i64> {
                    Ok(match row.get_ref(column)? {
                        rusqlite::types::ValueRef::Integer(i) => i,
                        _ => 0,
                    })
                };
                let seq = integer(0)?;
                let key = if from_index {
                    (seq, seq / 1000, integer(1)?, idx)
                } else {
                    let create = integer(1)?;
                    let seq = if seq == 0 { create * 1000 } else { seq };
                    (seq, create, integer(2)?, idx)
                };
                keys.push(key);
            }
            Ok(keys)
        })
    }

    /// The rows of a page showed that the keys the tables `fix` (indexes) took from the `sort_seq` index are not the
    /// real ones: reads the keys of those tables from their rows and remembers the tables; the other tables keep
    /// theirs and are still checked. Fails with [`KEYS_CHANGED`] when that changes the pages.
    fn rekey(&self, cursor: i64, fix: &[usize]) -> Result<()> {
        let rekeying = self.cursor_field(cursor, |c| c.rekeying.clone())?;
        let _one_at_a_time = rekeying
            .lock()
            .map_err(|_| anyhow!("rekey lock poisoned"))?;
        // tables another page fixed while this one waited are done
        let (tables, range, ascending, fix) = self.cursor_field(cursor, |c| {
            let fix: Vec<usize> = fix.iter().copied().filter(|&idx| c.derived[idx]).collect();
            (c.tables.clone(), c.range, c.ascending, fix)
        })?;
        if fix.is_empty() {
            return Ok(());
        }
        if let Ok(mut set) = EXACT_KEY_TABLES.lock() {
            for &idx in &fix {
                set.insert((tables[idx].db.clone(), tables[idx].table.clone()));
            }
        }
        let mut exact: Vec<(usize, Vec<Key>)> = Vec::with_capacity(fix.len());
        for &idx in &fix {
            exact.push((idx, self.table_keys(&tables[idx], idx, range, "")?));
        }
        let mut cursors = self
            .cursors
            .lock()
            .map_err(|_| anyhow!("cursor lock poisoned"))?;
        let c = cursors
            .open
            .get_mut(&cursor)
            .ok_or_else(|| anyhow!("unknown message cursor {cursor}"))?;
        // The rows are read from a snapshot that may be newer than the cursor: rows WeChat added since stay out, and a
        // row it deleted keeps its key (no page finds it), as when the keys are read at open. So only an order the
        // index got wrong changes the pages.
        for (idx, table) in &mut exact {
            let listed: HashMap<i64, Key> = c
                .keys
                .iter()
                .filter(|k| k.3 == *idx)
                .map(|k| (k.2, *k))
                .collect();
            table.retain(|k| listed.contains_key(&k.2));
            let found: HashSet<i64> = table.iter().map(|k| k.2).collect();
            table.extend(
                listed
                    .iter()
                    .filter(|(local, _)| !found.contains(local))
                    .map(|(_, k)| *k),
            );
        }
        let mut keys: Vec<Key> = c
            .keys
            .iter()
            .filter(|k| !exact.iter().any(|(idx, _)| *idx == k.3))
            .copied()
            .collect();
        for (idx, table) in exact {
            keys.extend(table);
            c.derived[idx] = false;
        }
        sort_keys(&mut keys, ascending);
        let same_pages = c.keys.len() == keys.len()
            && c.keys
                .iter()
                .zip(&keys)
                .all(|(a, b)| (a.2, a.3) == (b.2, b.3));
        c.keys = keys;
        if same_pages {
            Ok(())
        } else {
            Err(anyhow!(KEYS_CHANGED))
        }
    }

    /// `read` of the open cursor `cursor`.
    fn cursor_field<T>(&self, cursor: i64, read: impl FnOnce(&MessageCursor) -> T) -> Result<T> {
        let cursors = self
            .cursors
            .lock()
            .map_err(|_| anyhow!("cursor lock poisoned"))?;
        let c = cursors
            .open
            .get(&cursor)
            .ok_or_else(|| anyhow!("unknown message cursor {cursor}"))?;
        Ok(read(c))
    }

    /// Next batch of rows and whether more remain.
    pub fn fetch_message_batch(&self, cursor: i64) -> Result<(Vec<MsgRow>, bool)> {
        let (tables, chunk, columns, more, derived) = {
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
            (
                c.tables.clone(),
                chunk,
                c.columns,
                c.pos < c.keys.len(),
                c.derived.clone(),
            )
        };
        let (rows, mismatched) = self.rows_for_keys(&tables, &chunk, columns, &derived)?;
        if !mismatched.is_empty() {
            self.rekey(cursor, &mismatched)?;
        }
        Ok((rows, more))
    }

    /// (messages, batch size) of a cursor: the batches are the messages in cursor order, `batch size` at a time.
    pub fn message_cursor_len(&self, cursor: i64) -> Result<(usize, usize)> {
        let cursors = self
            .cursors
            .lock()
            .map_err(|_| anyhow!("cursor lock poisoned"))?;
        let c = cursors
            .open
            .get(&cursor)
            .ok_or_else(|| anyhow!("unknown message cursor {cursor}"))?;
        Ok((c.keys.len(), c.batch))
    }

    /// Batch number `page` of a cursor (the rows [`fetch_message_batch`](Self::fetch_message_batch) returns on its
    /// `page + 1`-th call), without moving the cursor: threads can read different batches of one cursor at once.
    /// Past the end it is empty.
    pub fn fetch_message_page(&self, cursor: i64, page: usize) -> Result<Vec<MsgRow>> {
        let (tables, chunk, columns, derived) = {
            let cursors = self
                .cursors
                .lock()
                .map_err(|_| anyhow!("cursor lock poisoned"))?;
            let c = cursors
                .open
                .get(&cursor)
                .ok_or_else(|| anyhow!("unknown message cursor {cursor}"))?;
            let start = page.saturating_mul(c.batch).min(c.keys.len());
            let end = start.saturating_add(c.batch).min(c.keys.len());
            (
                c.tables.clone(),
                c.keys[start..end].to_vec(),
                c.columns,
                c.derived.clone(),
            )
        };
        let (rows, mismatched) = self.rows_for_keys(&tables, &chunk, columns, &derived)?;
        if !mismatched.is_empty() {
            self.rekey(cursor, &mismatched)?;
        }
        Ok(rows)
    }

    /// The rows of `keys`, in that order, and the tables whose keys came from the `sort_seq` index (`derived`) and
    /// have a row with another key than the one it was listed under.
    fn rows_for_keys(
        &self,
        tables: &[MsgTable],
        keys: &[Key],
        columns: Columns,
        derived: &[bool],
    ) -> Result<(Vec<MsgRow>, Vec<usize>)> {
        let listed: HashMap<(usize, i64), (i64, i64)> = keys
            .iter()
            .filter(|k| derived.get(k.3).copied().unwrap_or(false))
            .map(|&(seq, create, local, idx)| ((idx, local), (seq, create)))
            .collect();
        let mut mismatched: Vec<usize> = Vec::new();
        let mut by_shard: HashMap<usize, Vec<i64>> = HashMap::new();
        for (_, _, local, idx) in keys {
            by_shard.entry(*idx).or_default().push(*local);
        }
        let mut fetched: HashMap<(usize, i64), MsgRow> = HashMap::new();
        for (idx, ids) in by_shard {
            let t = &tables[idx];
            let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            let sql = Self::select_sql_of(t, &format!("where m.local_id in ({list})"), columns);
            for r in self.read_rows(t, &sql, &[])? {
                let (seq, create, local) = r.key();
                if listed
                    .get(&(idx, local))
                    .is_some_and(|k| *k != (seq, create))
                    && !mismatched.contains(&idx)
                {
                    mismatched.push(idx);
                }
                fetched.insert((idx, local), r);
            }
        }
        let rows = keys
            .iter()
            .filter_map(|(_, _, local, idx)| fetched.remove(&(*idx, *local)))
            .collect();
        Ok((rows, mismatched))
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
#[expect(
    clippy::cast_possible_truncation,
    reason = "test values are known to fit, or are meant to wrap"
)]
mod tests {
    use super::*;
    use crate::fixture::TestDir;
    use crate::msg::table_name_for;
    use crate::msg::tests::{account, keyed_account, texts, DAY, T0};
    use crate::sqlcipher::testutil::{encrypt_db, plain_db_with, KEY, SALT};
    use crate::sqlcipher::PageCipher;

    /// An account with `shards` of `wxid_bob`'s messages, each in its own directory.
    fn export_world(tag: &str, shards: &[Vec<crate::fixture::MsgSpec>]) -> (TestDir, Account) {
        use crate::fixture::{Fixture, SessionSpec, T0};
        let root = TestDir::new(&format!("weflow-keys-{tag}"));
        let f = Fixture::new(&root, "wxid_me_ab12");
        f.session_db(&[SessionSpec {
            username: "wxid_bob",
            summary: "",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 1,
        }]);
        for (i, msgs) in shards.iter().enumerate() {
            f.message_shard(i as u32, &[("wxid_bob", msgs.clone())]);
        }
        let acct = Account::new(f.db_storage(), &f.key_hex()).unwrap();
        (root, acct)
    }

    fn cursor_state(acct: &Account, id: i64) -> (Vec<Key>, Vec<bool>) {
        let cursors = acct.cursors.lock().unwrap();
        let c = &cursors.open[&id];
        (c.keys.clone(), c.derived.clone())
    }

    fn page_ids(acct: &Account, id: i64) -> Result<Vec<i64>> {
        let mut ids = Vec::new();
        for page in 0.. {
            let rows = acct.fetch_message_page(id, page)?;
            if rows.is_empty() {
                return Ok(ids);
            }
            ids.extend(rows.iter().map(|r| r.local_id));
        }
        unreachable!()
    }

    #[test]
    fn cursor_keys_preserve_null_and_non_integer_fallbacks() {
        let root = TestDir::new("weflow-key-types");
        let fixture = crate::fixture::Fixture::new(&root, "wxid_synthetic_ab12");
        let table = table_name_for("wxid_bob");
        let plain = plain_db_with(|conn| {
            // No affinity on sort_seq: exercise all SQLite storage classes, including an integral REAL.
            conn.execute_batch(&format!(
                "create table Name2Id(user_name text);
                 insert into Name2Id values ('wxid_bob');
                 create table \"{table}\"(local_id integer primary key, server_id integer, local_type integer,
                     sort_seq, create_time integer, status integer, real_sender_id integer, compress_content,
                     message_content text, source text, packed_info_data blob);
                 create index key_types_sort on \"{table}\"(sort_seq);
                 insert into \"{table}\"(local_id, sort_seq, create_time, real_sender_id, message_content) values
                     (1, {seq}, {T0}, 1, 'integer'),
                     (2, null, {T0}+1, 1, 'null'),
                     (3, 0, {T0}+2, 1, 'zero'),
                     (4, -1, {T0}+3, 1, 'negative'),
                     (5, {seq}.0, {T0}+4, 1, 'real'),
                     (6, 'invalid', {T0}+5, 1, 'text'),
                     (7, x'31', {T0}+6, 1, 'blob');",
                seq = T0 * 1000 + 9,
            ))
            .unwrap();
        });
        let cipher = PageCipher::derive(&KEY, &SALT);
        std::fs::write(
            fixture.db_storage().join("message/message_0.db"),
            encrypt_db(&plain, &cipher),
        )
        .unwrap();
        let acct = Account::new(fixture.db_storage(), &fixture.key_hex()).unwrap();
        let exact = acct
            .open_message_cursor("wxid_bob", 2, true, 0, 0, false)
            .unwrap();
        assert_eq!(
            cursor_state(&acct, exact).0,
            vec![
                (-1, T0 + 3, 4, 0),
                (T0 * 1000 + 9, T0, 1, 0),
                ((T0 + 1) * 1000, T0 + 1, 2, 0),
                ((T0 + 2) * 1000, T0 + 2, 3, 0),
                ((T0 + 4) * 1000, T0 + 4, 5, 0),
                ((T0 + 5) * 1000, T0 + 5, 6, 0),
                ((T0 + 6) * 1000, T0 + 6, 7, 0),
            ]
        );
        let fast = acct.open_export_cursor("wxid_bob", 2, 0, 0).unwrap();
        assert_eq!(
            cursor_state(&acct, fast).0,
            vec![
                (-1, T0 + 3, 4, 0),
                (0, 0, 5, 0),
                (0, 0, 6, 0),
                (0, 0, 7, 0),
                (T0 * 1000 + 9, T0, 1, 0),
                ((T0 + 1) * 1000, T0 + 1, 2, 0),
                ((T0 + 2) * 1000, T0 + 2, 3, 0),
            ]
        );
        assert!(acct
            .fetch_message_page(fast, 0)
            .unwrap_err()
            .to_string()
            .contains(KEYS_CHANGED));
        let again = acct.open_export_cursor("wxid_bob", 2, 0, 0).unwrap();
        assert_eq!(cursor_state(&acct, again).0, cursor_state(&acct, exact).0);
        assert_eq!(page_ids(&acct, again).unwrap(), vec![4, 1, 2, 3, 5, 6, 7]);
    }

    #[test]
    fn bounded_export_keeps_messages_whose_sort_seq_time_is_outside_the_range() {
        use crate::fixture::{MsgSpec, T0};
        let (_dir, acct) = export_world(
            "range-excludes-derived-key",
            &[vec![
                MsgSpec::text(1, "wxid_bob", T0 + 100, "inside").with_sort_seq(T0 * 1000),
                MsgSpec::text(2, "wxid_bob", T0 + 200, "outside"),
            ]],
        );
        let cursor = acct
            .open_export_cursor("wxid_bob", 2, (T0 + 100) as i32, (T0 + 100) as i32)
            .unwrap();
        assert_eq!(acct.message_cursor_len(cursor).unwrap().0, 1,
            "the real timestamp is inside the range even though the index-derived timestamp is outside");
        assert_eq!(page_ids(&acct, cursor).unwrap(), vec![1]);
    }

    #[test]
    fn export_cursor_keys_from_the_index_are_the_keys_of_the_rows() {
        use crate::fixture::{MsgSpec, T0};
        let text = |id: i64, at: i64| MsgSpec::text(id, "wxid_bob", at, "x");
        // WeChat's rule (`sort_seq` = `create_time` in ms + a counter), ties in `sort_seq`, a row without `sort_seq`,
        // two shards with overlapping local ids
        let (_dir, acct) = export_world(
            "rule",
            &[
                vec![
                    text(1, T0).with_sort_seq(T0 * 1000 + 3),
                    text(2, T0 + 2),
                    text(3, T0 + 2),
                    text(4, T0 + 4).with_sort_seq(0),
                    text(5, T0 + 6).with_sort_seq((T0 + 6) * 1000 + 999),
                ],
                vec![text(1, T0 + 1), text(2, T0 + 5), text(3, T0 + 7)],
            ],
        );
        for (begin, end) in [
            (0, 0),
            (T0 + 2, T0 + 5),
            (T0 + 4, 0),
            (0, T0 + 6),
            (T0 + 6, T0 + 6),
        ] {
            let (b, e) = (begin as i32, end as i32);
            let exact = acct
                .open_message_cursor("wxid_bob", 2, true, b, e, false)
                .unwrap();
            let fast = acct.open_export_cursor("wxid_bob", 2, b, e).unwrap();
            let (exact_keys, _) = cursor_state(&acct, exact);
            let (fast_keys, derived) = cursor_state(&acct, fast);
            assert_eq!(fast_keys, exact_keys, "range {begin}..{end}");
            let expected_derived = vec![begin == 0 && end == 0; 2];
            assert_eq!(derived, expected_derived);
            assert_eq!(
                page_ids(&acct, fast).unwrap(),
                page_ids(&acct, exact).unwrap()
            );
            assert_eq!(
                cursor_state(&acct, fast).1,
                expected_derived,
                "the rows agreed with the keys"
            );
        }
    }

    #[test]
    fn a_table_another_page_read_again_is_not_read_once_more() {
        use crate::fixture::{MsgSpec, T0};
        let text = |id: i64, at: i64| MsgSpec::text(id, "wxid_bob", at, "x");
        let (_dir, acct) = export_world(
            "off-once",
            &[vec![
                text(1, T0),
                text(2, T0 + 1).with_sort_seq((T0 + 50) * 1000),
                text(3, T0 + 60),
            ]],
        );
        let id = acct.open_export_cursor("wxid_bob", 2, 0, 0).unwrap();
        let scans = || TABLE_SCANS.with(std::cell::Cell::get);
        let before = scans();
        // two pages found the table off the rule at once: the second waited for the first one's scan
        acct.rekey(id, &[0]).unwrap();
        acct.rekey(id, &[0]).unwrap();
        assert_eq!(scans() - before, 1, "the table was read once");
        assert_eq!(cursor_state(&acct, id).1, vec![false]);
        assert_eq!(page_ids(&acct, id).unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn a_table_off_the_rule_is_read_again_and_remembered() {
        use crate::fixture::{MsgSpec, T0};
        let text = |id: i64, at: i64| MsgSpec::text(id, "wxid_bob", at, "x");
        // message 2's sort_seq says T0 + 50 but it was sent at T0 + 1: the order stays the same. The second shard
        // follows the rule
        let (_dir, acct) = export_world(
            "off",
            &[
                vec![
                    text(1, T0),
                    text(2, T0 + 1).with_sort_seq((T0 + 50) * 1000),
                    text(3, T0 + 60),
                ],
                vec![text(11, T0 + 70), text(12, T0 + 80)],
            ],
        );
        let id = acct.open_export_cursor("wxid_bob", 2, 0, 0).unwrap();
        assert_eq!(
            page_ids(&acct, id).unwrap(),
            vec![1, 2, 3, 11, 12],
            "the pages did not change"
        );
        let (keys, derived) = cursor_state(&acct, id);
        assert_eq!(
            derived,
            vec![false, true],
            "only the table off the rule was read again"
        );
        assert_eq!(keys[1], ((T0 + 50) * 1000, T0 + 1, 2, 0));
        let again = acct.open_export_cursor("wxid_bob", 2, 0, 0).unwrap();
        assert_eq!(
            cursor_state(&acct, again).1,
            vec![false, true],
            "the table is remembered"
        );
    }

    #[test]
    fn a_table_off_the_rule_that_changes_the_pages_tells_the_export_to_start_over() {
        use crate::fixture::{MsgSpec, T0};
        let text = |id: i64, at: i64| MsgSpec::text(id, "wxid_bob", at, "x");
        // the same sort_seq, but message 2 was sent first: the rows order them 2, 1 and the index 1, 2
        let seq = (T0 + 1) * 1000;
        let (_dir, acct) = export_world(
            "pages",
            &[vec![
                text(1, T0 + 9).with_sort_seq(seq),
                text(2, T0 + 1).with_sort_seq(seq),
                text(3, T0 + 20),
            ]],
        );
        let id = acct.open_export_cursor("wxid_bob", 2, 0, 0).unwrap();
        let err = page_ids(&acct, id).unwrap_err();
        assert!(err.to_string().contains(KEYS_CHANGED), "{err}");
        let again = acct.open_export_cursor("wxid_bob", 2, 0, 0).unwrap();
        assert_eq!(page_ids(&acct, again).unwrap(), vec![2, 1, 3]);
    }

    #[test]
    fn rows_written_after_the_cursor_opened_do_not_change_its_pages() {
        use crate::fixture::{Fixture, MsgSpec, SessionSpec, T0};
        let text = |id: i64, at: i64| MsgSpec::text(id, "wxid_bob", at, "x");
        let root = TestDir::new("weflow-keys-newer");
        let f = Fixture::new(&root, "wxid_me_ab12");
        f.session_db(&[SessionSpec {
            username: "wxid_bob",
            summary: "",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 1,
        }]);
        // message 2 is off the rule (the table is read again at the first page), in the order of the index
        let off = |extra: Vec<MsgSpec>| {
            let mut msgs = vec![
                text(1, T0),
                text(2, T0 + 1).with_sort_seq((T0 + 50) * 1000),
                text(3, T0 + 60),
            ];
            msgs.extend(extra);
            msgs
        };
        f.message_shard(0, &[("wxid_bob", off(vec![]))]);
        let acct = Account::new(f.db_storage(), &f.key_hex()).unwrap();
        let id = acct.open_export_cursor("wxid_bob", 2, 0, 0).unwrap();
        // WeChat writes two messages, one between the listed ones, before the first page is read
        f.message_shard(
            0,
            &[("wxid_bob", off(vec![text(4, T0 + 30), text(5, T0 + 90)]))],
        );
        let shard = f.db_storage().join("message/message_0.db");
        std::fs::File::options()
            .write(true)
            .open(&shard)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600))
            .unwrap();
        assert_eq!(
            page_ids(&acct, id).unwrap(),
            vec![1, 2, 3],
            "the pages are the ones the cursor listed"
        );
        assert_eq!(
            cursor_state(&acct, id).1,
            vec![false],
            "the table was read again"
        );
        let again = acct.open_export_cursor("wxid_bob", 2, 0, 0).unwrap();
        assert_eq!(page_ids(&acct, again).unwrap(), vec![1, 4, 2, 3, 5]);
    }

    /// Equal keys of different databases (a message both of them keep) come by ascending database number in a
    /// cursor of either direction, as the pages by offset list them.
    #[test]
    fn equal_keys_of_two_databases_come_in_database_order_both_ways() {
        let row = vec![(1, "null".to_string(), (T0 + 5).to_string())];
        let (_dir, acct) = keyed_account("equal-keys", &[row.clone(), row]);
        for ascending in [false, true] {
            let cursor = acct
                .open_message_cursor("wxid_bob", 10, ascending, 0, 0, false)
                .unwrap();
            let (rows, _) = acct.fetch_message_batch(cursor).unwrap();
            assert_eq!(
                rows.iter().map(|r| r.shard).collect::<Vec<_>>(),
                [0, 1],
                "ascending {ascending}"
            );
        }
        assert_eq!(
            acct.messages("wxid_bob", 2, 0)
                .unwrap()
                .iter()
                .map(|r| r.shard)
                .collect::<Vec<_>>(),
            [0, 1]
        );
    }

    /// Each column typed, the content inflated, the sender resolved and whether the owner sent it, the database by its
    /// shard; lite rows of a cursor leave out the source and the packed info, an export's the source only.
    #[test]
    fn rows_carry_typed_columns_sender_and_inflated_content() {
        let (_dir, acct) = account("rows");
        let acct = acct.with_my_wxid(Some("wxid_me".into()));
        let rows = acct.messages("wxid_bob", 0, 0).unwrap();
        let packed = Some(vec![0u8, 1, 2, 0xff]);
        let row =
            |local_id: i64, day: i64, local_type: i64, sender: &str, content: &str, shard: u32| {
                let create_time = T0 + day * DAY;
                MsgRow {
                    local_id,
                    server_id: Some(day * DAY),
                    local_type,
                    sort_seq: Some(create_time * 1000),
                    create_time: Some(create_time),
                    status: None,
                    sender: Some(sender.to_string()),
                    is_send: sender == "wxid_me",
                    content: content.to_string(),
                    source: Some("<msgsource/>".into()),
                    packed_info: packed.clone(),
                    shard,
                }
            };
        assert_eq!(
            rows,
            [
                row(2, 4, 34, "wxid_bob", "voice", 1),
                row(1, 3, 1, "wxid_me", "fourth", 1),
                // stored as a zstd blob
                row(3, 2, 1, "wxid_bob", "third", 0),
                row(2, 1, 3, "wxid_me", "img", 0),
                row(1, 0, 1, "wxid_bob", "hello", 0),
            ]
        );
        let cursor = acct
            .open_message_cursor("wxid_bob", 100, false, 0, 0, true)
            .unwrap();
        let (lite, _) = acct.fetch_message_batch(cursor).unwrap();
        let unpacked: Vec<MsgRow> = rows
            .iter()
            .cloned()
            .map(|r| MsgRow {
                source: None,
                packed_info: None,
                ..r
            })
            .collect();
        assert_eq!(lite, unpacked);
        // oldest first
        let export = acct.open_export_cursor("wxid_bob", 100, 0, 0).unwrap();
        let (exported, _) = acct.fetch_message_batch(export).unwrap();
        let sourceless: Vec<MsgRow> = rows
            .into_iter()
            .rev()
            .map(|r| MsgRow { source: None, ..r })
            .collect();
        assert_eq!(exported, sourceless);
    }

    #[test]
    fn cursor_walks_all_messages_in_order_across_shards() {
        let (_dir, acct) = account("cursor");
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
    fn pages_by_number_match_the_batches_and_leave_the_cursor_alone() {
        let (_dir, acct) = account("cursor-pages");
        let c = acct
            .open_message_cursor("wxid_bob", 2, true, 0, 0, false)
            .unwrap();
        assert_eq!(acct.message_cursor_len(c).unwrap(), (5, 2));
        // out of order, repeated and past the end
        let page = |i| texts(&acct.fetch_message_page(c, i).unwrap());
        assert_eq!(page(2), ["voice"]);
        assert_eq!(page(0), ["hello", "img"]);
        assert_eq!(page(1), ["third", "fourth"]);
        assert_eq!(page(1), ["third", "fourth"]);
        assert!(page(3).is_empty());
        // the sequential reads still start at the beginning
        let (rows, more) = acct.fetch_message_batch(c).unwrap();
        assert_eq!(
            (texts(&rows), more),
            (vec!["hello".to_string(), "img".into()], true)
        );
        acct.close_message_cursor(c).unwrap();
        assert!(acct.fetch_message_page(c, 0).is_err());
        assert!(acct.message_cursor_len(c).is_err());
    }

    #[test]
    fn parallel_page_reads_open_at_most_the_allowed_connections() {
        let (dir, acct) = account("cursor-parallel");
        acct.set_read_connections(3);
        let c = acct
            .open_message_cursor("wxid_bob", 1, true, 0, 0, false)
            .unwrap();
        let expected: Vec<Vec<String>> = (0..5)
            .map(|i| texts(&acct.fetch_message_page(c, i).unwrap()))
            .collect();
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    for round in 0..20 {
                        let i = round % 5;
                        assert_eq!(texts(&acct.fetch_message_page(c, i).unwrap()), expected[i]);
                    }
                });
            }
        });
        for shard in ["message/message_0.db", "message/message_1.db"] {
            let n = acct.snapshot_connections(&dir.join(shard));
            assert!((1..=3).contains(&n), "{shard}: {n} connections");
        }
    }

    #[test]
    fn cursor_honours_time_range_direction_and_lite() {
        let (_dir, acct) = account("range");
        // [day1, day3] inclusive, newest first, lite rows
        let (begin, end) = ((T0 + DAY) as i32, (T0 + DAY * 3) as i32);
        let c = acct
            .open_message_cursor("wxid_bob", 10, false, begin, end, true)
            .unwrap();
        let (rows, more) = acct.fetch_message_batch(c).unwrap();
        assert!(!more);
        assert_eq!(texts(&rows), ["fourth", "third", "img"]);
        let first = &rows[0];
        assert!(first.packed_info.is_none() && first.source.is_none());
        assert!(!first.content.is_empty() && first.sender.is_some());
    }

    #[test]
    fn sender_filtered_cursor_reads_only_the_wanted_senders() {
        let (_dir, acct) = account("senders");
        let acct = acct.with_my_wxid(Some("wxid_me".into()));
        let read = |ok: &dyn Fn(&str) -> bool, mine: bool| -> Vec<String> {
            let c = acct
                .open_message_cursor_for_senders("wxid_bob", 100, true, (0, 0), true, (ok, mine))
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
