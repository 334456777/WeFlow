//! Message push (`electron/services/messagePushService.ts`).
//!
//! The desktop app reacts to WCDB database-change notifications; the CLI has no such callback,
//! so `message_push_loop` runs the same session-diff / revoke-detection pass on a timer.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use tokio::sync::broadcast;

use crate::chat_msg::{self, ChatMessage};
use crate::message::rx;
use crate::services::ServiceHub;

pub struct EventChannel {
    sender: broadcast::Sender<Value>,
}

impl EventChannel {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn sender(&self) -> broadcast::Sender<Value> {
        self.sender.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.sender.subscribe()
    }
}

const LOOKBACK_SECONDS: i64 = 2;
const RECENT_TTL: Duration = Duration::from_secs(10 * 60);
const GROUP_NICKNAME_TTL: Duration = Duration::from_secs(5 * 60);
const RECENT_REVOKE_SCAN_SECONDS: i64 = 150;
const DIRECT_REVOKE_SCAN_LIMIT: usize = 20;

#[derive(Clone, Copy, Debug, Default)]
struct Baseline {
    last_timestamp: i64,
    unread_count: i64,
}

#[derive(Default)]
struct PushResult {
    max_fetched_timestamp: i64,
    retry: bool,
}

#[derive(Clone, Debug)]
struct Session {
    username: String,
    last_timestamp: i64,
    unread_count: i64,
    last_msg_type: i64,
    summary: String,
    display_name: String,
    avatar_url: Option<String>,
    last_sender_display_name: Option<String>,
}

impl Session {
    fn from_value(v: &Value) -> Self {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Self {
            username: s("username").unwrap_or_default().trim().to_string(),
            last_timestamp: v.get("lastTimestamp").and_then(Value::as_i64).unwrap_or(0),
            unread_count: v.get("unreadCount").and_then(Value::as_i64).unwrap_or(0),
            last_msg_type: v.get("lastMsgType").and_then(Value::as_i64).unwrap_or(0),
            summary: s("summary").unwrap_or_default(),
            display_name: s("displayName").unwrap_or_default(),
            avatar_url: s("avatarUrl").filter(|a| !a.is_empty()),
            last_sender_display_name: s("lastSenderDisplayName").filter(|a| !a.is_empty()),
        }
    }
}

/// `normalizeMessageIdToken`
fn normalize_id_token(value: &str) -> String {
    let raw = value.trim();
    if raw.is_empty() {
        return String::new();
    }
    let numeric = if rx(r"^-?\d+$").is_match(raw) {
        let t = raw.trim_start_matches('-').trim_start_matches('0');
        if t.is_empty() {
            "0".to_string()
        } else {
            t.to_string()
        }
    } else {
        raw.to_string()
    };
    if numeric == "0" {
        String::new()
    } else {
        numeric
    }
}

fn push_xml_value(xml: &str, tag: &str) -> String {
    let decoded = xml.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&");
    let re = rx(&format!(r"(?is)<{0}>(.*?)</{0}>", regex::escape(tag)));
    re.captures(&decoded).map(|c| c[1].replace("<![CDATA[", "").replace("]]>", "").trim().to_string()).unwrap_or_default()
}

fn is_revoke_system_message(m: &ChatMessage) -> bool {
    let content = format!("{}\n{}", m.raw_content, m.parsed_content);
    if content.contains("revokemsg") || content.contains("<replacemsg") {
        return true;
    }
    if content.contains("撤回了一条消息") || content.contains("尝试撤回此消息") {
        return true;
    }
    (m.local_type == 10000 || m.local_type == 10002) && content.contains("撤回")
}

fn is_self_revoke(m: &ChatMessage) -> bool {
    format!("{}\n{}", m.raw_content, m.parsed_content).contains("你撤回")
}

fn is_revoke_session_summary(s: &Session) -> bool {
    s.last_msg_type == 10002 || s.summary.trim().contains("撤回了一条消息") || s.summary.trim().contains("尝试撤回此消息")
}

fn message_id_tokens(m: &ChatMessage) -> HashSet<String> {
    let mut tokens = HashSet::new();
    let mut add = |v: &str| {
        let n = normalize_id_token(v);
        if !n.is_empty() {
            tokens.insert(n);
        }
    };
    add(&m.server_id_raw);
    add(&m.server_id.to_string());
    add(&m.local_id.to_string());
    for tag in ["newmsgid", "msgid", "oldmsgid", "svrid"] {
        add(&push_xml_value(&m.raw_content, tag));
    }
    tokens
}

fn extract_revoked_message_id(m: &ChatMessage) -> Option<String> {
    let content = if !m.raw_content.is_empty() { &m.raw_content } else { &m.parsed_content };
    let candidates = [
        push_xml_value(content, "newmsgid"),
        push_xml_value(content, "msgid"),
        push_xml_value(content, "oldmsgid"),
        push_xml_value(content, "svrid"),
        m.server_id_raw.clone(),
        m.server_id.to_string(),
    ];
    candidates.iter().map(|c| normalize_id_token(c)).find(|c| !c.is_empty())
}

fn compare_position(a: &ChatMessage, b: &ChatMessage) -> std::cmp::Ordering {
    a.create_time.cmp(&b.create_time).then(a.sort_seq.cmp(&b.sort_seq)).then(a.local_id.cmp(&b.local_id)).then_with(|| a.message_key.cmp(&b.message_key))
}

fn message_display_content(m: &ChatMessage) -> Option<String> {
    let normalize_text = |v: &str| -> Option<String> {
        if v.is_empty() {
            return None;
        }
        let re = rx(r"(?i)^\s*([a-zA-Z0-9_@-]+):(?:\s*(?:\r?\n|<br\s*/?>)\s*|\s*)");
        let text = match re.find(v) {
            Some(mm) if !v[mm.end()..].starts_with("//") => v[mm.end()..].to_string(),
            _ => v.to_string(),
        };
        Some(text.trim().to_string())
    };
    let clean_official = |v: Option<String>| -> Option<String> {
        let v = v?;
        let cleaned = rx(r"^\s*\[视频号\]\s*").replace(&v, "").trim().to_string();
        Some(if cleaned.is_empty() { v } else { cleaned })
    };
    let text_source = if !m.parsed_content.is_empty() { &m.parsed_content } else { &m.raw_content };
    match m.local_type {
        1 => clean_official(normalize_text(text_source)),
        3 => Some("[图片]".into()),
        34 => Some("[语音]".into()),
        43 => Some("[视频]".into()),
        47 => Some("[表情]".into()),
        42 => clean_official(Some(m.card_nickname.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| "[名片]".into()))),
        48 => Some("[位置]".into()),
        49 => clean_official(Some(m.link_title().filter(|t| !t.is_empty()).or_else(|| m.file_name().filter(|t| !t.is_empty())).unwrap_or("[消息]").to_string())),
        _ => clean_official(normalize_text(text_source).filter(|s| !s.is_empty())),
    }
}

fn session_type(session_id: &str) -> &'static str {
    if session_id.ends_with("@chatroom") {
        "group"
    } else if session_id.starts_with("gh_") {
        "official"
    } else {
        // the desktop app compares a numeric `type` with 'friend' / 'official', which never matches
        "other"
    }
}

pub struct PushEngine {
    hub: ServiceHub,
    baseline: HashMap<String, Baseline>,
    recent: HashMap<String, Instant>,
    seen: HashMap<String, Instant>,
    recently_revoked: HashMap<String, Instant>,
    seen_primed: HashSet<String>,
    group_nicknames: HashMap<String, (HashMap<String, String>, Instant)>,
    avatar_data_cache: HashMap<String, String>,
    baseline_ready: bool,
}

fn prune(map: &mut HashMap<String, Instant>) {
    let now = Instant::now();
    map.retain(|_, t| now.duration_since(*t) <= RECENT_TTL);
}

impl PushEngine {
    pub fn new(hub: ServiceHub) -> Self {
        Self {
            hub,
            baseline: HashMap::new(),
            recent: HashMap::new(),
            seen: HashMap::new(),
            recently_revoked: HashMap::new(),
            seen_primed: HashSet::new(),
            group_nicknames: HashMap::new(),
            avatar_data_cache: HashMap::new(),
            baseline_ready: false,
        }
    }

    fn filter_allows(&self, session_id: &str) -> bool {
        let mode = self.hub.config_value("messagePushFilterMode");
        let mode = mode.as_str().unwrap_or("all");
        if mode != "whitelist" && mode != "blacklist" {
            return true;
        }
        let listed = self.hub.config_value("messagePushFilterList").as_array().map_or(false, |l| l.iter().any(|i| i.as_str().map(str::trim) == Some(session_id)));
        if mode == "whitelist" {
            listed
        } else {
            !listed
        }
    }

    fn sessions(&self) -> Option<Vec<Session>> {
        self.hub.chat_sessions_list().ok().map(|s| s.iter().map(Session::from_value).collect())
    }

    fn set_baseline(&mut self, sessions: &[Session]) {
        let previous = std::mem::take(&mut self.baseline);
        let now = chrono::Utc::now().timestamp();
        for s in sessions {
            if s.username.is_empty() {
                continue;
            }
            let prev = previous.get(&s.username);
            let initial = if s.last_timestamp > 0 { s.last_timestamp } else { now };
            self.baseline.insert(
                s.username.clone(),
                Baseline {
                    last_timestamp: s.last_timestamp.max(prev.map_or(0, |p| p.last_timestamp)).max(if prev.is_some() { 0 } else { initial }),
                    unread_count: s.unread_count,
                },
            );
        }
    }

    /// `bootstrapBaseline`
    pub fn bootstrap(&mut self) -> bool {
        match self.sessions() {
            Some(s) => {
                self.set_baseline(&s);
                self.baseline_ready = true;
                true
            }
            None => false,
        }
    }

    fn should_inspect(&self, prev: Option<&Baseline>, s: &Session) -> bool {
        if s.username.is_empty() || s.username.to_lowercase().contains("placeholder_foldgroup") {
            return false;
        }
        let Some(prev) = prev else { return s.unread_count > 0 && s.last_timestamp > 0 };
        if is_revoke_session_summary(s) && s.last_timestamp >= prev.last_timestamp {
            return true;
        }
        s.last_timestamp > prev.last_timestamp || s.unread_count != prev.unread_count
    }

    fn should_scan_message_backed(&self, prev: Option<&Baseline>, s: &Session) -> bool {
        if s.username.is_empty() || s.username.to_lowercase().contains("placeholder_foldgroup") {
            return false;
        }
        if session_type(&s.username) == "private" && !is_revoke_session_summary(s) {
            return false;
        }
        prev.is_some() || s.last_timestamp > 0
    }

    /// One `flushPendingChanges` pass. Returns the payloads that should be broadcast.
    pub fn flush(&mut self, scan_message_backed: bool) -> Vec<Value> {
        let mut out = Vec::new();
        let Some(sessions) = self.sessions() else { return out };
        if !self.baseline_ready {
            self.set_baseline(&sessions);
            self.baseline_ready = true;
            return out;
        }
        let previous = self.baseline.clone();
        let mut candidate_ids: HashSet<String> = HashSet::new();
        let candidates: Vec<&Session> = sessions
            .iter()
            .filter(|s| {
                let prev = previous.get(&s.username);
                self.should_inspect(prev, s) || (scan_message_backed && self.should_scan_message_backed(prev, s))
            })
            .collect();
        for s in candidates {
            if !s.username.is_empty() {
                candidate_ids.insert(s.username.clone());
            }
            let prev = previous.get(&s.username).or_else(|| self.baseline.get(&s.username)).copied();
            let decreased = prev.map_or(false, |p| s.unread_count < p.unread_count);
            let changed = prev.map_or(false, |p| s.unread_count != p.unread_count);
            let scan_revokes = decreased || (changed && is_revoke_session_summary(s));
            let result = self.push_session_messages(s, prev.as_ref(), scan_revokes, &mut out);
            self.update_inspected_baseline(s, previous.get(&s.username), &result);
        }
        for s in &sessions {
            if s.username.is_empty() || candidate_ids.contains(&s.username) {
                continue;
            }
            let prev = previous.get(&s.username);
            self.baseline.insert(
                s.username.clone(),
                Baseline { last_timestamp: s.last_timestamp.max(prev.map_or(0, |p| p.last_timestamp)), unread_count: s.unread_count },
            );
        }
        out
    }

    fn update_inspected_baseline(&mut self, s: &Session, prev: Option<&Baseline>, result: &PushResult) {
        if s.username.is_empty() {
            return;
        }
        let prev_ts = prev.map_or(0, |p| p.last_timestamp);
        let current = self.baseline.get(&s.username).copied().or(prev.copied()).unwrap_or_default();
        let next_ts = if result.retry { prev_ts } else { prev_ts.max(current.last_timestamp).max(result.max_fetched_timestamp) };
        self.baseline.insert(
            s.username.clone(),
            Baseline { last_timestamp: next_ts, unread_count: if result.retry { prev.map_or(0, |p| p.unread_count) } else { s.unread_count } },
        );
    }

    fn remember(map: &mut HashMap<String, Instant>, key: &str) {
        map.insert(key.to_string(), Instant::now());
        prune(map);
    }

    fn is_fresh(map: &mut HashMap<String, Instant>, key: &str) -> bool {
        prune(map);
        map.get(key).map_or(false, |t| t.elapsed() < RECENT_TTL)
    }

    fn bump_baseline(&mut self, session_id: &str, m: &ChatMessage) {
        let key = session_id.trim();
        if key.is_empty() || m.create_time <= 0 {
            return;
        }
        let cur = self.baseline.get(key).copied().unwrap_or_default();
        if m.create_time > cur.last_timestamp {
            self.baseline.insert(key.to_string(), Baseline { last_timestamp: m.create_time, ..cur });
        }
    }

    fn push_session_messages(&mut self, session: &Session, previous: Option<&Baseline>, scan_recent_revokes: bool, out: &mut Vec<Value>) -> PushResult {
        let prev_ts = previous.map_or(0, |p| p.last_timestamp.max(0));
        let prev_unread = previous.map_or(0, |p| p.unread_count.max(0));
        let current_unread = session.unread_count.max(0);
        let expected_incoming = if previous.is_some() { (current_unread - prev_unread).max(0) } else { 0 };
        let since = if previous.is_some() { (prev_ts - LOOKBACK_SECONDS).max(0) } else { 0 };
        let fetched: Vec<ChatMessage> = self.hub.chat_new_messages(&session.username, since, 1000).unwrap_or_default();
        if fetched.is_empty() && !scan_recent_revokes {
            return PushResult { max_fetched_timestamp: prev_ts, retry: expected_incoming > 0 };
        }
        let session_id = session.username.clone();
        let max_fetched = fetched.iter().map(|m| m.create_time).filter(|t| *t > prev_ts).max().unwrap_or(prev_ts);
        let seen_primed = self.seen_primed.contains(&session_id);
        let mut same_ts_incoming: Vec<&ChatMessage> = Vec::new();
        let mut candidates: Vec<&ChatMessage> = Vec::new();
        let mut observed_incoming = 0i64;

        for m in &fetched {
            let key = m.message_key.trim();
            if key.is_empty() {
                continue;
            }
            let create_time = m.create_time;
            let seen = Self::is_fresh(&mut self.seen, key);
            let recent = Self::is_fresh(&mut self.recent, key);
            let revoke = is_revoke_system_message(m);
            if m.is_send != Some(1) && (previous.is_none() || create_time > prev_ts || (seen_primed && create_time == prev_ts)) {
                observed_incoming += 1;
            }
            if previous.is_some() && !seen_primed && create_time < prev_ts {
                if revoke && !recent {
                    candidates.push(m);
                    continue;
                }
                Self::remember(&mut self.seen, key);
                continue;
            }
            if seen || recent {
                if seen && !recent && revoke {
                    candidates.push(m);
                }
                continue;
            }
            if m.is_send == Some(1) {
                continue;
            }
            if previous.is_some() && !seen_primed && create_time == prev_ts {
                if revoke {
                    candidates.push(m);
                } else {
                    same_ts_incoming.push(m);
                }
                continue;
            }
            candidates.push(m);
        }

        let future_incoming = candidates.iter().filter(|m| previous.is_none() || m.create_time > prev_ts || seen_primed).count() as i64;
        let allowance = if previous.is_some() && !seen_primed { (expected_incoming - future_incoming).max(0) as usize } else { 0 };
        let selected_same: Vec<&ChatMessage> = if allowance > 0 { same_ts_incoming.iter().rev().take(allowance).rev().copied().collect() } else { Vec::new() };
        let to_push: Vec<&ChatMessage> = selected_same.into_iter().chain(candidates).collect();

        // normal messages whose own revoke notice is also in this batch are suppressed
        let push_keys: HashSet<&str> = to_push.iter().map(|m| m.message_key.trim()).filter(|k| !k.is_empty()).collect();
        let mut suppressed: HashSet<String> = HashSet::new();
        for m in &to_push {
            if !is_revoke_system_message(m) {
                continue;
            }
            if let Some(orig) = Self::find_revoked_in(&fetched, m, extract_revoked_message_id(m).as_deref()) {
                let k = orig.message_key.trim();
                if !k.is_empty() && push_keys.contains(k) {
                    suppressed.insert(k.to_string());
                }
            }
        }
        for m in to_push {
            let key = m.message_key.trim().to_string();
            if key.is_empty() {
                continue;
            }
            let revoke = is_revoke_system_message(m);
            if !revoke && suppressed.contains(&key) {
                Self::remember(&mut self.recent, &key);
                continue;
            }
            if !revoke && self.is_recently_revoked_original(&session.username, m) {
                Self::remember(&mut self.recent, &key);
                Self::remember(&mut self.seen, &key);
                continue;
            }
            let payload = if revoke { self.build_revoke_payload(session, m, &fetched) } else { self.build_payload(session, m) };
            let Some(payload) = payload else { continue };
            if !self.filter_allows(payload["sessionId"].as_str().unwrap_or("")) {
                continue;
            }
            out.push(payload);
            Self::remember(&mut self.recent, &key);
            self.bump_baseline(&session.username, m);
        }
        for m in &fetched {
            let k = m.message_key.trim();
            if !k.is_empty() {
                Self::remember(&mut self.seen, k);
            }
        }
        if !session_id.is_empty() {
            self.seen_primed.insert(session_id.clone());
        }

        let (mut pushed_count, mut max_pushed) = (0usize, 0i64);
        if scan_recent_revokes {
            let since = self.recent_revoke_scan_since(session, previous);
            let revokes = self.recent_revoke_messages(&session_id, since);
            if !revokes.is_empty() {
                let mut merged: Vec<ChatMessage> = Vec::new();
                let mut keys = HashSet::new();
                for m in fetched.iter().chain(revokes.iter()) {
                    let k = m.message_key.trim().to_string();
                    if !k.is_empty() && !keys.insert(k) {
                        continue;
                    }
                    merged.push(m.clone());
                }
                for m in &revokes {
                    let key = m.message_key.trim().to_string();
                    if key.is_empty() || !is_revoke_system_message(m) || Self::is_fresh(&mut self.recent, &key) {
                        continue;
                    }
                    let Some(payload) = self.build_revoke_payload(session, m, &merged) else { continue };
                    if !self.filter_allows(payload["sessionId"].as_str().unwrap_or("")) {
                        continue;
                    }
                    out.push(payload);
                    Self::remember(&mut self.recent, &key);
                    Self::remember(&mut self.seen, &key);
                    self.bump_baseline(&session_id, m);
                    pushed_count += 1;
                    max_pushed = max_pushed.max(m.create_time);
                }
            }
        }
        let _ = pushed_count;
        PushResult { max_fetched_timestamp: max_fetched.max(max_pushed), retry: expected_incoming > 0 && observed_incoming < expected_incoming }
    }

    fn recent_revoke_scan_since(&self, session: &Session, previous: Option<&Baseline>) -> i64 {
        let now = chrono::Utc::now().timestamp();
        let anchor = now.max(session.last_timestamp).max(previous.map_or(0, |p| p.last_timestamp));
        (anchor - RECENT_REVOKE_SCAN_SECONDS).max(0)
    }

    fn candidate_tables(&self, wcdb: &weflow_native::wcdb::Wcdb, session_id: &str, since: i64) -> Vec<(String, String, i64)> {
        let Ok(Value::Array(tables)) = wcdb.message_table_stats(session_id) else { return Vec::new() };
        let mut v: Vec<(String, String, i64)> = tables
            .iter()
            .map(|t| {
                let g = |keys: &[&str]| keys.iter().filter_map(|k| t.get(*k)).find_map(|x| x.as_str().map(str::to_string)).unwrap_or_default().trim().to_string();
                let last = ["last_time", "lastTime"].iter().filter_map(|k| t.get(*k)).find_map(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0.0) as i64;
                (g(&["db_path", "dbPath"]), g(&["table_name", "tableName"]), last)
            })
            .filter(|(db, table, last)| !db.is_empty() && !table.is_empty() && (since <= 0 || *last >= since))
            .collect();
        v.sort_by(|a, b| b.2.cmp(&a.2));
        v
    }

    fn query_rows(&self, wcdb: &weflow_native::wcdb::Wcdb, db_path: &str, sql: &str) -> Vec<Value> {
        match wcdb.exec_query("message", db_path, sql) {
            Ok(Value::Array(rows)) => rows,
            Ok(v) => v.get("rows").and_then(Value::as_array).cloned().unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    fn recent_revoke_messages(&self, session_id: &str, since: i64) -> Vec<ChatMessage> {
        let Ok(wcdb) = self.hub.open_wcdb_pub() else { return Vec::new() };
        let my = self.hub.my_wxid_pub();
        let mut msgs = Vec::new();
        for (db_path, table, _) in self.candidate_tables(&wcdb, session_id, since) {
            let sql = format!(
                "SELECT *, '{}' AS _db_path, '{}' AS table_name FROM \"{}\" WHERE create_time >= {} AND (local_type IN (10000, 10002) OR message_content LIKE '%撤回%' OR message_content LIKE '%revokemsg%' OR message_content LIKE '%<replacemsg%' OR compress_content LIKE '%撤回%' OR compress_content LIKE '%revokemsg%') ORDER BY create_time ASC, sort_seq ASC, local_id ASC LIMIT {}",
                db_path.replace('\'', "''"),
                table.replace('\'', "''"),
                table.replace('"', "\"\""),
                since.max(0),
                DIRECT_REVOKE_SCAN_LIMIT
            );
            msgs.extend(chat_msg::map_rows(&self.query_rows(&wcdb, &db_path, &sql), &my));
        }
        msgs.retain(is_revoke_system_message);
        msgs.sort_by(compare_position);
        msgs
    }

    fn recent_context_messages(&self, session_id: &str, since: i64) -> Vec<ChatMessage> {
        let Ok(wcdb) = self.hub.open_wcdb_pub() else { return Vec::new() };
        let my = self.hub.my_wxid_pub();
        let mut msgs = Vec::new();
        for (db_path, table, _) in self.candidate_tables(&wcdb, session_id, since) {
            let sql = format!(
                "SELECT *, '{}' AS _db_path, '{}' AS table_name FROM \"{}\" WHERE create_time >= {} ORDER BY create_time ASC, sort_seq ASC, local_id ASC LIMIT {}",
                db_path.replace('\'', "''"),
                table.replace('\'', "''"),
                table.replace('"', "\"\""),
                since.max(0),
                DIRECT_REVOKE_SCAN_LIMIT * 4
            );
            msgs.extend(chat_msg::map_rows(&self.query_rows(&wcdb, &db_path, &sql), &my));
        }
        msgs.sort_by(compare_position);
        msgs
    }

    fn find_by_server_id_direct(&self, session_id: &str, revoke: &ChatMessage, server_id: &str) -> Option<ChatMessage> {
        let id = normalize_id_token(server_id);
        if id.is_empty() {
            return None;
        }
        let wcdb = self.hub.open_wcdb_pub().ok()?;
        let my = self.hub.my_wxid_pub();
        let source = parse_message_key_source(&revoke.message_key);
        let tables: Vec<(String, String)> = match source {
            Some(s) => vec![s],
            None => self.candidate_tables(&wcdb, session_id, (revoke.create_time - 5 * 60).max(0)).into_iter().map(|t| (t.0, t.1)).collect(),
        };
        for (db_path, table) in tables {
            let col = "\"server_id\"";
            let pred = if id.chars().all(|c| c.is_ascii_digit()) { format!("({col} = {id} OR CAST({col} AS TEXT) = '{id}')") } else { format!("CAST({col} AS TEXT) = '{}'", id.replace('\'', "''")) };
            let local_filter = if revoke.local_id > 0 { format!("AND local_id <> {}", revoke.local_id) } else { String::new() };
            let sql = format!(
                "SELECT *, '{}' AS _db_path, '{}' AS table_name FROM \"{}\" WHERE {pred} {local_filter} AND local_type NOT IN (10000, 10002) ORDER BY local_id ASC LIMIT 1",
                db_path.replace('\'', "''"),
                table.replace('\'', "''"),
                table.replace('"', "\"\"")
            );
            if let Some(m) = chat_msg::map_rows(&self.query_rows(&wcdb, &db_path, &sql), &my).into_iter().next() {
                if !is_revoke_system_message(&m) {
                    return Some(m);
                }
            }
        }
        None
    }

    fn find_revoked_in<'a>(messages: &'a [ChatMessage], revoke: &ChatMessage, revoked_id: Option<&str>) -> Option<&'a ChatMessage> {
        if let Some(id) = revoked_id {
            let target = normalize_id_token(id);
            if !target.is_empty() {
                if let Some(m) = messages.iter().find(|m| m.message_key != revoke.message_key && !is_revoke_system_message(m) && message_id_tokens(m).contains(&target)) {
                    return Some(m);
                }
            }
        }
        // nearest earlier incoming message
        let mut best: Option<&ChatMessage> = None;
        for m in messages {
            if m.message_key == revoke.message_key || m.is_send == Some(1) || is_revoke_system_message(m) {
                continue;
            }
            if revoke.create_time > 0 && m.create_time > revoke.create_time {
                continue;
            }
            if revoke.create_time > 0 && m.create_time == revoke.create_time {
                if revoke.sort_seq > 0 && m.sort_seq > revoke.sort_seq {
                    continue;
                }
                if revoke.sort_seq <= 0 && revoke.local_id > 0 && m.local_id > revoke.local_id {
                    continue;
                }
            }
            if best.map_or(true, |b| compare_position(m, b) == std::cmp::Ordering::Greater) {
                best = Some(m);
            }
        }
        best
    }

    fn find_revoked_original(&self, session_id: &str, revoke: &ChatMessage, fetched: &[ChatMessage], revoked_id: Option<&str>) -> Option<ChatMessage> {
        if let Some(m) = Self::find_revoked_in(fetched, revoke, revoked_id) {
            return Some(m.clone());
        }
        if revoke.create_time <= 0 {
            return None;
        }
        if let Some(id) = revoked_id {
            if let Some(m) = self.find_by_server_id_direct(session_id, revoke, id) {
                return Some(m);
            }
        }
        let lookup = self.recent_context_messages(session_id, (revoke.create_time - 5 * 60).max(0));
        if lookup.is_empty() {
            return None;
        }
        Self::find_revoked_in(&lookup, revoke, revoked_id).cloned()
    }

    fn is_recently_revoked_original(&mut self, session_id: &str, m: &ChatMessage) -> bool {
        let prefix = session_id.trim();
        if prefix.is_empty() {
            return false;
        }
        prune(&mut self.recently_revoked);
        message_id_tokens(m).iter().any(|t| self.recently_revoked.contains_key(&format!("{prefix}\u{0}{t}")))
    }

    fn remember_revoked_tokens(&mut self, session_id: &str, original: Option<&ChatMessage>, revoked_id: Option<&str>, revoke: &ChatMessage) {
        let prefix = session_id.trim();
        if prefix.is_empty() {
            return;
        }
        prune(&mut self.recently_revoked);
        let mut tokens: HashSet<String> = HashSet::new();
        let mut add = |v: &str| {
            let n = normalize_id_token(v);
            if !n.is_empty() {
                tokens.insert(n);
            }
        };
        if let Some(o) = original {
            add(&o.server_id_raw);
            add(&o.server_id.to_string());
        }
        if let Some(r) = revoked_id {
            add(r);
        }
        add(&revoke.server_id_raw);
        add(&revoke.server_id.to_string());
        for t in tokens {
            self.recently_revoked.insert(format!("{prefix}\u{0}{t}"), Instant::now());
        }
    }

    fn group_nicknames_for(&mut self, chatroom_id: &str) -> HashMap<String, String> {
        if let Some((n, at)) = self.group_nicknames.get(chatroom_id) {
            if at.elapsed() < GROUP_NICKNAME_TTL {
                return n.clone();
            }
        }
        let nicks = self.hub.group_nicknames_pub(chatroom_id);
        let trusted = crate::export_msg::build_trusted_group_nicknames(nicks.into_iter());
        self.group_nicknames.insert(chatroom_id.to_string(), (trusted.clone(), Instant::now()));
        trusted
    }

    fn resolve_group_source_name(&mut self, chatroom_id: &str, m: &ChatMessage, session: &Session) -> String {
        let sender = m.sender_username.clone().unwrap_or_default().trim().to_string();
        if sender.is_empty() {
            return session.last_sender_display_name.clone().unwrap_or_else(|| "未知发送者".into());
        }
        let nicks = self.group_nicknames_for(chatroom_id);
        if let Some(n) = nicks.get(&sender.to_lowercase()).filter(|n| !n.is_empty()) {
            return n.clone();
        }
        self.hub.chat_contact_avatar(&sender).map(|c| c.1).filter(|n| !n.is_empty()).unwrap_or(sender)
    }

    fn normalize_avatar(&mut self, avatar: Option<String>) -> Option<String> {
        let normalized = avatar.unwrap_or_default().trim().to_string();
        if normalized.is_empty() {
            return None;
        }
        if !normalized.starts_with("data:image/") {
            return Some(normalized);
        }
        if let Some(c) = self.avatar_data_cache.get(&normalized) {
            return Some(c.clone());
        }
        let caps = rx(r"(?i)^data:(image/[a-zA-Z0-9.+-]+);base64,(.+)$").captures(&normalized)?;
        let mime = caps[1].to_lowercase();
        let bytes = crate::sns::lenient_base64(&caps[2]);
        if bytes.is_empty() {
            return None;
        }
        let ext = match mime.as_str() {
            "image/png" => "png",
            "image/gif" => "gif",
            "image/webp" => "webp",
            _ => "jpg",
        };
        use sha1::{Digest, Sha1};
        let mut h = Sha1::new();
        h.update(normalized.as_bytes());
        let hash: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let dir = self.hub.push_avatar_dir();
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!("avatar_{hash}.{ext}"));
        if !path.exists() {
            std::fs::write(&path, &bytes).ok()?;
        }
        let url = format!("file://{}", path.to_string_lossy());
        self.avatar_data_cache.insert(normalized, url.clone());
        Some(url)
    }

    fn payload(&self, event: &str, session_id: &str, raw_id: String, avatar: Option<String>, group_name: Option<String>, source_name: String, content: Option<String>, timestamp: i64) -> Value {
        let mut o = Map::new();
        o.insert("event".into(), json!(event));
        o.insert("sessionId".into(), json!(session_id));
        o.insert("sessionType".into(), json!(session_type(session_id)));
        o.insert("rawid".into(), json!(raw_id));
        if let Some(a) = avatar {
            o.insert("avatarUrl".into(), json!(a));
        }
        if let Some(g) = group_name {
            o.insert("groupName".into(), json!(g));
        }
        o.insert("sourceName".into(), json!(source_name));
        o.insert("content".into(), content.map(Value::from).unwrap_or(Value::Null));
        o.insert("timestamp".into(), json!(timestamp));
        Value::Object(o)
    }

    fn build_payload(&mut self, session: &Session, m: &ChatMessage) -> Option<Value> {
        let session_id = session.username.trim().to_string();
        if session_id.is_empty() || m.message_key.trim().is_empty() {
            return None;
        }
        let content = message_display_content(m);
        let raw_id = m.server_id_raw.trim().to_string();
        if session_id.ends_with("@chatroom") {
            let info = self.hub.chat_contact_avatar(&session_id);
            let group_name = Some(session.display_name.clone()).filter(|n| !n.is_empty()).or_else(|| info.as_ref().map(|i| i.1.clone())).filter(|n| !n.is_empty()).unwrap_or_else(|| session_id.clone());
            let source = self.resolve_group_source_name(&session_id, m, session);
            let avatar = self.normalize_avatar(session.avatar_url.clone().or_else(|| info.and_then(|i| i.0)));
            return Some(self.payload("message.new", &session_id, raw_id, avatar, Some(group_name), source, content, m.create_time));
        }
        let info = self.hub.chat_contact_avatar(&session_id);
        let avatar = self.normalize_avatar(session.avatar_url.clone().or_else(|| info.as_ref().and_then(|i| i.0.clone())));
        let source = Some(session.display_name.clone()).filter(|n| !n.is_empty()).or_else(|| info.map(|i| i.1)).filter(|n| !n.is_empty()).unwrap_or_else(|| session_id.clone());
        Some(self.payload("message.new", &session_id, raw_id, avatar, None, source, content, m.create_time))
    }

    fn build_revoke_payload(&mut self, session: &Session, m: &ChatMessage, fetched: &[ChatMessage]) -> Option<Value> {
        let session_id = session.username.trim().to_string();
        if session_id.is_empty() || m.message_key.trim().is_empty() || is_self_revoke(m) {
            return None;
        }
        let revoked_id = extract_revoked_message_id(m);
        let original = self.find_revoked_original(&session_id, m, fetched, revoked_id.as_deref());
        let raw_id = {
            let candidates: Vec<String> = match &original {
                Some(o) => vec![o.server_id_raw.clone(), revoked_id.clone().unwrap_or_default()],
                None => vec![revoked_id.clone().unwrap_or_default(), m.server_id_raw.clone()],
            };
            candidates.iter().map(|c| normalize_id_token(c)).find(|c| !c.is_empty()).unwrap_or_else(|| "未知".into())
        };
        let original_content = match &original {
            Some(o) => message_display_content(o),
            None => {
                let content = if !m.raw_content.is_empty() { &m.raw_content } else { &m.parsed_content };
                let rep = push_xml_value(content, "replacemsg");
                (!rep.is_empty() && !rep.contains("撤回了一条消息")).then_some(rep)
            }
        };
        let safe = original_content.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).unwrap_or_else(|| "未知内容".into());
        let content = format!("对方撤回了一条消息（rawid：{raw_id}） 内容为“{safe}”");
        self.remember_revoked_tokens(&session_id, original.as_ref(), revoked_id.as_deref(), m);
        if session_id.ends_with("@chatroom") {
            let info = self.hub.chat_contact_avatar(&session_id);
            let group_name = Some(session.display_name.clone()).filter(|n| !n.is_empty()).or_else(|| info.as_ref().map(|i| i.1.clone())).filter(|n| !n.is_empty()).unwrap_or_else(|| session_id.clone());
            let revoker = [push_xml_value(&m.raw_content, "fromusername"), push_xml_value(&m.raw_content, "session"), m.sender_username.clone().unwrap_or_default()].into_iter().map(|s| s.trim().to_string()).find(|s| !s.is_empty());
            let mut source_msg = m.clone();
            if let Some(r) = revoker {
                source_msg.sender_username = Some(r);
            }
            let source = self.resolve_group_source_name(&session_id, &source_msg, session);
            let avatar = self.normalize_avatar(session.avatar_url.clone().or_else(|| info.and_then(|i| i.0)));
            return Some(self.payload("message.revoke", &session_id, raw_id, avatar, Some(group_name), source, Some(content), m.create_time));
        }
        let info = self.hub.chat_contact_avatar(&session_id);
        let avatar = self.normalize_avatar(session.avatar_url.clone().or_else(|| info.as_ref().and_then(|i| i.0.clone())));
        let source = Some(session.display_name.clone()).filter(|n| !n.is_empty()).or_else(|| info.map(|i| i.1)).filter(|n| !n.is_empty()).unwrap_or_else(|| session_id.clone());
        Some(self.payload("message.revoke", &session_id, raw_id, avatar, None, source, Some(content), m.create_time))
    }
}

/// `parseMessageKeySource`: (db path, table) out of a `db:table:localId` key.
pub fn parse_message_key_source(key: &str) -> Option<(String, String)> {
    let raw = key.trim();
    if raw.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = raw.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    parts.pop();
    let table = parts.pop()?.trim().to_string();
    let encoded = parts.join(":");
    if table.is_empty() || encoded.is_empty() {
        return None;
    }
    let decoded = percent_decode(&encoded);
    (!decoded.is_empty()).then_some((decoded, table))
}

fn percent_decode(s: &str) -> String {
    String::from_utf8(crate::message::percent_decode_bytes(s)).unwrap_or_else(|_| s.to_string())
}

/// SSE fan-out with a replay buffer (`broadcastMessagePush` of the HTTP service):
/// every payload gets an increasing id, is kept for 10 minutes / 1000 events and can be
/// replayed to clients that reconnect with `Last-Event-ID`.
pub struct PushBroker {
    inner: std::sync::Mutex<BrokerInner>,
    tx: broadcast::Sender<(u64, std::sync::Arc<String>)>,
}

struct BrokerInner {
    next_id: u64,
    buffer: std::collections::VecDeque<(u64, std::sync::Arc<String>, Instant)>,
}

const REPLAY_LIMIT: usize = 1000;

impl PushBroker {
    pub fn new() -> std::sync::Arc<Self> {
        let (tx, _) = broadcast::channel(512);
        std::sync::Arc::new(Self { inner: std::sync::Mutex::new(BrokerInner { next_id: 0, buffer: Default::default() }), tx })
    }

    fn prune(inner: &mut BrokerInner) {
        let now = Instant::now();
        while inner.buffer.front().map_or(false, |(_, _, at)| now.duration_since(*at) > RECENT_TTL) {
            inner.buffer.pop_front();
        }
    }

    pub fn broadcast(&self, payload: &Value) {
        let name = payload.get("event").and_then(Value::as_str).map(str::trim).filter(|n| rx(r"^[a-zA-Z0-9._-]+$").is_match(n)).unwrap_or("message.new").to_string();
        let mut inner = self.inner.lock().unwrap();
        inner.next_id += 1;
        let id = inner.next_id;
        let body = std::sync::Arc::new(format!("id: {id}\nevent: {name}\ndata: {}\n\n", serde_json::to_string(payload).unwrap_or_default()));
        Self::prune(&mut inner);
        inner.buffer.push_back((id, body.clone(), Instant::now()));
        while inner.buffer.len() > REPLAY_LIMIT {
            inner.buffer.pop_front();
        }
        drop(inner);
        let _ = self.tx.send((id, body));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(u64, std::sync::Arc<String>)> {
        self.tx.subscribe()
    }

    /// Buffered events newer than `last_id` (everything when `last_id` is 0).
    pub fn replay_since(&self, last_id: u64) -> Vec<(u64, std::sync::Arc<String>)> {
        let mut inner = self.inner.lock().unwrap();
        Self::prune(&mut inner);
        inner.buffer.iter().filter(|(id, _, _)| last_id == 0 || *id > last_id).map(|(id, body, _)| (*id, body.clone())).collect()
    }
}

/// Polls like the desktop app's debounced change handler and broadcasts each payload.
pub fn message_push_loop(hub: ServiceHub, sender: broadcast::Sender<Value>, interval_secs: u64) {
    let mut engine = PushEngine::new(hub);
    let mut attempts = 0;
    while !engine.bootstrap() && attempts < 3 {
        attempts += 1;
        std::thread::sleep(Duration::from_secs(interval_secs));
    }
    loop {
        std::thread::sleep(Duration::from_secs(interval_secs));
        for payload in engine.flush(true) {
            let _ = sender.send(payload);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(json_rows: Value) -> Vec<ChatMessage> {
        chat_msg::map_rows(json_rows.as_array().unwrap(), "wxid_me")
    }

    #[test]
    fn revoke_detection_and_original_lookup() {
        let rows = json!([
            {"local_id": "1", "server_id": "555", "create_time": "100", "local_type": "1", "message_content": "wxid_bob:\nsecret", "sender_username": "wxid_bob", "is_send": "0", "_db_path": "/m.db", "table_name": "Msg_x"},
            {"local_id": "2", "create_time": "101", "local_type": "10002", "message_content": "<sysmsg type=\"revokemsg\"><revokemsg><newmsgid>555</newmsgid><replacemsg><![CDATA[\"Bob\" 撤回了一条消息]]></replacemsg></revokemsg></sysmsg>", "sender_username": "wxid_bob", "is_send": "0", "_db_path": "/m.db", "table_name": "Msg_x"}
        ]);
        let m = msg(rows);
        assert!(!is_revoke_system_message(&m[0]));
        assert!(is_revoke_system_message(&m[1]));
        assert_eq!(extract_revoked_message_id(&m[1]).as_deref(), Some("555"));
        let found = PushEngine::find_revoked_in(&m, &m[1], Some("555")).unwrap();
        assert_eq!(found.local_id, 1);
        assert_eq!(message_display_content(found).as_deref(), Some("secret"));
        // without the id, the nearest earlier incoming message is used
        assert_eq!(PushEngine::find_revoked_in(&m, &m[1], None).unwrap().local_id, 1);
        assert_eq!(parse_message_key_source(&m[0].message_key), Some(("/m.db".into(), "Msg_x".into())));
    }

    #[test]
    fn broker_assigns_ids_and_replays() {
        let b = PushBroker::new();
        let mut rx = b.subscribe();
        b.broadcast(&json!({"event": "message.new", "content": "a"}));
        b.broadcast(&json!({"event": "bad event!", "content": "b"}));
        let first = rx.try_recv().unwrap();
        assert_eq!(first.0, 1);
        assert!(first.1.starts_with("id: 1\nevent: message.new\ndata: {"));
        let all = b.replay_since(0);
        assert_eq!(all.len(), 2);
        assert!(all[1].1.contains("event: message.new"), "invalid event names fall back to message.new");
        let after = b.replay_since(1);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].0, 2);
    }

    #[test]
    fn display_content_per_type() {
        let rows = json!([
            {"local_id": "1", "create_time": "1", "local_type": "3", "message_content": "<msg/>"},
            {"local_id": "2", "create_time": "1", "local_type": "49", "message_content": "<msg><appmsg><title>[视频号] T</title><type>5</type><url>http://x</url></appmsg></msg>"},
            {"local_id": "3", "create_time": "1", "local_type": "1", "message_content": "wxid_a:\nhi"},
        ]);
        let m = msg(rows);
        assert_eq!(message_display_content(&m[0]).as_deref(), Some("[图片]"));
        assert_eq!(message_display_content(&m[1]).as_deref(), Some("T"));
        assert_eq!(message_display_content(&m[2]).as_deref(), Some("hi"));
    }

    #[test]
    fn id_token_normalisation_and_session_types() {
        assert_eq!(normalize_id_token(" 000123 "), "123");
        assert_eq!(normalize_id_token("0"), "");
        assert_eq!(normalize_id_token("-45"), "45");
        assert_eq!(normalize_id_token("abc"), "abc");
        assert_eq!(session_type("a@chatroom"), "group");
        assert_eq!(session_type("gh_x"), "official");
        assert_eq!(session_type("wxid_a"), "other");
    }
}
