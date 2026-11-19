//! Port of the `InsightService` runtime: the silence scan, the activity trigger, insight
//! generation with optional chat / Moments / Weibo context, Telegram push, test and footprint
//! helpers. A desktop notification popup has no CLI counterpart; instead every generated insight
//! is handed to the caller (the `serve --insight` loop prints it as a JSON line).
use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone, Timelike};
use serde_json::{json, Value};

use super::*;
use crate::contract::{
    InsightCleared, InsightMarked, InsightRecords, InsightReply, InsightTodayStat, ModelList,
    ModelProbe, Session,
};
use crate::insight::{self as ins, fill, RecordFilters, RecordStore};
use crate::locale::tr;
use crate::model_api::{ModelApi, Prompt};
use crate::sync::MutexExt;

/// The most tokens of one answer when `ai_model_api_max_tokens` is not set: room for a short answer and for the
/// reasoning a model may do first, which counts against the same limit.
const DEFAULT_MAX_TOKENS: u64 = 4096;

/// The largest `ai_model_api_max_tokens` taken; a larger one is cut to it.
const LARGEST_MAX_TOKENS: u64 = 2_000_000;

/// How long one request of `insight test` and `insight models` may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long one request for an insight or a footprint summary may take: a model that reasons first can take a while.
const GENERATE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct InsightState {
    today_triggers: HashMap<String, Vec<i64>>,
    today_date: i64,
    last_activity: HashMap<String, i64>,
    last_seen: HashMap<String, i64>,
    session_cache: Option<(Vec<Session>, Instant)>,
}

/// `ai_model_api_max_tokens` as a number of tokens: a number or a numeric text, floored and kept between 1 and
/// [`LARGEST_MAX_TOKENS`]; [`DEFAULT_MAX_TOKENS`] when it is not set or not a number.
fn max_tokens_of(value: &Value) -> u64 {
    let n = match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    match n {
        Some(n) if n.is_finite() => crate::num::saturating_i64(n.floor())
            .clamp(1, i64::try_from(LARGEST_MAX_TOKENS).unwrap_or(i64::MAX))
            .unsigned_abs(),
        _ => DEFAULT_MAX_TOKENS,
    }
}

/// Why `api` cannot be asked yet, in the words of the settings: a model is needed, and a key unless the base URL
/// names a service of one's own (a local one may take none).
fn model_api_missing(api: &ModelApi) -> Option<&'static str> {
    if api.key.trim().is_empty() && api.base_url.trim().is_empty() {
        return Some(tr("Please enter the API key first", "请先填写 API Key"));
    }
    if api.model.trim().is_empty() {
        return Some(tr("Please enter the model name first", "请先填写模型名称"));
    }
    None
}

fn millis_since(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// What a test trigger says when the model had nothing to say about `name`; without the recent messages
/// (`aiInsightAllowContext` off) the model sees next to nothing, which is the usual reason.
fn skipped_message(name: &str, chat_sent: bool) -> String {
    let skipped = fill(
        tr(
            "The model had nothing to say about \"{name}\" (it answered SKIP); no insight was written",
            "模型认为「{name}」没有可说的（回答了 SKIP），没有生成见解",
        ),
        &[("name", name.to_string())],
    );
    if chat_sent {
        return skipped;
    }
    skipped
        + tr(
            ". The recent messages are not sent (aiInsightAllowContext is off), so the model saw no chat; turn it on with `weflow config set aiInsightAllowContext true`",
            "。没有开启发送聊天内容（aiInsightAllowContext），模型没有收到聊天记录；可以用 `weflow config set aiInsightAllowContext true` 开启",
        )
}

/// A failure of the engine, which goes on with the next conversation.
fn log_failure(why: &str) {
    eprintln!("[insight] {why}");
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trigger {
    Activity,
    Silence,
    Test,
}

impl Trigger {
    fn as_str(self) -> &'static str {
        match self {
            Trigger::Activity => "activity",
            Trigger::Silence => "silence",
            Trigger::Test => "test",
        }
    }
}

fn start_of_day() -> i64 {
    let n = Local::now();
    Local
        .with_ymd_and_hms(
            chrono::Datelike::year(&n),
            chrono::Datelike::month(&n),
            chrono::Datelike::day(&n),
            0,
            0,
            0,
        )
        .earliest()
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}

fn cfg_str(cfg: &crate::config::ConfigStore, profile: &str, key: &str) -> String {
    match cfg.get_key(Some(profile), key) {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

impl ServiceHub {
    fn insight_cfg(&self) -> crate::config::ConfigStore {
        self.fresh_config()
    }

    /// The model API of the profile's settings (`ai_model_api_*`, else WeFlow's older `aiInsightApi*` keys).
    fn model_api(&self) -> ModelApi {
        let c = self.insight_cfg();
        let p = &self.profile_name;
        let pick = |a: &str, b: &str| {
            let first = cfg_str(&c, p, a);
            if first.is_empty() {
                cfg_str(&c, p, b)
            } else {
                first
            }
        };
        ModelApi {
            format: cfg_str(&c, p, "aiModelApiFormat")
                .parse()
                .unwrap_or_default(),
            base_url: pick("aiModelApiBaseUrl", "aiInsightApiBaseUrl"),
            key: pick("aiModelApiKey", "aiInsightApiKey"),
            model: pick("aiModelApiModel", "aiInsightApiModel"),
            max_tokens: max_tokens_of(&c.get_key(Some(p), "aiModelApiMaxTokens")),
        }
    }

    pub fn insight_enabled(&self) -> bool {
        self.insight_cfg()
            .get_key(Some(&self.profile_name), "aiInsightEnabled")
            == Value::Bool(true)
    }

    /// Whether the recent messages go to the model with the question (`aiInsightAllowContext`, off by default).
    fn insight_sends_chat(&self) -> bool {
        self.insight_cfg()
            .get_key(Some(&self.profile_name), "aiInsightAllowContext")
            == Value::Bool(true)
    }

    fn insight_filter(&self) -> (String, Vec<String>) {
        let c = self.insight_cfg();
        let mode = cfg_str(&c, &self.profile_name, "aiInsightFilterMode").to_lowercase();
        (
            if mode == "blacklist" {
                "blacklist".into()
            } else {
                "whitelist".into()
            },
            ins::normalize_session_id_list(
                &c.get_key(Some(&self.profile_name), "aiInsightFilterList"),
            ),
        )
    }

    fn insight_allowed(&self, session_id: &str) -> bool {
        let (mode, list) = self.insight_filter();
        ins::session_allowed(&mode, &list, session_id)
    }

    /// `getCurrentAccountScope`
    fn insight_scope(&self) -> String {
        let my = self.my_wxid_cleaned();
        if !my.trim().is_empty() {
            return format!("wxid:{}", my.trim());
        }
        let db = self.configured_db_path().unwrap_or_default();
        if !db.trim().is_empty() {
            use sha1::{Digest, Sha1};
            let h: String = Sha1::digest(db.trim().as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            return format!("db:{}", &h[..16]);
        }
        "default".into()
    }

    fn record_store(&self) -> RecordStore {
        RecordStore::load(&self.ctx.home_dir)
    }

    // ── records ──

    pub fn insight_list_records(&self, f: &RecordFilters) -> InsightRecords {
        self.record_store().list(&self.insight_scope(), f)
    }

    pub fn insight_get_record(&self, id: &str) -> crate::contract::InsightLookup {
        self.record_store().get(&self.insight_scope(), id)
    }

    pub fn insight_mark_record_read(&self, id: &str) -> InsightMarked {
        self.record_store().mark_read(&self.insight_scope(), id)
    }

    pub fn insight_clear_records(&self, f: &RecordFilters) -> InsightCleared {
        self.record_store().clear(&self.insight_scope(), f)
    }

    /// `getTodayStats`
    pub fn insight_today_stats(&self) -> Vec<InsightTodayStat> {
        let mut st = self.insight_state.lock_or_panic();
        let today = start_of_day();
        if today > st.today_date {
            st.today_date = today;
            st.today_triggers.clear();
        }
        let mut rows: Vec<(&String, &Vec<i64>)> = st.today_triggers.iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        rows.into_iter()
            .map(|(sid, ts)| {
                let times: Vec<String> = ts
                    .iter()
                    .map(|t| {
                        Local
                            .timestamp_millis_opt(*t)
                            .single()
                            .map(|d| format!("{:02}:{:02}", d.hour(), chrono::Timelike::minute(&d)))
                            .unwrap_or_default()
                    })
                    .collect();
                InsightTodayStat {
                    session_id: sid.clone(),
                    count: ts.len(),
                    times,
                }
            })
            .collect()
    }

    // ── connection test / footprint ──

    /// `testConnection`: asks the model API for a short answer, and says where it asked and what came back.
    pub async fn insight_test_connection(&self) -> InsightReply {
        let api = self.model_api();
        if let Some(missing) = model_api_missing(&api) {
            return InsightReply::failed(missing);
        }
        let prompt = Prompt {
            system: None,
            user: tr(
                "Please reply with the two words \"connection successful\".",
                "请回复\"连接成功\"四个字。",
            ),
        };
        let started = Instant::now();
        match api.complete(&prompt, PROBE_TIMEOUT).await {
            Ok(reply) => InsightReply {
                probe: Some(ModelProbe {
                    format: api.format,
                    endpoint: api.endpoint(),
                    model: reply.model.unwrap_or_else(|| api.model.clone()),
                    duration_ms: millis_since(started),
                    retries: reply.retries,
                    reply: reply.text.clone(),
                }),
                ..InsightReply::done(fill(
                    tr(
                        "Connection successful. Model reply: {v0}",
                        "连接成功，模型回复：{v0}",
                    ),
                    &[("v0", reply.text.chars().take(50).collect())],
                ))
            },
            Err(e) => InsightReply::failed(fill(
                tr("Connection failed: {message}", "连接失败：{message}"),
                &[("message", e.to_string())],
            )),
        }
    }

    /// `insight models`: the models the model API offers.
    ///
    /// # Errors
    ///
    /// The settings lack a key, or the service answered with an error or something that is not a list of models.
    pub async fn insight_models(&self) -> AppResult<ModelList> {
        let api = self.model_api();
        if api.key.trim().is_empty() && api.base_url.trim().is_empty() {
            return Err(AppError::config(tr(
                "Please enter the API key first",
                "请先填写 API Key",
            )));
        }
        let models = api
            .list_models(PROBE_TIMEOUT)
            .await
            .map_err(|e| AppError::runtime(e.to_string()))?;
        Ok(ModelList {
            format: api.format,
            endpoint: api.models_endpoint(),
            models,
        })
    }

    /// `generateFootprintInsight`
    pub async fn insight_footprint_summary(&self, params: &Value) -> InsightReply {
        let c = self.insight_cfg();
        let p = &self.profile_name;
        if c.get_key(Some(p), "aiFootprintEnabled") != Value::Bool(true) {
            return InsightReply::failed(tr(
                "Please turn on \"AI Footprint summary\" in Settings first",
                "请先在设置中开启「AI 足迹总结」",
            ));
        }
        let api = self.model_api();
        if let Some(missing) = model_api_missing(&api) {
            return InsightReply::failed(missing);
        }
        let num = |v: Option<&Value>| {
            v.and_then(|v| {
                v.as_f64()
                    .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
            })
            .filter(|n| n.is_finite())
            .unwrap_or(0.0)
        };
        let summary = params.get("summary").cloned().unwrap_or_else(|| json!({}));
        let range_label = params
            .get("rangeLabel")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| tr("Current range", "当前范围").to_string());
        let list = |k: &str| -> Vec<Value> {
            params
                .get(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().take(6).cloned().collect())
                .unwrap_or_default()
        };
        let (private, mentions) = (list("privateSegments"), list("mentionGroups"));
        let name_of = |item: &Value, fallback: String| {
            item.get("displayName")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    item.get("session_id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                })
                .map(|s| s.trim().to_string())
                .unwrap_or(fallback)
        };
        let top_private = if private.is_empty() {
            tr("None", "无").to_string()
        } else {
            private
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    let name = name_of(
                        it,
                        fill(
                            tr("Contact {v0}", "联系人{v0}"),
                            &[("v0", (i + 1).to_string())],
                        ),
                    );
                    let (inb, outb) = (
                        crate::num::saturating_i64(num(it.get("incoming_count"))),
                        crate::num::saturating_i64(num(it.get("outgoing_count"))),
                    );
                    let total =
                        crate::num::saturating_i64(num(it.get("message_count"))).max(inb + outb);
                    let replied = if it.get("replied").and_then(Value::as_bool) == Some(true) {
                        tr("/replied", "/已回复")
                    } else {
                        ""
                    };
                    tr(
                        "{v0}. {name} (in {inbound}/out {outbound}/total {total}{v5})",
                        "{v0}. {name}（收{inbound}/发{outbound}/总{total}{v5}）",
                    )
                    .replace("{v0}", &(i + 1).to_string())
                    .replace("{name}", &name)
                    .replace("{inbound}", &inb.to_string())
                    .replace("{outbound}", &outb.to_string())
                    .replace("{total}", &total.to_string())
                    .replace("{v5}", replied)
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let top_mention = if mentions.is_empty() {
            tr("None", "无").to_string()
        } else {
            mentions
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    let name = name_of(
                        it,
                        fill(tr("Group {v0}", "群聊{v0}"), &[("v0", (i + 1).to_string())]),
                    );
                    tr(
                        "{v0}. {name} (@-mentioned me {count} times)",
                        "{v0}. {name}（@我 {count} 次）",
                    )
                    .replace("{v0}", &(i + 1).to_string())
                    .replace("{name}", &name)
                    .replace(
                        "{count}",
                        &crate::num::saturating_i64(num(it.get("count"))).to_string(),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let custom = cfg_str(&c, p, "aiFootprintSystemPrompt");
        let system = if custom.is_empty() {
            ins::default_footprint_prompt().to_string()
        } else {
            custom
        };
        let tpl = tr(
            "Range: {rangeLabel}\nPeople chatted with: {v1}\nPeople I replied to: {v2}\nReply rate: {v3}%\n@-mentions of me: {v4}\nGroups involved: {v5}\n\nPrivate chat highlights:\n{topPrivateText}\n\nGroup @-mention highlights:\n{topMentionText}\n\nGive the footprint review (2-3 sentences, with a suggestion):",
            "统计范围：{rangeLabel}\n有聊天的人数：{v1}\n我有回复的人数：{v2}\n回复率：{v3}%\n@我次数：{v4}\n涉及群聊：{v5}\n\n私聊重点：\n{topPrivateText}\n\n群聊@我重点：\n{topMentionText}\n\n请给出足迹复盘（2-3句，含建议）：",
        );
        let prompt = tpl
            .replace("{rangeLabel}", &range_label)
            .replace(
                "{v1}",
                &crate::num::saturating_i64(num(summary.get("private_inbound_people"))).to_string(),
            )
            .replace(
                "{v2}",
                &crate::num::saturating_i64(num(summary.get("private_outbound_people")))
                    .to_string(),
            )
            .replace(
                "{v3}",
                &format!("{:.1}", num(summary.get("private_reply_rate")) * 100.0),
            )
            .replace(
                "{v4}",
                &crate::num::saturating_i64(num(summary.get("mention_count"))).to_string(),
            )
            .replace(
                "{v5}",
                &crate::num::saturating_i64(num(summary.get("mention_group_count"))).to_string(),
            )
            .replace("{topPrivateText}", &top_private)
            .replace("{topMentionText}", &top_mention);
        let user = ins::append_current_time(&prompt);
        let prompt = Prompt {
            system: Some(&system),
            user: &user,
        };
        match api.complete(&prompt, GENERATE_TIMEOUT).await {
            Ok(reply) => InsightReply {
                insight: Some(reply.text),
                ..InsightReply::done(tr("Generated successfully", "生成成功"))
            },
            Err(e) => InsightReply::failed(fill(
                tr("Generation failed: {message}", "生成失败：{message}"),
                &[("message", e.to_string())],
            )),
        }
    }

    // ── sessions ──

    fn insight_sessions(&self, force: bool) -> Vec<Session> {
        {
            let st = self.insight_state.lock_or_panic();
            if let (false, Some((s, at))) = (force, &st.session_cache) {
                if at.elapsed() < std::time::Duration::from_secs(15 * 60) {
                    return s.clone();
                }
            }
        }
        match self.chat_sessions_list() {
            Ok(list) => {
                self.insight_state.lock_or_panic().session_cache =
                    Some((list.clone(), std::time::Instant::now()));
                list
            }
            Err(_) => self
                .insight_state
                .lock_or_panic()
                .session_cache
                .as_ref()
                .map(|(s, _)| s.clone())
                .unwrap_or_default(),
        }
    }

    async fn insight_display_name(&self, session_id: &str, fallback: &str) -> String {
        let fb = fallback.trim();
        if !fb.is_empty() && !ins::looks_like_wxid(fb) {
            return fb.to_string();
        }
        for s in self.insight_sessions(false) {
            if s.username.trim() == session_id {
                let n = s.display_name.trim();
                if !n.is_empty() && !ins::looks_like_wxid(n) {
                    return n.to_string();
                }
            }
        }
        if let Some((_, name)) = self.chat_contact_avatar(session_id) {
            let n = name.trim();
            if !n.is_empty() && !ins::looks_like_wxid(n) {
                return n.to_string();
            }
        }
        if fb.is_empty() {
            session_id.to_string()
        } else {
            fb.to_string()
        }
    }

    async fn insight_moments_section(&self, session_id: &str) -> String {
        let c = self.insight_cfg();
        let p = &self.profile_name;
        if c.get_key(Some(p), "aiInsightAllowMomentsContext") != Value::Bool(true) {
            return String::new();
        }
        if c.get_key(Some(p), "aiInsightMomentsBindings")
            .get(session_id)
            .and_then(|b| b.get("enabled"))
            .and_then(Value::as_bool)
            != Some(true)
        {
            return String::new();
        }
        let raw = c
            .get_key(Some(p), "aiInsightMomentsContextCount")
            .as_f64()
            .unwrap_or(5.0);
        let count = if raw.floor() >= 1.0 {
            crate::num::saturating_i64(raw.floor()).clamp(1, 20)
        } else {
            5
        };
        let Ok(posts) = self.sns_timeline_query(&super::sns::SnsTimelineQuery {
            limit: crate::num::saturating_i32(count),
            offset: 0,
            usernames: vec![session_id.to_string()],
            keyword: None,
            start: 0,
            end: 0,
        }) else {
            return String::new();
        };
        let lines: Vec<String> = posts
            .iter()
            .filter_map(|post| {
                let norm = |s: &str| {
                    ins::normalize_insight_text(s)
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                let text = norm(&post.content_desc);
                if text.is_empty() {
                    return None;
                }
                let short = if text.chars().count() > 180 {
                    format!("{}...", text.chars().take(180).collect::<String>())
                } else {
                    text
                };
                let ts = Some(post.create_time).filter(|n| *n > 0).map(|n| {
                    if n > 1_000_000_000_000 {
                        n
                    } else {
                        n * 1000
                    }
                });
                let time = ts
                    .and_then(|ms| Local.timestamp_millis_opt(ms).single())
                    .map(|d| d.format("%Y/%m/%d %H:%M:%S").to_string())
                    .unwrap_or_default();
                Some(if time.is_empty() {
                    fill(
                        tr("[Moments] {shortText}", "[朋友圈] {shortText}"),
                        &[("shortText", short)],
                    )
                } else {
                    fill(
                        tr(
                            "[Moments {time}] {shortText}",
                            "[朋友圈 {time}] {shortText}",
                        ),
                        &[("time", time), ("shortText", short)],
                    )
                })
            })
            .collect();
        if lines.is_empty() {
            return String::new();
        }
        fill(
            tr(
                "Recent Moments (latest {length} posts):\n{v1}",
                "近期朋友圈内容（最近 {length} 条）：\n{v1}",
            ),
            &[
                ("length", lines.len().to_string()),
                ("v1", lines.join("\n")),
            ],
        )
    }

    async fn insight_social_section(&self, session_id: &str) -> String {
        let c = self.insight_cfg();
        let p = &self.profile_name;
        if c.get_key(Some(p), "aiInsightAllowSocialContext") != Value::Bool(true) {
            return String::new();
        }
        let cookie = cfg_str(&c, p, "aiInsightWeiboCookie");
        let uid = c
            .get_key(Some(p), "aiInsightWeiboBindings")
            .get(session_id)
            .and_then(|b| b.get("uid"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if uid.is_empty() {
            return String::new();
        }
        let raw = c
            .get_key(Some(p), "aiInsightSocialContextCount")
            .as_f64()
            .unwrap_or(3.0);
        let count = if raw.floor() >= 1.0 {
            crate::num::saturating_i64(raw.floor()).clamp(1, 5)
        } else {
            3
        };
        let Ok(posts) = crate::weibo::fetch_recent_posts(&uid, &cookie, count).await else {
            return String::new();
        };
        if posts.is_empty() {
            return String::new();
        }
        let lines: Vec<String> = posts
            .iter()
            .map(|post| {
                let time = chrono::DateTime::parse_from_rfc2822(&post.created_at)
                    .or_else(|_| {
                        chrono::DateTime::parse_from_str(
                            &post.created_at,
                            "%a %b %d %H:%M:%S %z %Y",
                        )
                    })
                    .map(|d| {
                        d.with_timezone(&Local)
                            .format("%Y/%m/%d %H:%M:%S")
                            .to_string()
                    })
                    .unwrap_or_else(|_| post.created_at.trim().to_string());
                let text = if post.text.chars().count() > 180 {
                    format!("{}...", post.text.chars().take(180).collect::<String>())
                } else {
                    post.text.clone()
                };
                fill(
                    tr("[Weibo {time}] {text}", "[微博 {time}] {text}"),
                    &[("time", time), ("text", text)],
                )
            })
            .collect();
        fill(tr("Recent public social platform content (source: Weibo, latest {length} posts):\n{v1}", "近期公开社交平台内容（来源：微博，最近 {length} 条）：\n{v1}"), &[("length", lines.len().to_string()), ("v1", lines.join("\n"))])
    }

    // ── generation ──

    /// `generateInsightForSession`: the stored record when an insight was produced; `None` when insights are off or
    /// the model had nothing to say (`SKIP`).
    ///
    /// # Errors
    ///
    /// The model API is not set up, or asking it failed; the text says why.
    pub async fn insight_generate(
        &self,
        session_id: &str,
        display_name: &str,
        reason: Trigger,
        silent_days: Option<i64>,
    ) -> Result<Option<ins::InsightRecord>, String> {
        if session_id.is_empty() || !self.insight_enabled() && reason != Trigger::Test {
            return Ok(None);
        }
        let api = self.model_api();
        let c = self.insight_cfg();
        let p = &self.profile_name;
        let allow_context = self.insight_sends_chat();
        let ctx_raw = c
            .get_key(Some(p), "aiInsightContextCount")
            .as_f64()
            .unwrap_or(0.0);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a message count from settings; `as` saturates"
        )]
        let context_count = if ctx_raw >= 1.0 { ctx_raw as usize } else { 40 };
        let name = self.insight_display_name(session_id, display_name).await;
        let avatar = self
            .chat_contact_avatar(session_id)
            .and_then(|(a, _)| a)
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty());
        if let Some(missing) = model_api_missing(&api) {
            return Err(missing.to_string());
        }
        let mut context = String::new();
        if allow_context {
            if let Ok(msgs) = self.chat_latest_messages(session_id, context_count) {
                context = ins::build_context_section(&msgs, &name);
            }
        }
        let moments = self.insight_moments_section(session_id).await;
        let social = self.insight_social_section(session_id).await;
        let custom = cfg_str(&c, p, "aiInsightSystemPrompt");
        let system = if custom.trim().is_empty() {
            ins::default_system_prompt().to_string()
        } else {
            custom
        };
        let silence = match (reason, silent_days) {
            (Trigger::Silence, Some(d)) if d != 0 => fill(
                tr(
                    "It has been {silentDays} days since you contacted \"{resolvedDisplayName}\".",
                    "已 {silentDays} 天未联系「{resolvedDisplayName}」。",
                ),
                &[
                    ("silentDays", d.to_string()),
                    ("resolvedDisplayName", name.clone()),
                ],
            ),
            _ => String::new(),
        };
        let ask = tr(
            "Give your insight (under 50 words):",
            "请给出你的见解（≤80字）：",
        )
        .to_string();
        let base = [silence, context, moments, social, ask]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let user = ins::append_current_time(&base);
        let started = Instant::now();
        let prompt = Prompt {
            system: Some(&system),
            user: &user,
        };
        let reply = api.complete(&prompt, GENERATE_TIMEOUT).await.map_err(|e| {
            fill(
                tr(
                    "Asking the model about {name} failed: {message}",
                    "为「{name}」请求模型失败：{message}",
                ),
                &[("name", name.clone()), ("message", e.to_string())],
            )
        })?;
        let duration = i64::try_from(millis_since(started)).unwrap_or(i64::MAX);
        let result = reply.text;
        if result.eq_ignore_ascii_case("SKIP") || result.starts_with("SKIP") {
            return Ok(None);
        }
        if !self.insight_enabled() && reason != Trigger::Test {
            return Ok(None);
        }
        let insight = result.clone();
        let log = ins::InsightRecordLog {
            format: api.format,
            endpoint: api.endpoint(),
            model: reply.model.unwrap_or_else(|| api.model.clone()),
            max_tokens: api.max_tokens,
            trigger_reason: reason.as_str().into(),
            allow_context,
            context_count: context_count as u64,
            system_prompt: system,
            user_prompt: user,
            raw_output: result.clone(),
            final_insight: insight.clone(),
            duration_ms: duration,
            created_at: ins::now_millis(),
        };
        let record =
            self.record_store_add(session_id, &name, avatar, reason.as_str(), &insight, log);
        if c.get_key(Some(p), "aiInsightTelegramEnabled") == Value::Bool(true) {
            let token = cfg_str(&c, p, "aiInsightTelegramToken");
            let chat_ids = cfg_str(&c, p, "aiInsightTelegramChatIds");
            if !token.is_empty() && !chat_ids.is_empty() {
                let title = fill(
                    tr(
                        "Insight · {resolvedDisplayName}",
                        "见解 · {resolvedDisplayName}",
                    ),
                    &[("resolvedDisplayName", name.clone())],
                );
                let text = format!("【WeFlow】 {title}\n\n{insight}");
                for id in chat_ids.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if let Err(e) = ins::send_telegram(&token, id, &text).await {
                        eprintln!(
                            "[insight] {}",
                            crate::locale::localize(format!(
                                "Telegram push failed (chatId={id}): {e}"
                            ))
                        );
                    }
                }
            } else {
                eprintln!(
                    "[insight] {}",
                    crate::locale::tr(
                        "Telegram is enabled but the token or chat id is missing",
                        "已启用 Telegram，但缺少令牌或聊天 ID"
                    )
                );
            }
        }
        let mut st = self.insight_state.lock_or_panic();
        let today = start_of_day();
        if today > st.today_date {
            st.today_date = today;
            st.today_triggers.clear();
        }
        st.today_triggers
            .entry(session_id.to_string())
            .or_default()
            .push(ins::now_millis());
        Ok(Some(record))
    }

    fn record_store_add(
        &self,
        session_id: &str,
        name: &str,
        avatar: Option<String>,
        reason: &str,
        insight: &str,
        log: ins::InsightRecordLog,
    ) -> ins::InsightRecord {
        let mut store = self.record_store();
        store.add(
            ins::NewRecord {
                scope: &self.insight_scope(),
                session_id,
                display_name: name,
                avatar_url: avatar,
                trigger_reason: reason,
                insight,
            },
            log,
        )
    }

    /// `insight trigger <session>`: a test insight for that conversation, whatever the filters say.
    pub async fn insight_trigger_session(&self, session_id: &str) -> InsightReply {
        let name = self
            .chat_contact_avatar(session_id)
            .map(|(_, n)| n)
            .unwrap_or_else(|| session_id.to_string());
        match self
            .insight_generate(session_id, &name, Trigger::Test, None)
            .await
        {
            Ok(Some(record)) => InsightReply {
                message: None,
                record: Some(record.summary()),
                ..InsightReply::done("")
            },
            Ok(None) => InsightReply::failed(skipped_message(&name, self.insight_sends_chat())),
            Err(why) => InsightReply::failed(why),
        }
    }

    /// `triggerTest`: forces an insight for the first eligible private chat.
    pub async fn insight_trigger_test(&self) -> InsightReply {
        if let Some(missing) = model_api_missing(&self.model_api()) {
            return InsightReply::failed(missing);
        }
        let sessions = match self.chat_sessions_list() {
            Ok(s) => s,
            Err(_) => {
                return InsightReply::failed(
                    tr("Database connection failed. Please finish the setup on the \"Database connection\" page first", "数据库连接失败，请先在\"数据库连接\"页完成配置"),
                )
            }
        };
        if sessions.is_empty() {
            return InsightReply::failed(tr(
                "No conversations found. Please make sure the database is connected correctly",
                "未找到任何会话，请确认数据库已正确连接",
            ));
        }
        let Some(session) = sessions.iter().find(|s| {
            let id = s.username.trim();
            !id.is_empty()
                && !id.ends_with("@chatroom")
                && !id.to_lowercase().contains("placeholder")
                && self.insight_allowed(id)
        }) else {
            return InsightReply::failed(
                tr("No private chat that can be triggered was found (check the allowlist/blocklist mode and the selection list)", "未找到任何可触发的私聊会话（请检查黑白名单模式与选择列表）"),
            );
        };
        let id = session.username.trim().to_string();
        let name = Some(session.display_name.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&id)
            .to_string();
        let record = match self.insight_generate(&id, &name, Trigger::Test, None).await {
            Ok(Some(record)) => record,
            Ok(None) => {
                return InsightReply::failed(skipped_message(&name, self.insight_sends_chat()))
            }
            Err(why) => return InsightReply::failed(why),
        };
        let c = self.insight_cfg();
        let notify = c.get_key(Some(&self.profile_name), "aiInsightNotificationEnabled")
            != Value::Bool(false);
        let message = if notify {
            fill(
                tr(
                    "A test insight was sent for \"{displayName}\". Check the notification popup",
                    "已向「{displayName}」发送测试见解，请查看通知弹窗",
                ),
                &[("displayName", name)],
            )
        } else {
            fill(tr("A test insight was generated for \"{displayName}\", but AI insight notifications are currently off", "已生成「{displayName}」的测试见解，AI 见解消息通知当前已关闭"), &[("displayName", name)])
        };
        InsightReply {
            record: Some(record.summary()),
            ..InsightReply::done(message)
        }
    }

    /// `runSilenceScan`
    pub async fn insight_silence_scan(
        &self,
        on_insight: &mut dyn FnMut(&ins::InsightRecord),
    ) -> usize {
        if !self.insight_enabled() {
            return 0;
        }
        let c = self.insight_cfg();
        let days_raw = c
            .get_key(Some(&self.profile_name), "aiInsightSilenceDays")
            .as_f64()
            .unwrap_or(0.0);
        let days = if days_raw != 0.0 { days_raw } else { 3.0 };
        let threshold_ms = days * 86_400_000.0;
        let now = ins::now_millis() as f64;
        let sessions = self.insight_sessions(true);
        let mut generated = 0;
        for s in &sessions {
            if !self.insight_enabled() {
                return generated;
            }
            let id = s.username.trim().to_string();
            if id.is_empty()
                || id.ends_with("@chatroom")
                || id.to_lowercase().contains("placeholder")
                || !self.insight_allowed(&id)
            {
                continue;
            }
            let last = s.last_timestamp as f64 * 1000.0;
            if last <= 0.0 || now - last < threshold_ms {
                continue;
            }
            let silent = crate::num::saturating_i64(((now - last) / 86_400_000.0).floor());
            let name = s.display_name.clone();
            match self
                .insight_generate(&id, &name, Trigger::Silence, Some(silent))
                .await
            {
                Ok(Some(r)) => {
                    generated += 1;
                    on_insight(&r);
                }
                Ok(None) => {}
                Err(why) => {
                    log_failure(&why);
                    // the settings are missing: every other conversation would fail alike
                    if model_api_missing(&self.model_api()).is_some() {
                        return generated;
                    }
                }
            }
        }
        generated
    }

    /// `analyzeRecentActivity`: at most one session per call, honouring the cooldown.
    pub async fn insight_analyze_activity(&self, on_insight: &mut dyn FnMut(&ins::InsightRecord)) {
        if !self.insight_enabled() {
            return;
        }
        let c = self.insight_cfg();
        let cooldown_min = c
            .get_key(Some(&self.profile_name), "aiInsightCooldownMinutes")
            .as_f64()
            .unwrap_or(120.0);
        let cooldown_ms = crate::num::saturating_i64(cooldown_min * 60_000.0);
        let now = ins::now_millis();
        let (mode, list) = self.insight_filter();
        let cooling = |sid: &str| -> bool {
            cooldown_ms > 0
                && cooldown_ms
                    - (now
                        - self
                            .insight_state
                            .lock_or_panic()
                            .last_activity
                            .get(sid)
                            .copied()
                            .unwrap_or(0))
                    > 0
        };
        if mode == "whitelist" {
            for sid in &list {
                if sid.is_empty() || sid.to_lowercase().contains("placeholder") || cooling(sid) {
                    continue;
                }
                let Ok(latest) = self.chat_latest_messages(sid, 1) else {
                    continue;
                };
                let Some(m) = latest.last() else { continue };
                let ts = m.create_time;
                {
                    let mut st = self.insight_state.lock_or_panic();
                    if ts <= st.last_seen.get(sid).copied().unwrap_or(0) {
                        continue;
                    }
                    st.last_seen.insert(sid.clone(), ts);
                    st.last_activity.insert(sid.clone(), now);
                }
                match self
                    .insight_generate(sid, sid, Trigger::Activity, None)
                    .await
                {
                    Ok(Some(r)) => on_insight(&r),
                    Ok(None) => {}
                    Err(why) => log_failure(&why),
                }
                break;
            }
            return;
        }
        let sessions = self.insight_sessions(false);
        let candidates: Vec<&Session> = sessions
            .iter()
            .filter(|s| {
                let id = s.username.trim();
                !id.is_empty()
                    && !id.to_lowercase().contains("placeholder")
                    && self.insight_allowed(id)
            })
            .take(10)
            .collect();
        for s in candidates {
            let sid = s.username.trim().to_string();
            let ts = s.last_timestamp;
            {
                let mut st = self.insight_state.lock_or_panic();
                if ts <= st.last_seen.get(&sid).copied().unwrap_or(0) {
                    continue;
                }
                st.last_seen.insert(sid.clone(), ts);
            }
            if cooling(&sid) {
                continue;
            }
            self.insight_state
                .lock_or_panic()
                .last_activity
                .insert(sid.clone(), now);
            let name = s.display_name.clone();
            match self
                .insight_generate(&sid, &name, Trigger::Activity, None)
                .await
            {
                Ok(Some(r)) => on_insight(&r),
                Ok(None) => {}
                Err(why) => log_failure(&why),
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_tokens_are_read_and_clamped() {
        assert_eq!(max_tokens_of(&json!(0)), 1);
        assert_eq!(max_tokens_of(&json!("abc")), DEFAULT_MAX_TOKENS);
        assert_eq!(max_tokens_of(&Value::Null), DEFAULT_MAX_TOKENS);
        assert_eq!(max_tokens_of(&json!(9_999_999_999i64)), LARGEST_MAX_TOKENS);
        assert_eq!(max_tokens_of(&json!(512.9)), 512);
        assert_eq!(max_tokens_of(&json!(" 300 ")), 300);
    }

    #[test]
    fn a_key_or_an_own_service_and_a_model_are_needed() {
        let api = |base: &str, key: &str, model: &str| ModelApi {
            base_url: base.into(),
            key: key.into(),
            model: model.into(),
            ..ModelApi::default()
        };
        assert!(model_api_missing(&api("", "", "m")).is_some());
        assert!(model_api_missing(&api("", "k", "")).is_some());
        assert!(model_api_missing(&api("", "k", "m")).is_none());
        // a service of one's own may take no key
        assert!(model_api_missing(&api("http://127.0.0.1:11434/v1", "", "m")).is_none());
    }

    #[test]
    fn a_skip_without_the_chat_says_how_to_send_it() {
        let sent = skipped_message("Bob", true);
        assert!(
            sent.contains("SKIP") && !sent.contains("aiInsightAllowContext"),
            "{sent}"
        );
        let unsent = skipped_message("Bob", false);
        assert!(
            unsent.starts_with(&sent)
                && unsent.contains("weflow config set aiInsightAllowContext true"),
            "{unsent}"
        );
    }
}
