//! 消息表的统计。
//!
//! 涉及一张表全部消息的统计经 `leaf_scan` 只读需要的几个单元格，在这里计数；扫描读不了的表用分组的 SQL 统计。
//!
//! 日期、小时和星期按本地时间（SQLite 的 `localtime`），与桌面版显示的一致。
//! 结果的格式就是业务层读取的格式（`total/sent/received/...`）。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::db::Account;
use crate::msg::MsgTable;

const VOICE: i64 = 34;
const IMAGE: i64 = 3;
const VIDEO: i64 = 43;
const EMOJI: i64 = 47;
const CALL: i64 = 50;
/// `(2000 << 32) | 49` 和 `(2001 << 32) | 49`：微信 4 保存的转账和红包 app 消息。统计只读 `local_type`，不读内容，
/// 所以保存成 49、`<type>` 只在 XML 里的 app 消息算作 49，不算转账或红包（#1263）。
const TRANSFER: i64 = 8_589_934_592_049;
const RED_PACKET: i64 = 8_594_229_559_345;

fn int(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// 与 [`int`] 从 JSON 行读整数列的方式一致：不是整数的都算 0。
fn sql_int(v: rusqlite::types::ValueRef<'_>) -> i64 {
    match v {
        rusqlite::types::ValueRef::Integer(i) => i,
        _ => 0,
    }
}

/// 用一次查询得出每一刻钟（`create_time / 900`）开始时刻的本地 `%H`、`%w` 和 `%Y-%m-%d`，
/// 与 SQLite 的 `localtime` 给出的一致。
fn local_quarters(
    conn: &rusqlite::Connection,
    quarters: &BTreeSet<i64>,
) -> Result<HashMap<i64, (i64, i64, String)>> {
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
    let mut out = HashMap::with_capacity(quarters.len());
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

/// 一组消息的发送者名称、类型、本地小时和数量（见 [`Account::sender_groups`]）。
type SenderGroup = (Option<String>, i64, i64, i64);

/// 一条消息的类型、所在的一刻钟（`create_time / 900`）和是否由账号所有者发送。
type QuarterKey = (i64, i64, bool);

/// 统计要读的一条消息的单元格。
#[derive(Clone, Copy)]
struct Msg {
    /// 类型：`coalesce(nullif(local_type, 0), 1)`。
    kind: i64,
    time: i64,
    /// 发送者：`real_sender_id`。
    sender: Option<i64>,
}

// 表中每条消息一个，按值传给累加器：固定它的大小，只在有意时改变（#505）
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Msg>() == 32);

/// 一项统计同时读取的表的最大数目。
const TABLE_THREADS: usize = 8;

/// 在至多 [`TABLE_THREADS`] 个线程上对 `items` 的每一项运行 `f`（每张表的叶页扫描大部分时间花在 HMAC 上，
/// 而大多数会话太小，不值得再把一张表分给几个线程），结果按 `items` 的顺序排列；出错时返回按这个顺序的第一个错误。
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

/// 在 [`TypeCounts::kinds`] 中逐个计数的消息类型，按它在总数之后的顺序。
const COUNTED_KINDS: [i64; 7] = [VOICE, IMAGE, VIDEO, EMOJI, CALL, TRANSFER, RED_PACKET];

/// 一张表或一个会话的全部表中的消息，按导出窗口的统计方式计数。
#[derive(Default)]
struct TypeCounts {
    /// 先是总数，然后是 [`COUNTED_KINDS`] 中每种类型的数量。
    kinds: [i64; 8],
    /// 最早的 `create_time`（`0` 表示没有消息）。
    first: i64,
    last: i64,
    /// 账号所有者发送的消息数。
    mine: i64,
}

impl TypeCounts {
    fn add(&mut self, other: &Self) {
        for (slot, n) in self.kinds.iter_mut().zip(other.kinds) {
            *slot += n;
        }
        if other.first > 0 && (self.first == 0 || other.first < self.first) {
            self.first = other.first;
        }
        self.last = self.last.max(other.last);
        self.mine += other.mine;
    }
}

/// 把 `Name2Id` 读成发送者行号到名称的映射；`user_name` 是主键，所以一个行号只对应一个发送者。
fn sender_names(conn: &rusqlite::Connection) -> Result<HashMap<i64, String>> {
    let mut names = HashMap::new();
    let mut stmt = conn.prepare("select rowid, user_name from Name2Id")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        if let (rusqlite::types::ValueRef::Integer(id), Some(name)) =
            (r.get_ref(0)?, r.get_ref(1)?.as_str_or_null()?)
        {
            names.insert(id, name.to_string());
        }
    }
    Ok(names)
}

/// [`Account::my_ids`] 给出的列表中的发送者行号。
pub(crate) fn id_set(list: &str) -> HashSet<i64> {
    list.split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect()
}

/// 消息数，以及其中最早和最晚的 `create_time`。
struct Tally {
    count: i64,
    first: i64,
    last: i64,
}

/// 一张表中类型、本地小时、星期、日期都相同，发送方（是否由账号所有者发送）也相同的一组消息。
struct TimeGroup {
    kind: i64,
    /// 本地时间的 `%H`、`%w` 和 `%Y-%m-%d`。
    hour: i64,
    weekday: i64,
    day: String,
    /// 是否由账号所有者发送。
    mine: bool,
    count: i64,
    /// 最早和最晚的 `create_time`。
    first: i64,
    last: i64,
}

/// 正的边界写成 `and create_time >= b and create_time <= e`（`0` 表示不限）。
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

impl Account {
    /// 用叶页扫描读取 `t` 中时间为正、在 `begin..=end`（0 表示不限）之内的每条消息：
    /// 扫描的每个线程把消息计入自己的累加器（`start`、`each`），`finish` 拿到全部累加器和一个数据库连接。
    /// 扫描读不了这张表时为 `None`。
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

    /// 账号所有者在一个分片的 `Name2Id` 中的行号，写成 SQL 列表字面量的内容（用逗号分隔）。
    pub(crate) fn my_ids(&self, db: &Path) -> Result<String> {
        let rows = self.query(db, "select rowid as id, user_name from Name2Id", &[])?;
        let ids: Vec<String> = rows
            .iter()
            .filter(|r| self.is_me(r["user_name"].as_str().unwrap_or("")))
            .map(|r| int(r, "id").to_string())
            .collect();
        Ok(ids.join(","))
    }

    /// `tables` 所在的每个数据库的 [`my_ids`](Self::my_ids)，每个数据库只读一次，而不是每张表读一次。
    pub(crate) fn my_ids_of<'t>(
        &self,
        tables: impl IntoIterator<Item = &'t MsgTable>,
    ) -> Result<HashMap<PathBuf, String>> {
        let mut mine = HashMap::new();
        for t in tables {
            if !mine.contains_key(&t.db) {
                mine.insert(t.db.clone(), self.my_ids(&t.db)?);
            }
        }
        Ok(mine)
    }

    /// 消息的统计：`{ total, sent, received, firstTime, lastTime, typeCounts, hourly, weekday, daily, monthly,
    /// sessions }`。`begin`、`end` 为秒，0 表示不限。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn aggregate_stats(&self, session_ids: &[String], begin: i64, end: i64) -> Result<Value> {
        self.aggregate_inner(session_ids, begin, end, false)
    }

    /// 只取 [`aggregate_stats`](Self::aggregate_stats) 的 `sessions` 和 `total`，数字相同：每张表计一次数，
    /// 不做其余各项需要的按类型和时间分组。返回 `{ total, sessions }`。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn session_totals(&self, session_ids: &[String], begin: i64, end: i64) -> Result<Value> {
        let range = range_sql(begin, end);
        let mut total = 0i64;
        let mut sessions = Map::new();
        let tables = self.message_tables_of(session_ids)?;
        let mine = self.my_ids_of(tables.iter().flatten())?;
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
        Ok(json!({ "total": total, "sessions": sessions }))
    }

    /// [`aggregate_stats`](Self::aggregate_stats) 再给每个会话加上 `monthly`：一年中每个月的消息数（`"1"`..`"12"`，
    /// 范围内各年相加），年度报告按它给好友排名。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
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
        let tables = self.message_tables_of(session_ids)?;
        let mine_of = self.my_ids_of(tables.iter().flatten())?;
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
            "daily": named(&daily), "monthly": named(&monthly), "sessions": sessions
        }))
    }

    /// 一张表的消息按类型、本地小时、星期、日期以及是否由账号所有者发送分组。
    ///
    /// 消息经叶页扫描读出，在这里按类型、一刻钟（`create_time / 900`）和发送方分组。每一刻钟的本地小时、
    /// 星期和日期再用一次查询由 SQLite 的 `localtime` 给出，所以与逐条消息用 `strftime` 得到的相同：
    /// 现在使用的每个 UTC 偏移都是整数个一刻钟，切换也在一刻钟的整点，所以同一刻钟内的本地小时、星期和日期都相同。
    /// 扫描读不了的表用 SQL 分组。
    fn time_groups(&self, t: &MsgTable, mine: &str, range: (i64, i64)) -> Result<Vec<TimeGroup>> {
        let ids = id_set(mine);
        let scanned = self.scan_messages(
            t,
            range,
            HashMap::<QuarterKey, Tally>::new,
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
                let mut groups: HashMap<QuarterKey, Tally> = HashMap::new();
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

    /// 叶页扫描读不了的表的 [`time_groups`](Self::time_groups)：
    /// 用 SQL 分组，单元格按 SQLite 自己的比较和运算规则处理。
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

    /// 会话在每个分片中的表各一行：路径、表名、行数，以及最早和最晚的消息时间。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn message_table_stats(&self, session_id: &str) -> Result<Value> {
        self.tables_stats(&self.message_tables(session_id)?)
    }

    /// `session_ids` 中每个会话的 [`message_table_stats`](Self::message_table_stats)，顺序与 `session_ids` 相同。
    /// 各会话的表在一次列出的全部分片的表中查找（`message_tables_of`），而不是为每个会话查一遍每个分片的表结构；
    /// 每个会话的行各是一个结果，所以一个会话的表读不了时，其他会话的结果不受影响。
    ///
    /// # Errors
    ///
    /// 某个分片的表列不出来时整个调用失败。
    pub fn message_table_stats_of(&self, session_ids: &[String]) -> Result<Vec<Result<Value>>> {
        Ok(self
            .message_tables_of(session_ids)?
            .iter()
            .map(|tables| self.tables_stats(tables))
            .collect())
    }

    /// `tables` 的 [`message_table_stats`](Self::message_table_stats) 各行。
    fn tables_stats(&self, tables: &[MsgTable]) -> Result<Value> {
        let mut out = Vec::new();
        for t in tables {
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

    /// 一张消息表的 `{ first_ts, last_ts }`（秒）。
    ///
    /// # Errors
    ///
    /// `db` 不在账号的 `db_storage` 之内（为空时取第一个消息库，没有消息库也会失败），`table` 含有字母、数字和
    /// 下划线以外的字符，或者读取失败（打开、解密或查询失败）。
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

    /// 这些会话中任一会话有消息的年份（本地时间，降序）。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn available_years(&self, session_ids: &[String]) -> Result<Value> {
        let tables = self.message_tables_of(session_ids)?;
        let every: Vec<&MsgTable> = tables.iter().flatten().collect();
        let found = each_table(&every, |t| -> Result<Vec<i64>> {
            let t = *t;
            // 有消息的每一刻钟的本地年份（见 `time_groups`）
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

    /// 每个会话按类型的消息数，以及最早和最晚的时间；群聊另有 `group_my_messages` 和 `group_sender_count`。
    /// `options` 的 `begin`、`end` 为秒，没有或为 0 时不限。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn session_message_type_stats_batch(
        &self,
        session_ids: &[String],
        options: &Value,
    ) -> Result<Value> {
        let begin = options.get("begin").and_then(Value::as_i64).unwrap_or(0);
        let end = options.get("end").and_then(Value::as_i64).unwrap_or(0);
        let mut out = Map::new();
        let tables = self.message_tables_of(session_ids)?;
        let mine = self.my_ids_of(tables.iter().flatten())?;
        for (sid, tables) in session_ids.iter().zip(&tables) {
            let is_group = sid.ends_with("@chatroom");
            let mut all = TypeCounts::default();
            let mut senders: BTreeSet<String> = BTreeSet::new();
            for t in tables {
                let (counts, names) = self.type_counts(t, &mine[&t.db], (begin, end), is_group)?;
                all.add(&counts);
                senders.extend(names);
            }
            let TypeCounts {
                kinds: c,
                first,
                last,
                mine: mine_total,
            } = all;
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

    /// 一张表的 [`TypeCounts`]，`named` 为真时还有它的发送者的名称（来自 `Name2Id`），叶页扫描只读一遍表。
    /// 扫描读不了的表用 SQL 统计。
    fn type_counts(
        &self,
        t: &MsgTable,
        mine: &str,
        range: (i64, i64),
        named: bool,
    ) -> Result<(TypeCounts, Vec<String>)> {
        let ids = id_set(mine);
        let scanned = self.scan_messages(
            t,
            range,
            || (TypeCounts::default(), HashSet::<i64>::new()),
            |(counts, senders), m| {
                counts.kinds[0] += 1;
                if let Some(slot) = COUNTED_KINDS.iter().position(|&kind| kind == m.kind) {
                    counts.kinds[slot + 1] += 1;
                }
                counts.first = if counts.first == 0 {
                    m.time
                } else {
                    counts.first.min(m.time)
                };
                counts.last = counts.last.max(m.time);
                if let Some(id) = m.sender {
                    counts.mine += i64::from(ids.contains(&id));
                    senders.insert(id);
                }
            },
            |conn, parts| {
                let mut all = TypeCounts::default();
                for (counts, _) in parts {
                    all.add(counts);
                }
                let names = if named {
                    let known = sender_names(conn)?;
                    let senders: HashSet<i64> =
                        parts.iter().flat_map(|(_, s)| s.iter().copied()).collect();
                    senders
                        .iter()
                        .filter_map(|id| known.get(id).cloned())
                        .collect()
                } else {
                    Vec::new()
                };
                Ok((all, names))
            },
        )?;
        if let Some(counted) = scanned {
            return Ok(counted);
        }
        let range = range_sql(range.0, range.1);
        let sql = format!(
            "select count(*) as total, coalesce(sum(local_type = {VOICE}), 0) as voice, coalesce(sum(local_type = {IMAGE}), 0) as image, \
             coalesce(sum(local_type = {VIDEO}), 0) as video, coalesce(sum(local_type = {EMOJI}), 0) as emoji, \
             coalesce(sum(local_type = {CALL}), 0) as call, coalesce(sum(local_type = {TRANSFER}), 0) as transfer, \
             coalesce(sum(local_type = {RED_PACKET}), 0) as red, min(create_time) as mn, max(create_time) as mx, \
             coalesce(sum(real_sender_id in ({mine})), 0) as mine from \"{table}\" where create_time > 0{range}",
            table = t.table
        );
        let r = self
            .query(&t.db, &sql, &[])?
            .into_iter()
            .next()
            .unwrap_or(Value::Null);
        let mut counts = TypeCounts {
            first: int(&r, "mn"),
            last: int(&r, "mx"),
            mine: int(&r, "mine"),
            ..TypeCounts::default()
        };
        for (slot, key) in counts.kinds.iter_mut().zip([
            "total", "voice", "image", "video", "emoji", "call", "transfer", "red",
        ]) {
            *slot = int(&r, key);
        }
        let names = if named {
            self.sender_counts(t, &range)?
                .into_iter()
                .map(|(name, _)| name)
                .collect()
        } else {
            Vec::new()
        };
        Ok((counts, names))
    }

    /// 一张表中每个发送者的用户名和消息数；略过 `Name2Id` 里没有的发送者。
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

    /// 一遍读完一张表，按发送者名称（`Name2Id` 里没有的发送者为 `None`）、类型和本地小时统计消息：
    /// 经叶页扫描读取、在这里分组，每一刻钟只换算一次小时（见 [`time_groups`](Self::time_groups)）。
    /// 扫描读不了的表用 SQL 分组。
    fn sender_groups(&self, t: &MsgTable, range: (i64, i64)) -> Result<Vec<SenderGroup>> {
        let scanned = self.scan_messages(
            t,
            range,
            HashMap::<(Option<i64>, i64, i64), i64>::new,
            |counts, m| {
                *counts.entry((m.sender, m.kind, m.time / 900)).or_default() += 1;
            },
            |conn, parts| {
                let mut counts: HashMap<(Option<i64>, i64, i64), i64> = HashMap::new();
                for (&key, &n) in parts.iter().flatten() {
                    *counts.entry(key).or_default() += n;
                }
                let names = sender_names(conn)?;
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

    /// 群的统计：每个发送者的消息数（`sessions.<room>.senders` 的键是从 1 开始的编号，经 `idMap` 换成用户名，
    /// 因为同一发送者在不同分片中的行号不同）、按小时的分布和按类型的数量。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
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

    // ── 只读的原始查询 ──

    /// `kind` 的默认数据库；给出 `path`（绝对路径或相对于 `db_storage` 的路径）时取它，它必须在 `db_storage` 之内。
    fn resolve_db_path(&self, kind: &str, path: Option<&str>) -> Result<PathBuf> {
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

    /// 运行一条只读语句，把读出的行作为 JSON 对象返回。
    ///
    /// # Errors
    ///
    /// `path` 不在 `db_storage` 之内，没给 `path` 时 `kind` 未知或者没有消息库，或者读取失败（打开、解密、
    /// 语句出错，包括写数据库的语句）。
    pub fn exec_query(&self, kind: &str, path: Option<&str>, sql: &str) -> Result<Value> {
        let db = self.resolve_db_path(kind, path)?;
        Ok(Value::Array(self.query(&db, sql, &[])?))
    }
}

/// 表名来自本程序自己列出的结果，但会拼进 SQL，所以仍然检查一遍。
fn check_ident(name: &str) -> Result<()> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("invalid table name: {name}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Fixture, TestDir, DAY, T0};

    fn account(tag: &str) -> (TestDir, Account) {
        let root = TestDir::new(&format!("weflow-stats-{tag}"));
        let f = Fixture::standard(&root);
        let acct = Account::new(f.db_storage(), &f.key_hex())
            .unwrap()
            .with_my_wxid(Some("wxid_me_ab12".into()));
        (root, acct)
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
        let (_dir, a) = account("agg");
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
        assert!(v.get("idMap").is_none());
    }

    #[test]
    fn aggregate_honours_the_time_range_and_unknown_sessions() {
        let (_dir, a) = account("range");
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
        let (_dir, a) = account("tables");
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

    /// 一起查找的多个会话的表，与逐个会话查找的结果相同，按请求的顺序，包括没有消息的会话。
    #[test]
    fn table_stats_of_several_conversations_are_each_ones() {
        let (_dir, a) = account("tables-of");
        let ids = ["wxid_bob", "nobody", "room1@chatroom", "wxid_bob"].map(String::from);
        let together = a.message_table_stats_of(&ids).unwrap();
        assert_eq!(together.len(), ids.len());
        for (id, stats) in ids.iter().zip(together) {
            assert_eq!(stats.unwrap(), a.message_table_stats(id).unwrap(), "{id}");
        }
        assert!(a.message_table_stats_of(&[]).unwrap().is_empty());
    }

    #[test]
    fn years_and_date_count_batches() {
        let (_dir, a) = account("years");
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
        let (_dir, a) = account("types");
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
        // 时间范围：只取 bob 第 1 到第 3 个 `DAY` 的那一段
        let ranged = a
            .session_message_type_stats_batch(
                &ids(&["wxid_bob"]),
                &json!({"begin": T0 + DAY, "end": T0 + 3 * DAY}),
            )
            .unwrap();
        assert_eq!(ranged["wxid_bob"]["total_messages"], 2);
    }

    /// 转账和红包按 `local_type` 计数，即微信 4 保存的 `(<type> << 32) | 49`；
    /// 保存成 49、`<type>` 只在 XML 里的消息计入消息总数，不算转账或红包（#1263）。
    #[test]
    fn transfers_and_red_packets_are_counted_by_their_local_type() {
        use crate::fixture::{MsgSpec, SessionSpec};
        let root = TestDir::new("weflow-stats-app-kinds");
        let f = Fixture::new(&root, "wxid_me_ab12");
        f.session_db(&[SessionSpec {
            username: "wxid_pal",
            summary: "",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 1,
        }]);
        let app = |id, ty: i64| {
            let xml = format!("<msg><appmsg><title>t</title><type>{ty}</type></appmsg></msg>");
            MsgSpec::text(id, "wxid_pal", T0 + id, &xml)
        };
        f.message_shard(
            0,
            &[(
                "wxid_pal",
                vec![
                    app(1, 2000).of_type(TRANSFER),
                    app(2, 2001).of_type(RED_PACKET),
                    app(3, 2000).of_type(49),
                    app(4, 2001).of_type(49),
                ],
            )],
        );
        let a = Account::new(f.db_storage(), &f.key_hex())
            .unwrap()
            .with_my_wxid(Some("wxid_me_ab12".into()));
        let v = a
            .session_message_type_stats_batch(&ids(&["wxid_pal"]), &json!({}))
            .unwrap();
        let pal = &v["wxid_pal"];
        assert_eq!(
            (
                pal["total_messages"].as_i64(),
                pal["transfer_messages"].as_i64(),
                pal["red_packet_messages"].as_i64()
            ),
            (Some(4), Some(1), Some(1))
        );
    }

    #[test]
    fn group_stats_merge_senders_across_shards_by_username() {
        let (_dir, a) = account("group");
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
        // bob 的私聊在两个分片里都有，但这个群只在分片 0 里
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
        let (_dir, a) = account("exec");
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
