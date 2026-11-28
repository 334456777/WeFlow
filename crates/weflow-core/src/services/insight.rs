//! 移植自桌面版的 `InsightService` 运行时：静默扫描、活动触发、见解生成（可选附带聊天、朋友圈、微博的内容）、
//! Telegram 推送，以及测试和足迹的辅助功能。桌面版的通知弹窗在命令行中没有对应，
//! 生成的每条见解都交给调用方（`serve --insight` 的循环把它打印成一行 JSON）。
use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone, Timelike};
use serde_json::{json, Value};

use super::session_kind::{is_private_session, session_kind, SessionKind};
use super::*;
use crate::config::keys;
use crate::contract::{
    InsightClearPlan, InsightCleared, InsightMarked, InsightRecords, InsightReply,
    InsightTodayStat, ModelList, ModelProbe, Session,
};
use crate::insight::{self as ins, fill, RecordFilters, RecordStore};
use crate::locale::tr;
use crate::model_api::{ModelApi, Prompt};
use crate::num::i64_f64;
use crate::sync::MutexExt;

/// `insight test` 和 `insight models` 的一次请求最多可用的时间。
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// 请求一条见解或足迹总结最多可用的时间：先推理的模型可能要花一阵。
const GENERATE_TIMEOUT: Duration = Duration::from_secs(60);

/// 见解引擎在调用之间保留的状态。
#[derive(Default)]
pub struct InsightState {
    today_triggers: HashMap<String, Vec<i64>>,
    today_date: i64,
    /// 上次为某个会话写下活动见解（或在记录中找到它）的时间，Unix 毫秒。
    last_activity: HashMap<String, i64>,
    /// 在某个会话中看到的最新一条消息的时间，Unix 秒。
    last_seen: HashMap<String, i64>,
    /// 引擎第一次检查活动的时间，Unix 秒：还没见过的会话只有这之后的消息才算新消息，
    /// 引擎启动时已有的不算（#1140）。
    activity_since: Option<i64>,
    session_cache: Option<(Vec<Session>, Instant)>,
}

/// `api` 还不能提问的原因，用设置里的说法：需要模型名，还需要密钥，除非基础地址指向自建的服务（本地服务可能不要密钥）。
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

/// 测试触发时模型对 `name` 没有可说的内容时的提示。
///
/// 不发送最近的消息（`ai_insight_allow_context` 关闭）时模型几乎什么都看不到，这是常见的原因。
fn skipped_message(name: &str, chat_sent: bool) -> String {
    let skipped = fill(
        tr(
            "The model had nothing to say about \"{name}\": it answered SKIP, so no insight was written",
            "模型认为「{name}」没有可说的：它回答了 SKIP，没有生成见解",
        ),
        &[("name", name.to_string())],
    );
    if chat_sent {
        return skipped;
    }
    skipped
        + tr(
            "\nThe recent messages are not sent because ai_insight_allow_context is off, so the model saw no chat. Turn it on with `weflow config set ai_insight_allow_context true`.",
            "\n没有开启发送聊天内容 ai_insight_allow_context，模型没有收到聊天记录。可以用 `weflow config set ai_insight_allow_context true` 开启。",
        )
}

/// 引擎的一次失败：在 stderr 上给出警告，然后继续处理下一个会话。
fn log_failure(why: &str) {
    crate::output::warn(&format!("{}{why}", tr("insight: ", "洞察：")));
}

/// 引擎写见解的原因，即记录的 `triggerReason`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trigger {
    /// 会话中有新消息（`activity`）。
    Activity,
    /// 会话静默了一段时间（`silence`）。
    Silence,
    /// `insight trigger` 要求的（`test`），见解关闭时也生成。
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

/// 两次静默扫描的间隔；`hours` 是从 `ai_insight_scan_interval_hours` 读出的值，
/// 已在它的取值范围内（[`keys::AI_INSIGHT_SCAN_INTERVAL_HOURS`]）。
fn scan_interval(hours: f64) -> Duration {
    Duration::from_secs_f64(hours * 3600.0)
}

/// 静默扫描和不指定会话的 `insight trigger` 能否自己选中 `id`：只选和另一个人的私聊，
/// 不选群、公众号、文件传输助手和系统会话。
fn picked_as_private(id: &str, own_wxid: &str) -> bool {
    is_private_session(id, own_wxid) && !id.to_lowercase().contains("placeholder")
}

/// 黑名单模式的活动扫描能否自己选中 `id`：选私聊和群，不选公众号、文件传输助手、系统会话和账号自己的会话。
/// 白名单模式处理名单中的会话，不论是哪一类。
fn picked_for_activity(id: &str, own_wxid: &str) -> bool {
    matches!(
        session_kind(id, own_wxid),
        SessionKind::Private | SessionKind::Group
    ) && !id.to_lowercase().contains("placeholder")
}

fn cfg_str(cfg: &ConfigStore, profile: &str, key: &str) -> String {
    match cfg.get_key(Some(profile), key) {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

impl ServiceHub {
    fn insight_cfg(&self) -> ConfigStore {
        self.fresh_config()
    }

    /// 档案设置中的模型接口（`ai_model_api_*`）。
    fn model_api(&self) -> ModelApi {
        let c = self.insight_cfg();
        let p = &self.profile_name;
        ModelApi {
            format: c.setting(Some(p), &keys::AI_MODEL_API_FORMAT),
            base_url: cfg_str(&c, p, "ai_model_api_base_url"),
            key: cfg_str(&c, p, "ai_model_api_key"),
            model: cfg_str(&c, p, "ai_model_api_model"),
            max_tokens: c
                .setting(Some(p), &keys::AI_MODEL_API_MAX_TOKENS)
                .unsigned_abs(),
        }
    }

    /// 档案是否开启了 AI 见解（`ai_insight_enabled`）。
    pub fn insight_enabled(&self) -> bool {
        self.insight_cfg()
            .setting(Some(&self.profile_name), &keys::AI_INSIGHT_ENABLED)
    }

    /// `serve --insight` 是否打印它写下的见解（`ai_insight_notification_enabled`）。
    ///
    /// 与其他见解设置一样每次从配置文件读取，所以 `serve` 运行期间的修改从下一轮起生效。
    pub fn insight_notifies(&self) -> bool {
        self.insight_cfg().setting(
            Some(&self.profile_name),
            &keys::AI_INSIGHT_NOTIFICATION_ENABLED,
        )
    }

    /// `serve --insight` 两次静默扫描的间隔（`ai_insight_scan_interval_hours`），每次从配置文件读取。
    pub fn insight_scan_interval(&self) -> Duration {
        scan_interval(self.insight_cfg().setting(
            Some(&self.profile_name),
            &keys::AI_INSIGHT_SCAN_INTERVAL_HOURS,
        ))
    }

    /// 最近的消息是否随提问发给模型（`ai_insight_allow_context`）。
    fn insight_sends_chat(&self) -> bool {
        self.insight_cfg()
            .setting(Some(&self.profile_name), &keys::AI_INSIGHT_ALLOW_CONTEXT)
    }

    fn insight_filter(&self) -> (String, Vec<String>) {
        let c = self.insight_cfg();
        (
            c.setting(Some(&self.profile_name), &keys::AI_INSIGHT_FILTER_MODE)
                .to_string(),
            ins::normalize_session_id_list(
                &c.get_key(Some(&self.profile_name), "ai_insight_filter_list"),
            ),
        )
    }

    fn insight_allowed(&self, session_id: &str) -> bool {
        let (mode, list) = self.insight_filter();
        ins::session_allowed(&mode, &list, session_id)
    }

    /// 见解记录所属的账号：`wxid:<wxid>`；没有 wxid 时为 `db:<hash>`（数据目录路径的 SHA-1 的前 16 位十六进制）；
    /// 都没有时为 `default`。
    pub(super) fn insight_scope(&self) -> String {
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

    /// 当前账号的见解记录。
    fn record_store(&self) -> RecordStore {
        RecordStore::load(&self.ctx.home_dir, &self.insight_scope())
    }

    // ── 记录 ──

    /// 账号见解记录中的一页（`insight records`，见 [`RecordStore::list`]）。
    pub fn insight_list_records(&self, f: &RecordFilters) -> InsightRecords {
        self.record_store().list(f)
    }

    /// 账号的一条见解记录（`insight get`，见 [`RecordStore::get`]）。
    ///
    /// # Errors
    /// 同 [`RecordStore::get`]。
    pub fn insight_get_record(&self, id: &str) -> AppResult<crate::contract::InsightLookup> {
        self.record_store().get(id)
    }

    /// 把账号的一条见解记录标为已读（`insight mark-read`，见 [`RecordStore::mark_read`]）。
    ///
    /// # Errors
    /// 同 [`RecordStore::mark_read`]。
    pub fn insight_mark_record_read(&self, id: &str) -> AppResult<InsightMarked> {
        self.record_store().mark_read(id)
    }

    /// 带 `f` 的 `insight clear` 会删除账号的多少条见解记录：用于它的 `--dry-run`，以及删除前询问时显示的条数。
    pub fn insight_clear_plan(&self, f: &RecordFilters) -> InsightClearPlan {
        InsightClearPlan {
            dry_run: true,
            records: self.record_store().clearable(f),
        }
    }

    /// 删除账号中 `f` 匹配的见解记录（`insight clear`，见 [`RecordStore::clear`]）。
    ///
    /// # Errors
    /// 同 [`RecordStore::clear`]。
    pub fn insight_clear_records(&self, f: &RecordFilters) -> AppResult<InsightCleared> {
        self.record_store().clear(f)
    }

    /// 引擎今天为每个会话写了几次、在什么时间（`insight today-stats`）。
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
                            .map(|d| format!("{:02}:{:02}", d.hour(), Timelike::minute(&d)))
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

    // ── 连接测试和足迹 ──

    /// 向模型接口要一句简短的回答，并说明问的是哪里、回答了什么。
    ///
    /// # Errors
    /// 设置中缺少模型接口的配置，或接口回答了错误时返回运行时错误。
    pub async fn insight_test_connection(&self) -> AppResult<InsightReply> {
        let api = self.model_api();
        if let Some(missing) = model_api_missing(&api) {
            return Err(AppError::runtime(missing));
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
            Ok(reply) => Ok(InsightReply {
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
                        "Connection successful. Model reply: {reply}",
                        "连接成功，模型回复：{reply}",
                    ),
                    &[("reply", reply.text.chars().take(50).collect())],
                ))
            }),
            Err(e) => Err(AppError::runtime(fill(
                tr("Connection failed: {message}", "连接失败：{message}"),
                &[("message", e.to_string())],
            ))),
        }
    }

    /// `insight models`：模型接口提供的模型。
    ///
    /// # Errors
    ///
    /// 设置中既没有密钥也没有基础地址时返回配置错误；服务回答了错误，或者回答的不是模型列表时返回运行时错误。
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

    /// 足迹的 AI 总结（`insight footprint-summary`）。
    ///
    /// # Errors
    /// 足迹总结没有开启、设置中缺少模型接口的配置，或接口回答了错误时返回运行时错误。
    pub async fn insight_footprint_summary(&self, params: &Value) -> AppResult<InsightReply> {
        let c = self.insight_cfg();
        let p = &self.profile_name;
        if !c.setting(Some(p), &keys::AI_FOOTPRINT_ENABLED) {
            return Err(AppError::runtime(tr(
                "Please turn on \"AI Footprint summary\" in Settings first",
                "请先在设置中开启「AI 足迹总结」",
            )));
        }
        let api = self.model_api();
        if let Some(missing) = model_api_missing(&api) {
            return Err(AppError::runtime(missing));
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
            .get("range_label")
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
        let (private, mentions) = (list("private_segments"), list("mention_groups"));
        let name_of = |item: &Value, fallback: String| {
            item.get("display_name")
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
                            tr("Contact {n}", "联系人{n}"),
                            &[("n", (i + 1).to_string())],
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
                        "{n}. {name} (in {inbound}/out {outbound}/total {total}{replied})",
                        "{n}. {name}（收{inbound}/发{outbound}/总{total}{replied}）",
                    )
                    .replace("{n}", &(i + 1).to_string())
                    .replace("{name}", &name)
                    .replace("{inbound}", &inb.to_string())
                    .replace("{outbound}", &outb.to_string())
                    .replace("{total}", &total.to_string())
                    .replace("{replied}", replied)
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
                        fill(tr("Group {n}", "群聊{n}"), &[("n", (i + 1).to_string())]),
                    );
                    tr(
                        "{n}. {name} (@-mentioned me {count} times)",
                        "{n}. {name}（@我 {count} 次）",
                    )
                    .replace("{n}", &(i + 1).to_string())
                    .replace("{name}", &name)
                    .replace(
                        "{count}",
                        &crate::num::saturating_i64(num(it.get("count"))).to_string(),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let custom = cfg_str(&c, p, "ai_footprint_system_prompt");
        let system = if custom.is_empty() {
            ins::default_footprint_prompt().to_string()
        } else {
            custom
        };
        let tpl = tr(
            "Range: {range}\nPeople chatted with: {people}\nPeople I replied to: {replied_people}\nReply rate: {reply_rate}%\n@-mentions of me: {mentions}\nGroups involved: {groups}\n\nPrivate chat highlights:\n{private_highlights}\n\nGroup @-mention highlights:\n{mention_highlights}\n\nGive the footprint review (2-3 sentences, with a suggestion):",
            "统计范围：{range}\n有聊天的人数：{people}\n我有回复的人数：{replied_people}\n回复率：{reply_rate}%\n@我次数：{mentions}\n涉及群聊：{groups}\n\n私聊重点：\n{private_highlights}\n\n群聊@我重点：\n{mention_highlights}\n\n请给出足迹复盘（2-3句，含建议）：",
        );
        let prompt = tpl
            .replace("{range}", &range_label)
            .replace(
                "{people}",
                &crate::num::saturating_i64(num(summary.get("private_inbound_people"))).to_string(),
            )
            .replace(
                "{replied_people}",
                &crate::num::saturating_i64(num(summary.get("private_outbound_people")))
                    .to_string(),
            )
            .replace(
                "{reply_rate}",
                &format!("{:.1}", num(summary.get("private_reply_rate")) * 100.0),
            )
            .replace(
                "{mentions}",
                &crate::num::saturating_i64(num(summary.get("mention_count"))).to_string(),
            )
            .replace(
                "{groups}",
                &crate::num::saturating_i64(num(summary.get("mention_group_count"))).to_string(),
            )
            .replace("{private_highlights}", &top_private)
            .replace("{mention_highlights}", &top_mention);
        let user = ins::append_current_time(&prompt);
        let prompt = Prompt {
            system: Some(&system),
            user: &user,
        };
        match api.complete(&prompt, GENERATE_TIMEOUT).await {
            Ok(reply) => Ok(InsightReply {
                insight: Some(reply.text),
                ..InsightReply::done(tr("Generated successfully", "生成成功"))
            }),
            Err(e) => Err(AppError::runtime(fill(
                tr("Generation failed: {message}", "生成失败：{message}"),
                &[("message", e.to_string())],
            ))),
        }
    }

    // ── 会话 ──

    fn insight_sessions(&self, force: bool) -> Vec<Session> {
        {
            let st = self.insight_state.lock_or_panic();
            if let (false, Some((s, at))) = (force, &st.session_cache) {
                if at.elapsed() < Duration::from_secs(15 * 60) {
                    return s.clone();
                }
            }
        }
        match self.chat_sessions_list() {
            Ok(list) => {
                self.insight_state.lock_or_panic().session_cache =
                    Some((list.clone(), Instant::now()));
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
        if !c.setting(Some(p), &keys::AI_INSIGHT_ALLOW_MOMENTS_CONTEXT) {
            return String::new();
        }
        if c.get_key(Some(p), "ai_insight_moments_bindings")
            .get(session_id)
            .and_then(|b| b.get("enabled"))
            .and_then(Value::as_bool)
            != Some(true)
        {
            return String::new();
        }
        let count = c.setting(Some(p), &keys::AI_INSIGHT_MOMENTS_CONTEXT_COUNT);
        let Ok(posts) = self.sns_timeline_query(&SnsTimelineQuery {
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
                        tr("[Moments] {short_text}", "[朋友圈] {short_text}"),
                        &[("short_text", short)],
                    )
                } else {
                    fill(
                        tr(
                            "[Moments {time}] {short_text}",
                            "[朋友圈 {time}] {short_text}",
                        ),
                        &[("time", time), ("short_text", short)],
                    )
                })
            })
            .collect();
        if lines.is_empty() {
            return String::new();
        }
        fill(
            tr(
                "Recent Moments (latest {length} posts):\n{posts}",
                "近期朋友圈内容（最近 {length} 条）：\n{posts}",
            ),
            &[
                ("length", lines.len().to_string()),
                ("posts", lines.join("\n")),
            ],
        )
    }

    async fn insight_social_section(&self, session_id: &str) -> String {
        let c = self.insight_cfg();
        let p = &self.profile_name;
        if !c.setting(Some(p), &keys::AI_INSIGHT_ALLOW_SOCIAL_CONTEXT) {
            return String::new();
        }
        let cookie = cfg_str(&c, p, "ai_insight_weibo_cookie");
        let uid = c
            .get_key(Some(p), "ai_insight_weibo_bindings")
            .get(session_id)
            .and_then(|b| b.get("uid"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if uid.is_empty() {
            return String::new();
        }
        let count = c.setting(Some(p), &keys::AI_INSIGHT_SOCIAL_CONTEXT_COUNT);
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
        fill(tr("Recent public social platform content (source: Weibo, latest {length} posts):\n{posts}", "近期公开社交平台内容（来源：微博，最近 {length} 条）：\n{posts}"), &[("length", lines.len().to_string()), ("posts", lines.join("\n"))])
    }

    // ── 生成 ──

    /// 为一个会话生成见解：生成了见解时返回保存的记录；见解关闭或模型没有可说的（`SKIP`）时为 `None`。
    ///
    /// # Errors
    ///
    /// 模型接口没有配置好、请求失败或保存记录失败时，返回说明原因的文字。
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
        // 取值 1 到 1000，`usize` 装得下
        let context_count =
            usize::try_from(c.setting(Some(p), &keys::AI_INSIGHT_CONTEXT_COUNT)).unwrap_or(1);
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
        let custom = cfg_str(&c, p, "ai_insight_system_prompt");
        let system = if custom.trim().is_empty() {
            ins::default_system_prompt().to_string()
        } else {
            custom
        };
        let silence = match (reason, silent_days) {
            (Trigger::Silence, Some(d)) if d != 0 => fill(
                tr(
                    "It has been {silent_days} days since you contacted \"{name}\".",
                    "已 {silent_days} 天未联系「{name}」。",
                ),
                &[("silent_days", d.to_string()), ("name", name.clone())],
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
        let record = self
            .record_store_add(session_id, &name, avatar, reason.as_str(), &insight, log)
            .map_err(AppError::into_message)?;
        if c.setting(Some(p), &keys::AI_INSIGHT_TELEGRAM_ENABLED) {
            let token = cfg_str(&c, p, "ai_insight_telegram_token");
            let chat_ids = cfg_str(&c, p, "ai_insight_telegram_chat_ids");
            if !token.is_empty() && !chat_ids.is_empty() {
                let title = fill(
                    tr("Insight · {name}", "见解 · {name}"),
                    &[("name", name.clone())],
                );
                let text = format!("【WeFlow】 {title}\n\n{insight}");
                for id in chat_ids.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if let Err(e) = ins::send_telegram(&token, id, &text).await {
                        log_failure(&crate::locale::localize(format!(
                            "Telegram push failed for chat {id}: {e}"
                        )));
                    }
                }
            } else {
                log_failure(tr(
                    "Telegram is enabled but the token or chat id is missing",
                    "已启用 Telegram，但缺少令牌或聊天 ID",
                ));
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
    ) -> AppResult<ins::InsightRecord> {
        let mut store = self.record_store();
        store.add(
            ins::NewRecord {
                session_id,
                display_name: name,
                avatar_url: avatar,
                trigger_reason: reason,
                insight,
            },
            log,
        )
    }

    /// `insight trigger <session>`：不论名单怎样设置，为这个会话生成一条测试见解。
    ///
    /// # Errors
    /// 没有写出见解时返回运行时错误：模型接口失败，或者模型没有可说的。
    pub async fn insight_trigger_session(&self, session_id: &str) -> AppResult<InsightReply> {
        let name = self
            .chat_contact_avatar(session_id)
            .map(|(_, n)| n)
            .unwrap_or_else(|| session_id.to_string());
        match self
            .insight_generate(session_id, &name, Trigger::Test, None)
            .await
        {
            Ok(Some(record)) => Ok(InsightReply {
                message: None,
                record: Some(record.summary()),
                ..InsightReply::done("")
            }),
            Ok(None) => Err(AppError::runtime(skipped_message(
                &name,
                self.insight_sends_chat(),
            ))),
            Err(why) => Err(AppError::runtime(why)),
        }
    }

    /// 强制为第一个符合条件的私聊生成一条见解。
    ///
    /// # Errors
    /// 设置中缺少模型接口的配置、没有可写的会话或没能写出见解时返回运行时错误。
    pub async fn insight_trigger_test(&self) -> AppResult<InsightReply> {
        if let Some(missing) = model_api_missing(&self.model_api()) {
            return Err(AppError::runtime(missing));
        }
        let Ok(sessions) = self.chat_sessions_list() else {
            return Err(AppError::runtime(
                tr("Database connection failed. Please finish the setup on the \"Database connection\" page first", "数据库连接失败，请先在\"数据库连接\"页完成配置"),
            ));
        };
        if sessions.is_empty() {
            return Err(AppError::runtime(tr(
                "No conversations found. Please make sure the database is connected correctly",
                "未找到任何会话，请确认数据库已正确连接",
            )));
        }
        let own = self.my_wxid_cleaned();
        let Some(session) = sessions.iter().find(|s| {
            let id = s.username.trim();
            picked_as_private(id, &own) && self.insight_allowed(id)
        }) else {
            return Err(AppError::runtime(
                tr("No private chat that can be triggered was found\nCheck the allowlist or blocklist mode and the selection list.", "未找到任何可触发的私聊会话\n请检查黑白名单模式与选择列表。"),
            ));
        };
        let id = session.username.trim().to_string();
        let name = Some(session.display_name.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&id)
            .to_string();
        let record = match self.insight_generate(&id, &name, Trigger::Test, None).await {
            Ok(Some(record)) => record,
            Ok(None) => {
                return Err(AppError::runtime(skipped_message(
                    &name,
                    self.insight_sends_chat(),
                )))
            }
            Err(why) => return Err(AppError::runtime(why)),
        };
        let message = fill(
            tr(
                "A test insight was generated for \"{name}\"",
                "已生成「{name}」的测试见解",
            ),
            &[("name", name)],
        );
        Ok(InsightReply {
            record: Some(record.summary()),
            ..InsightReply::done(message)
        })
    }

    /// 为静默太久的私聊写见解，每段静默一条，返回写了几条。
    ///
    /// 最后一条消息之后已经写过静默见解的会话，在新消息结束这段静默之前不再写。
    pub async fn insight_silence_scan(
        &self,
        on_insight: &mut dyn FnMut(&ins::InsightRecord),
    ) -> usize {
        if !self.insight_enabled() {
            return 0;
        }
        let c = self.insight_cfg();
        let days = c.setting(Some(&self.profile_name), &keys::AI_INSIGHT_SILENCE_DAYS);
        let threshold_ms = i64_f64(days) * 86_400_000.0;
        let now = i64_f64(ins::now_millis());
        let sessions = self.insight_sessions(true);
        let records = self.record_store();
        let own = self.my_wxid_cleaned();
        let mut generated = 0;
        for s in &sessions {
            if !self.insight_enabled() {
                return generated;
            }
            let id = s.username.trim().to_string();
            if !picked_as_private(&id, &own) || !self.insight_allowed(&id) {
                continue;
            }
            let last = i64_f64(s.last_timestamp) * 1000.0;
            if last <= 0.0 || now - last < threshold_ms {
                continue;
            }
            // 每段静默一条见解（#1140）：直到有消息结束它、新的静默开始之前不再写
            if records
                .last_written(&id, Trigger::Silence.as_str())
                .is_some_and(|at| i64_f64(at) >= last)
            {
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
                    // 缺少设置：其他会话也都会同样失败
                    if model_api_missing(&self.model_api()).is_some() {
                        return generated;
                    }
                }
            }
        }
        generated
    }

    /// 为最近活跃的会话写一条见解：每次调用最多处理一个会话，遵守冷却时间。
    ///
    /// 只有引擎第一次调用之后的消息才算活动，引擎启动时会话里已有的消息不算（#1140）。
    /// 冷却时间也从记录中最新的一条活动见解算起，所以重启后同样有效。
    pub async fn insight_analyze_activity(&self, on_insight: &mut dyn FnMut(&ins::InsightRecord)) {
        if !self.insight_enabled() {
            return;
        }
        let c = self.insight_cfg();
        let cooldown_ms =
            c.setting(Some(&self.profile_name), &keys::AI_INSIGHT_COOLDOWN_MINUTES) * 60_000;
        let now = ins::now_millis();
        let since = *self
            .insight_state
            .lock_or_panic()
            .activity_since
            .get_or_insert(now / 1000);
        let (mode, list) = self.insight_filter();
        let cooling = |sid: &str| -> bool {
            let last = self
                .insight_state
                .lock_or_panic()
                .last_activity
                .get(sid)
                .copied();
            cooldown_ms > 0 && last.is_some_and(|at| now - at < cooldown_ms)
        };
        // 只对有新消息的会话调用，因为要从文件读记录
        let cooled_by_records = |sid: &str| -> bool {
            if cooldown_ms <= 0 {
                return false;
            }
            match self
                .record_store()
                .last_written(sid, Trigger::Activity.as_str())
            {
                Some(at) if now - at < cooldown_ms => {
                    self.insight_state
                        .lock_or_panic()
                        .last_activity
                        .insert(sid.to_string(), at);
                    true
                }
                _ => false,
            }
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
                let seen = self
                    .insight_state
                    .lock_or_panic()
                    .last_seen
                    .get(sid)
                    .copied();
                // 冷却中的会话留着它的新消息，等冷却结束再处理
                if ts <= seen.unwrap_or(since) || cooled_by_records(sid) {
                    continue;
                }
                {
                    let mut st = self.insight_state.lock_or_panic();
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
        let own = self.my_wxid_cleaned();
        let candidates: Vec<&Session> = sessions
            .iter()
            .filter(|s| {
                let id = s.username.trim();
                picked_for_activity(id, &own) && self.insight_allowed(id)
            })
            .take(10)
            .collect();
        for s in candidates {
            let sid = s.username.trim().to_string();
            let ts = s.last_timestamp;
            {
                let mut st = self.insight_state.lock_or_panic();
                if ts <= st.last_seen.get(&sid).copied().unwrap_or(since) {
                    continue;
                }
                st.last_seen.insert(sid.clone(), ts);
            }
            if cooling(&sid) || cooled_by_records(&sid) {
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
    fn the_engine_picks_people_and_never_services() {
        let own = "wxid_me";
        for (id, private, activity) in [
            ("wxid_friend", true, true),
            ("123@chatroom", false, true),
            ("filehelper", false, false),
            ("gh_abc", false, false),
            ("weixin", false, false),
            ("abc@openim", false, false),
            ("wxid_me", false, false),
            ("@placeholder_foldgroup", false, false),
            ("", false, false),
        ] {
            assert_eq!(picked_as_private(id, own), private, "{id}");
            assert_eq!(picked_for_activity(id, own), activity, "{id}");
        }
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
        // 自建的服务可以不要密钥
        assert!(model_api_missing(&api("http://127.0.0.1:11434/v1", "", "m")).is_none());
    }

    /// 扫描间隔就是设置的小时数。手写的配置文件里有 `config set` 不接受的值（0、过多的小时数、
    /// 大到无法表示时间的数）时，每 4 小时扫描一次（#1149、#1265）。
    #[test]
    fn the_scan_interval_is_the_hours_set() {
        use crate::config::keys::Setting;
        let hours = |h: f64| Duration::from_secs_f64(h * 3600.0);
        for (set, interval) in [
            (json!(1), hours(1.0)),
            (json!(0.5), hours(0.5)),
            (json!("2"), hours(2.0)),
            (json!(168), hours(168.0)),
            (json!(0.01), hours(4.0)),
            (json!(0), hours(4.0)),
            (json!(-1), hours(4.0)),
            (json!(169), hours(4.0)),
            (Value::Null, hours(4.0)),
            (json!(1e300), hours(4.0)),
        ] {
            let read = keys::AI_INSIGHT_SCAN_INTERVAL_HOURS.read(&set);
            assert_eq!(scan_interval(read), interval, "{set}");
        }
    }

    #[test]
    fn a_skip_without_the_chat_says_how_to_send_it() {
        let sent = skipped_message("Bob", true);
        assert!(
            sent.contains("SKIP") && !sent.contains("ai_insight_allow_context"),
            "{sent}"
        );
        let unsent = skipped_message("Bob", false);
        assert!(
            unsent.starts_with(&sent)
                && unsent.contains("weflow config set ai_insight_allow_context true"),
            "{unsent}"
        );
    }
}
