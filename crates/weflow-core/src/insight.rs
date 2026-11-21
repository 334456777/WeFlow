//! Pure parts of WeFlow's `insightService.ts` and `insightRecordService.ts`: record
//! storage ([`RecordStore`]), the Telegram push, prompt assembly and text helpers. The model API itself is
//! [`crate::model_api`].
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Local, TimeZone, Timelike};
use serde_json::{json, Value};

use crate::chat_msg::ChatMessage;
use crate::locale::tr;
use crate::message::xml::rx;

mod records;

pub(crate) use records::{records_file, split_legacy_records};
pub use records::{InsightRecord, InsightRecordLog, NewRecord, RecordFilters, RecordStore};

/// The time now in Unix milliseconds; 0 before 1970.
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// `{name}` placeholder substitution of WeFlow's `mt()` helper.
pub fn fill(template: &str, vars: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

// ───────────────────────── Telegram ─────────────────────────

/// Telegram Bot API `sendMessage` (HTML parse mode, 15 s timeout).
pub async fn send_telegram(token: &str, chat_id: &str, text: &str) -> Result<(), String> {
    let client = crate::http_client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(format!("https://api.telegram.org/bot{token}/sendMessage"))
        .header("Content-Type", "application/json")
        .body(json!({ "chat_id": chat_id, "text": text, "parse_mode": "HTML" }).to_string())
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                tr("Telegram request timed out", "Telegram 请求超时").to_string()
            } else {
                // the bot token is in the address
                crate::http_client::failure(e)
            }
        })?;
    let data = resp.text().await.map_err(crate::http_client::failure)?;
    let v: Value = serde_json::from_str(&data).map_err(|_| {
        fill(
            tr("Response parse failed: {v0}", "响应解析失败: {v0}"),
            &[("v0", data.chars().take(100).collect())],
        )
    })?;
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(v
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| tr("Unknown error", "未知错误").to_string()))
    }
}

// ───────────────────────── prompt assembly ─────────────────────────

/// The system prompt of an insight when the settings give none, in the server's language.
pub fn default_system_prompt() -> &'static str {
    tr(
        "You are the user's personal relationship observer, named \"Insight\". Your job is to proactively offer useful observations and advice.\n\nRequirements:\n1. Always give an insight. Based on the chat history, read the other person's mood, topic trends and relationship dynamics, or suggest a reply or a conversation topic.\n2. Keep it under 50 words: direct, specific and to the point. No filler.\n3. Output plain text, no Markdown.\n4. Reply \"SKIP\" only when there is truly nothing to say (for example the conversation is a single \"ok\"). In almost every case you should give an insight.",
        "你是用户的私人关系观察助手，名叫\"见解\"。你的任务是主动提供有价值的观察和建议。\n\n要求：\n1. 必须给出见解。基于聊天记录分析对方情绪、话题趋势、关系动态，或给出回复建议、聊天话题推荐。\n2. 控制在 80 字以内，直接、具体、一针见血。不要废话。\n3. 输出纯文本，不使用 Markdown。\n4. 只有在完全没有任何可说的内容时（比如对话只有一条\"嗯\"），才回复\"SKIP\"。绝大多数情况下你应该输出见解。",
    )
}

/// The system prompt of the footprint's summary when the settings give none, in the server's language.
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
        tr(
            "Current system time: {month}/{day}/{year} {hours}:{minutes}",
            "当前系统时间：{year}年{month}月{day}日 {hours}:{minutes}",
        ),
        &[
            ("year", n.year().to_string()),
            ("month", format!("{:02}", n.month())),
            ("day", format!("{:02}", n.day())),
            ("hours", format!("{:02}", n.hour())),
            ("minutes", format!("{:02}", n.minute())),
        ],
    );
    let base = prompt.trim_end();
    if base.is_empty() {
        line
    } else {
        format!("{base}\n\n{line}")
    }
}

/// Whether `text` is a `wxid_…` or a group's `…@chatroom` rather than a name.
pub fn looks_like_wxid(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty()
        && (rx(r"(?i)^wxid_[a-z0-9]+$").is_match(t) || rx(r"(?i)^[a-z0-9_]+@chatroom$").is_match(t))
}

/// Whether `text` begins as the XML of a message that is not text, escaped or not.
pub fn looks_like_xml_payload(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && rx(r"(?i)^(<\?xml|<msg\b|<appmsg\b|<img\b|<emoji\b|<voip\b|<sysmsg\b|&lt;\?xml|&lt;msg\b|&lt;appmsg\b)").is_match(t)
}

/// `text` with `\n` line ends, without NUL characters and runs of blank lines, trimmed.
pub fn normalize_insight_text(text: &str) -> String {
    let t = text.replace("\r\n", "\n").replace('\0', "");
    rx(r"\n{3,}").replace_all(&t, "\n\n").trim().to_string()
}

fn format_message_timestamp(create_time: i64) -> String {
    let ms = if create_time > 1_000_000_000_000 {
        create_time
    } else {
        create_time * 1000
    };
    match Local.timestamp_millis_opt(ms).single() {
        Some(d) => format!(
            "{}-{:02}-{:02} {:02}:{:02}:{:02}",
            d.year(),
            d.month(),
            d.day(),
            d.hour(),
            d.minute(),
            d.second()
        ),
        None => String::new(),
    }
}

fn format_message_content(m: &ChatMessage) -> String {
    let parsed = normalize_insight_text(&m.parsed_content);
    let quoted = normalize_insight_text(m.quoted_content.as_deref().unwrap_or(""));
    let quoted_sender = normalize_insight_text(m.quoted_sender.as_deref().unwrap_or(""));
    if !quoted.is_empty() {
        let clean_sender = if !quoted_sender.is_empty() && !looks_like_wxid(&quoted_sender) {
            quoted_sender
        } else {
            String::new()
        };
        let label = if clean_sender.is_empty() {
            quoted
        } else {
            format!("{clean_sender}：{quoted}")
        };
        let reply = if !parsed.is_empty() && parsed != "[引用消息]" {
            parsed
        } else {
            String::new()
        };
        return if reply.is_empty() {
            fill(
                tr("[Quote {quoteLabel}]", "[引用 {quoteLabel}]"),
                &[("quoteLabel", label)],
            )
        } else {
            fill(
                tr(
                    "{replyText}[Quote {quoteLabel}]",
                    "{replyText}[引用 {quoteLabel}]",
                ),
                &[("replyText", reply), ("quoteLabel", label)],
            )
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

/// The part of the user prompt that holds the recent `messages` of the chat with `peer`, one with its time and sender
/// to a paragraph; empty without messages.
pub fn build_context_section(messages: &[ChatMessage], peer: &str) -> String {
    if messages.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = messages
        .iter()
        .map(|m| {
            let sender = if m.is_send == Some(1) {
                tr("Me", "我").to_string()
            } else {
                peer.to_string()
            };
            format!(
                "{} '{}'\n{}",
                format_message_timestamp(m.create_time),
                sender,
                format_message_content(m)
            )
        })
        .collect();
    fill(
        tr(
            "Recent chat history (latest {length} messages):\n\n{v1}",
            "近期聊天记录（最近 {length} 条）：\n\n{v1}",
        ),
        &[
            ("length", lines.len().to_string()),
            ("v1", lines.join("\n\n")),
        ],
    )
}

/// Whether a conversation may trigger insights (`whitelist`: only listed ids, `blacklist`: all but listed).
pub fn session_allowed(mode: &str, list: &[String], session_id: &str) -> bool {
    let id = session_id.trim();
    if id.is_empty() {
        return false;
    }
    if mode.trim().to_lowercase() == "blacklist" {
        !list.iter().any(|l| l == id)
    } else {
        list.iter().any(|l| l == id)
    }
}

/// The conversation ids of a JSON array, trimmed, without empty ones and repeats; empty for anything but an array.
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
        let mut m = ChatMessage {
            parsed_content: "ok".into(),
            quoted_content: Some("hello".into()),
            quoted_sender: Some("wxid_abc".into()),
            ..Default::default()
        };
        assert_eq!(format_message_content(&m), "ok[Quote hello]");
        m.quoted_sender = Some("Bob".into());
        m.parsed_content = "[引用消息]".into();
        assert_eq!(format_message_content(&m), "[Quote Bob：hello]");
        let plain = ChatMessage {
            parsed_content: String::new(),
            raw_content: "<msg><a/></msg>".into(),
            ..Default::default()
        };
        assert_eq!(format_message_content(&plain), "[Other message]");
        let raw = ChatMessage {
            raw_content: "just text".into(),
            ..Default::default()
        };
        assert_eq!(format_message_content(&raw), "just text");
    }

    #[test]
    fn context_section_labels_speakers() {
        let msgs = vec![
            ChatMessage {
                is_send: Some(1),
                parsed_content: "hi".into(),
                create_time: 1_700_000_000,
                ..Default::default()
            },
            ChatMessage {
                is_send: Some(0),
                parsed_content: "yo".into(),
                create_time: 1_700_000_060,
                ..Default::default()
            },
        ];
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
}
