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
    /// How many members it has.
    pub member_count: i64,
    /// The address of its avatar; left out when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// The members of a group (`group members`): the owner first,
/// then friends, then by how many messages they sent.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupMembers {
    /// The group's id, `…@chatroom`.
    pub chatroom_id: String,
    /// How many members there are.
    pub count: usize,
    /// When the panel was built, in Unix milliseconds.
    pub updated_at: i64,
    /// The members.
    pub members: Vec<GroupMember>,
}

/// A member of a group's members panel.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupMember {
    /// The member's wxid.
    pub username: String,
    /// Their name as WeChat shows it in the group.
    pub display_name: String,
    /// The nickname of their contact; empty when they are not a contact.
    pub nickname: String,
    /// The WeChat ID of their contact; empty when they are not a contact or chose none.
    pub alias: String,
    /// My remark for them; empty when they are not a contact or have none.
    pub remark: String,
    /// The name they gave themselves in this group; empty when none.
    pub group_nickname: String,
    /// The address of their avatar; left out when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// Whether they own the group.
    pub is_owner: bool,
    /// Whether they are my friend.
    pub is_friend: bool,
    /// How many messages they sent in the group; 0 unless `counts=true`.
    pub message_count: i64,
}

/// Who sent how many messages in a group (`group ranking`), the most first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupRank {
    /// Who.
    pub member: RankedMember,
    /// How many messages they sent in the range asked for.
    pub message_count: i64,
}

/// A member of a ranking.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RankedMember {
    /// The member's wxid.
    pub username: String,
    /// Their name as WeChat shows it in the group.
    pub display_name: String,
    /// The address of their avatar; left out when there is none.
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
    /// The kinds of message, the most first.
    pub type_counts: Vec<MediaCount>,
    /// All the messages counted.
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
    /// How many messages of it there are.
    pub count: i64,
}

/// What one member of a group sent (`group member`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemberAnalytics {
    /// The numbers of their messages.
    pub statistics: MemberStatistics,
    /// By the hour, `"0"` to `"23"`, in the server's time zone.
    pub time_distribution: IndexMap<String, i64>,
    /// What they said most, the most first, up to 10.
    pub common_phrases: Vec<PhraseCount>,
    /// The WeChat emojis (`[微笑]` and the like) in their text, the most used first, up to 10.
    pub common_emojis: Vec<EmojiCount>,
}

/// The numbers of a member's messages.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemberStatistics {
    /// All their messages.
    pub total_messages: i64,
    /// Text, quotes included.
    pub text_messages: i64,
    /// Images.
    pub image_messages: i64,
    /// Voice messages.
    pub voice_messages: i64,
    /// Videos.
    pub video_messages: i64,
    /// Stickers.
    pub emoji_messages: i64,
    /// Every other kind.
    pub other_messages: i64,
    /// The time of their first message; null when they sent none.
    pub first_message_time: Option<i64>,
    /// The time of their last message; null when they sent none.
    pub last_message_time: Option<i64>,
    /// The days, in the server's time zone, with a message of theirs.
    pub active_days: usize,
    /// By WeChat's type of the message, as `localType`.
    pub message_type_counts: IndexMap<String, i64>,
}

/// A phrase and how often it was said.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhraseCount {
    /// A text message of at most 20 characters, counted whole.
    pub phrase: String,
    /// How many times it was sent.
    pub count: i64,
}

/// An emoji and how often it was sent.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct EmojiCount {
    /// The emoji as the text writes it, `[微笑]`.
    pub emoji: String,
    /// How many times it was used.
    pub count: i64,
}

/// A page of a member's messages in a group (`group member-messages`), newest first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemberMessages {
    /// The messages of the page.
    pub messages: Vec<ChatMessage>,
    /// Whether there are messages past this page.
    pub has_more: bool,
    /// Pass it as `cursor` for the next page.
    pub next_cursor: usize,
}

/// The private chats counted together (`analytics overall`), but for the people left out.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OverallStatistics {
    /// All the messages.
    pub total_messages: i64,
    /// Text, quotes included.
    pub text_messages: i64,
    /// Images.
    pub image_messages: i64,
    /// Voice messages.
    pub voice_messages: i64,
    /// Videos.
    pub video_messages: i64,
    /// Stickers.
    pub emoji_messages: i64,
    /// Every other kind.
    pub other_messages: i64,
    /// The ones I sent.
    pub sent_messages: i64,
    /// The ones I received.
    pub received_messages: i64,
    /// The time of the first message; null without messages.
    pub first_message_time: Option<i64>,
    /// The time of the last message; null without messages.
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
    /// The other party's wxid.
    pub username: String,
    /// The remark, else the nickname, else the WeChat ID, else the username.
    pub display_name: String,
    /// The address of their avatar; left out when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// The WeChat ID they chose, else the username when it is not a `wxid_…`; else empty.
    pub wechat_id: String,
    /// The messages of the chat in the range asked for.
    pub message_count: i64,
    /// The ones I sent.
    pub sent_count: i64,
    /// The ones I received.
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
    /// The other party's wxid.
    pub username: String,
    /// The remark, else the nickname, else the WeChat ID, else the username.
    pub display_name: String,
    /// The address of their avatar; left out when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// As in a ranking.
    pub wechat_id: String,
}
