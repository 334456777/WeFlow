//! The footprint and the AI insights.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// My footprint over a range of days (`insight footprint`): who I talked with and replied to, and where I was
/// @-mentioned. Its keys are the native layer's, in snake_case. Times are Unix seconds, 0 where there is none.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Footprint {
    pub summary: FootprintSummary,
    /// The private chats with a message in the range, the latest first.
    pub private_sessions: Vec<FootprintSession>,
    /// The stretches of each private chat, in time order: a new one begins after a long pause.
    pub private_segments: Vec<FootprintSegment>,
    /// The group messages that @-mention me, the latest first.
    pub mentions: Vec<FootprintMention>,
    /// The groups of those, the one with the most first.
    pub mention_groups: Vec<MentionGroup>,
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
    pub mention_count: u64,
    pub mention_group_count: u64,
}

/// A private chat of the footprint.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintSession {
    pub session_id: String,
    pub incoming_count: i64,
    pub outgoing_count: i64,
    pub replied: bool,
    pub first_incoming_ts: i64,
    pub first_reply_ts: i64,
    pub latest_ts: i64,
    /// The message to open the chat at, by its local id and time.
    pub anchor_local_id: i64,
    pub anchor_create_time: i64,
}

/// A stretch of a private chat.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintSegment {
    pub session_id: String,
    pub segment_index: i64,
    pub start_ts: i64,
    pub end_ts: i64,
    pub duration_sec: i64,
    pub incoming_count: i64,
    pub outgoing_count: i64,
    pub message_count: i64,
    pub replied: bool,
    pub first_incoming_ts: i64,
    pub first_reply_ts: i64,
    pub latest_ts: i64,
    pub anchor_local_id: i64,
    pub anchor_create_time: i64,
}

/// A group message that @-mentions me.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintMention {
    pub session_id: String,
    pub local_id: i64,
    pub create_time: i64,
    pub sender_username: String,
    pub message_content: String,
    /// The message's `source` XML, which lists who it mentions.
    pub source: String,
}

/// A group I was @-mentioned in.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MentionGroup {
    pub session_id: String,
    pub count: i64,
    pub latest_ts: i64,
}

/// How the footprint was read.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FootprintDiagnostics {
    /// Whether a list was cut at its limit.
    pub truncated: bool,
    pub private_truncated: bool,
    pub mention_truncated: bool,
    pub scanned_dbs: u64,
    pub elapsed_ms: u64,
    pub private_session_count: u64,
    pub group_session_count: u64,
    pub fallback_used: bool,
}

/// One insight record (`insight get`), with `success` inside. A record not found is a
/// failure of the command instead, so an answer that succeeds holds `success: true` and the record.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InsightLookup {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<crate::insight::InsightRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// An insight record as lists and events name it, without its log.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightSummary {
    pub id: String,
    /// Unix milliseconds.
    pub created_at: i64,
    pub session_id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// Why it was written: `activity`, `silence`, `test`, …
    pub trigger_reason: String,
    pub insight: String,
    pub read: bool,
}

/// A page of the account's insight records (`insight records`), the newest first, with counts over all of them.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightRecords {
    /// Always true.
    pub success: bool,
    pub records: Vec<InsightSummary>,
    /// The records the filters match.
    pub total: usize,
    /// The account's records written today.
    pub today_count: usize,
    pub unread_count: usize,
    /// Everyone with a record, the most records first.
    pub contacts: Vec<InsightContact>,
}

/// A conversation with insight records.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightContact {
    pub session_id: String,
    pub display_name: String,
    /// Its records.
    pub count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// How often the engine wrote about a conversation today (`insight today-stats`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightTodayStat {
    pub session_id: String,
    pub count: usize,
    /// When, `HH:MM` local time.
    pub times: Vec<String>,
}

/// What `insight mark-read` answers; a record not found is a failure of the command.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InsightMarked {
    pub success: bool,
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
}

impl InsightReply {
    /// A success that says `message`.
    pub fn done(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: Some(message.into()),
            record: None,
            insight: None,
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
    pub generated: usize,
    pub records: Vec<InsightSummary>,
}
