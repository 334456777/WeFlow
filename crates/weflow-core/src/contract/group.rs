//! Group chats and the analytics of the private chats: members, rankings, hours, kinds of messages.

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::chat_msg::ChatMessage;

/// A group chat of `group list`, the largest first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupChat {
    /// The group's id, `…@chatroom`.
    pub username: String,
    /// Its name, else its id.
    pub display_name: String,
    pub member_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// The members of a group (`group members`): the owner first,
/// then friends, then by how many messages they sent.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupMembers {
    pub chatroom_id: String,
    pub count: usize,
    /// When the panel was built, in Unix milliseconds.
    pub updated_at: i64,
    pub members: Vec<GroupMember>,
}

/// A member of a group's members panel.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupMember {
    pub username: String,
    /// Their name as WeChat shows it in the group.
    pub display_name: String,
    /// The fields of their contact; empty when they are not a contact.
    pub nickname: String,
    pub alias: String,
    pub remark: String,
    /// The name they gave themselves in this group; empty when none.
    pub group_nickname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub is_owner: bool,
    pub is_friend: bool,
    /// How many messages they sent in the group; 0 unless `counts=true`.
    pub message_count: i64,
}

/// Who sent how many messages in a group (`group ranking`), the most first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupRank {
    pub member: RankedMember,
    pub message_count: i64,
}

/// A member of a ranking.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RankedMember {
    pub username: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// How many messages a group has in each hour of the day (`group hours`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupHours {
    /// By the hour, `"0"` to `"23"`, in the server's time zone.
    pub hourly_distribution: IndexMap<String, i64>,
}

/// How many messages of each kind a group has (`group media`), the most first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupMedia {
    pub type_counts: Vec<MediaCount>,
    pub total: i64,
}

/// One kind of message and how many there are.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MediaCount {
    /// WeChat's type of the message, as `localType`; -1 for all the others together.
    #[serde(rename = "type")]
    pub kind: i64,
    /// Its name in Chinese.
    pub name: String,
    pub count: i64,
}

/// What one member of a group sent (`group member`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemberAnalytics {
    pub statistics: MemberStatistics,
    /// By the hour, `"0"` to `"23"`, in the server's time zone.
    pub time_distribution: IndexMap<String, i64>,
    /// What they said most, the most first, up to 10.
    pub common_phrases: Vec<PhraseCount>,
    pub common_emojis: Vec<EmojiCount>,
}

/// The numbers of a member's messages.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemberStatistics {
    pub total_messages: i64,
    pub text_messages: i64,
    pub image_messages: i64,
    pub voice_messages: i64,
    pub video_messages: i64,
    pub emoji_messages: i64,
    pub other_messages: i64,
    pub sent_messages: i64,
    /// Always 0: a member's messages are all sent by them.
    pub received_messages: i64,
    /// The time of their first message; null when they sent none.
    pub first_message_time: Option<i64>,
    pub last_message_time: Option<i64>,
    pub active_days: usize,
    /// By WeChat's type of the message, as `localType`.
    pub message_type_counts: IndexMap<String, i64>,
}

/// A phrase and how often it was said.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhraseCount {
    pub phrase: String,
    pub count: i64,
}

/// An emoji and how often it was sent.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct EmojiCount {
    pub emoji: String,
    pub count: i64,
}

/// A page of a member's messages in a group (`group member-messages`), newest first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemberMessages {
    pub messages: Vec<ChatMessage>,
    pub has_more: bool,
    /// Pass it as `cursor` for the next page.
    pub next_cursor: usize,
}

/// The private chats counted together (`analytics overall`), but for the people left out.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OverallStatistics {
    pub total_messages: i64,
    pub text_messages: i64,
    pub image_messages: i64,
    pub voice_messages: i64,
    pub video_messages: i64,
    pub emoji_messages: i64,
    pub other_messages: i64,
    pub sent_messages: i64,
    pub received_messages: i64,
    /// The time of the first message; null without messages.
    pub first_message_time: Option<i64>,
    pub last_message_time: Option<i64>,
    /// The days with a message.
    pub active_days: i64,
    /// By WeChat's type of the message, as `localType`.
    pub message_type_counts: IndexMap<String, i64>,
}

/// A private chat of `analytics rankings`, the one with the most messages first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContactRank {
    pub username: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// The WeChat ID they chose, else the username when it is not a `wxid_…`; else empty.
    pub wechat_id: String,
    pub message_count: i64,
    pub sent_count: i64,
    pub received_count: i64,
    /// The time of the last message; null without messages.
    pub last_message_time: Option<i64>,
}

/// When the private chats' messages were sent (`analytics time`), in the server's time zone.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TimeDistribution {
    /// By the hour, `"0"` to `"23"`.
    pub hourly_distribution: IndexMap<String, i64>,
    /// By the day of the week, `"1"` (Monday) to `"7"` (Sunday).
    pub weekday_distribution: IndexMap<String, i64>,
    /// By the month, `YYYY-MM`.
    pub monthly_distribution: IndexMap<String, i64>,
}

/// Someone who may be left out of the analytics (`analytics exclude-candidates`): the private chats, and those left
/// out already.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExcludeCandidate {
    pub username: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// As in a ranking.
    pub wechat_id: String,
}
