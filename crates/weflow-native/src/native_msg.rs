//! Message reading on top of [`NativeAccount`]: shard/table lookup, paging, per-day statistics
//! and cursors. Rows are read as [`MsgRow`]s.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{anyhow, Context, Result};
use md5::{Digest, Md5};
use serde_json::{json, Map, Value};

use crate::msg_row::{MsgRow, RowContext};
use crate::native_db::NativeAccount;

/// A quarter of an hour in seconds: the step of the local dates of messages.
const QUARTER: i64 = 900;

/// One message location: which shard database, which table.
#[derive(Clone, Debug)]
pub struct MsgTable {
    pub db: PathBuf,
    pub table: String,
}

/// Sort key of a message: (sort_seq, create_time, local_id, shard index).
type Key = (i64, i64, i64, usize);

/// Sender filter of a cursor: (does this username match, also keep the account owner's own messages).
pub type SenderFilter<'a> = (&'a dyn Fn(&str) -> bool, bool);

/// What a page read of an export cursor fails with when its keys had to be read again and the pages are no longer
/// the same (see [`NativeAccount::open_export_cursor`]): the export has to start over.
pub const KEYS_CHANGED: &str =
    "the order of the messages changed while they were read; the export has to start over";

/// Tables whose `create_time` turned out not to be `sort_seq / 1000`: export cursors read their keys from the table.
static EXACT_KEY_TABLES: LazyLock<Mutex<HashSet<(PathBuf, String)>>> =
    LazyLock::new(Mutex::default);

/// Which messages a cursor lists, and how it reads their keys.
enum CursorKeys<'a> {
    /// Every message, keys read from the rows.
    All,
    /// The messages of some senders (see [`NativeAccount::open_message_cursor_for_senders`]).
    Senders(SenderFilter<'a>),
    /// Every message, keys from the `sort_seq` index (see [`NativeAccount::open_export_cursor`]).
    Export,
}

#[cfg(test)]
thread_local! {
    /// Tables this thread read every key of ([`NativeAccount::table_keys`]).
    static TABLE_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn exact_key_table(t: &MsgTable) -> bool {
    EXACT_KEY_TABLES
        .lock()
        .is_ok_and(|set| set.contains(&(t.db.clone(), t.table.clone())))
}

pub struct MessageCursor {
    tables: Vec<MsgTable>,
    keys: Vec<Key>,
    pos: usize,
    batch: usize,
    lite: bool,
    /// Per table: whether its keys came from the `sort_seq` index ([`NativeAccount::open_export_cursor`]) and are
    /// still to be checked against the rows.
    derived: Vec<bool>,
    range: (i64, i64),
    ascending: bool,
    /// Held while keys are read again from the rows ([`NativeAccount::rekey`]): workers whose pages find the same
    /// table's keys wrong at once wait for one scan of it instead of each scanning the whole table.
    rekeying: Arc<Mutex<()>>,
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

/// Where a page of [`NativeAccount::messages_from`] starts.
#[derive(Clone, Copy, Debug)]
pub enum PageFrom {
    /// The messages listed after this one: older ones.
    Before(MsgPos),
    /// The messages listed before this one: newer ones, the closest first taken.
    After(MsgPos),
}

/// Where [`NativeAccount::messages`] starts a page deep into a conversation.
enum OffsetPosition {
    /// After this message, the last one the offset skips.
    At(MsgPos),
    /// The offset skips every message.
    Past,
    /// A sort key of the rows is not stored as an integer (a `sort_seq` may also be null): [`typed_row_key`] and SQLite
    /// order such values differently, so only reading the rows lists them as before.
    Unusual,
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

/// The number of the `message_<n>.db` holding a table.
pub(crate) fn shard_of(t: &MsgTable) -> Option<u32> {
    t.db.file_name()
        .and_then(|n| n.to_str())
        .and_then(is_message_db)
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

    /// `select` of the [`MsgRow`] columns over one message table with the sender resolved from `Name2Id`.
    pub(crate) fn select_sql(t: &MsgTable, tail: &str, lite: bool) -> String {
        format!(
            "select {} from \"{}\" m left join Name2Id n on n.rowid = m.real_sender_id {tail}",
            MsgRow::columns(lite),
            t.table
        )
    }

    /// The rows of [`select_sql`](Self::select_sql) with `indexed` (rows with a `sort_seq`), then those with
    /// `unsequenced` (rows without one), as one statement: a second one per table costs its own preparation.
    fn select_both_sql(t: &MsgTable, indexed: &str, unsequenced: &str) -> String {
        format!(
            "select * from ({}) union all select * from ({})",
            Self::select_sql(t, indexed, false),
            Self::select_sql(t, unsequenced, false)
        )
    }

    /// The rows of `sql` over `t`, whose select list is [`MsgRow::columns`].
    pub(crate) fn read_rows(
        &self,
        t: &MsgTable,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<MsgRow>> {
        let out = std::cell::RefCell::new(Vec::new());
        self.read_rows_each(
            t,
            sql,
            params,
            || out.borrow_mut().clear(),
            |row| out.borrow_mut().push(row),
        )?;
        Ok(out.into_inner())
    }

    /// [`read_rows`](Self::read_rows), each row handed to `each` as it is read. A query read again on a fresh
    /// snapshot (see `with_db`) starts over: `start` runs before every attempt.
    pub(crate) fn read_rows_each(
        &self,
        t: &MsgTable,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
        mut start: impl FnMut(),
        mut each: impl FnMut(MsgRow),
    ) -> Result<()> {
        let is_me = |sender: &str| self.is_me(sender);
        let ctx = RowContext {
            table: t,
            is_me: &is_me,
        };
        self.with_db(&t.db, |conn| {
            start();
            let mut stmt = conn
                .prepare(sql)
                .with_context(|| format!("bad query: {sql}"))?;
            let mut rows = stmt.query(params)?;
            while let Some(row) = rows.next()? {
                each(MsgRow::read(row, &ctx)?);
            }
            Ok(())
        })
    }

    /// The messages of `session_id` sent at `since` or later that `condition` keeps (SQL over the message table
    /// `m`), oldest first in each table: at most `limit` of each table, the tables one after the other.
    pub fn messages_where(
        &self,
        session_id: &str,
        since: i64,
        condition: &str,
        limit: usize,
    ) -> Result<Vec<MsgRow>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let tail = format!(
            "where m.create_time >= ?1 and ({condition}) \
             order by m.create_time asc, m.sort_seq asc, m.local_id asc limit ?2"
        );
        let mut out = Vec::new();
        for t in self.message_tables(session_id)? {
            let sql = Self::select_sql(&t, &tail, false);
            out.extend(self.read_rows(&t, &sql, &[&since.max(0), &limit])?);
        }
        Ok(out)
    }

    /// Newest-first page of messages across all shards.
    pub fn messages(&self, session_id: &str, limit: i32, offset: i32) -> Result<Vec<MsgRow>> {
        let (offset, limit) = (offset.max(0) as usize, limit.max(0) as usize);
        let tables = self.message_tables(session_id)?;
        if offset > 0 {
            // A page deep into a long conversation: reading every row before it costs what the offset does (issue
            // #486), so find the last message it skips from the sort keys alone and page on from there.
            match self.offset_position(&tables, offset, limit)? {
                OffsetPosition::At(pos) => {
                    return self.page_from(
                        &tables,
                        i32::try_from(limit).unwrap_or(i32::MAX),
                        PageFrom::Before(pos),
                    );
                }
                OffsetPosition::Past => return Ok(Vec::new()),
                OffsetPosition::Unusual => {}
            }
        }
        self.newest_rows(&tables, offset, limit)
    }

    /// The SQL of [`newest_rows`](Self::newest_rows) for one table: its `want` (`?1`) newest rows with a `sort_seq`,
    /// at `?2` or later when `floor`, then its `want` newest without one. The [`MsgRow`] columns with the sender joined,
    /// or only `columns`.
    fn newest_sql(t: &MsgTable, columns: Option<&str>, floor: bool) -> String {
        let order = "order by m.sort_seq desc, m.create_time desc, m.local_id desc limit ?1";
        let floor = if floor { "m.sort_seq >= ?2 and " } else { "" };
        let indexed = format!("where {floor}m.sort_seq <> 0 {order}");
        // rows without a sort_seq are keyed by create_time * 1000 (see [`MsgPos`]): below any floor, they are read
        // apart
        let unsequenced = "where (m.sort_seq is null or m.sort_seq = 0) \
                           order by m.create_time desc, m.local_id desc limit ?1";
        match columns {
            None => Self::select_both_sql(t, &indexed, unsequenced),
            Some(columns) => {
                let table = &t.table;
                format!(
                    "select * from (select {columns} from \"{table}\" m {indexed}) \
                     union all select * from (select {columns} from \"{table}\" m {unsequenced})"
                )
            }
        }
    }

    /// Where a page `offset` messages deep starts: the place of the last message it skips. Read from the sort keys of
    /// the rows [`newest_rows`](Self::newest_rows) would read, not from the rows themselves.
    fn offset_position(
        &self,
        tables: &[MsgTable],
        offset: usize,
        limit: usize,
    ) -> Result<OffsetPosition> {
        let want = if limit == 0 {
            -1
        } else {
            (offset + limit) as i64
        };
        let mut keys: Vec<((i64, i64, i64), u32)> = Vec::new();
        for t in tables {
            let Some(shard) = shard_of(t) else {
                return Ok(OffsetPosition::Unusual);
            };
            let floor = if want > 0 {
                self.sort_seq_floor(t, want)?
            } else {
                None
            };
            let sql = Self::newest_sql(t, Some("sort_seq, create_time, local_id"), floor.is_some());
            let read = self.with_db(&t.db, |conn| {
                // Each retry owns a fresh vector: a stale snapshot cannot leave partial or duplicate keys behind.
                let mut found = Vec::new();
                let mut stmt = conn
                    .prepare(&sql)
                    .with_context(|| format!("bad query: {sql}"))?;
                let mut rows = match floor {
                    Some(floor) => stmt.query(rusqlite::params![want, floor])?,
                    None => stmt.query(rusqlite::params![want])?,
                };
                while let Some(row) = rows.next()? {
                    let Some(key) =
                        typed_row_key(row.get_ref(0)?, row.get_ref(1)?, row.get_ref(2)?)
                    else {
                        return Ok(None);
                    };
                    found.push((key, shard));
                }
                Ok(Some(found))
            })?;
            let Some(read) = read else {
                return Ok(OffsetPosition::Unusual);
            };
            keys.extend(read);
        }
        // the listing's order: newest key first, equal keys by ascending database
        keys.sort_unstable_by(|(a, sa), (b, sb)| b.cmp(a).then(sa.cmp(sb)));
        Ok(match keys.get(offset - 1) {
            Some(&((sort_seq, create_time, local_id), shard)) => OffsetPosition::At(MsgPos {
                sort_seq,
                create_time,
                local_id,
                shard,
            }),
            None => OffsetPosition::Past,
        })
    }

    /// Newest-first page of the messages of `tables`, from the rows before it and the page read in full and sorted.
    fn newest_rows(&self, tables: &[MsgTable], offset: usize, limit: usize) -> Result<Vec<MsgRow>> {
        let want = if limit == 0 {
            -1
        } else {
            (offset + limit) as i64
        };
        let mut rows: Vec<MsgRow> = Vec::new();
        for t in tables {
            // Asked as is, SQLite sorts the whole table (it walks the sender index for the join). The `want`-th
            // newest sort_seq, read from the sort_seq index, bounds the same rows to a range that index serves.
            let floor = if want > 0 {
                self.sort_seq_floor(t, want)?
            } else {
                None
            };
            let sql = Self::newest_sql(t, None, floor.is_some());
            rows.extend(match floor {
                Some(floor) => self.read_rows(t, &sql, &[&want, &floor])?,
                None => self.read_rows(t, &sql, &[&want])?,
            });
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.key()));
        let it = rows.into_iter().skip(offset);
        Ok(if limit == 0 {
            it.collect()
        } else {
            it.take(limit).collect()
        })
    }

    /// A newest-first page of messages next to a message (see [`MsgPos`]): `limit` of the older ones, or of the newer
    /// ones closest to it, still listed newest first; `limit == 0` means all of them. Each table is read from the
    /// position on through its `sort_seq` index, so a page deep into a long conversation costs what the first does.
    pub fn messages_from(
        &self,
        session_id: &str,
        limit: i32,
        from: PageFrom,
    ) -> Result<Vec<MsgRow>> {
        self.page_from(&self.message_tables(session_id)?, limit, from)
    }

    /// [`messages_from`](Self::messages_from) over the message tables of a conversation.
    fn page_from(&self, tables: &[MsgTable], limit: i32, from: PageFrom) -> Result<Vec<MsgRow>> {
        let want = if limit <= 0 { -1 } else { i64::from(limit) };
        let (pos, older) = match from {
            PageFrom::Before(p) => (p, true),
            PageFrom::After(p) => (p, false),
        };
        let dir = if older { "desc" } else { "asc" };
        let mut rows: Vec<(MsgRow, u32)> = Vec::new();
        for t in tables {
            let Some(shard) = shard_of(t) else {
                continue;
            };
            // a row with the position's own key is past it only in a database listed after the position's
            let cmp = match (older, shard > pos.shard, shard < pos.shard) {
                (true, true, _) => "<=",
                (true, false, _) => "<",
                (false, _, true) => ">=",
                (false, _, false) => ">",
            };
            let bound = if older { "<=" } else { ">=" };
            // rows with a sort_seq, through its index; the few without one are keyed by create_time * 1000
            let indexed = format!(
                "where m.sort_seq {bound} ?1 and m.sort_seq <> 0 and (m.sort_seq, m.create_time, m.local_id) {cmp} (?1, ?2, ?3) \
                 order by m.sort_seq {dir}, m.create_time {dir}, m.local_id {dir} limit ?4"
            );
            let unsequenced = format!(
                "where (m.sort_seq is null or m.sort_seq = 0) \
                 and (m.create_time * 1000, m.create_time, m.local_id) {cmp} (?1, ?2, ?3) \
                 order by m.create_time {dir}, m.local_id {dir} limit ?4"
            );
            let page = self.read_rows(
                t,
                &Self::select_both_sql(t, &indexed, &unsequenced),
                &[&pos.sort_seq, &pos.create_time, &pos.local_id, &want],
            )?;
            rows.extend(page.into_iter().map(|r| (r, shard)));
        }
        // the listing's order: newest key first, equal keys by ascending database
        rows.sort_by(|(a, sa), (b, sb)| b.key().cmp(&a.key()).then(sa.cmp(sb)));
        if !older {
            // the newer rows closest to the position are at the end
            // a negative `want` keeps every row
            let skip = usize::try_from(want).map_or(0, |want| rows.len().saturating_sub(want));
            rows.drain(..skip);
        } else if let Ok(want) = usize::try_from(want) {
            rows.truncate(want);
        }
        debug_assert!(rows.iter().all(|(r, s)| {
            let k = r.key();
            if older {
                k < pos.key() || (k == pos.key() && *s > pos.shard)
            } else {
                k > pos.key() || (k == pos.key() && *s < pos.shard)
            }
        }));
        Ok(rows.into_iter().map(|(r, _)| r).collect())
    }

    /// Every message table of the account by its name ([`table_name_for`] a conversation), oldest shard first: one
    /// read of each database, for a listing across conversations (issue #178).
    pub fn message_tables_by_name(&self) -> Result<HashMap<String, Vec<MsgTable>>> {
        let dbs = self.message_dbs();
        self.prefetch(&dbs);
        let mut out: HashMap<String, Vec<MsgTable>> = HashMap::new();
        for db in dbs {
            let names = self.with_db(&db, |c| {
                let mut stmt = c.prepare(
                    "select name from sqlite_master where type = 'table' and name like 'Msg\\_%' escape '\\'",
                )?;
                let names = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(names)
            })?;
            for table in names {
                out.entry(table.clone()).or_default().push(MsgTable {
                    db: db.clone(),
                    table,
                });
            }
        }
        Ok(out)
    }

    /// Messages of `tables` (one conversation's, from [`message_tables_by_name`](Self::message_tables_by_name)) of
    /// the `local_types`, newest first in the chat's order, at most `limit` (all when negative): those listed after
    /// `before` (older; with its [`Ties`]), all when `None`, sent between the Unix seconds of `range` (inclusive; 0
    /// for no bound) and at `floor` or later in the chat's order (its `sort_seq`; 0 for no bound). One statement a
    /// table, of one part a type, so the `(local_type, sort_seq)` index gives each part's rows in order and the
    /// limit stops its read (issue #178).
    pub fn typed_before(
        &self,
        tables: &[MsgTable],
        local_types: &[i64],
        before: Option<(MsgPos, Ties)>,
        range: (i64, i64),
        floor: i64,
        limit: i64,
    ) -> Result<Vec<MsgRow>> {
        let (pos, ties) = before.unwrap_or((
            MsgPos {
                sort_seq: i64::MAX,
                create_time: i64::MAX,
                local_id: i64::MAX,
                shard: 0,
            },
            Ties::None,
        ));
        let (begin, end) = range;
        let mut sent = String::new();
        if begin > 0 {
            sent.push_str(&format!(" and m.create_time >= {begin}"));
        }
        if end > 0 {
            sent.push_str(&format!(" and m.create_time <= {end}"));
        }
        let cmp = match ties {
            Ties::All => "<=",
            Ties::None => "<",
        };
        let mut rows: Vec<(MsgRow, u32)> = Vec::new();
        for t in tables {
            let Some(shard) = shard_of(t) else {
                continue;
            };
            if local_types.is_empty() {
                continue;
            }
            let table = &t.table;
            // a part a type, so each reads its index in order; the few rows without a sort_seq in one part of all
            // the types; the senders joined once, outside (each part takes its time to plan)
            let mut parts: Vec<String> = local_types
                .iter()
                .map(|lt| {
                    format!(
                        "select * from (select * from \"{table}\" m where m.local_type = {lt} \
                         and m.sort_seq <= ?1 and m.sort_seq >= ?5 and m.sort_seq <> 0 \
                         and (m.sort_seq, m.create_time, m.local_id) {cmp} (?1, ?2, ?3){sent} \
                         order by m.sort_seq desc, m.create_time desc, m.local_id desc limit ?4)"
                    )
                })
                .collect();
            let types = local_types
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!(
                "select * from (select * from \"{table}\" m where m.local_type in ({types}) \
                 and (m.sort_seq is null or m.sort_seq = 0) and m.create_time * 1000 >= ?5 \
                 and (m.create_time * 1000, m.create_time, m.local_id) {cmp} (?1, ?2, ?3){sent} \
                 order by m.create_time desc, m.local_id desc limit ?4)"
            ));
            let sql = format!(
                "select {} from ({}) m left join Name2Id n on n.rowid = m.real_sender_id",
                MsgRow::columns(false),
                parts.join(" union all ")
            );
            let page = self.read_rows(
                t,
                &sql,
                &[
                    &pos.sort_seq,
                    &pos.create_time,
                    &pos.local_id,
                    &limit,
                    &floor,
                ],
            )?;
            rows.extend(page.into_iter().map(|r| (r, shard)));
        }
        rows.sort_by(|(a, sa), (b, sb)| b.key().cmp(&a.key()).then(sa.cmp(sb)));
        if let Ok(limit) = usize::try_from(limit) {
            rows.truncate(limit);
        }
        Ok(rows.into_iter().map(|(r, _)| r).collect())
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
        Ok(i32::try_from(total).unwrap_or(i32::MAX))
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
            // the local date of each quarter of an hour, not of each message: SQLite's `localtime` is the slow part,
            // and every time zone today is a whole number of quarters away from UTC, moving on a quarter
            let sql = format!(
                "select date(q * {QUARTER}, 'unixepoch', 'localtime') as d, sum(n) as n from \
                 (select create_time / {QUARTER} as q, count(*) as n from \"{}\" where create_time > 0 group by q) \
                 group by d",
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
    ) -> Result<Vec<MsgRow>> {
        let (offset, limit) = (offset.max(0) as usize, limit.max(0) as usize);
        let want = if limit == 0 {
            -1
        } else {
            (offset + limit) as i64
        };
        let dir = if ascending { "asc" } else { "desc" };
        let mut rows: Vec<MsgRow> = Vec::new();
        for t in self.message_tables(session_id)? {
            let sql = Self::select_sql(
                &t,
                &format!("where m.local_type = ?1 order by m.sort_seq {dir}, m.create_time {dir}, m.local_id {dir} limit ?2"),
                false,
            );
            rows.extend(self.read_rows(&t, &sql, &[&local_type, &want])?);
        }
        if ascending {
            rows.sort_by_key(MsgRow::key);
        } else {
            rows.sort_by_key(|r| std::cmp::Reverse(r.key()));
        }
        let it = rows.into_iter().skip(offset);
        Ok(if limit == 0 {
            it.collect()
        } else {
            it.take(limit).collect()
        })
    }

    /// Every message of a session with this `local_id`, newest shard first: the id is only unique within one
    /// `message_N.db`, so different messages in different shards can share it. `create_time` (seconds) keeps only
    /// the one sent then.
    pub fn messages_by_local_id(
        &self,
        session_id: &str,
        local_id: i64,
        create_time: Option<i64>,
    ) -> Result<Vec<MsgRow>> {
        let mut out = Vec::new();
        for t in self.message_tables(session_id)?.into_iter().rev() {
            out.extend(match create_time {
                Some(ct) => self.read_rows(
                    &t,
                    &Self::select_sql(
                        &t,
                        "where m.local_id = ?1 and m.create_time = ?2 limit 1",
                        false,
                    ),
                    &[&local_id, &ct],
                )?,
                None => self.read_rows(
                    &t,
                    &Self::select_sql(&t, "where m.local_id = ?1 limit 1", false),
                    &[&local_id],
                )?,
            });
        }
        Ok(out)
    }

    /// One message by server id (a decimal string, may exceed 2^53).
    pub fn message_by_server_id(
        &self,
        session_id: &str,
        server_id: &str,
    ) -> Result<Option<MsgRow>> {
        let id: i64 = server_id
            .trim()
            .parse()
            .map_err(|_| anyhow!("invalid server id: {server_id}"))?;
        self.find_one(session_id, "m.server_id = ?1", &id)
    }

    fn find_one(&self, session_id: &str, cond: &str, arg: &i64) -> Result<Option<MsgRow>> {
        for t in self.message_tables(session_id)?.into_iter().rev() {
            let sql = Self::select_sql(&t, &format!("where {cond} limit 1"), false);
            if let Some(r) = self.read_rows(&t, &sql, &[arg])?.into_iter().next() {
                return Ok(Some(r));
            }
        }
        Ok(None)
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
        self.open_cursor(
            session_id,
            batch,
            ascending,
            (begin, end),
            lite,
            CursorKeys::All,
        )
    }

    /// The cursor of an export: oldest first, full rows, like [`open_message_cursor`](Self::open_message_cursor).
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
            false,
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
            lite,
            CursorKeys::Senders(senders),
        )
    }

    fn open_cursor(
        &self,
        session_id: &str,
        batch: i32,
        ascending: bool,
        (begin, end): (i32, i32),
        lite: bool,
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
                keys.extend(self.index_keys(t, idx, (begin, end))?);
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
                    format!(" and ({known}real_sender_id is null or real_sender_id not in (select rowid from Name2Id))")
                }
            };
            keys.extend(self.table_keys(t, idx, (begin, end), &who)?);
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

    /// The keys of `t` in `create_time` range `(begin, end)` from the `sort_seq` index alone (`local_id` is the rowid,
    /// so the index covers the query): `create_time` is taken as `sort_seq / 1000` and the range is turned into a
    /// `sort_seq` range. Rows whose `sort_seq` says nothing about the time (0, negative or none) are read from the
    /// table, as [`typed_row_key`] does.
    fn index_keys(&self, t: &MsgTable, idx: usize, (begin, end): (i64, i64)) -> Result<Vec<Key>> {
        let lo = begin.saturating_mul(1000);
        let hi = if end == 0 {
            0
        } else {
            end.saturating_mul(1000).saturating_add(999)
        };
        let sql = format!(
            "select sort_seq, local_id from \"{}\" where sort_seq > 0 and sort_seq >= ?1 and (?2 = 0 or sort_seq <= ?2)",
            t.table
        );
        let mut keys = self.collect_keys(t, idx, &sql, &[&lo, &hi], true)?;
        let rest = format!(
            "select sort_seq, create_time, local_id from \"{}\" where (sort_seq is null or sort_seq <= 0) and create_time >= ?1 and (?2 = 0 or create_time <= ?2)",
            t.table
        );
        keys.extend(self.collect_keys(t, idx, &rest, &[&begin, &end], false)?);
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
        keys.sort();
        if !ascending {
            keys.reverse();
        }
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
        let (tables, chunk, lite, more, derived) = {
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
                c.lite,
                c.pos < c.keys.len(),
                c.derived.clone(),
            )
        };
        let (rows, mismatched) = self.rows_for_keys(&tables, &chunk, lite, &derived)?;
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
        let (tables, chunk, lite, derived) = {
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
                c.lite,
                c.derived.clone(),
            )
        };
        let (rows, mismatched) = self.rows_for_keys(&tables, &chunk, lite, &derived)?;
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
        lite: bool,
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
            let sql = Self::select_sql(t, &format!("where m.local_id in ({list})"), lite);
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

    fn texts(rows: &[MsgRow]) -> Vec<String> {
        rows.iter().map(|r| r.content.clone()).collect()
    }

    /// An account with `shards` of `wxid_bob`'s messages, each in its own directory.
    fn export_world(tag: &str, shards: &[Vec<crate::fixture::MsgSpec>]) -> NativeAccount {
        use crate::fixture::{Fixture, SessionSpec, T0};
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("weflow-keys-{}-{tag}-{n}", std::process::id()));
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
        NativeAccount::new(f.db_storage(), &f.key_hex()).unwrap()
    }

    fn cursor_state(acct: &NativeAccount, id: i64) -> (Vec<Key>, Vec<bool>) {
        let cursors = acct.cursors.lock().unwrap();
        let c = &cursors.open[&id];
        (c.keys.clone(), c.derived.clone())
    }

    fn page_ids(acct: &NativeAccount, id: i64) -> Result<Vec<i64>> {
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
        let root = std::env::temp_dir().join(format!("weflow-key-types-{}", std::process::id()));
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
        let acct = NativeAccount::new(fixture.db_storage(), &fixture.key_hex()).unwrap();
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
        let acct = export_world(
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
        let acct = export_world(
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
        let acct = export_world(
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
        let acct = export_world(
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
        let acct = export_world(
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
        let root = std::env::temp_dir().join(format!("weflow-keys-{}-newer", std::process::id()));
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
        let acct = NativeAccount::new(f.db_storage(), &f.key_hex()).unwrap();
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
        assert!(acct.messages("wxid_bob", 10, 5).unwrap().is_empty());
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
                "create table \"{table}\"(local_id integer primary key autoincrement, server_id integer, local_type integer, \
                 sort_seq integer, real_sender_id integer, create_time integer, status integer, compress_content, \
                 message_content, source text, packed_info_data blob)"
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
        // every row sorted by its key, then the page taken: rows without a sort_seq (null or 0) are keyed by
        // create_time * 1000, so here they are the newest ones
        let t = acct.message_tables("wxid_bob").unwrap().remove(0);
        let reference = |limit: i32, offset: i32| -> Vec<String> {
            let sql = NativeAccount::select_sql(&t, "", false);
            let mut rows = acct.read_rows(&t, &sql, &[]).unwrap();
            rows.sort_by_key(|r| std::cmp::Reverse(r.key()));
            let page: Vec<MsgRow> = rows
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .collect();
            texts(&page)
        };
        assert_eq!(acct.messages("wxid_bob", 0, 0).unwrap().len(), 60);
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

        // pages from a cursor, both ways, list the same order through the ties and the rows without a sort_seq
        let all = acct.messages("wxid_bob", 0, 0).unwrap();
        let pos = |row: &MsgRow| {
            let create_time = row.create_time.unwrap();
            MsgPos {
                sort_seq: row
                    .sort_seq
                    .filter(|s| *s != 0)
                    .unwrap_or(create_time * 1000),
                create_time,
                local_id: row.local_id,
                shard: row.shard,
            }
        };
        assert!(all.iter().all(|r| r.pos() == pos(r)));
        let mut back = texts(&acct.messages("wxid_bob", 7, 0).unwrap());
        while back.len() < 60 {
            let last = &all[back.len() - 1];
            let page = acct
                .messages_from("wxid_bob", 7, PageFrom::Before(pos(last)))
                .unwrap();
            assert!(!page.is_empty(), "stuck at {}", back.len());
            back.extend(texts(&page));
        }
        assert_eq!(back, reference(60, 0));
        assert!(acct
            .messages_from("wxid_bob", 7, PageFrom::Before(pos(&all[59])))
            .unwrap()
            .is_empty());
        for at in [1, 9, 30, 59] {
            // the 7 rows listed just before row `at`
            let page = acct
                .messages_from("wxid_bob", 7, PageFrom::After(pos(&all[at])))
                .unwrap();
            let from = at.saturating_sub(7);
            assert_eq!(
                texts(&page),
                reference((at - from) as i32, from as i32),
                "after row {at}"
            );
        }
        assert_eq!(MsgPos::parse(&pos(&all[3]).token()), Some(pos(&all[3])));
        assert_eq!(MsgPos::parse("1.2.3"), None);
        assert_eq!(MsgPos::parse("1.2.3.4.5"), None);
    }

    /// An account whose `wxid_bob` messages are `shards`, one `message_<n>.db` each: rows of (local_id, sort_seq as
    /// an SQL literal, create_time as one). Neither column has a type, so a row can store any SQLite value.
    fn keyed_account(tag: &str, shards: &[Vec<(i64, String, String)>]) -> NativeAccount {
        let dir = std::env::temp_dir().join(format!("weflow-msg-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("message")).unwrap();
        let table = table_name_for("wxid_bob");
        let cipher = PageCipher::derive(&KEY, &SALT);
        for (n, rows) in shards.iter().enumerate() {
            let values = rows
                .iter()
                .map(|(id, seq, create)| format!("({id}, {seq}, {create}, 1, 1, '{n}:{id}')"))
                .collect::<Vec<_>>()
                .join(", ");
            let table = table.clone();
            let plain = plain_db_with(move |c: &Connection| {
                c.execute_batch(&format!(
                    "create table Name2Id(user_name text primary key, is_session integer);
                     insert into Name2Id(rowid, user_name) values (1, 'wxid_bob');
                     create table \"{table}\"(local_id integer primary key, server_id integer, sort_seq,
                         create_time, status integer, real_sender_id integer, local_type integer, compress_content,
                         message_content text, source text, packed_info_data blob);
                     create index \"{table}_SORTSEQ\" on \"{table}\"(sort_seq);
                     insert into \"{table}\"(local_id, sort_seq, create_time, real_sender_id, local_type,
                         message_content) values {values};"
                ))
                .unwrap();
            });
            std::fs::write(
                dir.join(format!("message/message_{n}.db")),
                encrypt_db(&plain, &cipher),
            )
            .unwrap();
        }
        NativeAccount::new(&dir, &hex(&KEY)).unwrap()
    }

    #[test]
    fn offset_pages_are_the_pages_of_reading_every_row() {
        // sort_seq ties within a table, rows without one (null or 0), negative ones, and rows of the second database
        // with the very keys of the first one's
        let row = |i: i64, salt: i64| -> (i64, String, String) {
            let seq = match i {
                _ if i % 9 == 0 => "null".to_string(),
                _ if i % 11 == 0 => "0".to_string(),
                _ if i % 7 == 0 => "-5".to_string(),
                _ => (T0 * 1000 + (i * salt) % 8 * 1000).to_string(),
            };
            (i, seq, (T0 + (i * 5) % 13).to_string())
        };
        let first: Vec<_> = (1..=40).map(|i| row(i, 1)).collect();
        let second: Vec<_> = (1..=30)
            .map(|i| if i <= 10 { row(i, 1) } else { row(i, 3) })
            .collect();
        let acct = keyed_account("offset", &[first, second]);
        let tables = acct.message_tables("wxid_bob").unwrap();
        assert_eq!(tables.len(), 2);
        for limit in [0, 1, 3, 7, 50] {
            for offset in 0..=72 {
                if offset > 0 {
                    assert!(
                        !matches!(
                            acct.offset_position(&tables, offset, limit).unwrap(),
                            OffsetPosition::Unusual
                        ),
                        "limit {limit} offset {offset}"
                    );
                }
                assert_eq!(
                    acct.messages("wxid_bob", limit as i32, offset as i32)
                        .unwrap(),
                    acct.newest_rows(&tables, offset, limit).unwrap(),
                    "limit {limit} offset {offset}"
                );
            }
        }
    }

    #[test]
    fn offset_pages_over_keys_of_other_types_read_every_row() {
        for (tag, seq, create) in [
            ("numeric-text", "'1.5'", "1"),
            ("real", "1.5", "1"),
            ("text", "'x'", "1"),
            ("blob", "x'31'", "1"),
            ("null-time", "1000", "null"),
        ] {
            let mut rows: Vec<(i64, String, String)> = (1..=6)
                .map(|i| (i, (i * 1000).to_string(), i.to_string()))
                .collect();
            rows.push((7, seq.to_string(), create.to_string()));
            let acct = keyed_account(&format!("unusual-{tag}"), &[rows]);
            let tables = acct.message_tables("wxid_bob").unwrap();
            assert!(
                matches!(
                    acct.offset_position(&tables, 3, 0).unwrap(),
                    OffsetPosition::Unusual
                ),
                "{tag}"
            );
            for offset in 1..=8 {
                assert_eq!(
                    acct.messages("wxid_bob", 2, offset as i32).unwrap(),
                    acct.newest_rows(&tables, offset, 2).unwrap(),
                    "{tag} offset {offset}"
                );
            }
        }
    }

    /// Each column typed, the content inflated, the sender resolved and whether the owner sent it, the database by its
    /// shard; lite rows of a cursor leave out the source and the packed info.
    #[test]
    fn rows_carry_typed_columns_sender_and_inflated_content() {
        let (acct, _dir) = account("rows");
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
                    source: None,
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
            .into_iter()
            .map(|r| MsgRow {
                packed_info: None,
                ..r
            })
            .collect();
        assert_eq!(lite, unpacked);
    }

    #[test]
    fn messages_where_keeps_what_the_condition_does_from_a_time_on() {
        let (acct, _dir) = account("where");
        let all: Vec<MsgRow> = acct.messages("wxid_bob", 0, 0).unwrap();
        let since = all.iter().filter_map(|m| m.create_time).min().unwrap() + 1;
        let kept: Vec<MsgRow> = acct
            .messages_where("wxid_bob", since, "m.local_type = 1", 1000)
            .unwrap();
        let mut want: Vec<i64> = all
            .iter()
            .filter(|m| m.local_type == 1 && m.create_time.unwrap_or(0) >= since)
            .map(|m| m.local_id)
            .collect();
        let mut got: Vec<i64> = kept.iter().map(|m| m.local_id).collect();
        assert!(!got.is_empty());
        want.sort_unstable();
        got.sort_unstable();
        assert_eq!(got, want);
        // at most `limit` of each table, the oldest
        let one: Vec<MsgRow> = acct.messages_where("wxid_bob", 0, "1", 1).unwrap();
        let tables = acct.message_tables("wxid_bob").unwrap().len();
        assert!(!one.is_empty() && one.len() <= tables);
    }

    /// A cell of an integer column that holds another type is read as absent, and counted.
    #[test]
    fn odd_integer_cells_are_absent_and_counted() {
        use crate::fixture::T0;
        use crate::msg_row::odd_cell_count;
        let root = std::env::temp_dir().join(format!("weflow-odd-cells-{}", std::process::id()));
        let fixture = crate::fixture::Fixture::new(&root, "wxid_synthetic_ab12");
        let table = table_name_for("wxid_bob");
        let plain = plain_db_with(|conn| {
            conn.execute_batch(&format!(
                "create table Name2Id(user_name text);
                 insert into Name2Id values ('wxid_bob');
                 create table \"{table}\"(local_id integer primary key, server_id, local_type integer, sort_seq,
                     create_time integer, status integer, real_sender_id integer, compress_content, message_content,
                     source, packed_info_data);
                 insert into \"{table}\"(local_id, server_id, local_type, sort_seq, create_time, real_sender_id,
                     message_content) values (1, 'not a number', 1, 1.5, {T0}, 1, 'odd');"
            ))
            .unwrap();
        });
        let cipher = PageCipher::derive(&KEY, &SALT);
        std::fs::write(
            fixture.db_storage().join("message/message_0.db"),
            encrypt_db(&plain, &cipher),
        )
        .unwrap();
        let acct = NativeAccount::new(fixture.db_storage(), &fixture.key_hex()).unwrap();
        let before = odd_cell_count();
        let rows: Vec<MsgRow> = acct.messages("wxid_bob", 0, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].server_id, rows[0].sort_seq), (None, None));
        assert_eq!(rows[0].key(), (T0 * 1000, T0, 1));
        assert!(odd_cell_count() >= before + 2);
        let _ = std::fs::remove_dir_all(&root);
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
    fn days_are_counted_as_the_local_date_of_each_message() {
        let dir = std::env::temp_dir().join(format!("weflow-msg-{}-days", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("message")).unwrap();
        // a message every 6 min 13 s for ten days, so midnights fall between the quarters' own messages
        let texts: Vec<String> = (0..2400).map(|k| format!("m{k}")).collect();
        let msgs: Vec<(i64, i64, i64, i64, &str)> = texts
            .iter()
            .enumerate()
            .map(|(k, t)| (k as i64 + 1, k as i64 * 373 - 7, 1, 1, t.as_str()))
            .collect();
        let cipher = PageCipher::derive(&KEY, &SALT);
        std::fs::write(
            dir.join("message/message_0.db"),
            encrypt_db(&shard(&msgs, None), &cipher),
        )
        .unwrap();
        let acct = NativeAccount::new(&dir, &hex(&KEY)).unwrap();
        let sqlite = Connection::open_in_memory().unwrap();
        let mut want = serde_json::Map::new();
        for (_, offset, ..) in &msgs {
            let day: String = sqlite
                .query_row(
                    "select date(?1, 'unixepoch', 'localtime')",
                    [T0 + offset],
                    |r| r.get(0),
                )
                .unwrap();
            let n = want.entry(day).or_insert(json!(0));
            *n = json!(n.as_i64().unwrap() + 1);
        }
        assert!(want.len() >= 10);
        assert_eq!(
            acct.session_message_date_counts("wxid_bob").unwrap(),
            Value::Object(want)
        );
        let _ = std::fs::remove_dir_all(&dir);
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
        assert!(acct
            .messages_by_type("wxid_bob", 999, false, 0, 0)
            .unwrap()
            .is_empty());
        // local_id 1 exists in both shards: the newest shard first
        assert_eq!(
            texts(&acct.messages_by_local_id("wxid_bob", 1, None).unwrap()),
            ["fourth", "hello"]
        );
        assert_eq!(
            texts(&acct.messages_by_local_id("wxid_bob", 1, Some(T0)).unwrap()),
            ["hello"]
        );
        assert_eq!(
            texts(&acct.messages_by_local_id("wxid_bob", 3, None).unwrap()),
            ["third"]
        );
        assert!(acct
            .messages_by_local_id("wxid_bob", 77, None)
            .unwrap()
            .is_empty());
        assert_eq!(
            acct.message_by_server_id("wxid_bob", &(DAY * 4).to_string())
                .unwrap()
                .map(|r| r.content),
            Some("voice".to_string())
        );
        assert_eq!(acct.message_by_server_id("wxid_bob", "77").unwrap(), None);
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
    fn pages_by_number_match_the_batches_and_leave_the_cursor_alone() {
        let (acct, _dir) = account("cursor-pages");
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
        let (acct, dir) = account("cursor-parallel");
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
        let (acct, _dir) = account("range");
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
        let (acct, _dir) = account("senders");
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
