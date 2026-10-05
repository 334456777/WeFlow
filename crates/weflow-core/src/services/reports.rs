//! Annual and dual ("two people") reports: ports of `annualReportService.ts` and
//! `dualReportService.ts`.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};
use serde_json::{json, Map, Value};

use super::*;
use crate::message::{decode_message_content, row_int, rx};

const CONVERSATION_GAP: i64 = 3600;

fn num(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
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

fn year_range(year: i32) -> (i64, i64) {
    if year <= 0 {
        (0, 0)
    } else {
        (
            local_ts(year, 1, 1, 0, 0, 0),
            local_ts(year, 12, 31, 23, 59, 59),
        )
    }
}

fn is_report_private_session(username: &str, cleaned_wxid: &str) -> bool {
    if username.is_empty()
        || username.contains("@chatroom")
        || username == "filehelper"
        || username.starts_with("gh_")
    {
        return false;
    }
    if username.to_lowercase() == cleaned_wxid.to_lowercase() || username.to_lowercase() == "weixin"
    {
        return false;
    }
    for prefix in [
        "qqmail",
        "fmessage",
        "medianote",
        "floatbottle",
        "newsapp",
        "brandsessionholder",
        "brandservicesessionholder",
        "notifymessage",
        "opencustomerservicemsg",
        "notification_messages",
        "userexperience_alarm",
        "helper_folders",
        "placeholder_foldgroup",
        "@helper_folders",
        "@placeholder_foldgroup",
    ] {
        if username.starts_with(prefix) || username == prefix {
            return false;
        }
    }
    !(username.contains("@kefu.openim")
        || username.contains("@openim")
        || username.contains("service_"))
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
    // local midnight as unix days, like `Math.floor(dayDate.getTime() / 86400000)`
    let midnight = Local
        .from_local_datetime(&dt.date_naive().and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .unwrap_or(dt);
    midnight.timestamp().div_euclid(86400)
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

// ── emoji helpers (dual report) ──

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

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 1 && i + 2 <= b.len().saturating_sub(1) {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
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
    // owner
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

fn resolve_is_sent(row: &Value, raw_wxid: &str, cleaned_wxid: &str) -> bool {
    let raw = row
        .get("computed_is_send")
        .filter(|v| !v.is_null())
        .or_else(|| row.get("is_send"))
        .filter(|v| !v.is_null());
    if let Some(v) = raw {
        return crate::api::js_parse_int(
            &v.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| v.to_string()),
        ) == Some(1);
    }
    let sender = ["sender_username", "sender", "talker"]
        .iter()
        .filter_map(|k| row.get(*k).and_then(Value::as_str))
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_lowercase();
    if sender.is_empty() {
        return false;
    }
    let (r, c) = (raw_wxid.to_lowercase(), cleaned_wxid.to_lowercase());
    sender == r
        || sender == c
        || (!r.is_empty() && r.starts_with(&format!("{sender}_")))
        || (!c.is_empty() && c.starts_with(&format!("{sender}_")))
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
        let raw = self
            .wxid_override
            .clone()
            .or_else(|| profile.wxid.clone())
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
            .filter(|u| is_report_private_session(u, cleaned))
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

    /// `getAvailableYears`: native query, then table time ranges, then first/last message.
    pub fn report_available_years(&self) -> AppResult<Value> {
        let (wcdb, cleaned, _) = self.report_connect()?;
        let sessions = self.report_private_sessions(&wcdb, &cleaned);
        if sessions.is_empty() {
            return Err(AppError::runtime("no message sessions found"));
        }
        if let Ok(Value::Array(native)) = wcdb.available_years(&sessions) {
            if !native.is_empty() {
                return Ok(
                    json!({ "years": Self::normalize_years(native.iter().map(num)), "strategy": "native" }),
                );
            }
        }
        // table scan
        let mut years: HashSet<i32> = HashSet::new();
        for sid in &sessions {
            let Ok(Value::Array(tables)) = wcdb.message_table_stats(sid) else {
                continue;
            };
            for t in tables {
                let name = ["table_name", "name"]
                    .iter()
                    .filter_map(|k| t.get(*k).and_then(Value::as_str))
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let db = ["db_path", "dbPath"]
                    .iter()
                    .filter_map(|k| t.get(*k).and_then(Value::as_str))
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if name.is_empty() || db.is_empty() {
                    continue;
                }
                let Ok(range) = wcdb.invoke_json(
                    "wcdb_get_message_table_time_range",
                    &[
                        weflow_native::wcdb::Arg::S(&db),
                        weflow_native::wcdb::Arg::S(&name),
                    ],
                ) else {
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
                        (n / 1000.0).floor() as i64
                    } else {
                        n.floor() as i64
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
                let edge = |asc: bool| -> i64 {
                    let Ok(cursor) = wcdb.open_message_cursor(sid, 1, asc, 0, 0, false) else {
                        return 0;
                    };
                    let r = wcdb.fetch_message_batch(cursor);
                    let _ = wcdb.close_message_cursor(cursor);
                    r.ok()
                        .and_then(|(rows, _)| {
                            rows.as_array()
                                .and_then(|a| a.first().map(|r| num(&r["create_time"])))
                        })
                        .unwrap_or(0)
                };
                Self::add_years_from_range(&mut years, edge(true), edge(false));
            }
        }
        Ok(
            json!({ "years": Self::normalize_years(years.into_iter().map(|y| y as i64)), "strategy": strategy }),
        )
    }

    /// `annualReport:generateReport`
    pub fn report_annual(&self, year: i32) -> AppResult<Value> {
        self.emit_progress("annual-report", "connecting…", 5, 100);
        let (wcdb, cleaned, raw_wxid) = self.report_connect()?;
        let sessions = self.report_private_sessions(&wcdb, &cleaned);
        if sessions.is_empty() {
            return Err(AppError::runtime("no message sessions found"));
        }
        let is_all = year <= 0;
        let report_year = if is_all { 0 } else { year };
        let (start, end) = year_range(year);
        let now = Local::now();

        let (nb, ne) = normalize_range(start, end);
        let d = wcdb
            .invoke_json(
                "wcdb_get_annual_report_stats",
                &[
                    weflow_native::wcdb::Arg::S(&serde_json::to_string(&sessions).unwrap()),
                    weflow_native::wcdb::Arg::I32(nb),
                    weflow_native::wcdb::Arg::I32(ne),
                ],
            )
            .or_else(|_| self.aggregate_for_reports(&wcdb, &sessions, start, end))
            .map_err(|e| AppError::native(format!("base statistics failed: {e}")))?;
        let total_messages = num(&d["total"]);

        let mut contact_stats: Vec<(String, i64, i64)> = Vec::new(); // sid, sent, received
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
        let mut midnight_stats: HashMap<String, i64> = HashMap::new();
        let mut daily_contact: HashMap<String, HashMap<String, i64>> = HashMap::new();
        let mut conversation: HashMap<String, (i64, i64)> = HashMap::new();
        let mut response_sql: Option<Map<String, Value>> = None;
        let mut top_phrases_sql: Option<Vec<Value>> = None;
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
            &serde_json::to_string(&sessions).unwrap(),
            eb,
            ee,
            normalize_timestamp(peak_begin),
            normalize_timestamp(peak_end),
        );
        if let Ok(ex) = extras {
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
            response_sql = ex.get("response").and_then(Value::as_object).cloned();
            if let (false, Some(p)) = (
                peak_day_key.is_empty(),
                ex.get("peakDay").and_then(Value::as_object),
            ) {
                let m: HashMap<String, i64> = p.iter().map(|(k, v)| (k.clone(), num(v))).collect();
                if !m.is_empty() {
                    daily_contact.insert(peak_day_key.clone(), m);
                }
            }
            if let Some(p) = ex
                .get("topPhrases")
                .and_then(Value::as_array)
                .filter(|p| !p.is_empty())
            {
                top_phrases_sql = Some(p.clone());
            }
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

        // conversation / response / phrase state for the cursor fallback
        let mut response_times: HashMap<String, Vec<i64>> = HashMap::new();
        let mut phrase_count: HashMap<String, i64> = HashMap::new();
        let mut phrase_order: Vec<String> = Vec::new();
        if !use_sql_extras {
            let mut last_message: HashMap<String, (i64, bool)> = HashMap::new();
            for (i, sid) in sessions.iter().enumerate() {
                let Ok(cursor) = wcdb.open_message_cursor(
                    sid,
                    1000,
                    true,
                    start.clamp(0, i32::MAX as i64) as i32,
                    end.clamp(0, i32::MAX as i64) as i32,
                    true,
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
                    let Some(rows) = rows.as_array() else { break };
                    for row in rows {
                        let create_time = num(&row["create_time"]);
                        if create_time == 0 {
                            continue;
                        }
                        let raw_is_send = row
                            .get("computed_is_send")
                            .filter(|v| !v.is_null())
                            .or_else(|| row.get("is_send"))
                            .cloned()
                            .unwrap_or(json!("0"));
                        let mut sent = crate::api::js_parse_int(
                            &raw_is_send
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| raw_is_send.to_string()),
                        ) == Some(1);
                        let local_type = ["local_type", "type"]
                            .iter()
                            .filter_map(|k| row.get(*k))
                            .map(num)
                            .find(|n| *n != 0)
                            .unwrap_or(1);
                        if !sent {
                            let sender = ["sender_username", "sender", "talker"]
                                .iter()
                                .filter_map(|k| row.get(*k).and_then(Value::as_str))
                                .find(|s| !s.is_empty())
                                .unwrap_or("")
                                .to_lowercase();
                            if !sender.is_empty() {
                                let (r, c) = (raw_wxid.to_lowercase(), cleaned.to_lowercase());
                                if sender == r
                                    || sender == c
                                    || r.starts_with(&format!("{sender}_"))
                                    || c.starts_with(&format!("{sender}_"))
                                {
                                    sent = true;
                                }
                            }
                        }
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
                        if (local_type == 1 || local_type == 244813135921) && sent {
                            let text = decode_message_content(row).trim().to_string();
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
                            .from_local_datetime(&dt.date_naive().and_hms_opt(0, 0, 0).unwrap())
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
            // cursor-based longest streak
            for sid in &sessions {
                let Ok(cursor) = wcdb.open_message_cursor(
                    sid,
                    2000,
                    true,
                    start.clamp(0, i32::MAX as i64) as i32,
                    end.clamp(0, i32::MAX as i64) as i32,
                    true,
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
                    for row in rows.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
                        let ct = num(&row["create_time"]);
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
                            .from_local_datetime(&dt.date_naive().and_hms_opt(0, 0, 0).unwrap())
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

        // Moments
        self.emit_progress("annual-report", "analysing Moments…", 75, 100);
        let mut sns_stats: Option<Value> = None;
        let (sb, se) = if is_all {
            (0, now.timestamp())
        } else {
            (start, end)
        };
        if let Ok(sd) = wcdb.sns_annual_stats(
            sb.clamp(0, i32::MAX as i64) as i32,
            se.clamp(0, i32::MAX as i64) as i32,
        ) {
            if sd.is_object() {
                let mut users: Vec<String> = Vec::new();
                for key in ["topLikers", "topLiked"] {
                    for u in sd
                        .get(key)
                        .and_then(Value::as_array)
                        .map(|a| a.as_slice())
                        .unwrap_or(&[])
                    {
                        if let Some(n) = u.get("username").and_then(Value::as_str) {
                            if !users.contains(&n.to_string()) {
                                users.push(n.to_string());
                            }
                        }
                    }
                }
                let (names, avatars) = self.names_and_avatars_pub(&wcdb, &users);
                let enrich = |key: &str| -> Vec<Value> {
                    sd.get(key)
                        .and_then(Value::as_array)
                        .map(|a| a.as_slice())
                        .unwrap_or(&[])
                        .iter()
                        .map(|u| {
                            let mut o = u.as_object().cloned().unwrap_or_default();
                            let name = u.get("username").and_then(Value::as_str).unwrap_or("");
                            o.insert(
                                "displayName".into(),
                                json!(names
                                    .get(name)
                                    .filter(|n| !n.is_empty())
                                    .cloned()
                                    .unwrap_or_else(|| name.to_string())),
                            );
                            match avatars.get(name) {
                                Some(a) => {
                                    o.insert("avatarUrl".into(), json!(a));
                                }
                                None => {
                                    o.shift_remove("avatarUrl");
                                }
                            }
                            Value::Object(o)
                        })
                        .collect()
                };
                let mut o = Map::new();
                o.insert("totalPosts".into(), json!(num(&sd["totalPosts"])));
                if let Some(t) = sd.get("typeCounts").filter(|v| !v.is_null()) {
                    o.insert("typeCounts".into(), t.clone());
                }
                o.insert("topLikers".into(), json!(enrich("topLikers")));
                o.insert("topLiked".into(), json!(enrich("topLiked")));
                sns_stats = Some(Value::Object(o));
            }
        }
        if is_all
            && sns_stats
                .as_ref()
                .is_none_or(|s| num(&s["totalPosts"]) <= 0)
        {
            let id = if cleaned.is_empty() {
                raw_wxid.clone()
            } else {
                cleaned.clone()
            };
            if let Ok(st) = wcdb.sns_export_stats(Some(&id)) {
                let total = num(&st["total_posts"]).max(0);
                let mut o = Map::new();
                o.insert("totalPosts".into(), json!(total));
                if let Some(t) = sns_stats.as_ref().and_then(|s| s.get("typeCounts")) {
                    o.insert("typeCounts".into(), t.clone());
                }
                o.insert(
                    "topLikers".into(),
                    sns_stats
                        .as_ref()
                        .and_then(|s| s.get("topLikers").cloned())
                        .unwrap_or_else(|| json!([])),
                );
                o.insert(
                    "topLiked".into(),
                    sns_stats
                        .as_ref()
                        .and_then(|s| s.get("topLiked").cloned())
                        .unwrap_or_else(|| json!([])),
                );
                sns_stats = Some(Value::Object(o));
            }
        }

        // contact info
        self.emit_progress("annual-report", "collecting contact info…", 85, 100);
        let contact_ids: Vec<String> = contact_stats.iter().map(|c| c.0.clone()).collect();
        let (names, avatars) = self.names_and_avatars_pub(&wcdb, &contact_ids);
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
            wcdb.avatar_urls(&serde_json::to_string(&list).unwrap())
                .ok()
                .and_then(|m| {
                    m.get(&raw_wxid)
                        .or_else(|| m.get(&cleaned))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
        };

        let mut core: Vec<(String, i64, i64)> = contact_stats.clone();
        core.sort_by_key(|a| std::cmp::Reverse(a.1 + a.2));
        let core_friends: Vec<Value> = core
            .iter()
            .take(3)
            .map(|(sid, sent, recv)| {
                let (name, avatar) = info(sid);
                let mut o = Map::new();
                o.insert("username".into(), json!(sid));
                o.insert("displayName".into(), json!(name));
                if let Some(a) = avatar {
                    o.insert("avatarUrl".into(), json!(a));
                }
                o.insert("messageCount".into(), json!(sent + recv));
                o.insert("sentCount".into(), json!(sent));
                o.insert("receivedCount".into(), json!(recv));
                Value::Object(o)
            })
            .collect();

        let monthly_top: Vec<Value> = (1..=12i64)
            .map(|month| {
                let (mut max_count, mut top) = (0i64, String::new());
                for (sid, m) in &monthly_stats {
                    let c = m.get(&month).copied().unwrap_or(0);
                    if c > max_count {
                        max_count = c;
                        top = sid.clone();
                    }
                }
                let (name, avatar) = if top.is_empty() {
                    ("暂无".to_string(), None)
                } else {
                    info(&top)
                };
                let mut o = Map::new();
                o.insert("month".into(), json!(month));
                o.insert("displayName".into(), json!(name));
                if let Some(a) = avatar {
                    o.insert("avatarUrl".into(), json!(a));
                }
                o.insert("messageCount".into(), json!(max_count));
                Value::Object(o)
            })
            .collect();

        let mut peak_day = Value::Null;
        let mut max_day = 0i64;
        for (day, count) in &daily_stats {
            if *count > max_day {
                max_day = *count;
                let (mut top_friend, mut top_count) = (String::new(), 0i64);
                if let Some(m) = daily_contact.get(day) {
                    for (sid, c) in m {
                        if *c > top_count || (*c == top_count && false) {
                            top_count = *c;
                            top_friend = info(sid).0;
                        }
                    }
                }
                peak_day = json!({ "date": day, "messageCount": count, "topFriend": top_friend, "topFriendCount": top_count });
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
            json!({ "displayName": info(&sid).0, "count": max_m, "percentage": round1(max_m as f64 / total_midnight as f64 * 100.0) })
        } else {
            Value::Null
        };

        let longest_streak = match (
            streak_sid.is_empty(),
            streak_days > 0,
            streak_start,
            streak_end,
        ) {
            (false, true, Some(s), Some(e)) => {
                json!({ "friendName": info(&streak_sid).0, "days": streak_days, "startDate": ymd(s), "endDate": ymd(e) })
            }
            _ => Value::Null,
        };

        let mut mutual = Value::Null;
        let mut best_diff = f64::INFINITY;
        for (sid, sent, recv) in &contact_stats {
            if *sent >= 50 && *recv >= 50 {
                let ratio = *sent as f64 / *recv as f64;
                let diff = (ratio - 1.0).abs();
                if diff < best_diff {
                    best_diff = diff;
                    let (name, avatar) = info(sid);
                    let mut o = Map::new();
                    o.insert("displayName".into(), json!(name));
                    if let Some(a) = avatar {
                        o.insert("avatarUrl".into(), json!(a));
                    }
                    o.insert("sentCount".into(), json!(sent));
                    o.insert("receivedCount".into(), json!(recv));
                    o.insert("ratio".into(), json!((ratio * 100.0).round() / 100.0));
                    mutual = Value::Object(o);
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
        let social = if total_conv > 0 {
            let mut o = Map::new();
            o.insert("initiatedChats".into(), json!(total_init));
            o.insert("receivedChats".into(), json!(total_recv));
            o.insert(
                "initiativeRate".into(),
                json!(round1(total_init as f64 / total_conv as f64 * 100.0)),
            );
            if top_init > 0 {
                o.insert("topInitiatedFriend".into(), json!(info(&top_init_sid).0));
                o.insert("topInitiatedCount".into(), json!(top_init));
            }
            Value::Object(o)
        } else {
            Value::Null
        };

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
            if cnt > 0.0 {
                json!({ "avgResponseTime": (sum / cnt).round() as i64, "fastestFriend": info(&fastest_id).0, "fastestTime": fastest.round() as i64 })
            } else {
                Value::Null
            }
        } else {
            let (mut all, mut fastest_id, mut fastest) =
                (Vec::<i64>::new(), String::new(), f64::INFINITY);
            for (sid, times) in &response_times {
                if times.len() >= 10 {
                    all.extend(times);
                    let avg = times.iter().sum::<i64>() as f64 / times.len() as f64;
                    if avg < fastest {
                        fastest = avg;
                        fastest_id = sid.clone();
                    }
                }
            }
            if all.is_empty() {
                Value::Null
            } else {
                let avg = all.iter().sum::<i64>() as f64 / all.len() as f64;
                json!({ "avgResponseTime": avg.round() as i64, "fastestFriend": info(&fastest_id).0, "fastestTime": fastest.round() as i64 })
            }
        };

        let top_phrases: Vec<Value> = match top_phrases_sql {
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
                    .map(|(p, c)| json!({ "phrase": p, "count": c }))
                    .collect()
            }
        };

        // "once close friend"
        let mut lost = Value::Null;
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
            if days.len() >= 2 {
                let first = parse_local_day(&days[0])
                    .map(|d| d.timestamp())
                    .unwrap_or(0);
                let last = parse_local_day(days.last().unwrap())
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
                let total = now.year() * 12 + now.month0() as i32 - back;
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
            let (name, avatar) = if contact_ids.contains(&best_sid) {
                info(&best_sid)
            } else {
                let (n, a) = self.names_and_avatars_pub(&wcdb, std::slice::from_ref(&best_sid));
                (
                    n.get(&best_sid)
                        .filter(|x| !x.is_empty())
                        .cloned()
                        .unwrap_or_else(|| best_sid.clone()),
                    a.get(&best_sid).cloned(),
                )
            };
            let mut o = Map::new();
            o.insert("username".into(), json!(best_sid));
            o.insert("displayName".into(), json!(name));
            if let Some(a) = avatar {
                o.insert("avatarUrl".into(), json!(a));
            }
            o.insert("earlyCount".into(), json!(best_early));
            o.insert("lateCount".into(), json!(best_late));
            o.insert("periodDesc".into(), json!(best_desc));
            lost = Value::Object(o);
        }

        let mut report = Map::new();
        report.insert("year".into(), json!(report_year));
        report.insert("totalMessages".into(), json!(total_messages));
        report.insert("totalFriends".into(), json!(contact_stats.len()));
        report.insert("coreFriends".into(), json!(core_friends));
        report.insert("monthlyTopFriends".into(), json!(monthly_top));
        report.insert("peakDay".into(), peak_day);
        report.insert("longestStreak".into(), longest_streak);
        report.insert("activityHeatmap".into(), json!({ "data": heatmap }));
        report.insert("midnightKing".into(), midnight_king);
        if let Some(a) = self_avatar {
            report.insert("selfAvatarUrl".into(), json!(a));
        }
        report.insert("mutualFriend".into(), mutual);
        report.insert("socialInitiative".into(), social);
        report.insert("responseSpeed".into(), response_speed);
        report.insert("topPhrases".into(), json!(top_phrases));
        if let Some(s) = sns_stats {
            report.insert("snsStats".into(), s);
        }
        report.insert("lostFriend".into(), lost);
        Ok(Value::Object(report))
    }

    fn aggregate_for_reports(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        ids: &[String],
        begin: i64,
        end: i64,
    ) -> anyhow::Result<Value> {
        let (b, e) = normalize_range(begin, end);
        wcdb.invoke_json(
            "wcdb_get_aggregate_stats",
            &[
                weflow_native::wcdb::Arg::S(&serde_json::to_string(ids).unwrap()),
                weflow_native::wcdb::Arg::I32(b),
                weflow_native::wcdb::Arg::I32(e),
            ],
        )
    }

    fn first_messages(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        session: &str,
        limit: usize,
        begin: i64,
        end: i64,
    ) -> Vec<Value> {
        let safe_begin = begin.max(0);
        let safe_end = if end > 0 {
            end
        } else {
            chrono::Utc::now().timestamp()
        };
        let Ok(cursor) = wcdb.open_message_cursor(
            session,
            limit.max(1) as i32,
            true,
            safe_begin.clamp(0, i32::MAX as i64) as i32,
            safe_end.clamp(0, i32::MAX as i64) as i32,
            false,
        ) else {
            return Vec::new();
        };
        let mut rows: Vec<Value> = Vec::new();
        loop {
            let Ok((batch, more)) = wcdb.fetch_message_batch(cursor) else {
                break;
            };
            let Some(list) = batch.as_array() else { break };
            for r in list {
                rows.push(r.clone());
                if rows.len() >= limit {
                    break;
                }
            }
            if !more || rows.len() >= limit {
                break;
            }
        }
        let _ = wcdb.close_message_cursor(cursor);
        rows.truncate(limit);
        rows
    }

    /// `dualReport:generateReport`
    pub fn report_dual(
        &self,
        friend: &str,
        year: i32,
        exclude_words: &[String],
    ) -> AppResult<Value> {
        self.emit_progress("dual-report", "connecting…", 5, 100);
        let (wcdb, cleaned, raw_wxid) = self.report_connect()?;
        let report_year = year.max(0);
        let is_all = report_year == 0;
        let (start, end) = year_range(report_year);

        let display = |u: &str, fb: &str| -> String {
            wcdb.display_names(&serde_json::to_string(&[u]).unwrap())
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
        let avatars = wcdb
            .avatar_urls(&serde_json::to_string(&candidates).unwrap())
            .ok();
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

        let message_view = |row: &Value| -> Value {
            let create_ms = num(&row["create_time"]) * 1000;
            let raw_content = decode_message_content(row);
            let local_type = row_int(
                row,
                &["local_type", "localType", "type", "msg_type", "msgType"],
                0,
            );
            let (mut md5, mut url) = (None, None);
            if local_type == 47 {
                let stripped = strip_emoji_owner_prefix(&raw_content);
                md5 = normalize_emoji_md5(&coerce_string(record_field(
                    row,
                    &["emoji_md5", "emojiMd5", "md5"],
                )))
                .or_else(|| extract_emoji_md5(&stripped));
                url = normalize_emoji_url(&coerce_string(record_field(
                    row,
                    &["emoji_cdn_url", "emojiCdnUrl", "cdnurl"],
                )))
                .or_else(|| extract_emoji_url(&stripped));
            }
            let mut o = Map::new();
            o.insert("content".into(), json!(raw_content));
            o.insert(
                "isSentByMe".into(),
                json!(resolve_is_sent(row, &raw_wxid, &cleaned)),
            );
            o.insert("createTime".into(), json!(create_ms));
            o.insert("createTimeStr".into(), json!(format_date_time(create_ms)));
            o.insert("localType".into(), json!(local_type));
            if let Some(m) = md5 {
                o.insert("emojiMd5".into(), json!(m));
            }
            if let Some(u) = url {
                o.insert("emojiCdnUrl".into(), json!(u));
            }
            Value::Object(o)
        };

        self.emit_progress("dual-report", "first messages…", 15, 100);
        let first_rows = self.first_messages(&wcdb, friend, 10, 0, 0);
        let first_chat = first_rows.first().map(|row| {
            let mut v = message_view(row);
            let o = v.as_object_mut().unwrap();
            if let Some(s) = ["sender_username", "sender"]
                .iter()
                .filter_map(|k| row.get(*k))
                .find(|v| !v.is_null())
            {
                // `senderUsername` sits after isSentByMe in the desktop object
                let mut rebuilt = Map::new();
                for (k, val) in o.iter() {
                    rebuilt.insert(k.clone(), val.clone());
                    if k == "isSentByMe" {
                        rebuilt.insert("senderUsername".into(), s.clone());
                    }
                }
                return Value::Object(rebuilt);
            }
            v
        });
        let first_chat_messages: Vec<Value> = first_rows.iter().map(&message_view).collect();

        let mut year_first_chat = Value::Null;
        if !is_all {
            let rows = self.first_messages(&wcdb, friend, 10, start, end);
            if let Some(first) = rows.first() {
                let view = message_view(first);
                let create_ms = num(&first["create_time"]) * 1000;
                let mut o = Map::new();
                o.insert("createTime".into(), json!(create_ms));
                o.insert("createTimeStr".into(), json!(format_date_time(create_ms)));
                o.insert("content".into(), view["content"].clone());
                o.insert("isSentByMe".into(), view["isSentByMe"].clone());
                o.insert("friendName".into(), json!(friend_name));
                o.insert(
                    "firstThreeMessages".into(),
                    json!(rows.iter().map(&message_view).collect::<Vec<_>>()),
                );
                o.insert("localType".into(), view["localType"].clone());
                for k in ["emojiMd5", "emojiCdnUrl"] {
                    if let Some(v) = view.get(k) {
                        o.insert(k.into(), v.clone());
                    }
                }
                year_first_chat = Value::Object(o);
            }
        }

        self.emit_progress("dual-report", "chat statistics…", 30, 100);
        let (b, e) = normalize_range(start, end);
        let cpp = wcdb.dual_report_stats(friend, b, e).map_err(|err| {
            AppError::native(format!("failed to get dual-report statistics: {err}"))
        })?;
        let counts = cpp.get("counts").cloned().unwrap_or_else(|| json!({}));
        let mut stats = Map::new();
        stats.insert("totalMessages".into(), json!(num(&counts["total"])));
        stats.insert("totalWords".into(), json!(num(&counts["words"])));
        stats.insert("imageCount".into(), json!(num(&counts["image"])));
        stats.insert("voiceCount".into(), json!(num(&counts["voice"])));
        stats.insert("emojiCount".into(), json!(num(&counts["emoji"])));
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
            // fallback: scan the conversation
            let mut tally: Vec<(bool, String, Option<String>, i64)> = Vec::new();
            if let Ok(cursor) = wcdb.open_message_cursor(
                friend,
                500,
                true,
                start.clamp(0, i32::MAX as i64) as i32,
                end.clamp(0, i32::MAX as i64) as i32,
                false,
            ) {
                loop {
                    let Ok((rows, more)) = wcdb.fetch_message_batch(cursor) else {
                        break;
                    };
                    let Some(rows) = rows.as_array() else { break };
                    for row in rows {
                        if row_int(
                            row,
                            &[
                                "local_type",
                                "localType",
                                "type",
                                "msg_type",
                                "msgType",
                                "WCDB_CT_local_type",
                            ],
                            0,
                        ) != 47
                        {
                            continue;
                        }
                        let content = strip_emoji_owner_prefix(&decode_message_content(row));
                        let md5 = normalize_emoji_md5(&coerce_string(record_field(
                            row,
                            &["emoji_md5", "emojiMd5", "md5"],
                        )))
                        .or_else(|| extract_emoji_md5(&content));
                        let Some(md5) = md5 else { continue };
                        let url = normalize_emoji_url(&coerce_string(record_field(
                            row,
                            &[
                                "emoji_cdn_url",
                                "emojiCdnUrl",
                                "cdnurl",
                                "cdn_url",
                                "emoji_url",
                                "emojiUrl",
                                "url",
                                "thumburl",
                                "thumb_url",
                            ],
                        )))
                        .or_else(|| extract_emoji_url(&content));
                        let me = resolve_is_sent(row, &raw_wxid, &cleaned);
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
        let db_path = self
            .connection_inputs()
            .map(|(dir, _, _)| dir.to_string_lossy().to_string())
            .unwrap_or_default();
        if my_url.is_none() {
            if let Some(m) = &my_md5 {
                if let Ok(u) = wcdb.emoticon_cdn_url(&db_path, m) {
                    if !u.is_empty() {
                        my_url = Some(u);
                    }
                }
            }
        }
        if fr_url.is_none() {
            if let Some(m) = &fr_md5 {
                if let Ok(u) = wcdb.emoticon_cdn_url(&db_path, m) {
                    if !u.is_empty() {
                        fr_url = Some(u);
                    }
                }
            }
        }
        for (k, v) in [
            ("myTopEmojiMd5", &my_md5),
            ("myTopEmojiUrl", &my_url),
            ("friendTopEmojiMd5", &fr_md5),
            ("friendTopEmojiUrl", &fr_url),
        ] {
            if let Some(v) = v {
                stats.insert(k.into(), json!(v));
            }
        }
        if my_count >= 0 {
            stats.insert("myTopEmojiCount".into(), json!(my_count));
        }
        if fr_count >= 0 {
            stats.insert("friendTopEmojiCount".into(), json!(fr_count));
        }

        let exclude: HashSet<&str> = exclude_words.iter().map(String::as_str).collect();
        let filter = |key: &str| -> Vec<Value> {
            cpp.get(key)
                .and_then(Value::as_array)
                .map(|a| a.as_slice())
                .unwrap_or(&[])
                .iter()
                .filter(|p| !exclude.contains(p["phrase"].as_str().unwrap_or("")))
                .cloned()
                .collect()
        };
        let (clean, clean_my, clean_fr) = (
            filter("phrases"),
            filter("myPhrases"),
            filter("friendPhrases"),
        );
        let top_phrases: Vec<Value> = clean
            .iter()
            .map(|p| json!({ "phrase": p["phrase"], "count": p["count"] }))
            .collect();
        let phrase_map = |list: &[Value]| -> Vec<(String, i64)> {
            list.iter()
                .map(|p| {
                    (
                        p["phrase"].as_str().unwrap_or("").to_string(),
                        num(&p["count"]),
                    )
                })
                .collect()
        };
        let (my_map, fr_map) = (phrase_map(&clean_my), phrase_map(&clean_fr));
        let lookup = |m: &[(String, i64)], k: &str| {
            m.iter()
                .rev()
                .find(|(p, _)| p == k)
                .map(|(_, c)| *c)
                .unwrap_or(0)
        };
        let exclusive = |own: &[(String, i64)], other: &[(String, i64)]| -> Vec<Value> {
            let mut seen = HashSet::new();
            let mut v: Vec<(String, i64)> = Vec::new();
            for (phrase, _) in own {
                if !seen.insert(phrase.clone()) {
                    continue;
                }
                let c = lookup(own, phrase);
                let total = c + lookup(other, phrase);
                if c >= 2 && total > 0 && c as f64 / total as f64 >= 0.75 {
                    v.push((phrase.clone(), c));
                }
            }
            v.sort_by_key(|a| std::cmp::Reverse(a.1));
            v.truncate(20);
            v.into_iter()
                .map(|(p, c)| json!({ "phrase": p, "count": c }))
                .collect()
        };
        let my_excl = exclusive(&my_map, &fr_map);
        let fr_excl = exclusive(&fr_map, &my_map);

        let mut report = Map::new();
        report.insert("year".into(), json!(report_year));
        report.insert("selfName".into(), json!(my_name));
        if let Some(a) = self_avatar {
            report.insert("selfAvatarUrl".into(), json!(a));
        }
        report.insert("friendUsername".into(), json!(friend));
        report.insert("friendName".into(), json!(friend_name));
        if let Some(a) = friend_avatar {
            report.insert("friendAvatarUrl".into(), json!(a));
        }
        report.insert("firstChat".into(), first_chat.unwrap_or(Value::Null));
        report.insert("firstChatMessages".into(), json!(first_chat_messages));
        report.insert("yearFirstChat".into(), year_first_chat);
        report.insert("stats".into(), Value::Object(stats));
        report.insert("topPhrases".into(), json!(top_phrases));
        report.insert("myExclusivePhrases".into(), json!(my_excl));
        report.insert("friendExclusivePhrases".into(), json!(fr_excl));
        for (k, src) in [
            ("heatmap", "heatmap"),
            ("initiative", "initiative"),
            ("response", "response"),
            ("monthly", "monthly"),
            ("streak", "streak"),
        ] {
            if let Some(v) = cpp.get(src).filter(|v| !v.is_null()) {
                report.insert(k.into(), v.clone());
            }
        }
        self.emit_progress("dual-report", "done", 100, 100);
        Ok(Value::Object(report))
    }
}
