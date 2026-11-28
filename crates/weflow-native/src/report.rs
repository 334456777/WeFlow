//! 报告的统计：双人报告（`dual_report_stats`）和足迹（`footprint_stats`）。
//!
//! 公式取自桌面版，与业务层自己的游标扫描相同：一轮聊天是相邻消息间隔不超过一小时的一段连续消息；
//! 只统计账号所有者的回复；常用语是完全相同的短文字，取自文字消息和引用消息的回复（[`reply_text`]）；
//! 日期和小时按本地时间。年度报告的扩展统计（`annual_report_extras`）与业务层的游标扫描定义相同，
//! 所以两条路径得出的数字相同。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::db::Account;
use crate::msg::MsgTable;
use crate::quote::{reply_text, QUOTE};

/// 静默超过这么多秒后开始新的一轮聊天（或足迹的新一段）。
const GAP: i64 = 3600;
/// 比这更慢的回复不算回复（秒）。
const MAX_RESPONSE: i64 = 86_400;
/// 文字消息；与 [`QUOTE`] 一起，是文字由用户输入、可以成为常用语的两种类型（[`typed_text`]）。只有微信 4 保存的
/// `(57 << 32) | 49` 才算引用回复：报告只读 `local_type`，所以保存成 49、`<type>` 只在 XML 里的回复不算（#1263）。
const TEXT: i64 = 1;
const IMAGE: i64 = 3;
const VOICE: i64 = 34;
const EMOJI: i64 = 47;
/// 常用语列表的长度：总列表取这么多条，每一方的列表取 [`PHRASES_SIDE`] 条。
const PHRASES_TOP: usize = 50;
const PHRASES_SIDE: usize = 200;
/// 在足迹里从不算作「人」的账号。
const NOT_PEOPLE: &[&str] = &[
    "weixin",
    "medianote",
    "floatbottle",
    "qmessage",
    "qqmail",
    "fmessage",
    "notifymessage",
    "newsapp",
    "brandsessionholder",
    "brandservicesessionholder",
    "opencustomerservicemsg",
    "notification_messages",
    "userexperience_alarm",
    "helper_folders",
    "@helper_folders",
];

fn int(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn text(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// 会话中的一条消息，只保留报告需要的字段。
struct Msg {
    time: i64,
    local_id: i64,
    kind: i64,
    mine: bool,
    hour: usize,
    weekday_mon: usize,
    day: String,
    day_no: i64,
    content: String,
}

fn range_sql(begin: i64, end: i64) -> String {
    let mut s = String::new();
    if begin > 0 {
        s.push_str(&format!(" and m.create_time >= {begin}"));
    }
    if end > 0 {
        s.push_str(&format!(" and m.create_time <= {end}"));
    }
    s
}

/// 一方的常用语列表：`texts` 是 2..=20 个字符、不含 `http` 和 `<`、不以 `[` 开头的文字（[`is_phrase`]），
/// 按完全相同计数，至少出现两次才保留，按次数排序，次数相同时按第一次出现的先后，最多 `cap` 条。
fn phrase_list(texts: &[&str], cap: usize) -> Value {
    let mut counts: HashMap<&str, (i64, usize)> = HashMap::new();
    for (i, t) in texts.iter().enumerate() {
        counts.entry(t).or_insert((0, i)).0 += 1;
    }
    let mut v: Vec<(&str, i64, usize)> = counts
        .into_iter()
        .filter(|(_, (c, _))| *c >= 2)
        .map(|(p, (c, i))| (p, c, i))
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    Value::Array(
        v.into_iter()
            .take(cap)
            .map(|(p, c, _)| json!({ "phrase": p, "count": c }))
            .collect(),
    )
}

fn is_phrase(t: &str) -> bool {
    let n = t.chars().count();
    (2..=20).contains(&n) && !t.contains("http") && !t.contains('<') && !t.starts_with('[')
}

/// 类型为 `kind`、内容为 `content` 的消息中用户输入的文字：文字消息取内容，引用消息取回复的文字。
/// SQL 只读这两种类型的内容。
fn typed_text(kind: i64, content: String) -> String {
    if kind == QUOTE {
        reply_text(&content)
    } else {
        content
    }
}

/// 消息在会话顺序中的位置（`sort_seq`，没有时用 `create_time * 1000`；其次是 `create_time` 和 `local_id`），
/// 以及消息本身。
type Placed = ((i64, i64, i64), Msg);

/// [`Account::extras_messages`] 在页内解密的文字的最大长度，更长的改用 SQL 读取。
const LONGEST_TEXT: usize = 4096;

/// 年度报告的扩展统计从一个会话的消息中取得的数据（见 [`Account::annual_report_extras`]）。
struct SessionExtras {
    /// 星期（周一在前）× 小时。
    heatmap: [[i64; 24]; 7],
    night: i64,
    initiated: i64,
    received: i64,
    /// 账号所有者的回复次数，以及这些回复总共用的秒数。
    replies: usize,
    reply_seconds: i64,
    in_peak: i64,
    /// 账号所有者的常用语候选，按消息的顺序。
    phrases: Vec<String>,
    /// 最长的连续天数，以及它的第一天和最后一天。
    best: i64,
    best_start: String,
    best_end: String,
}

impl SessionExtras {
    /// 按顺序汇总一个会话的消息；会话没有消息时为 `None`。
    fn of(msgs: impl Iterator<Item = Msg>, peak_begin: i64, peak_end: i64) -> Option<Self> {
        let mut e = Self {
            heatmap: [[0; 24]; 7],
            night: 0,
            initiated: 0,
            received: 0,
            replies: 0,
            reply_seconds: 0,
            in_peak: 0,
            phrases: Vec::new(),
            best: 0,
            best_start: String::new(),
            best_end: String::new(),
        };
        let mut any = false;
        let mut last: Option<(i64, bool)> = None;
        let (mut last_day, mut run, mut run_start) = (None::<i64>, 0i64, String::new());
        for m in msgs {
            any = true;
            e.heatmap[m.weekday_mon][m.hour] += 1;
            if m.hour < 6 {
                e.night += 1;
            }
            if peak_begin > 0 && peak_end > 0 && m.time >= peak_begin && m.time <= peak_end {
                e.in_peak += 1;
            }
            match last {
                Some((t, was_mine)) if m.time - t <= GAP => {
                    if !was_mine && m.mine && m.time - t > 0 && m.time - t < MAX_RESPONSE {
                        e.replies += 1;
                        e.reply_seconds += m.time - t;
                    }
                }
                _ => {
                    if m.mine {
                        e.initiated += 1;
                    } else {
                        e.received += 1;
                    }
                }
            }
            last = Some((m.time, m.mine));
            if matches!(m.kind, TEXT | QUOTE) && m.mine {
                let t = m.content.trim();
                if is_phrase(t) {
                    e.phrases.push(t.to_string());
                }
            }
            if last_day != Some(m.day_no) {
                if last_day.is_some_and(|l| m.day_no - l == 1) {
                    run += 1;
                } else {
                    run = 1;
                    run_start = m.day.clone();
                }
                if run > e.best {
                    (e.best, e.best_start, e.best_end) = (run, run_start.clone(), m.day.clone());
                }
                last_day = Some(m.day_no);
            }
        }
        any.then_some(e)
    }
}

impl Account {
    /// 会话在 `[begin, end]` 之内的消息，按时间顺序，从它的 `tables` 读出（合并全部分片）；
    /// `mine` 是各数据库中账号所有者的发送者行号（[`my_ids_of`](Self::my_ids_of)）。
    fn conversation(
        &self,
        tables: &[MsgTable],
        mine: &HashMap<PathBuf, String>,
        begin: i64,
        end: i64,
    ) -> Result<Vec<Msg>> {
        let mut rows: Vec<Placed> = Vec::new();
        for t in tables {
            rows.extend(self.table_messages(t, &mine[&t.db], begin, end)?);
        }
        rows.sort_by_key(|a| a.0);
        Ok(rows.into_iter().map(|(_, m)| m).collect())
    }

    /// 用 SQL 读出一张表在 `[begin, end]` 之内的消息，每条带着它的位置。
    fn table_messages(
        &self,
        t: &MsgTable,
        mine: &str,
        begin: i64,
        end: i64,
    ) -> Result<Vec<Placed>> {
        let range = range_sql(begin, end);
        let local = |f: &str| format!("strftime('{f}', m.create_time, 'unixepoch', 'localtime')");
        let sql = format!(
            "select m.create_time as t, m.sort_seq as s, m.local_id as l, coalesce(nullif(m.local_type, 0), 1) as k, \
             (m.real_sender_id in ({mine})) as mine, cast({h} as integer) as h, cast({w} as integer) as w, \
             date(m.create_time, 'unixepoch', 'localtime') as d, \
             cast(julianday(date(m.create_time, 'unixepoch', 'localtime')) + 0.5 as integer) as dn, \
             case when m.local_type in ({TEXT}, {QUOTE}) then m.message_content end as c \
             from \"{table}\" m where m.create_time > 0{range}",
            h = local("%H"),
            w = local("%w"),
            table = t.table
        );
        // 边读边转换成 `Msg`（JSON 行要大好几倍）
        let rows = std::cell::RefCell::new(Vec::<Placed>::new());
        self.query_each(
            &t.db,
            &sql,
            &[],
            || rows.borrow_mut().clear(),
            |r| {
                let time = int(&r, "t");
                let seq = match int(&r, "s") {
                    0 => time * 1000,
                    s => s,
                };
                let kind = int(&r, "k");
                rows.borrow_mut().push((
                    (seq, time, int(&r, "l")),
                    Msg {
                        time,
                        local_id: int(&r, "l"),
                        kind,
                        mine: int(&r, "mine") == 1,
                        hour: usize::try_from(int(&r, "h").clamp(0, 23)).unwrap_or(0),
                        weekday_mon: usize::try_from((int(&r, "w") + 6).rem_euclid(7)).unwrap_or(0),
                        day: text(&r, "d"),
                        day_no: int(&r, "dn"),
                        content: typed_text(kind, text(&r, "c")),
                    },
                ));
            },
        )?;
        Ok(rows.into_inner())
    }

    /// 年度报告扩展统计用的 [`table_messages`](Self::table_messages)，经叶页扫描读取。
    /// 只有扩展统计要读的文字才保留，即账号所有者的、文字可能成为常用语的文字消息和引用消息（[`typed_text`]、
    /// [`is_phrase`]）；扫描不解密的文字（长于 [`LONGEST_TEXT`]、存在溢出页上或经过压缩）改用 SQL 读取。
    /// 扫描读不了这张表时全部改用 SQL 读取。
    fn extras_messages(
        &self,
        t: &MsgTable,
        mine: &str,
        begin: i64,
        end: i64,
    ) -> Result<Vec<Placed>> {
        let ids = crate::stats::id_set(mine);
        /// 扫描找到的一条消息：位置、类型、是否由账号所有者发送，以及保留下来的文字。
        struct Found {
            key: (i64, i64, i64),
            kind: i64,
            mine: bool,
            text: Option<String>,
            /// 文字要改用 SQL 读取。
            unread: bool,
        }
        let scanned = self.read_db(
            &t.db,
            |conn| {
                // 扫描给出 rowid，只有 `local_id` 就是 rowid 时，它才是排序和 SQL 所用的 `local_id`
                let local_id_is_rowid: bool = conn.query_row(
                    "select count(*) = 1 and coalesce(sum(name = 'local_id' and upper(type) = 'INTEGER'), 0) = 1 \
                     from pragma_table_info(?1) where pk > 0",
                    [&t.table],
                    |r| r.get(0),
                )?;
                if !local_id_is_rowid {
                    return Ok(None);
                }
                crate::leaf_scan::target(
                    conn,
                    &t.table,
                    &[
                        "sort_seq",
                        "create_time",
                        "local_type",
                        "real_sender_id",
                        "message_content",
                    ],
                )
            },
            |file, target| {
                let Some((root, columns)) = target else {
                    return Ok(None);
                };
                crate::leaf_scan::rows(
                    file,
                    root,
                    &columns[..4],
                    Some((columns[4], LONGEST_TEXT)),
                    Vec::<Found>::new,
                    |found, row| {
                        let [seq, time, kind, sender] = row.ints else {
                            return;
                        };
                        let Some(time) = time.filter(|&t| t > 0) else {
                            return;
                        };
                        if (begin > 0 && time < begin) || (end > 0 && time > end) {
                            return;
                        }
                        let mine = sender.is_some_and(|s| ids.contains(&s));
                        // 与 SQL 一致：只取这两种类型的内容，类型缺失的行不取
                        let wanted = mine && matches!(*kind, Some(TEXT | QUOTE));
                        let (text, unread) = match (wanted, row.text) {
                            (false, _) | (true, crate::leaf_scan::Text::Null) => (None, false),
                            (true, crate::leaf_scan::Text::Short(t)) if *kind == Some(QUOTE) => {
                                (Some(reply_text(t)).filter(|said| is_phrase(said)), false)
                            }
                            (true, crate::leaf_scan::Text::Short(t)) => {
                                (is_phrase(t.trim()).then(|| t.to_string()), false)
                            }
                            (true, crate::leaf_scan::Text::Other) => (None, true),
                        };
                        let seq = match seq.unwrap_or(0) {
                            0 => time * 1000,
                            s => s,
                        };
                        found.push(Found {
                            key: (seq, time, row.rowid),
                            kind: match kind {
                                None | Some(0) => 1,
                                Some(k) => *k,
                            },
                            mine,
                            text,
                            unread,
                        });
                    },
                )
            },
        )?;
        let Some(parts) = scanned else {
            return self.table_messages(t, mine, begin, end);
        };
        let mut found: Vec<Found> = parts.into_iter().flatten().collect();
        // 扫描留下没读的文字
        let unread: Vec<i64> = found.iter().filter(|f| f.unread).map(|f| f.key.2).collect();
        let mut texts: HashMap<i64, String> = HashMap::new();
        for ids in unread.chunks(500) {
            let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            let sql = format!(
                "select m.local_id as l, case when m.local_type in ({TEXT}, {QUOTE}) then m.message_content end as c \
                 from \"{}\" m where m.local_id in ({list})",
                t.table
            );
            for r in self.query(&t.db, &sql, &[])? {
                texts.insert(int(&r, "l"), text(&r, "c"));
            }
        }
        for f in found.iter_mut().filter(|f| f.unread) {
            f.text = texts.remove(&f.key.2).map(|c| typed_text(f.kind, c));
        }
        // 每一刻钟的本地小时、星期和日期（见 `stats::time_groups`）
        let quarters: std::collections::BTreeSet<i64> =
            found.iter().map(|f| f.key.1 / 900).collect();
        let list = serde_json::to_string(&quarters.iter().collect::<Vec<_>>())?;
        let day = "date(value * 900, 'unixepoch', 'localtime')";
        let sql = format!(
            "select value, cast(strftime('%H', value * 900, 'unixepoch', 'localtime') as integer), \
             cast(strftime('%w', value * 900, 'unixepoch', 'localtime') as integer), {day}, \
             cast(julianday({day}) + 0.5 as integer) from json_each(?1)"
        );
        let local: HashMap<i64, (i64, i64, String, i64)> = self.with_db(&t.db, |conn| {
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query([&list])?;
            let mut out = HashMap::new();
            while let Some(r) = rows.next()? {
                let n = |i: usize| -> Result<i64> {
                    Ok(match r.get_ref(i)? {
                        rusqlite::types::ValueRef::Integer(v) => v,
                        _ => 0,
                    })
                };
                let d = r.get_ref(3)?.as_str_or_null()?.unwrap_or("").to_string();
                out.insert(n(0)?, (n(1)?, n(2)?, d, n(4)?));
            }
            Ok(out)
        })?;
        Ok(found
            .into_iter()
            .map(|f| {
                let (hour, weekday, day, day_no) =
                    local.get(&(f.key.1 / 900)).cloned().unwrap_or_default();
                let msg = Msg {
                    time: f.key.1,
                    local_id: f.key.2,
                    kind: f.kind,
                    mine: f.mine,
                    hour: usize::try_from(hour.clamp(0, 23)).unwrap_or(0),
                    weekday_mon: usize::try_from((weekday + 6).rem_euclid(7)).unwrap_or(0),
                    day,
                    day_no,
                    content: f.text.unwrap_or_default(),
                };
                (f.key, msg)
            })
            .collect())
    }

    /// `session_ids` 在 `[begin, end]` 之内的年度报告扩展统计：`heatmap`（星期 × 小时，周一在前）、
    /// `midnight`（每个会话 00:00 到 05:59 的消息数）、`conversation`（每个会话的 `initiated` 和 `received`）、
    /// `response`（账号所有者的回复，只算至少有 10 次回复的会话）、
    /// `peakDay`（每个会话在 `[peak_begin, peak_end]` 之内的消息数）、`topPhrases`（账号所有者的常用语，最多 32 条），
    /// 以及 `streak`（一个会话中连续有消息的最长天数；天数相同时取排在前面的会话）。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn annual_report_extras(
        &self,
        session_ids: &[String],
        begin: i64,
        end: i64,
        peak_begin: i64,
        peak_end: i64,
    ) -> Result<Value> {
        /// 会话中账号所有者的回复至少要有这么多次，才计入回复的统计。
        const MIN_REPLIES: usize = 10;
        const PHRASES: usize = 32;
        let mut heatmap = vec![vec![0i64; 24]; 7];
        let (mut midnight, mut conversation, mut response, mut peak) =
            (Map::new(), Map::new(), Map::new(), Map::new());
        let mut my_texts: Vec<String> = Vec::new();
        let mut streak: Option<(String, i64, String, String)> = None;
        let tables = self.message_tables_of(session_ids)?;
        let mine = self.my_ids_of(tables.iter().flatten())?;
        // 每个会话在各自的线程上读取并汇总：内存里只留下汇总的结果
        let sessions: Vec<(&String, &Vec<MsgTable>)> = session_ids.iter().zip(&tables).collect();
        let summed = crate::stats::each_table(&sessions, |(_, ts)| {
            let mut placed: Vec<Placed> = Vec::new();
            for t in *ts {
                placed.extend(self.extras_messages(t, &mine[&t.db], begin, end)?);
            }
            placed.sort_by_key(|a| a.0);
            Ok(SessionExtras::of(
                placed.into_iter().map(|(_, m)| m),
                peak_begin,
                peak_end,
            ))
        })?;
        for ((sid, _), e) in sessions.iter().zip(summed) {
            let Some(e) = e else { continue };
            for (row, sums) in heatmap.iter_mut().zip(&e.heatmap) {
                for (cell, n) in row.iter_mut().zip(sums) {
                    *cell += n;
                }
            }
            my_texts.extend(e.phrases);
            if e.night > 0 {
                midnight.insert((*sid).clone(), json!(e.night));
            }
            conversation.insert(
                (*sid).clone(),
                json!({ "initiated": e.initiated, "received": e.received }),
            );
            if e.replies >= MIN_REPLIES {
                #[expect(clippy::cast_precision_loss, reason = "秒数和回复次数，远小于 2^53")]
                let avg = e.reply_seconds as f64 / e.replies as f64;
                response.insert((*sid).clone(), json!({ "count": e.replies, "avg": avg }));
            }
            if e.in_peak > 0 {
                peak.insert((*sid).clone(), json!(e.in_peak));
            }
            if e.best > streak.as_ref().map_or(0, |s| s.1) {
                streak = Some(((*sid).clone(), e.best, e.best_start, e.best_end));
            }
        }
        let refs: Vec<&str> = my_texts.iter().map(String::as_str).collect();
        let mut out = json!({
            "heatmap": heatmap, "midnight": midnight, "conversation": conversation, "response": response,
            "peakDay": peak, "topPhrases": phrase_list(&refs, PHRASES),
        });
        if let Some((sid, days, start, end)) = streak {
            out["streak"] =
                json!({ "sessionId": sid, "days": days, "startDate": start, "endDate": end });
        }
        Ok(out)
    }

    /// 与 `friend` 的双人报告统计（定义见模块的说明）。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn dual_report_stats(&self, friend: &str, begin: i64, end: i64) -> Result<Value> {
        let tables = self.message_tables(friend)?;
        let msgs = self.conversation(&tables, &self.my_ids_of(&tables)?, begin, end)?;
        let (mut words, mut images, mut voices, mut emojis) = (0i64, 0i64, 0i64, 0i64);
        let mut heatmap = vec![vec![0i64; 24]; 7];
        let mut monthly: BTreeMap<String, i64> = BTreeMap::new();
        let (mut initiated, mut received) = (0i64, 0i64);
        let mut responses: Vec<i64> = Vec::new();
        let mut last: Option<(i64, bool)> = None;
        let (mut mine_texts, mut their_texts): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
        // 按本地日期最长的连续天数
        let (mut last_day, mut run, mut run_start) = (None::<i64>, 0i64, String::new());
        let (mut best, mut best_start, mut best_end) = (0i64, String::new(), String::new());

        for m in &msgs {
            match m.kind {
                IMAGE => images += 1,
                VOICE => voices += 1,
                EMOJI => emojis += 1,
                TEXT | QUOTE => {
                    let t = m.content.trim();
                    if m.kind == TEXT {
                        words += i64::try_from(t.chars().count()).unwrap_or(i64::MAX);
                    }
                    if is_phrase(t) {
                        if m.mine {
                            mine_texts.push(t);
                        } else {
                            their_texts.push(t);
                        }
                    }
                }
                _ => {}
            }
            heatmap[m.weekday_mon][m.hour] += 1;
            if m.day.len() >= 7 {
                *monthly.entry(m.day[..7].to_string()).or_default() += 1;
            }
            match last {
                Some((t, was_mine)) if m.time - t <= GAP => {
                    if !was_mine && m.mine {
                        let rt = m.time - t;
                        if rt > 0 && rt < MAX_RESPONSE {
                            responses.push(rt);
                        }
                    }
                }
                _ => {
                    if m.mine {
                        initiated += 1;
                    } else {
                        received += 1;
                    }
                }
            }
            last = Some((m.time, m.mine));
            if last_day != Some(m.day_no) {
                if last_day.is_some_and(|l| m.day_no - l == 1) {
                    run += 1;
                } else {
                    run = 1;
                    run_start = m.day.clone();
                }
                if run > best {
                    (best, best_start, best_end) = (run, run_start.clone(), m.day.clone());
                }
                last_day = Some(m.day_no);
            }
        }

        let all_texts: Vec<&str> = mine_texts
            .iter()
            .chain(their_texts.iter())
            .copied()
            .collect();
        let mut out = json!({
            "counts": { "total": msgs.len(), "words": words, "image": images, "voice": voices, "emoji": emojis },
            // 留空，由业务层自己统计表情
            "emojis": [],
            "phrases": phrase_list(&all_texts, PHRASES_TOP),
            "myPhrases": phrase_list(&mine_texts, PHRASES_SIDE),
            "friendPhrases": phrase_list(&their_texts, PHRASES_SIDE),
            "heatmap": heatmap,
            "initiative": { "initiated": initiated, "received": received },
            "monthly": monthly,
        });
        if !responses.is_empty() {
            let sum: i64 = responses.iter().sum();
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_precision_loss,
                reason = "`i64` 值的平均数取整，秒数之和远小于 2^53；浮点数转整数的 `as` 会饱和"
            )]
            let avg = (sum as f64 / responses.len() as f64).round() as i64;
            out["response"] = json!({
                "avg": avg,
                "fastest": responses.iter().min(), "slowest": responses.iter().max(), "count": responses.len()
            });
        }
        if best > 0 {
            out["streak"] = json!({ "days": best, "startDate": best_start, "endDate": best_end });
        }
        Ok(out)
    }

    /// 消息的 `source` XML 中 `<atuserlist>` 是否列出了账号所有者（或所有人）。
    fn mentions_me(&self, source: &str) -> bool {
        let lower = source.to_ascii_lowercase();
        let Some(start) = lower.find("<atuserlist") else {
            return false;
        };
        let Some(open_end) = lower[start..].find('>') else {
            return false;
        };
        let body_start = start + open_end + 1;
        let Some(len) = lower[body_start..].find("</atuserlist>") else {
            return false;
        };
        let body = &source[body_start..body_start + len];
        let body = body
            .trim()
            .trim_start_matches("<![CDATA[")
            .trim_end_matches("]]>");
        body.split(',')
            .map(|t| t.trim().trim_start_matches('@'))
            .any(|t| t.eq_ignore_ascii_case("notify@all") || self.is_me(t))
    }

    /// 足迹的统计：`{ summary, private_sessions, private_segments, mentions, mention_groups, diagnostics }`。
    ///
    /// `options` 中可以给出 `begin`、`end`（秒）、`mention_limit`、`private_limit`（0 表示不限）、
    /// `private_session_ids` 和 `group_session_ids`，前四个也接受 `beginTimestamp` 这样的驼峰写法；
    /// 两个会话列表都没给时，取会话列表中的全部群聊和私聊，私聊不含公众号（`gh_` 开头）
    /// 和微信的系统会话（`NOT_PEOPLE`）。
    ///
    /// # Errors
    ///
    /// 读取会话列表或某个消息库失败（打开、解密或查询失败）。
    pub fn footprint_stats(&self, options: &Value) -> Result<Value> {
        let started = Instant::now();
        let pick = |keys: &[&str]| keys.iter().find_map(|k| options.get(*k));
        let num = |keys: &[&str]| pick(keys).and_then(Value::as_i64).unwrap_or(0).max(0);
        let (begin, end) = (
            num(&["begin", "beginTimestamp"]),
            num(&["end", "endTimestamp"]),
        );
        let (mention_limit, private_limit) = (
            usize::try_from(num(&["mention_limit", "mentionLimit"])).unwrap_or(usize::MAX),
            usize::try_from(num(&["private_limit", "privateLimit"])).unwrap_or(usize::MAX),
        );
        let list = |key: &str| -> Vec<String> {
            options
                .get(key)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };
        let (mut private_ids, mut group_ids) =
            (list("private_session_ids"), list("group_session_ids"));
        if private_ids.is_empty() && group_ids.is_empty() {
            for u in crate::msg::session_usernames(self)? {
                if u.ends_with("@chatroom") {
                    group_ids.push(u);
                } else if !u.starts_with("gh_") && !NOT_PEOPLE.contains(&u.as_str()) {
                    private_ids.push(u);
                }
            }
        }

        // 私聊
        let mut sessions: Vec<Value> = Vec::new();
        let mut segments: Vec<Value> = Vec::new();
        let private_tables = self.message_tables_of(&private_ids)?;
        let mine = self.my_ids_of(private_tables.iter().flatten())?;
        for (sid, tables) in private_ids.iter().zip(&private_tables) {
            let msgs = self.conversation(tables, &mine, begin, end)?;
            if msgs.is_empty() {
                continue;
            }
            let (mut inc, mut out, mut first_in, mut first_reply) = (0i64, 0i64, 0i64, 0i64);
            let (mut latest, mut anchor) = (0i64, 0i64);
            let mut seg: Option<Segment> = None;
            let mut seg_no = 0i64;
            let mut last_ts = 0i64;
            for m in &msgs {
                if last_ts > 0 && m.time - last_ts > GAP {
                    if let Some(s) = seg.take() {
                        segments.push(s.finish(sid));
                    }
                }
                let s = seg.get_or_insert_with(|| {
                    seg_no += 1;
                    Segment::new(seg_no, m.time, m.local_id)
                });
                if m.mine {
                    out += 1;
                    if first_in > 0 && m.time >= first_in && first_reply <= 0 {
                        first_reply = m.time;
                    }
                    s.out += 1;
                    if s.first_in > 0 && m.time >= s.first_in && s.first_reply <= 0 {
                        s.first_reply = m.time;
                    }
                } else {
                    inc += 1;
                    if first_in <= 0 || m.time < first_in {
                        first_in = m.time;
                    }
                    s.inc += 1;
                    if s.first_in <= 0 || m.time < s.first_in {
                        s.first_in = m.time;
                    }
                }
                if latest <= 0 || m.time > latest || (m.time == latest && m.local_id > anchor) {
                    (latest, anchor) = (m.time, m.local_id);
                }
                s.end = m.time;
                last_ts = m.time;
            }
            if let Some(s) = seg.take() {
                segments.push(s.finish(sid));
            }
            sessions.push(json!({
                "session_id": sid, "incoming_count": inc, "outgoing_count": out, "replied": inc > 0 && out > 0,
                "first_incoming_ts": first_in, "first_reply_ts": first_reply, "latest_ts": latest,
                "anchor_local_id": anchor, "anchor_create_time": latest
            }));
        }
        sessions.sort_by(|a, b| {
            int(b, "latest_ts")
                .cmp(&int(a, "latest_ts"))
                .then(text(a, "session_id").cmp(&text(b, "session_id")))
        });
        segments.sort_by(|a, b| {
            int(a, "start_ts")
                .cmp(&int(b, "start_ts"))
                .then(text(a, "session_id").cmp(&text(b, "session_id")))
                .then(int(a, "segment_index").cmp(&int(b, "segment_index")))
        });
        let private_truncated = private_limit > 0 && sessions.len() > private_limit;
        if private_truncated {
            sessions.truncate(private_limit);
            let keep: HashSet<String> = sessions.iter().map(|s| text(s, "session_id")).collect();
            segments.retain(|s| keep.contains(&text(s, "session_id")));
        }

        // 群里的 @
        let range = range_sql(begin, end);
        let mut mentions: Vec<Value> = Vec::new();
        for (sid, tables) in group_ids.iter().zip(self.message_tables_of(&group_ids)?) {
            for t in tables {
                let sql = format!(
                    "select m.local_id as local_id, m.create_time as create_time, m.message_content as message_content, m.source as source, \
                     n.user_name as sender_username from \"{}\" m left join Name2Id n on n.rowid = m.real_sender_id \
                     where m.local_type = {TEXT}{range} and (typeof(m.source) = 'blob' or m.source like '%atuserlist%')",
                    t.table
                );
                // 边读边匹配：压缩的 source 只有解码后才能检查，这样的行占大多数
                let found = std::cell::RefCell::new(Vec::new());
                self.query_each(&t.db, &sql, &[], || found.borrow_mut().clear(), |r| {
                    let content = text(&r, "message_content");
                    if (content.contains('@') || content.contains('\u{ff20}')) && self.mentions_me(&text(&r, "source")) {
                        found.borrow_mut().push(json!({
                            "session_id": sid, "local_id": int(&r, "local_id"), "create_time": int(&r, "create_time"),
                            "sender_username": text(&r, "sender_username"), "message_content": content, "source": text(&r, "source")
                        }));
                    }
                })?;
                mentions.extend(found.into_inner());
            }
        }
        mentions.sort_by(|a, b| {
            int(b, "create_time")
                .cmp(&int(a, "create_time"))
                .then(int(b, "local_id").cmp(&int(a, "local_id")))
        });
        let mention_truncated = mention_limit > 0 && mentions.len() > mention_limit;
        if mention_truncated {
            mentions.truncate(mention_limit);
        }
        let mut groups: HashMap<String, (i64, i64)> = HashMap::new();
        for m in &mentions {
            let g = groups.entry(text(m, "session_id")).or_default();
            g.0 += 1;
            g.1 = g.1.max(int(m, "create_time"));
        }
        let mut mention_groups: Vec<(String, i64, i64)> =
            groups.into_iter().map(|(s, (c, l))| (s, c, l)).collect();
        mention_groups.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));

        let inbound = sessions
            .iter()
            .filter(|s| int(s, "incoming_count") > 0)
            .count();
        let replied = sessions.iter().filter(|s| s["replied"] == true).count();
        let outbound = sessions
            .iter()
            .filter(|s| int(s, "outgoing_count") > 0)
            .count();
        #[expect(clippy::cast_precision_loss, reason = "会话的个数，远小于 2^53")]
        let reply_rate = if inbound > 0 {
            replied as f64 / inbound as f64
        } else {
            0.0
        };
        Ok(json!({
            "summary": {
                "private_inbound_people": inbound, "private_replied_people": replied, "private_outbound_people": outbound,
                "private_reply_rate": reply_rate,
                "mention_count": mentions.len(), "mention_group_count": mention_groups.len()
            },
            "private_sessions": sessions, "private_segments": segments, "mentions": mentions,
            "mention_groups": mention_groups.into_iter().map(|(s, c, l)| json!({ "session_id": s, "count": c, "latest_ts": l })).collect::<Vec<_>>(),
            "diagnostics": {
                "truncated": private_truncated || mention_truncated, "private_truncated": private_truncated, "mention_truncated": mention_truncated,
                "scanned_dbs": self.message_dbs().len(), "elapsed_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "private_session_count": private_ids.len(), "group_session_count": group_ids.len()
            }
        }))
    }
}

/// 正在构造的私聊的一段。
struct Segment {
    index: i64,
    start: i64,
    end: i64,
    inc: i64,
    out: i64,
    first_in: i64,
    first_reply: i64,
    anchor_local_id: i64,
}

impl Segment {
    fn new(index: i64, start: i64, anchor_local_id: i64) -> Self {
        Self {
            index,
            start,
            end: start,
            inc: 0,
            out: 0,
            first_in: 0,
            first_reply: 0,
            anchor_local_id,
        }
    }

    fn finish(self, session_id: &str) -> Value {
        let mut o = Map::new();
        o.insert("session_id".into(), json!(session_id));
        o.insert("segment_index".into(), json!(self.index));
        o.insert("start_ts".into(), json!(self.start));
        o.insert("end_ts".into(), json!(self.end));
        o.insert("duration_sec".into(), json!((self.end - self.start).max(0)));
        o.insert("incoming_count".into(), json!(self.inc));
        o.insert("outgoing_count".into(), json!(self.out));
        o.insert("message_count".into(), json!(self.inc + self.out));
        o.insert("replied".into(), json!(self.inc > 0 && self.out > 0));
        o.insert("first_incoming_ts".into(), json!(self.first_in));
        o.insert("first_reply_ts".into(), json!(self.first_reply));
        o.insert("latest_ts".into(), json!(self.end));
        o.insert("anchor_local_id".into(), json!(self.anchor_local_id));
        o.insert("anchor_create_time".into(), json!(self.start));
        Value::Object(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Fixture, MsgSpec, SessionSpec, TestDir, DAY, T0};

    fn world(
        tag: &str,
        sessions: &[SessionSpec<'_>],
        shard: &[(&str, Vec<MsgSpec>)],
    ) -> (TestDir, Account) {
        let root = TestDir::new(&format!("weflow-report-{tag}"));
        let f = Fixture::new(&root, "wxid_me_ab12");
        f.session_db(sessions);
        f.message_shard(0, shard);
        let acct = Account::new(f.db_storage(), &f.key_hex())
            .unwrap()
            .with_my_wxid(Some("wxid_me_ab12".into()));
        (root, acct)
    }

    fn session(username: &'static str) -> SessionSpec<'static> {
        SessionSpec {
            username,
            summary: "",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 1,
        }
    }

    fn dual() -> (TestDir, Account) {
        let m = |id, who: &'static str, dt: i64, text: &str| MsgSpec::text(id, who, T0 + dt, text);
        world(
            "dual",
            &[session("wxid_pal")],
            &[(
                "wxid_pal",
                vec![
                    m(1, "wxid_pal", 0, "ok"),
                    m(2, "wxid_me", 10, "ok"),
                    m(3, "wxid_pal", 20, "haha"),
                    m(4, "wxid_me", 30, "haha"),
                    m(5, "wxid_me", 7_200, "ok"),
                    m(6, "wxid_pal", 7_210, "ok"),
                    m(7, "wxid_me", DAY, "ok"),
                    m(8, "wxid_pal", DAY + 100, "<img/>").of_type(3),
                    m(9, "wxid_pal", 2 * DAY, "").of_type(34),
                    m(10, "wxid_me", 2 * DAY + 5, "").of_type(47),
                ],
            )],
        )
    }

    #[test]
    fn dual_report_counts_words_and_kinds() {
        let v = dual().1.dual_report_stats("wxid_pal", 0, 0).unwrap();
        assert_eq!(
            v["counts"],
            json!({"total": 10, "words": 18, "image": 1, "voice": 1, "emoji": 1})
        );
        assert_eq!(
            v["emojis"],
            json!([]),
            "the service layer tallies stickers itself"
        );
        let heat = v["heatmap"].as_array().unwrap();
        assert_eq!((heat.len(), heat[0].as_array().unwrap().len()), (7, 24));
        let total: i64 = heat
            .iter()
            .flat_map(|r| r.as_array().unwrap())
            .map(|n| n.as_i64().unwrap())
            .sum();
        assert_eq!(total, 10);
        let monthly: i64 = v["monthly"]
            .as_object()
            .unwrap()
            .values()
            .map(|n| n.as_i64().unwrap())
            .sum();
        assert_eq!(monthly, 10);
        assert!(v["monthly"]
            .as_object()
            .unwrap()
            .keys()
            .all(|k| k.len() == 7));
    }

    #[test]
    fn dual_report_initiative_and_response_follow_the_one_hour_rule() {
        let v = dual().1.dual_report_stats("wxid_pal", 0, 0).unwrap();
        // 各轮聊天分别从第 1 条（pal）、第 5 条（我）、第 7 条（我）、第 9 条（pal）开始
        assert_eq!(v["initiative"], json!({"initiated": 2, "received": 2}));
        // 同一轮聊天中我对 pal 的回复：10 秒、10 秒和 5 秒
        assert_eq!(
            v["response"],
            json!({"avg": 8, "fastest": 5, "slowest": 10, "count": 3})
        );
    }

    #[test]
    fn dual_report_phrases_and_streak() {
        let v = dual().1.dual_report_stats("wxid_pal", 0, 0).unwrap();
        assert_eq!(
            v["phrases"],
            json!([{"phrase": "ok", "count": 5}, {"phrase": "haha", "count": 2}])
        );
        assert_eq!(
            v["myPhrases"],
            json!([{"phrase": "ok", "count": 3}]),
            "haha was typed once: below the 2-occurrence floor"
        );
        assert_eq!(v["friendPhrases"], json!([{"phrase": "ok", "count": 2}]));
        assert_eq!(v["streak"]["days"], 3);
        let (s, e) = (
            v["streak"]["startDate"].as_str().unwrap(),
            v["streak"]["endDate"].as_str().unwrap(),
        );
        assert!(s.len() == 10 && e.len() == 10 && s < e, "{s} .. {e}");
    }

    #[test]
    fn dual_report_respects_the_range_and_handles_empty_conversations() {
        let (_dir, a) = dual();
        let v = a.dual_report_stats("wxid_pal", T0 + DAY, 0).unwrap();
        assert_eq!(v["counts"]["total"], 4);
        let none = a.dual_report_stats("nobody", 0, 0).unwrap();
        assert_eq!(none["counts"]["total"], 0);
        assert!(none.get("response").is_none() && none.get("streak").is_none());
        assert_eq!(none["initiative"], json!({"initiated": 0, "received": 0}));
    }

    #[test]
    fn phrase_rule_filters_links_markup_and_length() {
        assert!(is_phrase("ok") && is_phrase("好的呀"));
        assert!(!is_phrase("a") && !is_phrase(&"x".repeat(21)));
        assert!(!is_phrase("see http://x") && !is_phrase("<b>hi</b>") && !is_phrase("[Smile]"));
        assert_eq!(
            phrase_list(&["b", "a", "b", "a", "c"], 10),
            json!([{"phrase": "b", "count": 2}, {"phrase": "a", "count": 2}]),
            "ties keep first-seen order"
        );
        assert_eq!(
            phrase_list(&["a", "a", "a", "b", "b"], 1),
            json!([{"phrase": "a", "count": 3}])
        );
    }

    fn footprint() -> (TestDir, Account) {
        let at = |who: &str| format!("<msgsource><atuserlist>{who}</atuserlist></msgsource>");
        world(
            "footprint",
            &[
                "wxid_pal",
                "wxid_lone",
                "wxid_quiet",
                "gh_x",
                "medianote",
                "g1@chatroom",
                "g2@chatroom",
            ]
            .map(session),
            &[
                (
                    "wxid_pal",
                    vec![
                        MsgSpec::text(1, "wxid_pal", T0, "a"),
                        MsgSpec::text(2, "wxid_me", T0 + 60, "b"),
                        MsgSpec::text(3, "wxid_pal", T0 + DAY, "c"),
                        MsgSpec::text(4, "wxid_me", T0 + DAY + 30, "d"),
                        MsgSpec::text(5, "wxid_pal", T0 + 3 * DAY, "e"),
                    ],
                ),
                (
                    "wxid_lone",
                    vec![
                        MsgSpec::text(1, "wxid_lone", T0 + 5, "x"),
                        MsgSpec::text(2, "wxid_lone", T0 + 6, "y"),
                    ],
                ),
                (
                    "wxid_quiet",
                    vec![MsgSpec::text(1, "wxid_me", T0 + 70, "hello?")],
                ),
                (
                    "g1@chatroom",
                    vec![
                        MsgSpec::text(1, "wxid_bob", T0 + 10, "wxid_bob:\n@Me hello")
                            .with_source(&at("wxid_me")),
                        MsgSpec::text(2, "wxid_bob", T0 + 20, "wxid_bob:\n@Carol hi")
                            .with_source(&at("wxid_carol")),
                        MsgSpec::text(3, "wxid_quiet", T0 + 100, "wxid_quiet:\n@all meeting")
                            .with_source(&at("notify@all")),
                        MsgSpec::text(4, "wxid_bob", T0 + 150, "no at sign here")
                            .with_source(&at("wxid_me")),
                        MsgSpec::text(5, "wxid_bob", T0 + 200, "wxid_bob:\n@Me again")
                            .with_source(&at("wxid_me_ab12,wxid_carol")),
                    ],
                ),
                (
                    "g2@chatroom",
                    vec![MsgSpec::text(1, "wxid_bob", T0 + 50, "@Me there")
                        .with_source(&at("wxid_me"))],
                ),
            ],
        )
    }

    #[test]
    fn footprint_private_sessions_summary_and_segments() {
        let v = footprint()
            .1
            .footprint_stats(&json!({"myWxid": "wxid_me_ab12"}))
            .unwrap();
        let ids: Vec<&str> = v["private_sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["session_id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            ["wxid_pal", "wxid_quiet", "wxid_lone"],
            "people only, newest activity first; gh_/medianote are not people"
        );
        let pal = &v["private_sessions"][0];
        assert_eq!(
            (
                pal["incoming_count"].as_i64(),
                pal["outgoing_count"].as_i64(),
                pal["replied"].as_bool()
            ),
            (Some(3), Some(2), Some(true))
        );
        assert_eq!(
            (
                pal["first_incoming_ts"].as_i64(),
                pal["first_reply_ts"].as_i64(),
                pal["latest_ts"].as_i64()
            ),
            (Some(T0), Some(T0 + 60), Some(T0 + 3 * DAY))
        );
        assert_eq!(pal["anchor_local_id"], 5);
        assert_eq!(v["private_sessions"][2]["replied"], false);
        assert_eq!(v["summary"]["private_inbound_people"], 2);
        assert_eq!(v["summary"]["private_replied_people"], 1);
        assert_eq!(v["summary"]["private_outbound_people"], 2);
        assert_eq!(v["summary"]["private_reply_rate"], 0.5);

        let segs: Vec<&Value> = v["private_segments"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["session_id"] == "wxid_pal")
            .collect();
        assert_eq!(segs.len(), 3, "gaps of more than an hour split the chat");
        assert_eq!(
            (
                segs[0]["message_count"].as_i64(),
                segs[0]["duration_sec"].as_i64(),
                segs[0]["replied"].as_bool()
            ),
            (Some(2), Some(60), Some(true))
        );
        assert_eq!(
            (
                segs[2]["message_count"].as_i64(),
                segs[2]["replied"].as_bool(),
                segs[2]["segment_index"].as_i64()
            ),
            (Some(1), Some(false), Some(3))
        );
        assert_eq!(segs[1]["anchor_local_id"], 3);
    }

    #[test]
    fn footprint_mentions_use_the_atuserlist_and_count_at_all() {
        let v = footprint()
            .1
            .footprint_stats(&json!({"myWxid": "wxid_me_ab12"}))
            .unwrap();
        let got: Vec<(String, i64)> = v["mentions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m["session_id"].as_str().unwrap().to_string(),
                    m["local_id"].as_i64().unwrap(),
                )
            })
            .collect();
        // 最新的在前；@Carol 不是我；`no at sign here` 的文字里没有 @
        assert_eq!(
            got,
            [
                ("g1@chatroom".into(), 5),
                ("g1@chatroom".into(), 3),
                ("g2@chatroom".into(), 1),
                ("g1@chatroom".into(), 1)
            ]
        );
        assert_eq!(v["mentions"][0]["sender_username"], "wxid_bob");
        assert_eq!(
            (
                v["summary"]["mention_count"].as_i64(),
                v["summary"]["mention_group_count"].as_i64()
            ),
            (Some(4), Some(2))
        );
        assert_eq!(
            v["mention_groups"],
            json!([{"session_id": "g1@chatroom", "count": 3, "latest_ts": T0 + 200}, {"session_id": "g2@chatroom", "count": 1, "latest_ts": T0 + 50}])
        );
    }

    #[test]
    fn footprint_options_limit_scope_and_truncate() {
        let (_dir, a) = footprint();
        let v = a
            .footprint_stats(
                &json!({"begin": T0 + 60, "end": T0 + 150, "mention_limit": 1, "private_limit": 1}),
            )
            .unwrap();
        // 在 [T0+60, T0+150] 之内只有 g1#3（T0+100 的 @all）是 @；g2#1（T0+50）在范围之前
        assert_eq!(v["mentions"].as_array().unwrap().len(), 1);
        assert_eq!(
            v["diagnostics"]["mention_truncated"], false,
            "the limit was not exceeded"
        );
        // pal（我在 T0+60 的回复）和 quiet（T0+70）在范围内都有消息，所以上限为 1 时会截断
        assert_eq!(v["private_sessions"].as_array().unwrap().len(), 1);
        assert_eq!(v["diagnostics"]["private_truncated"], true);
        assert_eq!(v["diagnostics"]["truncated"], true);
        let scoped = a
            .footprint_stats(
                &json!({"private_session_ids": ["wxid_lone"], "group_session_ids": []}),
            )
            .unwrap();
        assert_eq!(scoped["private_sessions"].as_array().unwrap().len(), 1);
        assert_eq!(
            scoped["mentions"],
            json!([]),
            "explicit scope: no groups were asked for"
        );
        let cut = a.footprint_stats(&json!({"private_limit": 1})).unwrap();
        assert_eq!(cut["private_sessions"].as_array().unwrap().len(), 1);
        assert_eq!(cut["diagnostics"]["private_truncated"], true);
        assert!(cut["private_segments"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["session_id"] == "wxid_pal"));
    }

    #[test]
    fn mention_parsing_edge_cases() {
        let (_dir, a) = account_for_mentions();
        assert!(a.mentions_me(
            "<msgsource><atuserlist><![CDATA[wxid_me, wxid_x]]></atuserlist></msgsource>"
        ));
        assert!(a.mentions_me("<ATUSERLIST>@wxid_me</ATUSERLIST>"));
        assert!(a.mentions_me("<atuserlist>notify@all</atuserlist>"));
        assert!(!a.mentions_me("<atuserlist>wxid_other</atuserlist>"));
        assert!(!a.mentions_me("<msgsource><other/></msgsource>"));
        assert!(!a.mentions_me(""));
        assert!(!a.mentions_me("<atuserlist>wxid_me"), "unterminated list");
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn annual_extras_follow_the_service_layer_definitions() {
        let (_dir, a) = dual();
        let v = a
            .annual_report_extras(&ids(&["wxid_pal"]), 0, 0, 0, 0)
            .unwrap();
        let heat: i64 = v["heatmap"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| r.as_array().unwrap())
            .map(|n| n.as_i64().unwrap())
            .sum();
        assert_eq!((v["heatmap"].as_array().unwrap().len(), heat), (7, 10));
        assert_eq!(
            v["conversation"],
            json!({"wxid_pal": {"initiated": 2, "received": 2}})
        );
        assert_eq!(
            v["response"],
            json!({}),
            "3 replies is below the 10-reply floor"
        );
        assert_eq!(
            v["topPhrases"],
            json!([{"phrase": "ok", "count": 3}]),
            "only my phrases, seen at least twice"
        );
        assert_eq!(
            (
                v["streak"]["sessionId"].clone(),
                v["streak"]["days"].clone()
            ),
            (json!("wxid_pal"), json!(3))
        );
        assert!(
            v["streak"]["startDate"].as_str().unwrap() < v["streak"]["endDate"].as_str().unwrap()
        );
        assert_eq!(v["peakDay"], json!({}), "no peak window asked for");
        assert!(v["midnight"]
            .as_object()
            .unwrap()
            .values()
            .all(|n| n.as_i64().unwrap() <= 10));

        let peak = a
            .annual_report_extras(&ids(&["wxid_pal"]), 0, 0, T0, T0 + 100)
            .unwrap();
        assert_eq!(peak["peakDay"], json!({"wxid_pal": 4}));
        let ranged = a
            .annual_report_extras(&ids(&["wxid_pal"]), T0 + DAY, 0, 0, 0)
            .unwrap();
        assert_eq!(
            ranged["conversation"],
            json!({"wxid_pal": {"initiated": 1, "received": 1}})
        );
        let none = a
            .annual_report_extras(&ids(&["nobody"]), 0, 0, 0, 0)
            .unwrap();
        assert!(
            none.get("streak").is_none()
                && none["conversation"] == json!({})
                && none["topPhrases"] == json!([])
        );
    }

    #[test]
    fn what_quotes_say_counts_as_phrases() {
        let quote = |id, who: &'static str, dt: i64, said: &str, quoted: &str| {
            let xml = format!(
                "<msg><appmsg><title>{said}</title><type>57</type><refermsg><type>1</type>\
                 <content>{quoted}</content></refermsg></appmsg></msg>"
            );
            MsgSpec::text(id, who, T0 + dt, &xml).of_type(QUOTE)
        };
        let long = "长".repeat(LONGEST_TEXT);
        let (_dir, a) = world(
            "quotes",
            &[session("wxid_pal")],
            &[(
                "wxid_pal",
                vec![
                    MsgSpec::text(1, "wxid_pal", T0, "吃了吗"),
                    // 依次是扫描能读的、压缩的和长于扫描解密范围的：后两条改用 SQL 读取
                    quote(2, "wxid_me", 10, "吃过了", "吃了吗"),
                    quote(3, "wxid_me", 20, "吃过了", "吃了吗").compressed(),
                    quote(4, "wxid_me", 30, "吃过了", &long),
                    quote(5, "wxid_pal", 40, "好的呀", "吃过了"),
                    quote(6, "wxid_pal", 50, "好的呀", "吃过了"),
                ],
            )],
        );
        let mine = json!([{"phrase": "吃过了", "count": 3}]);
        let extras = a
            .annual_report_extras(&ids(&["wxid_pal"]), 0, 0, 0, 0)
            .unwrap();
        assert_eq!(
            extras["topPhrases"], mine,
            "my replies, not the quoted messages"
        );
        let dual = a.dual_report_stats("wxid_pal", 0, 0).unwrap();
        assert_eq!(dual["myPhrases"], mine);
        assert_eq!(
            dual["friendPhrases"],
            json!([{"phrase": "好的呀", "count": 2}])
        );
        assert_eq!(
            dual["counts"]["words"], 3,
            "only text messages count their words"
        );
    }

    #[test]
    fn annual_extras_response_needs_ten_replies_and_streak_ties_go_to_the_first_session() {
        let mut quick = Vec::new();
        for i in 0..10 {
            quick.push(MsgSpec::text(
                i * 2 + 1,
                "wxid_fast",
                T0 + i * 7_200,
                "ping",
            ));
            quick.push(MsgSpec::text(
                i * 2 + 2,
                "wxid_me",
                T0 + i * 7_200 + 30,
                "pong",
            ));
        }
        let (_dir, a) = world(
            "annual-response",
            &[session("wxid_fast"), session("wxid_slow")],
            &[
                ("wxid_fast", quick),
                (
                    "wxid_slow",
                    vec![
                        MsgSpec::text(1, "wxid_slow", T0, "hi"),
                        MsgSpec::text(2, "wxid_me", T0 + 20, "yo"),
                        MsgSpec::text(3, "wxid_slow", T0 + DAY, "again"),
                    ],
                ),
            ],
        );
        let v = a
            .annual_report_extras(&ids(&["wxid_slow", "wxid_fast"]), 0, 0, 0, 0)
            .unwrap();
        assert_eq!(
            v["response"],
            json!({"wxid_fast": {"count": 10, "avg": 30.0}})
        );
        assert_eq!(
            v["conversation"]["wxid_fast"],
            json!({"initiated": 0, "received": 10})
        );
        // wxid_slow 跨 2 天；wxid_fast 最多跨 2 天（18 小时），而 slow 排在前面，所以取 slow
        assert_eq!(
            (
                v["streak"]["sessionId"].clone(),
                v["streak"]["days"].clone()
            ),
            (json!("wxid_slow"), json!(2))
        );
        let tie = a
            .annual_report_extras(&ids(&["wxid_fast", "wxid_slow"]), 0, T0 + 2 * 3_600, 0, 0)
            .unwrap();
        assert_eq!(
            tie["streak"]["sessionId"], "wxid_fast",
            "a tie keeps the first session"
        );
    }

    #[test]
    fn annual_stats_add_month_of_year_counts_per_session() {
        let (_dir, a) = dual();
        let v = a.annual_report_stats(&ids(&["wxid_pal"]), 0, 0).unwrap();
        let plain = a.aggregate_stats(&ids(&["wxid_pal"]), 0, 0).unwrap();
        assert_eq!(v["total"], plain["total"]);
        assert!(
            plain["sessions"]["wxid_pal"].get("monthly").is_none(),
            "the plain aggregate keeps its shape"
        );
        let monthly = v["sessions"]["wxid_pal"]["monthly"].as_object().unwrap();
        assert!(
            monthly
                .keys()
                .all(|k| (1..=12).contains(&k.parse::<i64>().unwrap())),
            "{monthly:?}"
        );
        assert_eq!(
            monthly.values().map(|n| n.as_i64().unwrap()).sum::<i64>(),
            10
        );
        assert_eq!(
            (
                v["sessions"]["wxid_pal"]["sent"].clone(),
                v["sessions"]["wxid_pal"]["received"].clone()
            ),
            (json!(5), json!(5))
        );
    }

    fn account_for_mentions() -> (TestDir, Account) {
        let (dir, acct) = world("mentions", &[], &[]);
        (dir, acct.with_my_wxid(Some("wxid_me".into())))
    }
}
