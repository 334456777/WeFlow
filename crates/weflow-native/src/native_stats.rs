//! Statistics over the message tables. The ones over every message of a table read the few cells they need through
//! [`crate::leaf_scan`] and count them here; a table the scan does not read is counted with grouped SQL.
//!
//! Dates, hours and weekdays are in local time (SQLite `localtime`), matching what WeFlow's desktop app shows.
//! Output shapes are the ones the service layer already consumes (`total/sent/received/...`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::native_db::NativeAccount;
use crate::native_msg::MsgTable;

const VOICE: i64 = 34;
const IMAGE: i64 = 3;
const VIDEO: i64 = 43;
const EMOJI: i64 = 47;
const CALL: i64 = 50;
/// `(2000 << 32) | 49` and `(2001 << 32) | 49`: transfer and red-packet app messages.
const TRANSFER: i64 = 8_589_934_592_049;
const RED_PACKET: i64 = 8_594_229_559_345;

fn int(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// An integer column as [`int`] reads one from a JSON row: anything but an integer is 0.
fn sql_int(v: rusqlite::types::ValueRef<'_>) -> i64 {
    match v {
        rusqlite::types::ValueRef::Integer(i) => i,
        _ => 0,
    }
}

/// The local `%H`, `%w` and `%Y-%m-%d` of the start of each quarter hour (`create_time / 900`), as SQLite's
/// `localtime` gives them, in one query.
fn local_quarters(
    conn: &rusqlite::Connection,
    quarters: &BTreeSet<i64>,
) -> Result<std::collections::HashMap<i64, (i64, i64, String)>> {
    let list = serde_json::to_string(&quarters.iter().collect::<Vec<_>>())?;
    let local_q = |expr: &str| local(expr).replace("create_time", "value * 900");
    let sql = format!(
        "select value, {h}, {w}, {d} from json_each(?1)",
        h = local_q("%H"),
        w = local_q("%w"),
        d = local_q("%Y-%m-%d"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([list])?;
    let mut out = std::collections::HashMap::with_capacity(quarters.len());
    while let Some(r) = rows.next()? {
        let text = |i: usize| -> Result<String> {
            Ok(r.get_ref(i)?.as_str_or_null()?.unwrap_or("").to_string())
        };
        out.insert(
            sql_int(r.get_ref(0)?),
            (
                text(1)?.parse().unwrap_or(0),
                text(2)?.parse().unwrap_or(0),
                text(3)?,
            ),
        );
    }
    Ok(out)
}

/// Sender name, type, local hour and count of a group of messages (see [`NativeAccount::sender_groups`]).
type SenderGroup = (Option<String>, i64, i64, i64);

/// Type, quarter hour (`create_time / 900`) and whether the owner sent it, of a message.
type QuarterKey = (i64, i64, bool);

/// The cells of a message the statistics read.
#[derive(Clone, Copy)]
struct Msg {
    /// `coalesce(nullif(local_type, 0), 1)`.
    kind: i64,
    time: i64,
    /// `real_sender_id`.
    sender: Option<i64>,
}

// one per message of a table, passed to the accumulators by value: its size changes only on purpose (#505)
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Msg>() == 32);

/// Most tables a statistic reads at once.
const TABLE_THREADS: usize = 8;

/// `f` of each of `items`, run on up to [`TABLE_THREADS`] threads (each table's leaf scan spends most of its time on
/// HMACs, and most conversations are too small to share theirs out), the results in the order of `items`; the
/// first error in that order.
pub(crate) fn each_table<I: Sync, T: Send>(
    items: &[I],
    f: impl Fn(&I) -> Result<T> + Sync,
) -> Result<Vec<T>> {
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    let threads = items.len().min(TABLE_THREADS).min(cpus);
    if threads <= 1 {
        return items.iter().map(&f).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (f, next) = (&f, &next);
    let mut done: Vec<(usize, Result<T>)> = std::thread::scope(|s| {
        let running: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(move || {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        out.push((i, f(item)));
                    }
                    out
                })
            })
            .collect();
        running
            .into_iter()
            .flat_map(|h| {
                h.join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

/// The message tables of each of `session_ids`, and every one of them in that order.
pub(crate) fn tables_of(
    acct: &NativeAccount,
    session_ids: &[String],
) -> Result<Vec<Vec<MsgTable>>> {
    session_ids
        .iter()
        .map(|sid| acct.message_tables(sid))
        .collect()
}

/// Sender row ids in the list [`NativeAccount::my_ids`] gives.
pub(crate) fn id_set(list: &str) -> std::collections::HashSet<i64> {
    list.split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect()
}

/// How many messages, and the earliest and latest `create_time` of them.
struct Tally {
    count: i64,
    first: i64,
    last: i64,
}

/// Messages of one table with the same type, local hour, weekday and day and the same sender side.
struct TimeGroup {
    kind: i64,
    /// `%H`, `%w` and `%Y-%m-%d` in local time.
    hour: i64,
    weekday: i64,
    day: String,
    /// Sent by the account owner.
    mine: bool,
    count: i64,
    /// Earliest and latest `create_time`.
    first: i64,
    last: i64,
}

/// `and create_time >= b and create_time <= e` for the positive bounds (`0` = unbounded).
fn range_sql(begin: i64, end: i64) -> String {
    let mut s = String::new();
    if begin > 0 {
        s.push_str(&format!(" and create_time >= {begin}"));
    }
    if end > 0 {
        s.push_str(&format!(" and create_time <= {end}"));
    }
    s
}

fn local(expr: &str) -> String {
    format!("strftime('{expr}', create_time, 'unixepoch', 'localtime')")
}

impl NativeAccount {
    /// Every message of `t` with a positive time inside `begin..=end` (0: unbounded), read through the leaf scan:
    /// each thread of the scan counts its messages into an accumulator of its own (`start`, `each`), and `finish`
    /// gets them all with a connection to the database. `None` when the scan does not read the table.
    fn scan_messages<A: Send, T>(
        &self,
        t: &MsgTable,
        (begin, end): (i64, i64),
        start: impl Fn() -> A + Sync,
        each: impl Fn(&mut A, Msg) + Sync,
        finish: impl Fn(&rusqlite::Connection, &[A]) -> Result<T>,
    ) -> Result<Option<T>> {
        let parts = self.read_db(
            &t.db,
            |conn| {
                crate::leaf_scan::target(
                    conn,
                    &t.table,
                    &["local_type", "create_time", "real_sender_id"],
                )
            },
            |file, target| {
                let Some((root, columns)) = target else {
                    return Ok(None);
                };
                crate::leaf_scan::int_columns(file, root, &columns, &start, |acc, cells| {
                    let Some(time) = cells[1] else { return };
                    if time <= 0 || (begin > 0 && time < begin) || (end > 0 && time > end) {
                        return;
                    }
                    let kind = match cells[0] {
                        None | Some(0) => 1,
                        Some(kind) => kind,
                    };
                    each(
                        acc,
                        Msg {
                            kind,
                            time,
                            sender: cells[2],
                        },
                    );
                })
            },
        )?;
        match parts {
            Some(parts) => self.with_db(&t.db, |conn| finish(conn, &parts)).map(Some),
            None => Ok(None),
        }
    }

    /// Ids of the account owner in a shard's `Name2Id` (as a SQL list literal body).
    pub(crate) fn my_ids(&self, db: &Path) -> Result<String> {
        let rows = self.query(db, "select rowid as id, user_name from Name2Id", &[])?;
        let ids: Vec<String> = rows
            .iter()
            .filter(|r| self.is_me(r["user_name"].as_str().unwrap_or("")))
            .map(|r| int(r, "id").to_string())
            .collect();
        Ok(ids.join(","))
    }

    /// `{ total, sent, received, firstTime, lastTime, typeCounts, hourly, weekday, daily, monthly, sessions, idMap }`.
    pub fn aggregate_stats(&self, session_ids: &[String], begin: i64, end: i64) -> Result<Value> {
        self.aggregate_inner(session_ids, begin, end, false)
    }

    /// Only the `sessions` and `total` of [`aggregate_stats`](Self::aggregate_stats), the same numbers: one count per
    /// table, without the grouping by type and time the rest needs. `{ total, sessions, idMap }`.
    pub fn session_totals(&self, session_ids: &[String], begin: i64, end: i64) -> Result<Value> {
        let range = range_sql(begin, end);
        let mut total = 0i64;
        let mut sessions = Map::new();
        let tables = tables_of(self, session_ids)?;
        let mut mine: BTreeMap<PathBuf, String> = BTreeMap::new();
        for t in tables.iter().flatten() {
            if !mine.contains_key(&t.db) {
                mine.insert(t.db.clone(), self.my_ids(&t.db)?);
            }
        }
        let every: Vec<&MsgTable> = tables.iter().flatten().collect();
        let counted = each_table(&every, |t| -> Result<(i64, i64, i64)> {
            let t = *t;
            let ids = id_set(&mine[&t.db]);
            let scanned = self.scan_messages(
                t,
                (begin, end),
                || (0i64, 0i64, 0i64),
                |(n, s, mx), m| {
                    *n += 1;
                    *s += i64::from(m.sender.is_some_and(|id| ids.contains(&id)));
                    *mx = (*mx).max(m.time);
                },
                |_, parts| {
                    Ok(parts
                        .iter()
                        .fold((0, 0, 0), |(n, s, mx), p| (n + p.0, s + p.1, mx.max(p.2))))
                },
            )?;
            let (n, s, mx) = match scanned {
                Some(counts) => counts,
                None => {
                    let sql = format!(
                            "select count(*) as n, sum(real_sender_id in ({mine})) as s, max(create_time) as mx \
                             from \"{table}\" where create_time > 0{range}",
                            mine = mine[&t.db],
                            table = t.table,
                        );
                    self.query(&t.db, &sql, &[])?
                        .iter()
                        .fold((0, 0, 0), |(n, s, mx), r| {
                            (n + int(r, "n"), s + int(r, "s"), mx.max(int(r, "mx")))
                        })
                }
            };
            Ok((n, s, mx))
        })?;
        let mut counted = counted.into_iter();
        for (sid, ts) in session_ids.iter().zip(&tables) {
            let (mut s_total, mut s_sent, mut s_last) = (0i64, 0i64, 0i64);
            for (n, s, mx) in counted.by_ref().take(ts.len()) {
                s_total += n;
                s_sent += s;
                s_last = s_last.max(mx);
            }
            total += s_total;
            if s_total > 0 {
                sessions.insert(
                    sid.clone(),
                    json!({ "total": s_total, "sent": s_sent, "received": s_total - s_sent, "lastTime": s_last }),
                );
            }
        }
        Ok(json!({ "total": total, "sessions": sessions, "idMap": {} }))
    }

    /// [`aggregate_stats`](Self::aggregate_stats) plus, for every session, `monthly`: messages per month of the
    /// year (`"1"`..`"12"`, summed over the years in range) — what the annual report ranks friends by.
    pub fn annual_report_stats(
        &self,
        session_ids: &[String],
        begin: i64,
        end: i64,
    ) -> Result<Value> {
        self.aggregate_inner(session_ids, begin, end, true)
    }

    fn aggregate_inner(
        &self,
        session_ids: &[String],
        begin: i64,
        end: i64,
        per_session_months: bool,
    ) -> Result<Value> {
        let (mut total, mut sent, mut first, mut last) = (0i64, 0i64, 0i64, 0i64);
        let mut types: BTreeMap<i64, i64> = BTreeMap::new();
        let mut hourly: BTreeMap<i64, i64> = BTreeMap::new();
        let mut weekday: BTreeMap<i64, i64> = BTreeMap::new();
        let mut daily: BTreeMap<String, i64> = BTreeMap::new();
        let mut monthly: BTreeMap<String, i64> = BTreeMap::new();
        let mut sessions = Map::new();
        let tables = tables_of(self, session_ids)?;
        // the owner's sender ids, read once a database rather than once a table
        let mut mine_of: BTreeMap<PathBuf, String> = BTreeMap::new();
        for t in tables.iter().flatten() {
            if !mine_of.contains_key(&t.db) {
                mine_of.insert(t.db.clone(), self.my_ids(&t.db)?);
            }
        }
        let every: Vec<&MsgTable> = tables.iter().flatten().collect();
        let grouped = each_table(&every, |t| {
            self.time_groups(t, &mine_of[&t.db], (begin, end))
        })?;
        let mut grouped = grouped.into_iter();
        for (sid, ts) in session_ids.iter().zip(&tables) {
            let (mut s_total, mut s_sent, mut s_last) = (0i64, 0i64, 0i64);
            let mut s_months: BTreeMap<i64, i64> = BTreeMap::new();
            for groups in grouped.by_ref().take(ts.len()) {
                for g in groups {
                    let n = g.count;
                    total += n;
                    s_total += n;
                    if g.mine {
                        sent += n;
                        s_sent += n;
                    }
                    if first == 0 || g.first < first {
                        first = g.first;
                    }
                    last = last.max(g.last);
                    s_last = s_last.max(g.last);
                    *types.entry(g.kind).or_default() += n;
                    *hourly.entry(g.hour).or_default() += n;
                    *weekday.entry(g.weekday).or_default() += n;
                    if g.day.len() >= 7 {
                        *monthly.entry(g.day[..7].to_string()).or_default() += n;
                        if let Ok(month) = g.day[5..7].parse::<i64>() {
                            *s_months.entry(month).or_default() += n;
                        }
                    }
                    *daily.entry(g.day).or_default() += n;
                }
            }
            if s_total > 0 {
                let mut entry = json!({ "total": s_total, "sent": s_sent, "received": s_total - s_sent, "lastTime": s_last });
                if per_session_months {
                    entry["monthly"] = Value::Object(
                        s_months
                            .iter()
                            .map(|(k, v)| (k.to_string(), json!(v)))
                            .collect(),
                    );
                }
                sessions.insert(sid.clone(), entry);
            }
        }
        let keyed = |m: &BTreeMap<i64, i64>| {
            Value::Object(m.iter().map(|(k, v)| (k.to_string(), json!(v))).collect())
        };
        let named = |m: &BTreeMap<String, i64>| {
            Value::Object(m.iter().map(|(k, v)| (k.clone(), json!(v))).collect())
        };
        Ok(json!({
            "total": total, "sent": sent, "received": total - sent, "firstTime": first, "lastTime": last,
            "typeCounts": keyed(&types), "hourly": keyed(&hourly), "weekday": keyed(&weekday),
            "daily": named(&daily), "monthly": named(&monthly), "sessions": sessions, "idMap": {}
        }))
    }

    /// One table's messages grouped by type, local hour, weekday and day, and whether the owner sent them.
    ///
    /// The messages are read through the leaf scan and grouped here by type, quarter hour (`create_time / 900`) and
    /// side. The local hour, weekday and day of each quarter hour then come from SQLite's `localtime` in one query, so
    /// they are what a per-message `strftime` gives: every UTC offset in use is a whole number of quarter hours and
    /// changes on a quarter hour, so all of a quarter hour has one local hour, weekday and day. A table the scan does
    /// not read is grouped by SQL.
    fn time_groups(&self, t: &MsgTable, mine: &str, range: (i64, i64)) -> Result<Vec<TimeGroup>> {
        let ids = id_set(mine);
        let scanned = self.scan_messages(
            t,
            range,
            std::collections::HashMap::<QuarterKey, Tally>::new,
            |groups, m| {
                let side = m.sender.is_some_and(|id| ids.contains(&id));
                let g = groups.entry((m.kind, m.time / 900, side)).or_insert(Tally {
                    count: 0,
                    first: m.time,
                    last: m.time,
                });
                g.count += 1;
                g.first = g.first.min(m.time);
                g.last = g.last.max(m.time);
            },
            |conn, parts| {
                let mut groups: std::collections::HashMap<QuarterKey, Tally> =
                    std::collections::HashMap::new();
                for (&key, tally) in parts.iter().flatten() {
                    let g = groups.entry(key).or_insert(Tally {
                        count: 0,
                        first: tally.first,
                        last: tally.last,
                    });
                    g.count += tally.count;
                    g.first = g.first.min(tally.first);
                    g.last = g.last.max(tally.last);
                }
                let quarters: BTreeSet<i64> = groups.keys().map(|k| k.1).collect();
                let local = local_quarters(conn, &quarters)?;
                Ok(groups
                    .into_iter()
                    .map(|((kind, q, mine), Tally { count, first, last })| {
                        let (hour, weekday, day) = local.get(&q).cloned().unwrap_or_default();
                        TimeGroup {
                            kind,
                            hour,
                            weekday,
                            day,
                            mine,
                            count,
                            first,
                            last,
                        }
                    })
                    .collect())
            },
        )?;
        match scanned {
            Some(groups) => Ok(groups),
            None => self.time_groups_by_sql(t, mine, &range_sql(range.0, range.1)),
        }
    }

    /// [`time_groups`](Self::time_groups) for a table the leaf scan does not read: grouped by SQL, so that SQLite's
    /// comparisons and arithmetic apply to its cells as they always did.
    fn time_groups_by_sql(&self, t: &MsgTable, mine: &str, range: &str) -> Result<Vec<TimeGroup>> {
        let local_q = |expr: &str| local(expr).replace("create_time", "q");
        let sql = format!(
            "select t, {h} as h, {w} as w, {d} as d, s, n, mn, mx from ( \
             select coalesce(nullif(local_type, 0), 1) as t, create_time / 900 * 900 as q, \
             (real_sender_id in ({mine})) as s, count(*) as n, min(create_time) as mn, max(create_time) as mx \
             from \"{table}\" where create_time > 0{range} group by t, create_time / 900, s)",
            h = local_q("%H"),
            w = local_q("%w"),
            d = local_q("%Y-%m-%d"),
            table = t.table,
        );
        self.with_db(&t.db, |conn| {
            let mut stmt = conn
                .prepare(&sql)
                .with_context(|| format!("bad query: {sql}"))?;
            let mut rows = stmt.query([])?;
            let mut groups: Vec<TimeGroup> = Vec::new();
            while let Some(r) = rows.next()? {
                let text = |i: usize| -> Result<String> {
                    Ok(r.get_ref(i)?.as_str_or_null()?.unwrap_or("").to_string())
                };
                groups.push(TimeGroup {
                    kind: sql_int(r.get_ref(0)?),
                    hour: text(1)?.parse().unwrap_or(0),
                    weekday: text(2)?.parse().unwrap_or(0),
                    day: text(3)?,
                    mine: sql_int(r.get_ref(4)?) == 1,
                    count: sql_int(r.get_ref(5)?),
                    first: sql_int(r.get_ref(6)?),
                    last: sql_int(r.get_ref(7)?),
                });
            }
            Ok(groups)
        })
    }

    /// One row per shard table of the session: path, table, row count and first/last message time.
    pub fn message_table_stats(&self, session_id: &str) -> Result<Value> {
        let mut out = Vec::new();
        for t in self.message_tables(session_id)? {
            let sql = format!(
                "select count(*) as n, min(nullif(create_time, 0)) as mn, max(create_time) as mx from \"{}\"",
                t.table
            );
            let r = self
                .query(&t.db, &sql, &[])?
                .into_iter()
                .next()
                .unwrap_or(Value::Null);
            let (mn, mx) = (int(&r, "mn"), int(&r, "mx"));
            out.push(json!({
                "db_path": t.db.to_string_lossy(), "table_name": t.table, "count": int(&r, "n"),
                "first_timestamp": mn, "last_timestamp": mx, "first_time": mn, "last_time": mx
            }));
        }
        Ok(Value::Array(out))
    }

    /// `{ first_ts, last_ts }` of one message table (seconds).
    pub fn message_table_time_range(&self, db: &str, table: &str) -> Result<Value> {
        let path = self.resolve_db_path("message", Some(db))?;
        check_ident(table)?;
        let sql = format!(
            "select min(nullif(create_time, 0)) as mn, max(create_time) as mx from \"{table}\""
        );
        let r = self
            .query(&path, &sql, &[])?
            .into_iter()
            .next()
            .unwrap_or(Value::Null);
        Ok(json!({ "first_ts": int(&r, "mn"), "last_ts": int(&r, "mx") }))
    }

    /// Calendar years (local time, descending) in which any of the sessions has messages.
    pub fn available_years(&self, session_ids: &[String]) -> Result<Value> {
        let tables = tables_of(self, session_ids)?;
        let every: Vec<&MsgTable> = tables.iter().flatten().collect();
        let found = each_table(&every, |t| -> Result<Vec<i64>> {
            let t = *t;
            // the local year of each quarter of an hour that has a message (see `time_groups`)
            let scanned = self.scan_messages(
                t,
                (0, 0),
                BTreeSet::<i64>::new,
                |quarters, m| {
                    quarters.insert(m.time / 900);
                },
                |conn, parts| {
                    let quarters: BTreeSet<i64> = parts.iter().flatten().copied().collect();
                    Ok(local_quarters(conn, &quarters)?
                        .into_values()
                        .filter_map(|(_, _, day)| day.get(..4)?.parse::<i64>().ok())
                        .collect::<Vec<_>>())
                },
            )?;
            match scanned {
                Some(found) => Ok(found),
                None => {
                    let sql = format!(
                            "select distinct cast({} as integer) as y from \"{}\" where create_time > 0",
                            local("%Y"),
                            t.table
                        );
                    Ok(self
                        .query(&t.db, &sql, &[])?
                        .iter()
                        .map(|r| int(r, "y"))
                        .collect())
                }
            }
        })?;
        let years: BTreeSet<i64> = found.into_iter().flatten().collect();
        Ok(Value::Array(
            years.into_iter().rev().map(|y| json!(y)).collect(),
        ))
    }

    /// Per session: message counts by kind plus first/last time; groups also get `group_my_messages`
    /// and `group_sender_count`.
    pub fn session_message_type_stats_batch(
        &self,
        session_ids: &[String],
        options: &Value,
    ) -> Result<Value> {
        let begin = options.get("begin").and_then(Value::as_i64).unwrap_or(0);
        let end = options.get("end").and_then(Value::as_i64).unwrap_or(0);
        let range = range_sql(begin, end);
        let mut out = Map::new();
        for sid in session_ids {
            let is_group = sid.ends_with("@chatroom");
            let mut c = [0i64; 8]; // total voice image video emoji call transfer red
            let (mut first, mut last, mut mine_total) = (0i64, 0i64, 0i64);
            let mut senders: BTreeSet<String> = BTreeSet::new();
            for t in self.message_tables(sid)? {
                let mine = self.my_ids(&t.db)?;
                let sql = format!(
                    "select count(*) as total, coalesce(sum(local_type = {VOICE}), 0) as voice, coalesce(sum(local_type = {IMAGE}), 0) as image, \
                     coalesce(sum(local_type = {VIDEO}), 0) as video, coalesce(sum(local_type = {EMOJI}), 0) as emoji, \
                     coalesce(sum(local_type = {CALL}), 0) as call, coalesce(sum(local_type = {TRANSFER}), 0) as transfer, \
                     coalesce(sum(local_type = {RED_PACKET}), 0) as red, min(nullif(create_time, 0)) as mn, max(create_time) as mx, \
                     coalesce(sum(real_sender_id in ({mine})), 0) as mine from \"{table}\" where 1 = 1{range}",
                    table = t.table
                );
                let r = self
                    .query(&t.db, &sql, &[])?
                    .into_iter()
                    .next()
                    .unwrap_or(Value::Null);
                for (slot, key) in c.iter_mut().zip([
                    "total", "voice", "image", "video", "emoji", "call", "transfer", "red",
                ]) {
                    *slot += int(&r, key);
                }
                let (mn, mx) = (int(&r, "mn"), int(&r, "mx"));
                if mn > 0 && (first == 0 || mn < first) {
                    first = mn;
                }
                last = last.max(mx);
                mine_total += int(&r, "mine");
                if is_group {
                    for (name, _) in self.sender_counts(&t, &range)? {
                        senders.insert(name);
                    }
                }
            }
            let mut o = json!({
                "total_messages": c[0], "voice_messages": c[1], "image_messages": c[2], "video_messages": c[3],
                "emoji_messages": c[4], "call_messages": c[5], "transfer_messages": c[6], "red_packet_messages": c[7],
                "first_timestamp": first, "last_timestamp": last
            });
            if is_group {
                o["group_my_messages"] = json!(mine_total);
                o["group_sender_count"] = json!(senders.len());
            }
            out.insert(sid.clone(), o);
        }
        Ok(Value::Object(out))
    }

    /// `(sender username, message count)` of one table; unknown senders are skipped.
    fn sender_counts(&self, t: &MsgTable, range: &str) -> Result<Vec<(String, i64)>> {
        let sql = format!(
            "select n.user_name as u, count(*) as n from \"{}\" m join Name2Id n on n.rowid = m.real_sender_id \
             where m.create_time > 0{} group by n.user_name",
            t.table,
            range.replace("create_time", "m.create_time")
        );
        Ok(self
            .query(&t.db, &sql, &[])?
            .into_iter()
            .filter_map(|r| Some((r["u"].as_str()?.to_string(), int(&r, "n"))))
            .collect())
    }

    /// One table's messages counted by sender name (`None`: a sender `Name2Id` does not name), type and local hour,
    /// in one pass: read through the leaf scan and grouped here, the hours worked out once a quarter hour (see
    /// [`time_groups`](Self::time_groups)). A table the scan does not read is grouped by SQL.
    fn sender_groups(&self, t: &MsgTable, range: (i64, i64)) -> Result<Vec<SenderGroup>> {
        let scanned = self.scan_messages(
            t,
            range,
            std::collections::HashMap::<(Option<i64>, i64, i64), i64>::new,
            |counts, m| {
                *counts.entry((m.sender, m.kind, m.time / 900)).or_default() += 1;
            },
            |conn, parts| {
                let mut counts: std::collections::HashMap<(Option<i64>, i64, i64), i64> =
                    std::collections::HashMap::new();
                for (&key, &n) in parts.iter().flatten() {
                    *counts.entry(key).or_default() += n;
                }
                // `user_name` is the primary key: a sender row id names one sender
                let mut names: std::collections::HashMap<i64, String> =
                    std::collections::HashMap::new();
                let mut stmt = conn.prepare("select rowid, user_name from Name2Id")?;
                let mut rows = stmt.query([])?;
                while let Some(r) = rows.next()? {
                    if let (rusqlite::types::ValueRef::Integer(id), Some(name)) =
                        (r.get_ref(0)?, r.get_ref(1)?.as_str_or_null()?)
                    {
                        names.insert(id, name.to_string());
                    }
                }
                let quarters: BTreeSet<i64> = counts.keys().map(|k| k.2).collect();
                let local = local_quarters(conn, &quarters)?;
                Ok(counts
                    .into_iter()
                    .map(|((sender, kind, q), n)| {
                        let name = sender.and_then(|id| names.get(&id).cloned());
                        (name, kind, local.get(&q).map_or(0, |l| l.0), n)
                    })
                    .collect())
            },
        )?;
        if let Some(groups) = scanned {
            return Ok(groups);
        }
        let range = range_sql(range.0, range.1);
        let sql = format!(
            "select n.user_name as u, coalesce(nullif(m.local_type, 0), 1) as t, {h} as h, count(*) as n \
             from \"{table}\" m left join Name2Id n on n.rowid = m.real_sender_id \
             where m.create_time > 0{range} group by m.real_sender_id, t, h",
            h = local("%H").replace("create_time", "m.create_time"),
            table = t.table,
            range = range.replace("create_time", "m.create_time"),
        );
        Ok(self
            .query(&t.db, &sql, &[])?
            .into_iter()
            .map(|r| {
                (
                    r["u"].as_str().map(str::to_string),
                    int(&r, "t"),
                    r["h"].as_str().and_then(|v| v.parse().ok()).unwrap_or(0),
                    int(&r, "n"),
                )
            })
            .collect())
    }

    /// Group statistics: per-sender counts (`sessions.<room>.senders` keyed by synthetic ids resolved through
    /// `idMap`, because sender row ids differ between shards), hour histogram and type counts.
    pub fn group_stats(&self, chatroom_id: &str, begin: i64, end: i64) -> Result<Value> {
        let mut totals: BTreeMap<String, i64> = BTreeMap::new();
        let mut hourly: BTreeMap<i64, i64> = BTreeMap::new();
        let mut types: BTreeMap<i64, i64> = BTreeMap::new();
        for t in self.message_tables(chatroom_id)? {
            for (name, kind, hour, n) in self.sender_groups(&t, (begin, end))? {
                if let Some(name) = name {
                    *totals.entry(name).or_default() += n;
                }
                *types.entry(kind).or_default() += n;
                *hourly.entry(hour).or_default() += n;
            }
        }
        let mut senders = Map::new();
        let mut id_map = Map::new();
        for (i, (name, n)) in totals.into_iter().enumerate() {
            let id = (i + 1).to_string();
            senders.insert(id.clone(), json!(n));
            id_map.insert(id, json!(name));
        }
        let keyed = |m: &BTreeMap<i64, i64>| {
            Value::Object(m.iter().map(|(k, v)| (k.to_string(), json!(v))).collect())
        };
        Ok(json!({
            "sessions": { chatroom_id: { "senders": senders } },
            "idMap": id_map, "hourly": keyed(&hourly), "typeCounts": keyed(&types)
        }))
    }

    // ── raw read-only queries ──

    /// Default database of a `kind`, or `path` (absolute or relative to `db_storage`) which must stay inside `db_storage`.
    pub fn resolve_db_path(&self, kind: &str, path: Option<&str>) -> Result<PathBuf> {
        let root = self.db_storage().to_path_buf();
        match path.map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => {
                let candidate = if Path::new(p).is_absolute() {
                    PathBuf::from(p)
                } else {
                    root.join(p)
                };
                let inside = candidate.starts_with(&root)
                    && !candidate
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir));
                if !inside {
                    bail!("database path is outside the account's db_storage: {p}");
                }
                Ok(candidate)
            }
            None => match kind {
                "session" => Ok(root.join("session/session.db")),
                "contact" => Ok(root.join("contact/contact.db")),
                "sns" => Ok(root.join("sns/sns.db")),
                "emoticon" => Ok(root.join("emoticon/emoticon.db")),
                "hardlink" => Ok(root.join("hardlink/hardlink.db")),
                "head_image" => Ok(root.join("head_image/head_image.db")),
                "message" => self
                    .message_dbs()
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow!("no message database found")),
                other => bail!("unknown database kind: {other}"),
            },
        }
    }

    /// Run a read-only statement and return the rows as JSON objects.
    pub fn exec_query(&self, kind: &str, path: Option<&str>, sql: &str) -> Result<Value> {
        let db = self.resolve_db_path(kind, path)?;
        Ok(Value::Array(self.query(&db, sql, &[])?))
    }
}

/// Table names come from our own listings, but they are interpolated into SQL, so check them anyway.
pub(crate) fn check_ident(name: &str) -> Result<()> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("invalid table name: {name}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Fixture, DAY, T0};

    fn account(tag: &str) -> NativeAccount {
        let root = std::env::temp_dir().join(format!("weflow-stats-{}-{tag}", std::process::id()));
        let f = Fixture::standard(&root);
        NativeAccount::new(f.db_storage(), &f.key_hex())
            .unwrap()
            .with_my_wxid(Some("wxid_me_ab12".into()))
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn sum(v: &Value) -> i64 {
        v.as_object()
            .unwrap()
            .values()
            .map(|n| n.as_i64().unwrap())
            .sum()
    }

    #[test]
    fn aggregate_counts_split_by_direction_type_and_time() {
        let a = account("agg");
        let v = a
            .aggregate_stats(&ids(&["wxid_bob", "room1@chatroom"]), 0, 0)
            .unwrap();
        assert_eq!(
            (
                v["total"].as_i64(),
                v["sent"].as_i64(),
                v["received"].as_i64()
            ),
            (Some(8), Some(3), Some(5))
        );
        assert_eq!(
            (v["firstTime"].as_i64(), v["lastTime"].as_i64()),
            (Some(T0), Some(T0 + 4 * DAY))
        );
        assert_eq!(v["typeCounts"], json!({"1": 7, "3": 1}));
        for key in ["hourly", "weekday", "daily", "monthly"] {
            assert_eq!(sum(&v[key]), 8, "{key}");
        }
        assert!(
            v["monthly"]
                .as_object()
                .unwrap()
                .keys()
                .all(|k| k.len() == 7 && &k[4..5] == "-"),
            "{}",
            v["monthly"]
        );
        assert_eq!(
            v["sessions"]["wxid_bob"],
            json!({"total": 5, "sent": 2, "received": 3, "lastTime": T0 + 4 * DAY})
        );
        assert_eq!(v["sessions"]["room1@chatroom"]["total"], 3);
        assert_eq!(v["idMap"], json!({}));
    }

    #[test]
    fn aggregate_honours_the_time_range_and_unknown_sessions() {
        let a = account("range");
        let v = a
            .aggregate_stats(
                &ids(&["wxid_bob", "room1@chatroom", "nobody"]),
                T0 + DAY,
                T0 + 3 * DAY,
            )
            .unwrap();
        assert_eq!(
            (v["total"].as_i64(), v["sent"].as_i64()),
            (Some(3), Some(1))
        );
        assert!(v["sessions"].get("nobody").is_none());
        let empty = a.aggregate_stats(&[], 0, 0).unwrap();
        assert_eq!(
            (empty["total"].as_i64(), empty["firstTime"].as_i64()),
            (Some(0), Some(0))
        );
    }

    #[test]
    fn table_stats_and_time_range_per_shard() {
        let a = account("tables");
        let v = a.message_table_stats("wxid_bob").unwrap();
        let rows = v.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0]["count"].as_i64(), rows[1]["count"].as_i64()),
            (Some(3), Some(2))
        );
        assert_eq!(
            (
                rows[0]["first_timestamp"].as_i64(),
                rows[0]["last_timestamp"].as_i64()
            ),
            (Some(T0), Some(T0 + DAY))
        );
        assert_eq!(
            rows[1]["last_time"].as_i64(),
            Some(T0 + 4 * DAY),
            "push.rs reads last_time"
        );
        let range = a
            .message_table_time_range(
                rows[1]["db_path"].as_str().unwrap(),
                rows[1]["table_name"].as_str().unwrap(),
            )
            .unwrap();
        assert_eq!(
            range,
            json!({"first_ts": T0 + 3 * DAY, "last_ts": T0 + 4 * DAY})
        );
        assert!(a
            .message_table_time_range(
                rows[1]["db_path"].as_str().unwrap(),
                "x\"; drop table y; --"
            )
            .is_err());
        assert_eq!(a.message_table_stats("nobody").unwrap(), json!([]));
    }

    #[test]
    fn years_and_date_count_batches() {
        let a = account("years");
        assert_eq!(
            a.available_years(&ids(&["wxid_bob", "room1@chatroom"]))
                .unwrap(),
            json!([2023])
        );
        assert_eq!(sum(&a.session_message_date_counts("wxid_bob").unwrap()), 5);
        assert_eq!(a.session_message_date_counts("nobody").unwrap(), json!({}));
    }

    #[test]
    fn type_stats_batch_for_private_and_group_sessions() {
        let a = account("types");
        let v = a
            .session_message_type_stats_batch(&ids(&["wxid_bob", "room1@chatroom"]), &json!({}))
            .unwrap();
        let bob = &v["wxid_bob"];
        assert_eq!(
            (
                bob["total_messages"].as_i64(),
                bob["image_messages"].as_i64(),
                bob["voice_messages"].as_i64()
            ),
            (Some(5), Some(1), Some(0))
        );
        assert_eq!(
            (
                bob["first_timestamp"].as_i64(),
                bob["last_timestamp"].as_i64()
            ),
            (Some(T0), Some(T0 + 4 * DAY))
        );
        assert!(bob.get("group_my_messages").is_none());
        let room = &v["room1@chatroom"];
        assert_eq!(
            (
                room["group_my_messages"].as_i64(),
                room["group_sender_count"].as_i64()
            ),
            (Some(1), Some(3))
        );
        // range: only the day-1..day-3 slice of bob
        let ranged = a
            .session_message_type_stats_batch(
                &ids(&["wxid_bob"]),
                &json!({"begin": T0 + DAY, "end": T0 + 3 * DAY}),
            )
            .unwrap();
        assert_eq!(ranged["wxid_bob"]["total_messages"], 2);
    }

    #[test]
    fn group_stats_merge_senders_across_shards_by_username() {
        let a = account("group");
        let v = a.group_stats("room1@chatroom", 0, 0).unwrap();
        let senders = v["sessions"]["room1@chatroom"]["senders"]
            .as_object()
            .unwrap();
        assert_eq!(senders.len(), 3);
        let names: BTreeSet<&str> = v["idMap"]
            .as_object()
            .unwrap()
            .values()
            .map(|n| n.as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["wxid_bob", "wxid_me", "wxid_quiet"].into_iter().collect()
        );
        assert!(
            senders.keys().all(|id| v["idMap"].get(id).is_some()),
            "every sender id resolves through idMap"
        );
        assert_eq!(sum(&v["typeCounts"]), 3);
        assert_eq!(sum(&v["hourly"]), 3);
        // bob talks in both shards of the private chat, but the group is only in shard 0
        assert_eq!(
            a.group_stats("room1@chatroom", T0 + DAY, 0).unwrap()["sessions"]["room1@chatroom"]
                ["senders"]
                .as_object()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn exec_query_is_read_only_and_stays_inside_db_storage() {
        let a = account("exec");
        let rows = a
            .exec_query("session", None, "select count(*) as n from SessionTable")
            .unwrap();
        assert_eq!(rows[0]["n"], 3);
        let shard = a.message_dbs()[0].to_string_lossy().to_string();
        assert!(
            a.exec_query("message", Some(&shard), "select count(*) as n from Name2Id")
                .unwrap()[0]["n"]
                .as_i64()
                .unwrap()
                > 0
        );
        assert!(
            a.exec_query("message", None, "select 1 as x").is_ok(),
            "default message db is the first shard"
        );
        assert!(a
            .exec_query("session", Some("../../elsewhere.db"), "select 1")
            .is_err());
        assert!(a
            .exec_query("session", Some("/etc/passwd"), "select 1")
            .is_err());
        assert!(a.exec_query("nope", None, "select 1").is_err());
        assert!(
            a.exec_query("session", None, "delete from SessionTable")
                .is_err(),
            "snapshots are read-only"
        );
    }
}
