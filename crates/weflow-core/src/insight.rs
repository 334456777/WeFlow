//! Pure parts of `electron/services/insightService.ts` and `insightRecordService.ts`: record
//! storage, the OpenAI-compatible API call, prompt assembly and text helpers.
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::chat_msg::ChatMessage;
use crate::locale::tr;
use crate::message::rx;

pub const API_TIMEOUT_MS: u64 = 45_000;
pub const API_TEMPERATURE: f64 = 0.7;
pub const API_MAX_TOKENS_DEFAULT: u64 = 1024;
pub const MAX_RECORDS_PER_SCOPE: usize = 1000;
pub const RECORDS_FILE: &str = "weflow-insight-records.json";

pub fn now_millis() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// `{name}` placeholder substitution of the desktop `mt()` helper.
pub fn fill(template: &str, vars: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

// ───────────────────────── records ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InsightRecordLog {
    pub endpoint: String,
    pub model: String,
    pub max_tokens: u64,
    pub temperature: f64,
    pub trigger_reason: String,
    pub allow_context: bool,
    pub context_count: u64,
    pub system_prompt: String,
    pub user_prompt: String,
    pub raw_output: String,
    pub final_insight: String,
    pub duration_ms: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InsightRecord {
    pub id: String,
    pub account_scope: String,
    pub created_at: i64,
    pub session_id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub trigger_reason: String,
    pub insight: String,
    pub read: bool,
    pub log: InsightRecordLog,
}

impl InsightRecord {
    pub fn summary(&self) -> Value {
        let mut o = serde_json::Map::new();
        o.insert("id".into(), json!(self.id));
        o.insert("createdAt".into(), json!(self.created_at));
        o.insert("sessionId".into(), json!(self.session_id));
        o.insert("displayName".into(), json!(self.display_name));
        if let Some(a) = &self.avatar_url {
            o.insert("avatarUrl".into(), json!(a));
        }
        o.insert("triggerReason".into(), json!(self.trigger_reason));
        o.insert("insight".into(), json!(self.insight));
        o.insert("read".into(), json!(self.read));
        Value::Object(o)
    }
}

#[derive(Debug, Clone, Default)]
pub struct RecordFilters {
    pub keyword: String,
    pub session_id: String,
    pub start_time: i64,
    pub end_time: i64,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub struct RecordStore {
    path: PathBuf,
    records: Vec<InsightRecord>,
}

fn start_of_today_ms() -> i64 {
    let n = Local::now();
    Local.with_ymd_and_hms(n.year(), n.month(), n.day(), 0, 0, 0).earliest().map(|d| d.timestamp_millis()).unwrap_or(0)
}

impl RecordStore {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(RECORDS_FILE);
        let records = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|v| {
                let list = if v.is_array() { v } else { v.get("records").cloned()? };
                Some(list.as_array()?.iter().filter_map(|r| serde_json::from_value::<InsightRecord>(r.clone()).ok()).collect::<Vec<_>>())
            })
            .unwrap_or_default();
        Self { path, records }
    }

    fn persist(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, serde_json::to_vec_pretty(&json!({ "version": 1, "records": self.records })).unwrap_or_default());
    }

    fn scoped<'a>(&'a self, scope: &'a str) -> impl Iterator<Item = &'a InsightRecord> {
        self.records.iter().filter(move |r| r.account_scope == scope)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(&mut self, scope: &str, session_id: &str, display_name: &str, avatar_url: Option<String>, trigger_reason: &str, insight: &str, log: InsightRecordLog) -> InsightRecord {
        let record = InsightRecord { id: uuid_v4(), account_scope: scope.into(), created_at: now_millis(), session_id: session_id.into(), display_name: display_name.into(), avatar_url, trigger_reason: trigger_reason.into(), insight: insight.into(), read: false, log };
        self.records.push(record.clone());
        let mut mine: Vec<(i64, String)> = self.scoped(scope).map(|r| (r.created_at, r.id.clone())).collect();
        mine.sort_by(|a, b| b.0.cmp(&a.0));
        let keep: std::collections::HashSet<String> = mine.into_iter().take(MAX_RECORDS_PER_SCOPE).map(|(_, id)| id).collect();
        self.records.retain(|r| r.account_scope != scope || keep.contains(&r.id));
        self.persist();
        record
    }

    pub fn list(&self, scope: &str, f: &RecordFilters) -> Value {
        let all: Vec<&InsightRecord> = self.scoped(scope).collect();
        let today = start_of_today_ms();
        let mut contacts: Vec<(String, String, Option<String>, i64)> = Vec::new();
        for r in &all {
            match contacts.iter_mut().find(|c| c.0 == r.session_id) {
                Some(c) => c.3 += 1,
                None => contacts.push((r.session_id.clone(), r.display_name.clone(), r.avatar_url.clone(), 1)),
            }
        }
        contacts.sort_by(|a, b| b.3.cmp(&a.3));
        let keyword = f.keyword.trim().to_lowercase();
        let offset = f.offset.unwrap_or(0).max(0) as usize;
        let limit = f.limit.filter(|l| *l != 0).unwrap_or(100).clamp(1, 200) as usize;
        let mut filtered: Vec<&InsightRecord> = all
            .iter()
            .copied()
            .filter(|r| {
                (f.session_id.is_empty() || r.session_id == f.session_id)
                    && !(f.start_time > 0 && r.created_at < f.start_time)
                    && !(f.end_time > 0 && r.created_at > f.end_time)
                    && (keyword.is_empty() || format!("{}\n{}\n{}", r.display_name, r.session_id, r.insight).to_lowercase().contains(&keyword))
            })
            .collect();
        filtered.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        json!({
            "success": true,
            "records": filtered.iter().skip(offset).take(limit).map(|r| r.summary()).collect::<Vec<_>>(),
            "total": filtered.len(),
            "todayCount": all.iter().filter(|r| r.created_at >= today).count(),
            "unreadCount": all.iter().filter(|r| !r.read).count(),
            "contacts": contacts.iter().map(|(s, n, a, c)| { let mut o = json!({ "sessionId": s, "displayName": n, "count": c }); if let Some(a) = a { o["avatarUrl"] = json!(a); } o }).collect::<Vec<_>>()
        })
    }

    pub fn get(&self, scope: &str, id: &str) -> Value {
        let id = id.trim();
        if id.is_empty() {
            return json!({ "success": false, "error": tr("record id is empty", "记录 ID 为空") });
        }
        match self.scoped(scope).find(|r| r.id == id) {
            Some(r) => json!({ "success": true, "record": r }),
            None => json!({ "success": false, "error": tr("insight record not found", "未找到该见解记录") }),
        }
    }

    pub fn mark_read(&mut self, scope: &str, id: &str) -> Value {
        let id = id.trim();
        match self.records.iter_mut().find(|r| r.id == id && r.account_scope == scope) {
            None => json!({ "success": false, "error": tr("insight record not found", "未找到该见解记录") }),
            Some(r) => {
                if !r.read {
                    r.read = true;
                    self.persist();
                }
                json!({ "success": true })
            }
        }
    }

    pub fn clear(&mut self, scope: &str, f: &RecordFilters) -> Value {
        let mut removed = 0;
        self.records.retain(|r| {
            if r.account_scope != scope
                || (!f.session_id.is_empty() && r.session_id != f.session_id)
                || (f.start_time > 0 && r.created_at < f.start_time)
                || (f.end_time > 0 && r.created_at > f.end_time)
            {
                return true;
            }
            removed += 1;
            false
        });
        self.persist();
        json!({ "success": true, "removed": removed })
    }
}

fn uuid_v4() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

// ───────────────────────── API ─────────────────────────

pub fn build_api_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if path.starts_with('/') { format!("{base}{path}") } else { format!("{base}/{path}") }
}

pub fn normalize_api_max_tokens(v: &Value) -> u64 {
    let n = match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    match n {
        Some(n) if n.is_finite() => (n.floor() as i64).clamp(1, 2_000_000) as u64,
        _ => API_MAX_TOKENS_DEFAULT,
    }
}

/// `callApi`: non-streaming chat completion, returns the trimmed first message.
pub async fn call_api(base_url: &str, api_key: &str, model: &str, messages: &[(&str, &str)], timeout_ms: u64, max_tokens: u64) -> Result<String, String> {
    let endpoint = build_api_url(base_url, "/chat/completions");
    let body = json!({
        "model": model,
        "messages": messages.iter().map(|(r, c)| json!({ "role": r, "content": c })).collect::<Vec<_>>(),
        "max_tokens": normalize_api_max_tokens(&json!(max_tokens)),
        "temperature": API_TEMPERATURE,
        "stream": false
    });
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_millis(timeout_ms)).build().map_err(|e| e.to_string())?;
    let resp = client
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {api_key}"))
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| {
            if e.is_builder() {
                fill(tr("Invalid API URL: {endpoint}", "无效的 API URL: {endpoint}"), &[("endpoint", endpoint.clone())])
            } else if e.is_timeout() {
                tr("API request timed out", "API 请求超时").to_string()
            } else {
                e.to_string()
            }
        })?;
    let data = resp.text().await.map_err(|e| e.to_string())?;
    let head: String = data.chars().take(200).collect();
    let parsed: Value = serde_json::from_str(&data).map_err(|_| fill(tr("JSON parse failed: {v0}", "JSON 解析失败: {v0}"), &[("v0", head.clone())]))?;
    match parsed.pointer("/choices/0/message/content").and_then(Value::as_str) {
        Some(c) if !c.trim().is_empty() => Ok(c.trim().to_string()),
        _ => Err(fill(tr("Unexpected API response format: {v0}", "API 返回格式异常: {v0}"), &[("v0", head)])),
    }
}

/// Telegram Bot API `sendMessage` (HTML parse mode, 15 s timeout).
pub async fn send_telegram(token: &str, chat_id: &str, text: &str) -> Result<(), String> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    let resp = client.post(format!("https://api.telegram.org/bot{token}/sendMessage")).header("Content-Type", "application/json").body(json!({ "chat_id": chat_id, "text": text, "parse_mode": "HTML" }).to_string()).send().await.map_err(|e| if e.is_timeout() { tr("Telegram request timed out", "Telegram 请求超时").to_string() } else { e.to_string() })?;
    let data = resp.text().await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&data).map_err(|_| fill(tr("Response parse failed: {v0}", "响应解析失败: {v0}"), &[("v0", data.chars().take(100).collect())]))?;
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(v.get("description").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| tr("Unknown error", "未知错误").to_string()))
    }
}

// ───────────────────────── prompt assembly ─────────────────────────

pub fn default_system_prompt() -> &'static str {
    tr(
        "You are the user's personal relationship observer, named \"Insight\". Your job is to proactively offer useful observations and advice.\n\nRequirements:\n1. Always give an insight. Based on the chat history, read the other person's mood, topic trends and relationship dynamics, or suggest a reply or a conversation topic.\n2. Keep it under 50 words: direct, specific and to the point. No filler.\n3. Output plain text, no Markdown.\n4. Reply \"SKIP\" only when there is truly nothing to say (for example the conversation is a single \"ok\"). In almost every case you should give an insight.",
        "你是用户的私人关系观察助手，名叫\"见解\"。你的任务是主动提供有价值的观察和建议。\n\n要求：\n1. 必须给出见解。基于聊天记录分析对方情绪、话题趋势、关系动态，或给出回复建议、聊天话题推荐。\n2. 控制在 80 字以内，直接、具体、一针见血。不要废话。\n3. 输出纯文本，不使用 Markdown。\n4. 只有在完全没有任何可说的内容时（比如对话只有一条\"嗯\"），才回复\"SKIP\"。绝大多数情况下你应该输出见解。",
    )
}

pub fn default_footprint_prompt() -> &'static str {
    tr(
        "You are the user's chat footprint coach. Based on the statistics, give a short review.\nRequirements:\n1. Output 2-3 sentences, no more than 100 words in total.\n2. It must include: an overall observation + one actionable suggestion.\n3. Keep the tone practical, not exaggerated, and do not use Markdown.",
        "你是用户的聊天足迹教练，负责基于统计数据给出一段简明复盘。\n要求：\n1. 输出 2-3 句，总长度不超过 180 字。\n2. 必须包含：总体观察 + 一个可执行建议。\n3. 语气务实，不夸张，不使用 Markdown。",
    )
}

/// `formatPromptCurrentTime` / `appendPromptCurrentTime`
pub fn append_current_time(prompt: &str) -> String {
    let n = Local::now();
    let line = fill(
        tr("Current system time: {month}/{day}/{year} {hours}:{minutes}", "当前系统时间：{year}年{month}月{day}日 {hours}:{minutes}"),
        &[("year", n.year().to_string()), ("month", format!("{:02}", n.month())), ("day", format!("{:02}", n.day())), ("hours", format!("{:02}", n.hour())), ("minutes", format!("{:02}", n.minute()))],
    );
    let base = prompt.trim_end();
    if base.is_empty() { line } else { format!("{base}\n\n{line}") }
}

pub fn looks_like_wxid(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && (rx(r"(?i)^wxid_[a-z0-9]+$").is_match(t) || rx(r"(?i)^[a-z0-9_]+@chatroom$").is_match(t))
}

pub fn looks_like_xml_payload(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && rx(r"(?i)^(<\?xml|<msg\b|<appmsg\b|<img\b|<emoji\b|<voip\b|<sysmsg\b|&lt;\?xml|&lt;msg\b|&lt;appmsg\b)").is_match(t)
}

pub fn normalize_insight_text(text: &str) -> String {
    let t = text.replace("\r\n", "\n").replace('\0', "");
    rx(r"\n{3,}").replace_all(&t, "\n\n").trim().to_string()
}

pub fn format_message_timestamp(create_time: i64) -> String {
    let ms = if create_time > 1_000_000_000_000 { create_time } else { create_time * 1000 };
    match Local.timestamp_millis_opt(ms).single() {
        Some(d) => format!("{}-{:02}-{:02} {:02}:{:02}:{:02}", d.year(), d.month(), d.day(), d.hour(), d.minute(), d.second()),
        None => String::new(),
    }
}

pub fn format_message_content(m: &ChatMessage) -> String {
    let parsed = normalize_insight_text(&m.parsed_content);
    let quoted = normalize_insight_text(m.quoted_content.as_deref().unwrap_or(""));
    let quoted_sender = normalize_insight_text(m.quoted_sender.as_deref().unwrap_or(""));
    if !quoted.is_empty() {
        let clean_sender = if !quoted_sender.is_empty() && !looks_like_wxid(&quoted_sender) { quoted_sender } else { String::new() };
        let label = if clean_sender.is_empty() { quoted } else { format!("{clean_sender}：{quoted}") };
        let reply = if !parsed.is_empty() && parsed != "[引用消息]" { parsed } else { String::new() };
        return if reply.is_empty() {
            fill(tr("[Quote {quoteLabel}]", "[引用 {quoteLabel}]"), &[("quoteLabel", label)])
        } else {
            fill(tr("{replyText}[Quote {quoteLabel}]", "{replyText}[引用 {quoteLabel}]"), &[("replyText", reply), ("quoteLabel", label)])
        };
    }
    if !parsed.is_empty() {
        return parsed;
    }
    let raw = normalize_insight_text(&m.raw_content);
    if !raw.is_empty() && !looks_like_xml_payload(&raw) {
        return raw;
    }
    tr("[Other message]", "[其他消息]").to_string()
}

pub fn build_context_section(messages: &[ChatMessage], peer: &str) -> String {
    if messages.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = messages
        .iter()
        .map(|m| {
            let sender = if m.is_send == Some(1) { tr("Me", "我").to_string() } else { peer.to_string() };
            format!("{} '{}'\n{}", format_message_timestamp(m.create_time), sender, format_message_content(m))
        })
        .collect();
    fill(tr("Recent chat history (latest {length} messages):\n\n{v1}", "近期聊天记录（最近 {length} 条）：\n\n{v1}"), &[("length", lines.len().to_string()), ("v1", lines.join("\n\n"))])
}

/// Whether a conversation may trigger insights (`whitelist`: only listed ids, `blacklist`: all but listed).
pub fn session_allowed(mode: &str, list: &[String], session_id: &str) -> bool {
    let id = session_id.trim();
    if id.is_empty() {
        return false;
    }
    if mode.trim().to_lowercase() == "blacklist" { !list.iter().any(|l| l == id) } else { list.iter().any(|l| l == id) }
}

pub fn normalize_session_id_list(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let t = s.as_str().unwrap_or("").trim().to_string();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_do_not_gain_a_v1() {
        assert_eq!(build_api_url("https://api.ohmygpt.com/v1/", "/chat/completions"), "https://api.ohmygpt.com/v1/chat/completions");
        assert_eq!(build_api_url("https://x.test", "chat/completions"), "https://x.test/chat/completions");
    }

    #[test]
    fn max_tokens_are_clamped() {
        assert_eq!(normalize_api_max_tokens(&json!(0)), 1);
        assert_eq!(normalize_api_max_tokens(&json!("abc")), 1024);
        assert_eq!(normalize_api_max_tokens(&json!(9_999_999_999i64)), 2_000_000);
        assert_eq!(normalize_api_max_tokens(&json!(512.9)), 512);
    }

    #[test]
    fn filters_follow_the_list_mode() {
        let list = vec!["a".to_string()];
        assert!(session_allowed("whitelist", &list, "a"));
        assert!(!session_allowed("whitelist", &list, "b"));
        assert!(!session_allowed("blacklist", &list, "a"));
        assert!(session_allowed("Blacklist", &list, "b"));
        assert!(!session_allowed("blacklist", &list, " "));
    }

    #[test]
    fn message_content_prefers_quotes_then_text_then_plain_raw() {
        let mut m = ChatMessage { parsed_content: "ok".into(), quoted_content: Some("hello".into()), quoted_sender: Some("wxid_abc".into()), ..Default::default() };
        assert_eq!(format_message_content(&m), "ok[Quote hello]");
        m.quoted_sender = Some("Bob".into());
        m.parsed_content = "[引用消息]".into();
        assert_eq!(format_message_content(&m), "[Quote Bob：hello]");
        let plain = ChatMessage { parsed_content: String::new(), raw_content: "<msg><a/></msg>".into(), ..Default::default() };
        assert_eq!(format_message_content(&plain), "[Other message]");
        let raw = ChatMessage { raw_content: "just text".into(), ..Default::default() };
        assert_eq!(format_message_content(&raw), "just text");
    }

    #[test]
    fn context_section_labels_speakers() {
        let msgs = vec![ChatMessage { is_send: Some(1), parsed_content: "hi".into(), create_time: 1_700_000_000, ..Default::default() }, ChatMessage { is_send: Some(0), parsed_content: "yo".into(), create_time: 1_700_000_060, ..Default::default() }];
        let s = build_context_section(&msgs, "Bob");
        assert!(s.starts_with("Recent chat history (latest 2 messages):\n\n"));
        assert!(s.contains(" 'Me'\nhi") && s.contains(" 'Bob'\nyo"));
        assert_eq!(build_context_section(&[], "Bob"), "");
    }

    #[test]
    fn wxid_and_xml_detection() {
        assert!(looks_like_wxid("wxid_abc123") && looks_like_wxid("room1@chatroom"));
        assert!(!looks_like_wxid("Bobby"));
        assert!(looks_like_xml_payload("<msg><x/></msg>") && looks_like_xml_payload("&lt;?xml"));
        assert!(!looks_like_xml_payload("hello <b>"));
    }

    #[test]
    fn record_store_scopes_filters_and_trims() {
        let dir = std::env::temp_dir().join(format!("weflow-insight-{}", now_millis()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = RecordStore::load(&dir);
        let a = s.add("wxid:me", "wxid_a", "Alice", None, "activity", "be nice", InsightRecordLog::default());
        s.add("wxid:me", "wxid_b", "Bob", Some("http://a".into()), "silence", "call him", InsightRecordLog::default());
        s.add("wxid:other", "wxid_a", "Alice", None, "test", "hidden", InsightRecordLog::default());
        let list = s.list("wxid:me", &RecordFilters::default());
        assert_eq!(list["total"], 2);
        assert_eq!(list["unreadCount"], 2);
        assert_eq!(list["contacts"].as_array().unwrap().len(), 2);
        let f = RecordFilters { keyword: "CALL".into(), ..Default::default() };
        assert_eq!(s.list("wxid:me", &f)["total"], 1);
        assert_eq!(s.get("wxid:other", &a.id)["success"], false, "records of other accounts are invisible");
        assert_eq!(s.mark_read("wxid:me", &a.id)["success"], true);
        assert_eq!(s.list("wxid:me", &RecordFilters::default())["unreadCount"], 1);
        // persisted and reloadable
        let again = RecordStore::load(&dir);
        assert_eq!(again.list("wxid:me", &RecordFilters::default())["total"], 2);
        let mut again = again;
        assert_eq!(again.clear("wxid:me", &RecordFilters { session_id: "wxid_a".into(), ..Default::default() })["removed"], 1);
        assert_eq!(again.list("wxid:other", &RecordFilters::default())["total"], 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
