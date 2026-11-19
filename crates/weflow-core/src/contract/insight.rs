//! The footprint and the AI insights.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// My footprint over a range of days (`insight footprint`).
///
/// It says who I talked with and replied to, and where I was @-mentioned. Its keys are the native layer's, in
/// `snake_case`. Times are Unix seconds, 0 where there is none.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Footprint {
    /// The counts over the range.
    pub summary: FootprintSummary,
    /// The private chats with a message in the range, the latest first.
    pub private_sessions: Vec<FootprintSession>,
    /// The stretches of each private chat, in time order: a new one begins after a long pause.
    pub private_segments: Vec<FootprintSegment>,
    /// The group messages that @-mention me, the latest first.
    pub mentions: Vec<FootprintMention>,
    /// The groups of those, the one with the most first.
    pub mention_groups: Vec<MentionGroup>,
    /// How it was read.
    pub diagnostics: FootprintDiagnostics,
}

impl Footprint {
    /// The native layer's answer, read into the contract's types: a field they do not know fails here rather than
    /// leave the answer unseen.
    pub fn of(value: serde_json::Value) -> crate::error::AppResult<Self> {
        serde_json::from_value(value)
            .map_err(|err| crate::error::AppError::runtime(format!("the footprint: {err}")))
    }
}

/// The counts of the footprint.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintSummary {
    /// The people who wrote to me.
    pub private_inbound_people: u64,
    /// Those of them I answered.
    pub private_replied_people: u64,
    /// The people I wrote to.
    pub private_outbound_people: u64,
    /// Replied over inbound, from 0 to 1.
    pub private_reply_rate: f64,
    /// The mentions listed.
    pub mention_count: u64,
    /// The groups of those.
    pub mention_group_count: u64,
}

/// A private chat of the footprint.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintSession {
    /// The other party's wxid.
    pub session_id: String,
    /// The messages they sent in the range.
    pub incoming_count: i64,
    /// The messages I sent in the range.
    pub outgoing_count: i64,
    /// Whether both of us wrote.
    pub replied: bool,
    /// The time of their first message.
    pub first_incoming_ts: i64,
    /// The time of my first message after it.
    pub first_reply_ts: i64,
    /// The time of the last message.
    pub latest_ts: i64,
    /// The message to open the chat at, the last one, by its local id and time.
    pub anchor_local_id: i64,
    /// The time of that message.
    pub anchor_create_time: i64,
}

/// A stretch of a private chat.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintSegment {
    /// The other party's wxid.
    pub session_id: String,
    /// Its place among the chat's stretches, from 1.
    pub segment_index: i64,
    /// The time of its first message.
    pub start_ts: i64,
    /// The time of its last message.
    pub end_ts: i64,
    /// From the first message to the last, in seconds.
    pub duration_sec: i64,
    /// The messages they sent.
    pub incoming_count: i64,
    /// The messages I sent.
    pub outgoing_count: i64,
    /// All its messages.
    pub message_count: i64,
    /// Whether both of us wrote.
    pub replied: bool,
    /// The time of their first message.
    pub first_incoming_ts: i64,
    /// The time of my first message after it.
    pub first_reply_ts: i64,
    /// As `end_ts`.
    pub latest_ts: i64,
    /// The message to open the chat at, the first one, by its local id and time.
    pub anchor_local_id: i64,
    /// As `start_ts`.
    pub anchor_create_time: i64,
}

/// A group message that @-mentions me.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintMention {
    /// The group's id, `…@chatroom`.
    pub session_id: String,
    /// The message's id in its conversation's table.
    pub local_id: i64,
    /// When it was sent.
    pub create_time: i64,
    /// Who sent it; empty when the row does not say.
    pub sender_username: String,
    /// Its text as WeChat keeps it.
    pub message_content: String,
    /// The message's `source` XML, which lists who it mentions.
    pub source: String,
}

/// A group I was @-mentioned in.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MentionGroup {
    /// The group's id, `…@chatroom`.
    pub session_id: String,
    /// The mentions listed in it.
    pub count: i64,
    /// The time of the last of them.
    pub latest_ts: i64,
}

/// How the footprint was read.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintDiagnostics {
    /// Whether a list was cut at its limit.
    pub truncated: bool,
    /// Whether the private chats were cut at their limit.
    pub private_truncated: bool,
    /// Whether the mentions were cut at their limit.
    pub mention_truncated: bool,
    /// The message databases of the account.
    pub scanned_dbs: u64,
    /// How long the reading took.
    pub elapsed_ms: u64,
    /// The private chats looked through.
    pub private_session_count: u64,
    /// The groups looked through.
    pub group_session_count: u64,
    /// Always false.
    pub fallback_used: bool,
}

/// One insight record (`insight get`), with `success` inside. A record not found is a
/// failure of the command instead, so an answer that succeeds holds `success: true` and the record.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InsightLookup {
    /// Always true.
    pub success: bool,
    /// The record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<crate::insight::InsightRecord>,
    /// Always left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// An insight record as lists and events name it, without its log.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightSummary {
    /// The record's id, as `insight get` takes it.
    pub id: String,
    /// Unix milliseconds.
    pub created_at: i64,
    /// The conversation it is about.
    pub session_id: String,
    /// The conversation's name.
    pub display_name: String,
    /// The address of the conversation's avatar; left out when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// Why it was written: `activity`, `silence`, `test`, …
    pub trigger_reason: String,
    /// What the model wrote.
    pub insight: String,
    /// Whether it was marked read.
    pub read: bool,
}

/// A page of the account's insight records (`insight records`), the newest first, with counts over all of them.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightRecords {
    /// Always true.
    pub success: bool,
    /// The records of the page.
    pub records: Vec<InsightSummary>,
    /// The records the filters match.
    pub total: usize,
    /// The account's records written today.
    pub today_count: usize,
    /// The account's records not read yet.
    pub unread_count: usize,
    /// Everyone with a record, the most records first.
    pub contacts: Vec<InsightContact>,
}

/// A conversation with insight records.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightContact {
    /// The conversation's username.
    pub session_id: String,
    /// The conversation's name.
    pub display_name: String,
    /// Its records.
    pub count: usize,
    /// The address of its avatar; left out when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// How often the engine wrote about a conversation today (`insight today-stats`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightTodayStat {
    /// The conversation's username.
    pub session_id: String,
    /// The records written about it today.
    pub count: usize,
    /// When, `HH:MM` local time.
    pub times: Vec<String>,
}

/// What `insight mark-read` answers; a record not found is a failure of the command.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InsightMarked {
    /// Always true.
    pub success: bool,
    /// Always left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What `insight clear` answers.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InsightCleared {
    /// Always true.
    pub success: bool,
    /// The records removed.
    pub removed: usize,
}

/// What `insight test`, `trigger` and `footprint-summary` answer; when the AI is not set up or answers nothing, the
/// command fails with `message` instead.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InsightReply {
    /// Always true.
    pub success: bool,
    /// What happened, in the server's language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The record written (`trigger`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<InsightSummary>,
    /// The summary written (`footprint-summary`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insight: Option<String>,
    /// Where the question went and what came back (`test`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<ModelProbe>,
}

/// What `insight test` asked of the model API and what it answered.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelProbe {
    /// The API format asked in.
    pub format: crate::model_api::ModelApiFormat,
    /// The address asked.
    pub endpoint: String,
    /// The model that answered, as the service names it, else the one asked for.
    pub model: String,
    /// The answer, trimmed.
    pub reply: String,
    /// From the request to the answer, retries included.
    pub duration_ms: u64,
    /// The requests tried again after a transient failure.
    pub retries: u32,
}

/// The models the model API offers (`insight models`), as the service lists them.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelList {
    /// The API format asked in.
    pub format: crate::model_api::ModelApiFormat,
    /// The address asked.
    pub endpoint: String,
    /// The models, in the service's order.
    pub models: Vec<crate::model_api::ModelInfo>,
}

impl InsightReply {
    /// A success that says `message`.
    pub fn done(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: Some(message.into()),
            record: None,
            insight: None,
            probe: None,
        }
    }

    /// A failure that says `message`; the command reports it as its error.
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            success: false,
            ..Self::done(message)
        }
    }
}

/// What `insight scan` answers: the insights the silence scan wrote.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InsightScan {
    /// How many it wrote.
    pub generated: usize,
    /// The records written.
    pub records: Vec<InsightSummary>,
}
