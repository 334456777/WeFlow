//! 年度报告和双人报告。

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Local, NaiveDate, NaiveTime, TimeZone, Timelike};
use indexmap::IndexMap;
use serde_json::{json, Map, Value};

use super::session_kind::is_private_session;
use super::*;
use crate::contract::{
    ActivityHeatmap, AnnualReport, AnnualSnsStats, CoreFriend, DualMessage, DualReport, DualStats,
    FriendCount, LongestStreak, LostFriend, MidnightKing, MonthlyTopFriend, MutualFriend, PeakDay,
    PhraseCount, ResponseSpeed, SocialInitiative, YearFirstChat,
};
use crate::message::emoji::percent_decode;
use crate::message::transfer::is_same_wxid;
use crate::message::xml::rx;
use crate::num::{count_f64, i64_f64};
use weflow_native::msg::Order;
use weflow_native::msg_row::{Columns, MsgRow};
use weflow_native::quote::{reply_text, QUOTE};
use weflow_native::wcdb::Databases;

const CONVERSATION_GAP: i64 = 3600;

/// 从数据库层的回答中取出 `key`，读成契约类型：没有或为 null 时为 `None`；
/// 类型不认识的字段在这里就报错，而不是让这部分回答无人察觉。
fn from_native<T: serde::de::DeserializeOwned>(
    answer: &mut Value,
    key: &str,
) -> AppResult<Option<T>> {
    match answer.get_mut(key).map(Value::take) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v)
            .map(Some)
            .map_err(|e| AppError::runtime(format!("the native {key}: {e}"))),
    }
}

fn num(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_f64().map(crate::num::saturating_i64))
        .or_else(|| v.as_str().and_then(crate::api::js_parse_int))
        .unwrap_or(0)
}

fn fnum(v: &Value) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .unwrap_or(0.0)
}

fn local_ts(year: i32, month: u32, day: u32, h: u32, m: u32, s: u32) -> i64 {
    NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|d| d.and_hms_opt(h, m, s))
        .and_then(|n| Local.from_local_datetime(&n).earliest())
        .map(|d| d.timestamp())
        .unwrap_or(0)
}

/// 命令接受的报告年份：`0` 表示所有年份，否则必须是 [`crate::dates::YEARS`] 中的一年。
///
/// 1970 年以前的 Unix 时间是负数，读取层把它当作不限：这样的年份会把所有年份都报告出来。
fn checked_year(year: i32) -> AppResult<i32> {
    if year == 0 || crate::dates::YEARS.contains(&year) {
        Ok(year)
    } else {
        Err(AppError::usage(format!(
            "invalid year {year}; use 0 for all years, or 1970-2100"
        )))
    }
}

/// 本机时区中一年的第一秒和最后一秒；`0`（所有年份）时为 `(0, 0)`，即不限。
fn year_range(year: i32) -> (i64, i64) {
    year_range_in(&Local, year)
}

/// 时区 `tz` 中的 [`year_range`]，`year` 是 [`checked_year`] 检查过的年份。
///
/// 开始时间至少为 1：在 UTC 以东的时区，1970 年的开始是负的 Unix 时间，同样会被当作不限。
fn year_range_in<Tz: TimeZone>(tz: &Tz, year: i32) -> (i64, i64) {
    if year == 0 {
        return (0, 0);
    }
    let at = |month, day, hour, minute, second| {
        tz.with_ymd_and_hms(year, month, day, hour, minute, second)
            .earliest()
            .map_or(0, |t| t.timestamp())
    };
    (at(1, 1, 0, 0, 0).max(1), at(12, 31, 23, 59, 59))
}

fn ymd(d: chrono::DateTime<Local>) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())
}

fn parse_local_day(s: &str) -> Option<chrono::DateTime<Local>> {
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    Local
        .from_local_datetime(&d.and_hms_opt(0, 0, 0)?)
        .earliest()
}

fn day_index(dt: chrono::DateTime<Local>) -> i64 {
    // 本地午夜换算成 Unix 天数，与桌面版的 `Math.floor(dayDate.getTime() / 86400000)` 相同
    let midnight = Local
        .from_local_datetime(&dt.date_naive().and_time(NaiveTime::MIN))
        .earliest()
        .unwrap_or(dt);
    midnight.timestamp().div_euclid(86400)
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// `sender` 是否是账号自己：按数据库层的规则（[`is_same_wxid`]）与配置的 wxid 或它的裸 wxid 比较。
fn sent_by_me(sender: &str, raw_wxid: &str, cleaned: &str) -> bool {
    is_same_wxid(sender, raw_wxid) || is_same_wxid(sender, cleaned)
}

// ── 双人报告的表情辅助函数 ──

fn normalize_emoji_md5(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    rx(r"([a-fA-F0-9]{16,64})")
        .captures(t)
        .map(|c| c[1].to_lowercase())
}

fn normalize_emoji_url(raw: &str) -> Option<String> {
    let mut url = raw.trim().replace("&amp;", "&");
    if url.is_empty() {
        return None;
    }
    if url.contains('%') {
        url = percent_decode(&url);
    }
    (!url.is_empty()).then_some(url)
}

fn extract_emoji_url(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    if let Some(d) = normalize_emoji_url(content).filter(|d| rx(r"(?i)^https?://").is_match(d)) {
        return Some(d);
    }
    let attr = rx(r#"(?i)(?:cdnurl|thumburl)\s*=\s*['"]([^'"]+)['"]"#)
        .captures(content)
        .or_else(|| rx(r#"(?i)(?:cdnurl|thumburl)\s*=\s*([^'"\s>]+)"#).captures(content));
    if let Some(c) = attr {
        return normalize_emoji_url(&c[1]);
    }
    let tag = rx(r"(?i)<(?:cdnurl|thumburl)>([^<]+)</(?:cdnurl|thumburl)>")
        .captures(content)
        .or_else(|| rx(r"(?i)(?:cdnurl|thumburl)[^>]*>([^<]+)").captures(content));
    tag.and_then(|c| normalize_emoji_url(&c[1]))
}

fn extract_emoji_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    if let Some(d) = normalize_emoji_md5(content).filter(|d| d.len() >= 24) {
        return Some(d);
    }
    let m = rx(r#"(?i)md5\s*=\s*['"]([a-fA-F0-9]{16,64})['"]"#)
        .captures(content)
        .or_else(|| rx(r"(?i)md5\s*=\s*([a-fA-F0-9]{16,64})").captures(content))
        .or_else(|| rx(r"(?i)<md5>([a-fA-F0-9]{16,64})</md5>").captures(content));
    m.and_then(|c| normalize_emoji_md5(&c[1]))
}

fn strip_emoji_owner_prefix(content: &str) -> String {
    rx(r"^\s*[01]\s*:\s*").replace(content, "").to_string()
}

fn record_field<'a>(rec: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|k| rec.get(*k))
        .find(|v| !v.is_null())
}

fn coerce_string(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn coerce_bool(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Null => None,
        Value::Bool(b) => Some(*b),
        Value::Number(n) => Some(n.as_f64() != Some(0.0)),
        Value::String(s) => {
            let n = s.trim().to_lowercase();
            if n.is_empty() {
                None
            } else if [
                "1", "true", "yes", "me", "self", "mine", "sent", "out", "outgoing",
            ]
            .contains(&n.as_str())
            {
                Some(true)
            } else if [
                "0", "false", "no", "friend", "peer", "other", "recv", "received", "in", "incoming",
            ]
            .contains(&n.as_str())
            {
                Some(false)
            } else {
                None
            }
        }
        other => coerce_bool(Some(&Value::String(other.to_string()))),
    }
}

struct EmojiCandidate {
    is_me: Option<bool>,
    md5: Option<String>,
    url: Option<String>,
    count: i64,
}

fn parse_emoji_candidate(item: &Value) -> EmojiCandidate {
    let raw_content = coerce_string(record_field(
        item,
        &[
            "content",
            "xml",
            "message_content",
            "messageContent",
            "msg",
            "payload",
            "raw",
        ],
    ));
    let content = strip_emoji_owner_prefix(&raw_content);
    let count = record_field(item, &["count", "cnt", "times", "total", "num"])
        .map(num)
        .filter(|n| *n > 0)
        .unwrap_or(0);
    let direct_md5 = normalize_emoji_md5(&coerce_string(record_field(
        item,
        &["md5", "emojiMd5", "emoji_md5", "emd5"],
    )));
    let md5 = direct_md5.or_else(|| extract_emoji_md5(&content));
    let direct_url = normalize_emoji_url(&coerce_string(record_field(
        item,
        &[
            "cdnUrl",
            "cdnurl",
            "emojiUrl",
            "emoji_url",
            "url",
            "thumbUrl",
            "thumburl",
        ],
    )));
    let url = direct_url.or_else(|| extract_emoji_url(&content));
    // 谁发的
    let mut is_me = coerce_bool(record_field(
        item,
        &[
            "isMe", "is_me", "isSent", "is_sent", "isSend", "is_send", "fromMe", "from_me",
        ],
    ));
    if is_me.is_none() {
        let side = coerce_string(record_field(
            item,
            &["side", "sender", "from", "owner", "role", "direction"],
        ))
        .trim()
        .to_lowercase();
        if ["me", "self", "mine", "out", "outgoing", "sent"].contains(&side.as_str()) {
            is_me = Some(true);
        } else if [
            "friend", "peer", "other", "in", "incoming", "received", "recv",
        ]
        .contains(&side.as_str())
        {
            is_me = Some(false);
        }
    }
    if is_me.is_none() {
        if let Some(c) = rx(r"^\s*([01])\s*:\s*").captures(&raw_content) {
            is_me = Some(&c[1] == "1");
        }
    }
    EmojiCandidate {
        is_me,
        md5,
        url,
        count,
    }
}

/// 从表情消息的 XML 中读出的 md5 和 CDN 地址。
fn emoji_of(content: &str) -> (Option<String>, Option<String>) {
    let stripped = strip_emoji_owner_prefix(content);
    (extract_emoji_md5(&stripped), extract_emoji_url(&stripped))
}

/// app 消息的种类（`appMsgKind`）：报告与聊天窗口一样按它称呼消息的种类（#1263）。不是 app 消息时为 `None`。
fn app_msg_kind_of(row: &MsgRow) -> Option<String> {
    crate::chat_msg::map_msg_row(row.clone(), "", "")
        .app_msg_kind()
        .map(str::to_string)
}

fn format_date_time(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(d) => format!(
            "{:02}/{:02} {:02}:{:02}",
            d.month(),
            d.day(),
            d.hour(),
            d.minute()
        ),
        None => String::new(),
    }
}

impl ServiceHub {
    fn report_connect(&self) -> AppResult<(weflow_native::wcdb::Wcdb, String, String)> {
        let profile = self.profile()?;
        let raw = profile
            .wxid
            .clone()
            .filter(|w| !w.is_empty())
            .ok_or_else(|| AppError::config("wxid is not configured"))?;
        let wcdb = self.open_wcdb()?;
        let cleaned = clean_account_dir_name(&raw);
        Ok((wcdb, cleaned, raw))
    }

    fn report_private_sessions(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        cleaned: &str,
    ) -> Vec<String> {
        let rows = wcdb
            .sessions()
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        rows.iter()
            .map(|r| {
                ["username", "user_name", "userName"]
                    .iter()
                    .filter_map(|k| r.get(*k).and_then(Value::as_str))
                    .find(|s| !s.is_empty())
                    .unwrap_or("")
                    .to_string()
            })
            .filter(|u| is_private_session(u, cleaned))
            .collect()
    }

    fn add_years_from_range(years: &mut HashSet<i32>, first: i64, last: i64) {
        let current = Local::now().year();
        let min_ts = if first > 0 { first } else { last };
        let max_ts = if last > 0 { last } else { first };
        if min_ts <= 0 || max_ts <= 0 {
            return;
        }
        let year_of = |ts: i64| {
            Local
                .timestamp_opt(ts, 0)
                .single()
                .map(|d| d.year())
                .unwrap_or(0)
        };
        for y in year_of(min_ts)..=year_of(max_ts) {
            if (2010..=current).contains(&y) {
                years.insert(y);
            }
        }
    }

    fn normalize_years(years: impl IntoIterator<Item = i64>) -> Vec<i64> {
        let mut v: Vec<i64> = years
            .into_iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        v.sort_by(|a, b| b.cmp(a));
        v
    }

    /// 可以生成年度报告的年份：先用数据库层的查询，再看各表的时间范围，最后看最早和最晚的消息。
    ///
    /// # Errors
    ///
    /// 账号没有配置好（档案、wxid、数据目录或密钥）时返回配置错误，数据库打不开时返回数据库层的错误，
    /// 没有私聊会话时返回运行时错误。
    pub fn report_available_years(&self) -> AppResult<crate::contract::ReportYears> {
        let (wcdb, cleaned, _) = self.report_connect()?;
        let sessions = self.report_private_sessions(&wcdb, &cleaned);
        if sessions.is_empty() {
            return Err(AppError::runtime("no message sessions found"));
        }
        if let Ok(Value::Array(native)) = wcdb.available_years(&sessions) {
            if !native.is_empty() {
                return Ok(crate::contract::ReportYears {
                    years: Self::normalize_years(native.iter().map(num)),
                    strategy: "native".into(),
                });
            }
        }
        // 按表扫描
        let mut years: HashSet<i32> = HashSet::new();
        for sid in &sessions {
            let Ok(Value::Array(tables)) = wcdb.message_table_stats(sid) else {
                continue;
            };
            for t in tables {
                let name = ["table_name", "name"]
                    .iter()
                    .find_map(|k| t.get(*k).and_then(Value::as_str))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let db = ["db_path", "dbPath"]
                    .iter()
                    .find_map(|k| t.get(*k).and_then(Value::as_str))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if name.is_empty() || db.is_empty() {
                    continue;
                }
                let Ok(range) = wcdb.message_table_time_range(&db, &name) else {
                    continue;
                };
                let ts = |keys: &[&str]| {
                    let n = keys
                        .iter()
                        .filter_map(|k| range.get(*k))
                        .map(fnum)
                        .find(|n| *n != 0.0)
                        .unwrap_or(0.0);
                    if n <= 0.0 || !n.is_finite() {
                        0
                    } else if n > 1e12 {
                        crate::num::saturating_i64((n / 1000.0).floor())
                    } else {
                        crate::num::saturating_i64(n.floor())
                    }
                };
                Self::add_years_from_range(
                    &mut years,
                    ts(&["first_ts", "firstTs", "min_ts", "minTs"]),
                    ts(&["last_ts", "lastTs", "max_ts", "maxTs"]),
                );
            }
        }
        let mut strategy = "table-scan";
        if years.is_empty() {
            strategy = "edge-scan";
            for sid in &sessions {
                let edge = |order: Order| -> i64 {
                    let Ok(cursor) = wcdb.open_message_cursor(sid, 1, order, 0, 0, Columns::Lite)
                    else {
                        return 0;
                    };
                    let r = wcdb.fetch_message_batch(cursor);
                    let _ = wcdb.close_message_cursor(cursor);
                    r.ok()
                        .and_then(|(rows, _)| rows.first().and_then(|r| r.create_time))
                        .unwrap_or(0)
                };
                Self::add_years_from_range(
                    &mut years,
                    edge(Order::OldestFirst),
                    edge(Order::NewestFirst),
                );
            }
        }
        Ok(crate::contract::ReportYears {
            years: Self::normalize_years(years.into_iter().map(i64::from)),
            strategy: strategy.into(),
        })
    }

    /// 年度报告（桌面版的 `annualReport:generateReport`）。
    ///
    /// # Errors
    /// `year` 既不是 `0`（所有年份）也不是 1970 到 2100 之间的年份时返回 `invalid_arguments`；
    /// 账号没有配置好时返回配置错误，读取数据库或统计失败时返回数据库层的错误；
    /// 没有私聊会话，或数据库层的回答不符合契约类型时返回运行时错误。
    pub fn report_annual(&self, year: i32) -> AppResult<AnnualReport> {
        let year = checked_year(year)?;
        self.emit_progress("annual-report", "connecting…", 5, 100);
        let (wcdb, cleaned, raw_wxid) = self.report_connect()?;
        // 报告依次读会话、消息、朋友圈和联系人
        wcdb.prefetch(&[
            Databases::Sessions,
            Databases::Messages,
            Databases::Moments,
            Databases::Contacts,
        ])
        .map_err(|e| AppError::native(format!("{e:#}")))?;
        let sessions = self.report_private_sessions(&wcdb, &cleaned);
        if sessions.is_empty() {
            return Err(AppError::runtime("no message sessions found"));
        }
        let is_all = year == 0;
        let (start, end) = year_range(year);
        let now = Local::now();

        let (nb, ne) = normalize_range(start, end);
        let d = wcdb
            .annual_report_stats(&sessions, nb, ne)
            .or_else(|_| self.aggregate_for_reports(&wcdb, &sessions, start, end))
            .map_err(|e| AppError::native(format!("base statistics failed: {e}")))?;
        let total_messages = num(&d["total"]);

        let mut contact_stats: Vec<(String, i64, i64)> = Vec::new(); // 会话 ID、发出、收到
        let mut monthly_stats: Vec<(String, HashMap<i64, i64>)> = Vec::new();
        if let Some(ss) = d.get("sessions").and_then(Value::as_object) {
            for (sid, stat) in ss {
                contact_stats.push((sid.clone(), num(&stat["sent"]), num(&stat["received"])));
                let m: HashMap<i64, i64> = stat
                    .get("monthly")
                    .and_then(Value::as_object)
                    .map(|o| {
                        o.iter()
                            .map(|(k, v)| (crate::api::js_parse_int(k).unwrap_or(0), num(v)))
                            .collect()
                    })
                    .unwrap_or_default();
                monthly_stats.push((sid.clone(), m));
            }
        }
        let mut daily_stats: Vec<(String, i64)> = Vec::new();
        let (mut peak_day_key, mut peak_day_count) = (String::new(), 0i64);
        if let Some(days) = d.get("daily").and_then(Value::as_object) {
            for (day, c) in days {
                let c = num(c);
                daily_stats.push((day.clone(), c));
                if c > peak_day_count {
                    peak_day_count = c;
                    peak_day_key = day.clone();
                }
            }
        }
        let mut heatmap = vec![vec![0i64; 24]; 7];
        let mut midnight_stats: IndexMap<String, i64> = IndexMap::new();
        let mut daily_contact: HashMap<String, IndexMap<String, i64>> = HashMap::new();
        let mut conversation: IndexMap<String, (i64, i64)> = IndexMap::new();
        let mut response_sql: Option<Map<String, Value>> = None;
        let mut top_phrases_sql: Option<Vec<PhraseCount>> = None;
        let (mut streak_sid, mut streak_days) = (String::new(), 0i64);
        let (mut streak_start, mut streak_end): (
            Option<chrono::DateTime<Local>>,
            Option<chrono::DateTime<Local>>,
        ) = (None, None);
        let mut streak_done = false;
        let mut use_sql_extras = false;

        let (peak_begin, peak_end) = if peak_day_key.is_empty() {
            (0, 0)
        } else {
            match parse_local_day(&peak_day_key) {
                Some(d) => (d.timestamp(), d.timestamp() + 24 * 3600 - 1),
                None => (0, 0),
            }
        };
        self.emit_progress("annual-report", "loading extended statistics…", 30, 100);
        let (eb, ee) = normalize_range(start, end);
        let extras = wcdb.annual_report_extras(
            &sessions,
            eb,
            ee,
            normalize_timestamp(peak_begin),
            normalize_timestamp(peak_end),
        );
        if let Ok(mut ex) = extras {
            if let Some(hm) = ex
                .get("heatmap")
                .and_then(Value::as_array)
                .filter(|h| h.len() == 7)
            {
                for (w, row) in hm.iter().enumerate() {
                    if let Some(row) = row.as_array() {
                        for (h, cell) in heatmap[w].iter_mut().enumerate() {
                            *cell = row.get(h).map(num).unwrap_or(0);
                        }
                    }
                }
            }
            if let Some(m) = ex.get("midnight").and_then(Value::as_object) {
                for (sid, c) in m {
                    midnight_stats.insert(sid.clone(), num(c));
                }
            }
            if let Some(c) = ex.get("conversation").and_then(Value::as_object) {
                for (sid, st) in c {
                    conversation.insert(sid.clone(), (num(&st["initiated"]), num(&st["received"])));
                }
            }
            response_sql = match ex.get_mut("response").map(Value::take) {
                Some(Value::Object(m)) => Some(m),
                _ => None,
            };
            if let (false, Some(p)) = (
                peak_day_key.is_empty(),
                ex.get("peakDay").and_then(Value::as_object),
            ) {
                let m: IndexMap<String, i64> = p.iter().map(|(k, v)| (k.clone(), num(v))).collect();
                if !m.is_empty() {
                    daily_contact.insert(peak_day_key.clone(), m);
                }
            }
            top_phrases_sql =
                from_native::<Vec<PhraseCount>>(&mut ex, "topPhrases")?.filter(|p| !p.is_empty());
            if let Some(s) = ex.get("streak") {
                let sid = s.get("sessionId").and_then(Value::as_str).unwrap_or("");
                let days = num(&s["days"]);
                if !sid.is_empty() && days > 0 {
                    streak_sid = sid.to_string();
                    streak_days = days;
                    streak_start = s
                        .get("startDate")
                        .and_then(Value::as_str)
                        .and_then(parse_local_day);
                    streak_end = s
                        .get("endDate")
                        .and_then(Value::as_str)
                        .and_then(parse_local_day);
                    if streak_start.is_some() && streak_end.is_some() {
                        streak_done = true;
                    }
                }
            }
            use_sql_extras = true;
        }

        // 逐条读取的后备方式所用的对话、回复和常用语的状态
        let mut response_times: IndexMap<String, Vec<i64>> = IndexMap::new();
        let mut phrase_count: HashMap<String, i64> = HashMap::new();
        let mut phrase_order: Vec<String> = Vec::new();
        if !use_sql_extras {
            let mut last_message: HashMap<String, (i64, bool)> = HashMap::new();
            for (i, sid) in sessions.iter().enumerate() {
                let Ok(cursor) = wcdb.open_message_cursor(
                    sid,
                    1000,
                    Order::OldestFirst,
                    crate::num::saturating_i32(start.max(0)),
                    crate::num::saturating_i32(end.max(0)),
                    Columns::Lite,
                ) else {
                    continue;
                };
                let (mut last_day, mut cur_streak, mut max_streak) = (None::<i64>, 0i64, 0i64);
                let (mut cur_start, mut max_start, mut max_end) = (
                    None::<chrono::DateTime<Local>>,
                    None::<chrono::DateTime<Local>>,
                    None::<chrono::DateTime<Local>>,
                );
                loop {
                    let Ok((rows, more)) = wcdb.fetch_message_batch(cursor) else {
                        break;
                    };
                    for row in &rows {
                        let create_time = row.create_time.unwrap_or(0);
                        if create_time == 0 {
                            continue;
                        }
                        let sent = row.is_send
                            || sent_by_me(row.sender.as_deref().unwrap_or(""), &raw_wxid, &cleaned);
                        let local_type = if row.local_type == 0 {
                            1
                        } else {
                            row.local_type
                        };
                        let conv = conversation.entry(sid.clone()).or_insert((0, 0));
                        match last_message.get(sid) {
                            Some(&(t, was_sent)) if create_time - t <= CONVERSATION_GAP => {
                                if was_sent != sent && sent && !was_sent {
                                    let rt = create_time - t;
                                    if rt > 0 && rt < 86400 {
                                        response_times.entry(sid.clone()).or_default().push(rt);
                                    }
                                }
                            }
                            _ => {
                                if sent {
                                    conv.0 += 1;
                                } else {
                                    conv.1 += 1;
                                }
                            }
                        }
                        last_message.insert(sid.clone(), (create_time, sent));
                        if (local_type == 1 || local_type == QUOTE) && sent {
                            // 引用消息的内容是它的 appmsg 的 XML：说的话在它的标题里
                            let text = if local_type == QUOTE {
                                reply_text(&row.content)
                            } else {
                                row.content.trim().to_string()
                            };
                            let n = text.chars().count();
                            if (2..=20).contains(&n)
                                && !text.contains("http")
                                && !text.contains('<')
                                && !text.starts_with('[')
                            {
                                let e = phrase_count.entry(text.clone()).or_insert(0);
                                if *e == 0 {
                                    phrase_order.push(text);
                                }
                                *e += 1;
                            }
                        }
                        let Some(dt) = Local.timestamp_opt(create_time, 0).single() else {
                            continue;
                        };
                        let wd = dt.weekday().num_days_from_monday() as usize;
                        heatmap[wd][dt.hour() as usize] += 1;
                        let idx = day_index(dt);
                        let day_date = Local
                            .from_local_datetime(&dt.date_naive().and_time(NaiveTime::MIN))
                            .earliest()
                            .unwrap_or(dt);
                        if last_day != Some(idx) {
                            if last_day.is_some_and(|l| idx - l == 1) {
                                cur_streak += 1;
                            } else {
                                cur_streak = 1;
                                cur_start = Some(day_date);
                            }
                            if cur_streak > max_streak {
                                max_streak = cur_streak;
                                max_start = cur_start;
                                max_end = Some(day_date);
                            }
                            last_day = Some(idx);
                        }
                        if dt.hour() < 6 {
                            *midnight_stats.entry(sid.clone()).or_insert(0) += 1;
                        }
                        if !peak_day_key.is_empty() && ymd(dt) == peak_day_key {
                            *daily_contact
                                .entry(peak_day_key.clone())
                                .or_default()
                                .entry(sid.clone())
                                .or_insert(0) += 1;
                        }
                    }
                    if !more {
                        break;
                    }
                }
                let _ = wcdb.close_message_cursor(cursor);
                if max_streak > streak_days {
                    streak_days = max_streak;
                    streak_sid = sid.clone();
                    streak_start = max_start;
                    streak_end = max_end;
                }
                self.emit_progress("annual-report", "analysing chats…", i + 1, sessions.len());
            }
            streak_done = true;
        }
        if !streak_done {
            // 逐条读取，计算最长的连续聊天天数
            for sid in &sessions {
                let Ok(cursor) = wcdb.open_message_cursor(
                    sid,
                    2000,
                    Order::OldestFirst,
                    crate::num::saturating_i32(start.max(0)),
                    crate::num::saturating_i32(end.max(0)),
                    Columns::Lite,
                ) else {
                    continue;
                };
                let (mut last_day, mut cur_streak, mut max_streak) = (None::<i64>, 0i64, 0i64);
                let (mut cur_start, mut max_start, mut max_end) = (
                    None::<chrono::DateTime<Local>>,
                    None::<chrono::DateTime<Local>>,
                    None::<chrono::DateTime<Local>>,
                );
                loop {
                    let Ok((rows, more)) = wcdb.fetch_message_batch(cursor) else {
                        break;
                    };
                    for row in &rows {
                        let ct = row.create_time.unwrap_or(0);
                        if ct == 0 {
                            continue;
                        }
                        let Some(dt) = Local.timestamp_opt(ct, 0).single() else {
                            continue;
                        };
                        let idx = day_index(dt);
                        if last_day == Some(idx) {
                            continue;
                        }
                        let day_date = Local
                            .from_local_datetime(&dt.date_naive().and_time(NaiveTime::MIN))
                            .earliest()
                            .unwrap_or(dt);
                        if last_day.is_some_and(|l| idx - l == 1) {
                            cur_streak += 1;
                        } else {
                            cur_streak = 1;
                            cur_start = Some(day_date);
                        }
                        if cur_streak > max_streak {
                            max_streak = cur_streak;
                            max_start = cur_start;
                            max_end = Some(day_date);
                        }
                        last_day = Some(idx);
                    }
                    if !more {
                        break;
                    }
                }
                let _ = wcdb.close_message_cursor(cursor);
                if max_streak > streak_days {
                    streak_days = max_streak;
                    streak_sid = sid.clone();
                    streak_start = max_start;
                    streak_end = max_end;
                }
            }
        }

        // 朋友圈
        self.emit_progress("annual-report", "analysing Moments…", 75, 100);
        let mut sns_stats: Option<AnnualSnsStats> = None;
        let (sb, se) = if is_all {
            (0, now.timestamp())
        } else {
            (start, end)
        };
        if let Ok(sd) = wcdb.sns_annual_stats(
            crate::num::saturating_i32(sb.max(0)),
            crate::num::saturating_i32(se.max(0)),
        ) {
            if sd.is_object() {
                let listed = |key: &str| -> Vec<(String, i64)> {
                    sd.get(key)
                        .and_then(Value::as_array)
                        .map(|a| a.as_slice())
                        .unwrap_or(&[])
                        .iter()
                        .map(|u| {
                            let name = u.get("username").and_then(Value::as_str).unwrap_or("");
                            (name.to_string(), num(&u["count"]))
                        })
                        .collect()
                };
                let (likers, liked) = (listed("topLikers"), listed("topLiked"));
                let mut users: Vec<String> = Vec::new();
                for (n, _) in likers.iter().chain(&liked) {
                    if !n.is_empty() && !users.contains(n) {
                        users.push(n.clone());
                    }
                }
                let (names, avatars) = self.names_and_avatars(&wcdb, &users);
                let enrich = |list: Vec<(String, i64)>| -> Vec<FriendCount> {
                    list.into_iter()
                        .map(|(username, count)| FriendCount {
                            display_name: names
                                .get(&username)
                                .filter(|n| !n.is_empty())
                                .cloned()
                                .unwrap_or_else(|| username.clone()),
                            avatar_url: avatars.get(&username).cloned(),
                            username,
                            count,
                        })
                        .collect()
                };
                let type_counts = sd
                    .get("typeCounts")
                    .and_then(Value::as_object)
                    .map(|t| t.iter().map(|(k, v)| (k.clone(), num(v))).collect());
                sns_stats = Some(AnnualSnsStats {
                    total_posts: num(&sd["totalPosts"]),
                    type_counts,
                    top_likers: enrich(likers),
                    top_liked: enrich(liked),
                });
            }
        }
        if is_all && sns_stats.as_ref().is_none_or(|s| s.total_posts <= 0) {
            let id = if cleaned.is_empty() {
                &raw_wxid
            } else {
                &cleaned
            };
            if let Ok(st) = wcdb.sns_export_stats(Some(id)) {
                let total_posts = num(&st["total_posts"]).max(0);
                sns_stats = Some(match sns_stats {
                    Some(s) => AnnualSnsStats { total_posts, ..s },
                    None => AnnualSnsStats {
                        total_posts,
                        type_counts: None,
                        top_likers: Vec::new(),
                        top_liked: Vec::new(),
                    },
                });
            }
        }

        // 联系人信息
        self.emit_progress("annual-report", "collecting contact info…", 85, 100);
        let contact_ids: Vec<String> = contact_stats.iter().map(|c| c.0.clone()).collect();
        let (names, avatars) = self.names_and_avatars(&wcdb, &contact_ids);
        let info = |sid: &str| -> (String, Option<String>) {
            (
                names
                    .get(sid)
                    .filter(|n| !n.is_empty())
                    .cloned()
                    .unwrap_or_else(|| sid.to_string()),
                avatars.get(sid).cloned(),
            )
        };
        let self_avatar = {
            let list = vec![raw_wxid.clone(), cleaned.clone()];
            wcdb.avatar_urls(&list).ok().and_then(|m| {
                m.get(&raw_wxid)
                    .or_else(|| m.get(&cleaned))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        };

        let mut core: Vec<(String, i64, i64)> = contact_stats.clone();
        core.sort_by_key(|a| std::cmp::Reverse(a.1 + a.2));
        let core_friends: Vec<CoreFriend> = core
            .iter()
            .take(3)
            .map(|(sid, sent, recv)| {
                let (display_name, avatar_url) = info(sid);
                CoreFriend {
                    username: sid.clone(),
                    display_name,
                    avatar_url,
                    message_count: sent + recv,
                    sent_count: *sent,
                    received_count: *recv,
                }
            })
            .collect();

        let monthly_top: Vec<MonthlyTopFriend> = (1..=12i64)
            .map(|month| {
                let (mut max_count, mut top) = (0i64, String::new());
                for (sid, m) in &monthly_stats {
                    let c = m.get(&month).copied().unwrap_or(0);
                    if c > max_count {
                        max_count = c;
                        top = sid.clone();
                    }
                }
                let (display_name, avatar_url) = if top.is_empty() {
                    ("暂无".to_string(), None)
                } else {
                    info(&top)
                };
                MonthlyTopFriend {
                    month,
                    display_name,
                    avatar_url,
                    message_count: max_count,
                }
            })
            .collect();

        let mut peak_day = None;
        let mut max_day = 0i64;
        for (day, count) in &daily_stats {
            if *count > max_day {
                max_day = *count;
                let (mut top_friend, mut top_count) = (String::new(), 0i64);
                if let Some(m) = daily_contact.get(day) {
                    // 这个表保持读取会话的顺序，所以次数相同时保留先读到的好友
                    for (sid, c) in m {
                        if *c > top_count {
                            top_count = *c;
                            top_friend = info(sid).0;
                        }
                    }
                }
                peak_day = Some(PeakDay {
                    date: day.clone(),
                    message_count: *count,
                    top_friend,
                    top_friend_count: top_count,
                });
            }
        }

        let total_midnight: i64 = midnight_stats.values().sum();
        let midnight_king = if total_midnight > 0 {
            let (mut max_m, mut sid) = (0i64, String::new());
            for (s, c) in &midnight_stats {
                if *c > max_m {
                    max_m = *c;
                    sid = s.clone();
                }
            }
            Some(MidnightKing {
                display_name: info(&sid).0,
                count: max_m,
                percentage: round1(i64_f64(max_m) / i64_f64(total_midnight) * 100.0),
            })
        } else {
            None
        };

        let longest_streak = match (
            streak_sid.is_empty(),
            streak_days > 0,
            streak_start,
            streak_end,
        ) {
            (false, true, Some(s), Some(e)) => Some(LongestStreak {
                friend_name: info(&streak_sid).0,
                days: streak_days,
                start_date: ymd(s),
                end_date: ymd(e),
            }),
            _ => None,
        };

        let mut mutual = None;
        let mut best_diff = f64::INFINITY;
        for (sid, sent, recv) in &contact_stats {
            if *sent >= 50 && *recv >= 50 {
                let ratio = i64_f64(*sent) / i64_f64(*recv);
                let diff = (ratio - 1.0).abs();
                if diff < best_diff {
                    best_diff = diff;
                    let (display_name, avatar_url) = info(sid);
                    mutual = Some(MutualFriend {
                        display_name,
                        avatar_url,
                        sent_count: *sent,
                        received_count: *recv,
                        ratio: (ratio * 100.0).round() / 100.0,
                    });
                }
            }
        }

        let (mut total_init, mut total_recv, mut top_init_sid, mut top_init) =
            (0i64, 0i64, String::new(), 0i64);
        for (sid, (i, r)) in &conversation {
            total_init += i;
            total_recv += r;
            if *i > top_init {
                top_init = *i;
                top_init_sid = sid.clone();
            }
        }
        let total_conv = total_init + total_recv;
        let social = (total_conv > 0).then(|| SocialInitiative {
            initiated_chats: total_init,
            received_chats: total_recv,
            initiative_rate: round1(i64_f64(total_init) / i64_f64(total_conv) * 100.0),
            top_initiated_friend: (top_init > 0).then(|| info(&top_init_sid).0),
            top_initiated_count: (top_init > 0).then_some(top_init),
        });

        self.emit_progress("annual-report", "building report…", 95, 100);
        let response_speed = if let Some(resp) = response_sql.as_ref().filter(|r| !r.is_empty()) {
            let (mut sum, mut cnt, mut fastest_id, mut fastest) =
                (0.0f64, 0.0f64, String::new(), f64::INFINITY);
            for (sid, st) in resp {
                let (count, avg) = (fnum(&st["count"]), fnum(&st["avg"]));
                if count <= 0.0 || avg <= 0.0 {
                    continue;
                }
                sum += avg * count;
                cnt += count;
                if avg < fastest {
                    fastest = avg;
                    fastest_id = sid.clone();
                }
            }
            (cnt > 0.0).then(|| ResponseSpeed {
                avg_response_time: crate::num::saturating_i64((sum / cnt).round()),
                fastest_friend: info(&fastest_id).0,
                fastest_time: crate::num::saturating_i64(fastest.round()),
            })
        } else {
            let (mut all, mut fastest_id, mut fastest) =
                (Vec::<i64>::new(), String::new(), f64::INFINITY);
            for (sid, times) in &response_times {
                if times.len() >= 10 {
                    all.extend(times);
                    let avg = i64_f64(times.iter().sum::<i64>()) / count_f64(times.len());
                    if avg < fastest {
                        fastest = avg;
                        fastest_id = sid.clone();
                    }
                }
            }
            (!all.is_empty()).then(|| ResponseSpeed {
                avg_response_time: crate::num::saturating_i64(
                    (i64_f64(all.iter().sum::<i64>()) / count_f64(all.len())).round(),
                ),
                fastest_friend: info(&fastest_id).0,
                fastest_time: crate::num::saturating_i64(fastest.round()),
            })
        };

        let top_phrases: Vec<PhraseCount> = match top_phrases_sql {
            Some(p) if !p.is_empty() => p,
            _ => {
                let mut v: Vec<(String, i64)> = phrase_order
                    .iter()
                    .map(|k| (k.clone(), phrase_count[k]))
                    .filter(|(_, c)| *c >= 2)
                    .collect();
                v.sort_by_key(|a| std::cmp::Reverse(a.1));
                v.truncate(32);
                v.into_iter()
                    .map(|(phrase, count)| PhraseCount { phrase, count })
                    .collect()
            }
        };

        // 曾经亲密、后来很少联系的好友（`lost_friend`）
        let mut lost = None;
        let (mut max_early, mut best_early, mut best_late, mut best_sid, mut best_desc) =
            (80i64, 0i64, 0i64, String::new(), String::new());
        let current_year = now.year();
        let sums = |data: &Value, sid: &str| -> i64 {
            data.get("sessions")
                .and_then(|s| s.get(sid))
                .map(|s| num(&s["sent"]) + num(&s["received"]))
                .unwrap_or(0)
        };
        if is_all {
            let mut days: Vec<String> = d
                .get("daily")
                .and_then(Value::as_object)
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            days.sort();
            if let [first_day, .., last_day] = days.as_slice() {
                let first = parse_local_day(first_day)
                    .map(|d| d.timestamp())
                    .unwrap_or(0);
                let last = parse_local_day(last_day)
                    .map(|d| d.timestamp())
                    .unwrap_or(0);
                let mid = (first + last).div_euclid(2);
                if let (Ok(e), Ok(l)) = (
                    self.aggregate_for_reports(&wcdb, &sessions, 0, mid),
                    self.aggregate_for_reports(&wcdb, &sessions, mid, 0),
                ) {
                    for sid in &sessions {
                        let (early, late) = (sums(&e, sid), sums(&l, sid));
                        if early > 100 && early > late * 5 && early > max_early {
                            (max_early, best_early, best_late, best_sid, best_desc) =
                                (early, early, late, sid.clone(), "这段时间以来".into());
                        }
                    }
                }
            }
        } else if year == current_year {
            let month_start = |back: i32| {
                let total = now.year() * 12 + now.month0().cast_signed() - back;
                local_ts(
                    total.div_euclid(12),
                    (total.rem_euclid(12) + 1) as u32,
                    1,
                    0,
                    0,
                    0,
                )
            };
            let (rolling_start, rolling_mid, rolling_end) =
                (month_start(11), month_start(5), now.timestamp());
            if let (Ok(e), Ok(l)) = (
                self.aggregate_for_reports(&wcdb, &sessions, rolling_start, rolling_mid - 1),
                self.aggregate_for_reports(&wcdb, &sessions, rolling_mid, rolling_end),
            ) {
                for sid in &sessions {
                    let (early, late) = (sums(&e, sid), sums(&l, sid));
                    if early > 80 && early > late * 5 && early > max_early {
                        (max_early, best_early, best_late, best_sid, best_desc) =
                            (early, early, late, sid.clone(), "去年的这个时候".into());
                    }
                }
            }
        } else if let Some(ss) = d.get("sessions").and_then(Value::as_object) {
            for (sid, stat) in ss {
                let monthly = stat.get("monthly");
                let m = |k: i64| {
                    monthly
                        .and_then(|m| m.get(k.to_string()))
                        .map(num)
                        .unwrap_or(0)
                };
                let early: i64 = (1..=6).map(m).sum();
                let late: i64 = (7..=12).map(m).sum();
                if early > 80 && early > late * 5 && early > max_early {
                    (max_early, best_early, best_late, best_sid, best_desc) =
                        (early, early, late, sid.clone(), format!("{year}年上半年"));
                }
            }
        }
        if !best_sid.is_empty() {
            let (display_name, avatar_url) = if contact_ids.contains(&best_sid) {
                info(&best_sid)
            } else {
                let (n, a) = self.names_and_avatars(&wcdb, std::slice::from_ref(&best_sid));
                (
                    n.get(&best_sid)
                        .filter(|x| !x.is_empty())
                        .cloned()
                        .unwrap_or_else(|| best_sid.clone()),
                    a.get(&best_sid).cloned(),
                )
            };
            lost = Some(LostFriend {
                username: best_sid,
                display_name,
                avatar_url,
                early_count: best_early,
                late_count: best_late,
                period_desc: best_desc,
            });
        }

        Ok(AnnualReport {
            year,
            total_messages,
            total_friends: contact_stats.len(),
            core_friends,
            monthly_top_friends: monthly_top,
            peak_day,
            longest_streak,
            activity_heatmap: ActivityHeatmap { data: heatmap },
            midnight_king,
            self_avatar_url: self_avatar,
            mutual_friend: mutual,
            social_initiative: social,
            response_speed,
            top_phrases,
            sns_stats,
            lost_friend: lost,
        })
    }

    fn aggregate_for_reports(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        ids: &[String],
        begin: i64,
        end: i64,
    ) -> anyhow::Result<Value> {
        let (b, e) = normalize_range(begin, end);
        wcdb.aggregate_stats(ids, b, e)
    }

    fn first_messages(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        session: &str,
        limit: usize,
        begin: i64,
        end: i64,
    ) -> Vec<MsgRow> {
        let safe_begin = begin.max(0);
        let safe_end = if end > 0 {
            end
        } else {
            chrono::Utc::now().timestamp()
        };
        let Ok(cursor) = wcdb.open_message_cursor(
            session,
            crate::num::saturating_i32(limit.max(1)),
            Order::OldestFirst,
            crate::num::saturating_i32(safe_begin.max(0)),
            crate::num::saturating_i32(safe_end.max(0)),
            Columns::Lite,
        ) else {
            return Vec::new();
        };
        let mut rows: Vec<MsgRow> = Vec::new();
        loop {
            let Ok((batch, more)) = wcdb.fetch_message_batch(cursor) else {
                break;
            };
            rows.extend(batch.into_iter().take(limit - rows.len()));
            if !more || rows.len() >= limit {
                break;
            }
        }
        let _ = wcdb.close_message_cursor(cursor);
        rows.truncate(limit);
        rows
    }

    /// 双人报告（桌面版的 `dualReport:generateReport`）。
    ///
    /// # Errors
    /// `year` 既不是 `0`（所有年份）也不是 1970 到 2100 之间的年份时返回 `invalid_arguments`；
    /// 账号没有配置好时返回配置错误，读取数据库或统计失败时返回数据库层的错误；
    /// 数据库层的回答不符合契约类型时返回运行时错误。
    pub fn report_dual(
        &self,
        friend: &str,
        year: i32,
        exclude_words: &[String],
    ) -> AppResult<DualReport> {
        let year = checked_year(year)?;
        self.emit_progress("dual-report", "connecting…", 5, 100);
        let (wcdb, cleaned, raw_wxid) = self.report_connect()?;
        // 报告先读好友的联系人信息，再读消息
        wcdb.prefetch(&[Databases::Contacts, Databases::Messages])
            .map_err(|e| AppError::native(format!("{e:#}")))?;
        let is_all = year == 0;
        let (start, end) = year_range(year);

        let display = |u: &str, fb: &str| -> String {
            wcdb.display_names(&[u])
                .ok()
                .and_then(|m| {
                    m.get(u)
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| fb.to_string())
        };
        let friend_name = display(friend, friend);
        let mut my_name = display(&raw_wxid, &raw_wxid);
        if my_name == raw_wxid && !cleaned.is_empty() && cleaned != raw_wxid {
            my_name = display(&cleaned, &raw_wxid);
        }
        let candidates: Vec<String> = {
            let mut seen = HashSet::new();
            [friend.to_string(), raw_wxid.clone(), cleaned.clone()]
                .into_iter()
                .filter(|s| !s.is_empty() && seen.insert(s.clone()))
                .collect()
        };
        let avatars = wcdb.avatar_urls(&candidates).ok();
        let av = |k: &str| {
            avatars
                .as_ref()
                .and_then(|m| m.get(k))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let self_avatar = av(&raw_wxid).or_else(|| av(&cleaned));
        let friend_avatar = av(friend);

        let message_view = |row: &MsgRow| -> DualMessage {
            let create_ms = row.create_time.unwrap_or(0) * 1000;
            let local_type = row.local_type;
            let (md5, url) = if local_type == 47 {
                emoji_of(&row.content)
            } else {
                (None, None)
            };
            DualMessage {
                content: row.content.clone(),
                is_sent_by_me: row.is_send,
                sender_username: None,
                create_time: create_ms,
                create_time_str: format_date_time(create_ms),
                local_type,
                app_msg_kind: app_msg_kind_of(row),
                emoji_md5: md5,
                emoji_cdn_url: url,
            }
        };

        self.emit_progress("dual-report", "first messages…", 15, 100);
        let first_rows = self.first_messages(&wcdb, friend, 10, 0, 0);
        let first_chat = first_rows.first().map(|row| DualMessage {
            sender_username: row.sender.clone(),
            ..message_view(row)
        });
        let first_chat_messages: Vec<DualMessage> = first_rows.iter().map(&message_view).collect();

        let mut year_first_chat = None;
        if !is_all {
            let rows = self.first_messages(&wcdb, friend, 10, start, end);
            if let Some(first) = rows.first() {
                let view = message_view(first);
                year_first_chat = Some(YearFirstChat {
                    create_time: view.create_time,
                    create_time_str: view.create_time_str,
                    content: view.content,
                    is_sent_by_me: view.is_sent_by_me,
                    friend_name: friend_name.clone(),
                    first_three_messages: rows.iter().map(&message_view).collect(),
                    local_type: view.local_type,
                    app_msg_kind: view.app_msg_kind,
                    emoji_md5: view.emoji_md5,
                    emoji_cdn_url: view.emoji_cdn_url,
                });
            }
        }

        self.emit_progress("dual-report", "chat statistics…", 30, 100);
        let (b, e) = normalize_range(start, end);
        let mut cpp = wcdb.dual_report_stats(friend, b, e).map_err(|err| {
            AppError::native(format!("failed to get dual-report statistics: {err}"))
        })?;
        let counts = cpp
            .get_mut("counts")
            .map(Value::take)
            .unwrap_or_else(|| json!({}));
        let emoji_count = num(&counts["emoji"]);

        let (mut my_md5, mut my_url, mut my_count) = (None::<String>, None::<String>, -1i64);
        let (mut fr_md5, mut fr_url, mut fr_count) = (None::<String>, None::<String>, -1i64);
        for item in cpp
            .get("emojis")
            .and_then(Value::as_array)
            .map(|a| a.as_slice())
            .unwrap_or(&[])
        {
            let c = parse_emoji_candidate(item);
            let (Some(md5), Some(is_me)) = (c.md5.clone(), c.is_me) else {
                continue;
            };
            if c.count <= 0 {
                continue;
            }
            if is_me {
                if c.count > my_count {
                    (my_count, my_md5, my_url) = (c.count, Some(md5), c.url);
                }
            } else if c.count > fr_count {
                (fr_count, fr_md5, fr_url) = (c.count, Some(md5), c.url);
            }
        }
        if emoji_count > 0 && (my_md5.is_none() || fr_md5.is_none()) {
            // 后备：逐条扫描会话
            let mut tally: Vec<(bool, String, Option<String>, i64)> = Vec::new();
            if let Ok(cursor) = wcdb.open_message_cursor(
                friend,
                500,
                Order::OldestFirst,
                crate::num::saturating_i32(start.max(0)),
                crate::num::saturating_i32(end.max(0)),
                Columns::Lite,
            ) {
                loop {
                    let Ok((rows, more)) = wcdb.fetch_message_batch(cursor) else {
                        break;
                    };
                    for row in &rows {
                        if row.local_type != 47 {
                            continue;
                        }
                        let (Some(md5), url) = emoji_of(&row.content) else {
                            continue;
                        };
                        let me = row.is_send;
                        match tally.iter_mut().find(|t| t.0 == me && t.1 == md5) {
                            Some(t) => {
                                t.3 += 1;
                                if t.2.is_none() {
                                    t.2 = url;
                                }
                            }
                            None => tally.push((me, md5, url, 1)),
                        }
                    }
                    if !more {
                        break;
                    }
                }
                let _ = wcdb.close_message_cursor(cursor);
            }
            type TallyRow = (bool, String, Option<String>, i64);
            let (mut my_top, mut fr_top): (Option<&TallyRow>, Option<&TallyRow>) = (None, None);
            for t in &tally {
                if t.0 {
                    if my_top.is_none_or(|m| t.3 > m.3) {
                        my_top = Some(t);
                    }
                } else if fr_top.is_none_or(|m| t.3 > m.3) {
                    fr_top = Some(t);
                }
            }
            if my_md5.is_none() {
                if let Some(t) = my_top {
                    my_md5 = Some(t.1.clone());
                    my_url = my_url.or(t.2.clone());
                    my_count = t.3;
                }
            }
            if fr_md5.is_none() {
                if let Some(t) = fr_top {
                    fr_md5 = Some(t.1.clone());
                    fr_url = fr_url.or(t.2.clone());
                    fr_count = t.3;
                }
            }
        }
        // 消息里没有地址的表情：用表情数据库中给它记的地址
        let stored_url = |md5: &Option<String>| {
            let url = wcdb.emoticon_cdn_url(md5.as_deref()?).ok()?;
            (!url.is_empty()).then_some(url)
        };
        let my_url = my_url.or_else(|| stored_url(&my_md5));
        let fr_url = fr_url.or_else(|| stored_url(&fr_md5));
        let stats = DualStats {
            total_messages: num(&counts["total"]),
            total_words: num(&counts["words"]),
            image_count: num(&counts["image"]),
            voice_count: num(&counts["voice"]),
            emoji_count,
            my_top_emoji_md5: my_md5,
            my_top_emoji_url: my_url,
            friend_top_emoji_md5: fr_md5,
            friend_top_emoji_url: fr_url,
            my_top_emoji_count: (my_count >= 0).then_some(my_count),
            friend_top_emoji_count: (fr_count >= 0).then_some(fr_count),
        };

        let exclude: HashSet<&str> = exclude_words.iter().map(String::as_str).collect();
        let mut filter = |key: &str| -> AppResult<Vec<PhraseCount>> {
            let list: Vec<PhraseCount> = from_native(&mut cpp, key)?.unwrap_or_default();
            Ok(list
                .into_iter()
                .filter(|p| !exclude.contains(p.phrase.as_str()))
                .collect())
        };
        let (top_phrases, clean_my, clean_fr) = (
            filter("phrases")?,
            filter("myPhrases")?,
            filter("friendPhrases")?,
        );
        let phrase_map = |list: &[PhraseCount]| -> Vec<(String, i64)> {
            list.iter().map(|p| (p.phrase.clone(), p.count)).collect()
        };
        let (my_map, fr_map) = (phrase_map(&clean_my), phrase_map(&clean_fr));
        let lookup = |m: &[(String, i64)], k: &str| {
            m.iter()
                .rev()
                .find(|(p, _)| p == k)
                .map(|(_, c)| *c)
                .unwrap_or(0)
        };
        let exclusive = |own: &[(String, i64)], other: &[(String, i64)]| -> Vec<PhraseCount> {
            let mut seen = HashSet::new();
            let mut v: Vec<(String, i64)> = Vec::new();
            for (phrase, _) in own {
                if !seen.insert(phrase.clone()) {
                    continue;
                }
                let c = lookup(own, phrase);
                let total = c + lookup(other, phrase);
                if c >= 2 && total > 0 && i64_f64(c) / i64_f64(total) >= 0.75 {
                    v.push((phrase.clone(), c));
                }
            }
            v.sort_by_key(|a| std::cmp::Reverse(a.1));
            v.truncate(20);
            v.into_iter()
                .map(|(phrase, count)| PhraseCount { phrase, count })
                .collect()
        };
        let my_excl = exclusive(&my_map, &fr_map);
        let fr_excl = exclusive(&fr_map, &my_map);

        let report = DualReport {
            year,
            self_name: my_name,
            self_avatar_url: self_avatar,
            friend_username: friend.to_string(),
            friend_name,
            friend_avatar_url: friend_avatar,
            first_chat,
            first_chat_messages,
            year_first_chat,
            stats,
            top_phrases,
            my_exclusive_phrases: my_excl,
            friend_exclusive_phrases: fr_excl,
            heatmap: from_native(&mut cpp, "heatmap")?,
            initiative: from_native(&mut cpp, "initiative")?,
            response: from_native(&mut cpp, "response")?,
            monthly: from_native(&mut cpp, "monthly")?,
            streak: from_native(&mut cpp, "streak")?,
        };
        self.emit_progress("dual-report", "done", 100, 100);
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};

    #[test]
    fn a_report_year_is_all_years_or_one_from_1970_to_2100() {
        for year in [0, 1970, 1971, 2026, 2100] {
            assert_eq!(checked_year(year).unwrap(), year);
        }
        for year in [1, 1800, 1969, -1, -5, 2101, i32::MIN, i32::MAX] {
            let err = checked_year(year).unwrap_err();
            assert_eq!(err.code, "invalid_arguments", "{year}");
            assert_eq!(err.exit_code, 2, "{year}");
            assert!(err.message.contains(&year.to_string()), "{}", err.message);
        }
    }

    #[test]
    fn a_year_range_is_a_limit_in_every_time_zone() {
        for hours in -12..=14 {
            let tz = FixedOffset::east_opt(hours * 3600).unwrap();
            for year in crate::dates::YEARS {
                let (start, end) = year_range_in(&tz, year);
                assert!(
                    0 < start && start < end,
                    "{year} at UTC{hours:+}: {start}..{end}"
                );
            }
        }
    }

    #[test]
    fn a_year_range_runs_from_the_first_to_the_last_second_of_the_year() {
        let utc = year_range_in(&Utc, 2023);
        assert_eq!(utc, (1_672_531_200, 1_704_067_199));
        let tokyo = FixedOffset::east_opt(9 * 3600).unwrap();
        assert_eq!(
            year_range_in(&tokyo, 2023),
            (1_672_531_200 - 32_400, 1_704_067_199 - 32_400)
        );
        let new_york = FixedOffset::west_opt(5 * 3600).unwrap();
        assert_eq!(
            year_range_in(&new_york, 2023),
            (1_672_531_200 + 18_000, 1_704_067_199 + 18_000)
        );
        // 1970 年的开始在 UTC 是纪元本身，在 UTC 以东早于纪元：两者都会被当作不限，所以固定为 1
        assert_eq!(year_range_in(&Utc, 1970), (1, 31_535_999));
        assert_eq!(year_range_in(&tokyo, 1970), (1, 31_535_999 - 32_400));
        assert_eq!(
            year_range_in(&new_york, 1970),
            (18_000, 31_535_999 + 18_000)
        );
        assert_eq!(year_range_in(&Utc, 0), (0, 0), "all years have no limit");
    }

    #[test]
    fn emoji_urls_with_broken_escapes_do_not_panic() {
        // `%` 后面紧跟多字节字符，在地址中和整条消息中各试一次
        assert_eq!(normalize_emoji_url("%a中").as_deref(), Some("%a中"));
        let xml = r#"<msg><emoji cdnurl="http://emoji.example/%a中?m=%E4%B8%AD" /></msg>"#;
        assert_eq!(
            extract_emoji_url(xml).as_deref(),
            Some("http://emoji.example/%a中?m=中")
        );
        assert_eq!(emoji_of(&format!("0:{xml}")).1, extract_emoji_url(xml));
        assert_eq!(
            normalize_emoji_url("http://e.example/%+5").as_deref(),
            Some("http://e.example/%+5"),
            "a sign is not a hex digit"
        );
    }

    #[test]
    fn own_messages_are_told_by_the_account_s_id_not_by_a_prefix() {
        assert!(sent_by_me("wxid_me", "wxid_me_1a2b", "wxid_me"));
        assert!(sent_by_me("WXID_ME", "wxid_me_1a2b", "wxid_me"));
        assert!(sent_by_me("john", "john_1a2b", "john"));
        // 好友的自定义 ID 是账号的 ID 加 `_` 和别的字符，以及反过来的情形
        assert!(!sent_by_me("john", "john_smith", "john_smith"));
        assert!(!sent_by_me("john_smith", "john", "john"));
        assert!(!sent_by_me("", "john", "john"));
    }

    #[test]
    fn a_report_names_an_app_message_as_the_chat_window_does() {
        let row = |local_type: i64, content: &str| MsgRow {
            local_type,
            content: content.into(),
            ..MsgRow::default()
        };
        let transfer = "<msg><appmsg><title>微信转账</title><type>2000</type></appmsg></msg>";
        // 转账保存为普通的 app 消息，以及在高 32 位带着自己的类型
        for local_type in [49, (2000 << 32) | 49] {
            assert_eq!(
                app_msg_kind_of(&row(local_type, transfer)).as_deref(),
                Some("transfer")
            );
        }
        assert_eq!(app_msg_kind_of(&row(1, "hi")), None);
        assert_eq!(
            app_msg_kind_of(&row(3, r#"<msg><img md5="x"/></msg>"#)),
            None
        );
    }
}
