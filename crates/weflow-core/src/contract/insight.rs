//! 足迹和 AI 见解。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 账号所有者在一段日期内的足迹（`insight footprint`）。
///
/// 说明和谁聊过、回复过谁、在哪里被 @。键沿用只读数据库层的写法，为 `snake_case`。时间为 Unix 秒，没有时为 0。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Footprint {
    /// 整个范围内的计数。
    pub summary: FootprintSummary,
    /// 范围内有消息的私聊，最近的在前。
    pub private_sessions: Vec<FootprintSession>,
    /// 各私聊的分段，按开始的时间排列：静默超过 1 小时后开始新的一段。
    pub private_segments: Vec<FootprintSegment>,
    /// 群里 @ 了账号所有者的消息，最新的在前；`@所有人` 也算。
    pub mentions: Vec<FootprintMention>,
    /// 这些消息所在的群，@ 得多的在前。
    pub mention_groups: Vec<MentionGroup>,
    /// 读取的情况。
    pub diagnostics: FootprintDiagnostics,
}

impl Footprint {
    /// 把只读数据库层的回答读进契约类型：有契约类型不认识的字段时在这里失败，不会把没有描述的内容传出去。
    ///
    /// # Errors
    ///
    /// 回答不符合契约类型（多出或缺少字段、类型不对）时返回运行时错误。
    pub fn of(value: serde_json::Value) -> crate::error::AppResult<Self> {
        serde_json::from_value(value)
            .map_err(|err| crate::error::AppError::runtime(format!("the footprint: {err}")))
    }
}

/// 足迹的计数。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintSummary {
    /// 在私聊中给账号所有者发过消息的人数。
    pub private_inbound_people: u64,
    /// 其中账号所有者也发过消息的人数。
    pub private_replied_people: u64,
    /// 账号所有者在私聊中发过消息的人数。
    pub private_outbound_people: u64,
    /// `private_replied_people` 除以 `private_inbound_people`，0 到 1；没有人发来消息时为 0。
    pub private_reply_rate: f64,
    /// 列出的 @ 的条数。
    pub mention_count: u64,
    /// 这些 @ 所在的群数。
    pub mention_group_count: u64,
}

/// 足迹中的一个私聊。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintSession {
    /// 对方的 wxid。
    pub session_id: String,
    /// 范围内对方发的消息数。
    pub incoming_count: i64,
    /// 范围内账号所有者发的消息数。
    pub outgoing_count: i64,
    /// 双方是否都发过消息。
    pub replied: bool,
    /// 对方第一条消息的时间。
    pub first_incoming_ts: i64,
    /// 在那之后账号所有者第一条消息的时间。
    pub first_reply_ts: i64,
    /// 最后一条消息的时间。
    pub latest_ts: i64,
    /// 打开聊天时定位到的消息（最后一条）的本地 ID。
    pub anchor_local_id: i64,
    /// 那条消息的时间，与 `latest_ts` 相同。
    pub anchor_create_time: i64,
}

/// 私聊中的一段。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintSegment {
    /// 对方的 wxid。
    pub session_id: String,
    /// 这一段在这个私聊的各段中的序号，从 1 开始。
    pub segment_index: i64,
    /// 第一条消息的时间。
    pub start_ts: i64,
    /// 最后一条消息的时间。
    pub end_ts: i64,
    /// 从第一条消息到最后一条的秒数。
    pub duration_sec: i64,
    /// 对方发的消息数。
    pub incoming_count: i64,
    /// 账号所有者发的消息数。
    pub outgoing_count: i64,
    /// 这一段的全部消息数。
    pub message_count: i64,
    /// 双方是否都发过消息。
    pub replied: bool,
    /// 对方第一条消息的时间。
    pub first_incoming_ts: i64,
    /// 在那之后账号所有者第一条消息的时间。
    pub first_reply_ts: i64,
    /// 与 `end_ts` 相同。
    pub latest_ts: i64,
    /// 打开聊天时定位到的消息（这一段的第一条）的本地 ID。
    pub anchor_local_id: i64,
    /// 与 `start_ts` 相同。
    pub anchor_create_time: i64,
}

/// 群里 @ 了账号所有者的一条消息。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintMention {
    /// 群的 ID，`…@chatroom`。
    pub session_id: String,
    /// 消息的本地 ID。
    pub local_id: i64,
    /// 发送时间。
    pub create_time: i64,
    /// 发送者；消息行中没有时为空。
    pub sender_username: String,
    /// 微信保存的消息文字。
    pub message_content: String,
    /// 消息的 `source` XML，其中列出了 @ 的人。
    pub source: String,
}

/// 账号所有者被 @ 过的一个群。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MentionGroup {
    /// 群的 ID，`…@chatroom`。
    pub session_id: String,
    /// 其中列出的 @ 的条数。
    pub count: i64,
    /// 其中最后一条的时间。
    pub latest_ts: i64,
}

/// 读取足迹的情况。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintDiagnostics {
    /// 是否有列表因为达到上限而截断，即下面两项之一为 `true`。
    pub truncated: bool,
    /// 私聊是否因为达到上限而截断；`insight footprint` 不设上限。
    pub private_truncated: bool,
    /// @ 是否因为达到上限而截断；`insight footprint` 不设上限。
    pub mention_truncated: bool,
    /// 账号的消息库个数。
    pub scanned_dbs: u64,
    /// 读取所用的毫秒数。
    pub elapsed_ms: u64,
    /// 查看过的私聊数。
    pub private_session_count: u64,
    /// 查看过的群聊数。
    pub group_session_count: u64,
}

/// 一条见解记录（`insight get`）；找不到记录时命令失败。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InsightLookup {
    /// 这条记录。
    pub record: crate::insight::InsightRecord,
}

/// 列表和事件中的一条见解记录，不含它的日志。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightSummary {
    /// 记录的 ID，即 `insight get` 接受的写法。
    pub id: String,
    /// 写下记录的时间，Unix 毫秒。
    pub created_at: i64,
    /// 记录所说的会话。
    pub session_id: String,
    /// 写下记录时会话的名字。
    pub display_name: String,
    /// 会话头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// 写下它的原因：`activity`、`silence`、`test` 等。
    pub trigger_reason: String,
    /// 模型写的见解。
    pub insight: String,
    /// 是否已标为已读。
    pub read: bool,
}

/// 账号见解记录中的一页（`insight records`），最新的在前，并带上全部记录的计数。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightRecords {
    /// 这一页的记录。
    pub records: Vec<InsightSummary>,
    /// 符合筛选条件的记录数。
    pub total: usize,
    /// 账号今天写下的记录数，不论筛选条件。
    pub today_count: usize,
    /// 账号还没读的记录数，不论筛选条件。
    pub unread_count: usize,
    /// 所有有记录的会话，记录多的在前，不论筛选条件。
    pub contacts: Vec<InsightContact>,
}

/// 一个有见解记录的会话。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightContact {
    /// 会话的用户名。
    pub session_id: String,
    /// 会话的名字，取自它的一条记录。
    pub display_name: String,
    /// 它的记录数。
    pub count: usize,
    /// 头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// 见解引擎今天为一个会话写了几条见解（`insight today-stats`）。
///
/// 计数只保存在运行引擎的进程的内存中：`serve --insight` 由服务进程回答，其他情况下为空。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightTodayStat {
    /// 会话的用户名。
    pub session_id: String,
    /// 今天为它写下的记录数。
    pub count: usize,
    /// 写下的时间，服务端所在时区的 `HH:MM`。
    pub times: Vec<String>,
}

/// `insight mark-read` 的回答；找不到记录时命令失败。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InsightMarked {
    /// 标为已读的记录。
    pub id: String,
}

/// `insight clear` 的回答。
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
pub struct InsightCleared {
    /// 删除的记录数。
    pub removed: usize,
}

/// `insight clear` 将要删除多少（`--dry-run`），供页面在删除前询问。
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightClearPlan {
    /// 总是 `true`。
    pub dry_run: bool,
    /// 将要删除的记录数。
    pub records: usize,
}

/// `insight test`、`trigger` 和 `footprint-summary` 的回答；没有设置 AI 或 AI 没有回答时，命令改为失败，
/// 报错中写明发生了什么。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InsightReply {
    /// 发生了什么，使用服务端的语言。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// 写下的记录（`trigger`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<InsightSummary>,
    /// 写出的总结（`footprint-summary`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insight: Option<String>,
    /// 问题发到了哪里、得到了什么回答（`test`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<ModelProbe>,
}

/// `insight test` 向模型接口问了什么、它回答了什么。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelProbe {
    /// 请求所用的接口格式。
    pub format: crate::model_api::ModelApiFormat,
    /// 请求的地址。
    pub endpoint: String,
    /// 回答的模型，按服务给出的名字；服务没有给出时为请求的模型。
    pub model: String,
    /// 回答，去掉了首尾空白。
    pub reply: String,
    /// 从请求到回答的毫秒数，包括重试。
    pub duration_ms: u64,
    /// 暂时失败后重试的次数。
    pub retries: u32,
}

/// 模型接口提供的模型（`insight models`），按服务列出的样子。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelList {
    /// 请求所用的接口格式。
    pub format: crate::model_api::ModelApiFormat,
    /// 请求的地址，即模型列表的地址。
    pub endpoint: String,
    /// 这些模型，按服务给出的顺序。
    pub models: Vec<crate::model_api::ModelInfo>,
}

impl InsightReply {
    /// 只带 `message` 的回答。
    pub fn done(message: impl Into<String>) -> Self {
        Self {
            message: Some(message.into()),
            record: None,
            insight: None,
            probe: None,
        }
    }
}

/// `insight scan` 的回答：静默扫描写下的见解。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InsightScan {
    /// 写下的条数。
    pub generated: usize,
    /// 写下的记录。
    pub records: Vec<InsightSummary>,
}
