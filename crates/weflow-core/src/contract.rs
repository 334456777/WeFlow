//! The command API's contract (#399): what the `data` of its key reads holds, as Rust types that serialize to the
//! answer and describe it as JSON Schema. `/openapi.json` carries the schemas, and the WebUI's types are generated
//! from them, so a change here that its clients would not read the same shows in their types.
//!
//! Throughout: times are Unix seconds unless a field says otherwise; an id of WeChat's that may pass 2^53 (a
//! message's server id) is a decimal string; an optional field is left out when there is nothing to say, a nullable
//! one is `null`. The rows of WeChat's own tables (`chat messages` without `parsed`, `chat contacts` without
//! `parsed`…) have the columns of those tables and are not described further.

use std::collections::BTreeMap;
use std::path::PathBuf;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use crate::chat_msg::{ChatMessage, Type49Info};
use crate::message::{extract_arkme_app_message_meta, parse_message_content, ExportMsg};

/// A message of a page as the chat window shows it (`chat messages`, `chat message` and `chat search` with
/// `parsed`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ParsedMessage {
    #[serde(flatten)]
    pub message: ChatMessage,
    /// Where the message stands in its conversation: pass it as `before` or `after` to read the messages next to it.
    /// It is opaque; only the server reads it.
    #[serde(rename = "_cursor", skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// The conversation of a search hit.
    #[serde(rename = "_session_id", skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// A conversation of `chat sessions`, newest first.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// The other party's wxid, `…@chatroom` for a group, `gh_…` for an official account.
    pub username: String,
    /// WeChat's type of the conversation's row; 0 for an official account WeChat lists without one.
    #[serde(rename = "type")]
    pub kind: i64,
    pub unread_count: i64,
    /// The last message as the list shows it.
    pub summary: String,
    /// What the list is ordered by; 0 for an official account WeChat lists without a conversation.
    pub sort_timestamp: i64,
    pub last_timestamp: i64,
    /// WeChat's type of the last message, as a message's `localType`.
    pub last_msg_type: i64,
    /// About how many messages the conversation has, when WeChat's row or its tables say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_count_hint: Option<i64>,
    /// The remark, else the nickname, else the username.
    pub display_name: String,
    /// In a group: who sent the last message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_msg_sender: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sender_display_name: Option<String>,
    /// The account's own wxid.
    pub self_wxid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// What a contact of `chat contacts --parsed` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContactKind {
    Friend,
    Group,
    Official,
    /// Someone who was a friend.
    FormerFriend,
}

impl ContactKind {
    /// The name it is serialized as (`export contacts --kind` takes the same).
    pub fn name(self) -> &'static str {
        match self {
            Self::Friend => "friend",
            Self::Group => "group",
            Self::Official => "official",
            Self::FormerFriend => "former_friend",
        }
    }
}

/// A contact of `chat contacts --parsed`, as the desktop app lists it: the most recent conversation first, then by
/// name. A text field is left out when it is empty.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub username: String,
    /// The remark, else the nickname, else the WeChat ID, else the username.
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    /// The WeChat ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// The names of the contact's labels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    /// The signature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(rename = "type")]
    pub kind: ContactKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// A message as the query commands list it (`chat resources`, `chat images`, `chat media-stream`…): the main fields
/// of its row, its text and the keys of its media. An optional field is left out when the message has none.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    pub local_id: i64,
    /// WeChat's id of the message, in decimal: a string, as it may pass 2^53.
    pub server_id: String,
    pub create_time: i64,
    /// WeChat's type of the message, as a parsed message's `localType`.
    pub local_type: i64,
    /// 1 when I sent it, else 0.
    pub is_send: i64,
    pub sender_username: String,
    /// The message as WeChat keeps it (often XML).
    pub content: String,
    /// Its text as the chat window shows it; null when it has none.
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_md5: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_dat_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_md5: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_md5: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_cdn_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xml_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_ext: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_md5: Option<String>,
    /// In bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_lng: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_poiname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_label: Option<String>,
    /// The kind of an app message (`link`, `file`, `music`…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_kind: Option<String>,
}

impl MessageView {
    pub fn of(m: &ExportMsg, my_wxid: &str) -> Self {
        let text = parse_message_content(
            &m.content,
            m.local_type,
            Some(my_wxid),
            Some(&m.sender_username),
            m.emoji_caption.as_deref(),
        );
        let app_msg_kind = extract_arkme_app_message_meta(&m.content, m.local_type)
            .and_then(|meta| meta.get("appMsgKind")?.as_str().map(String::from));
        Self {
            local_id: m.local_id,
            server_id: m
                .server_id_raw
                .clone()
                .unwrap_or_else(|| m.server_id.to_string()),
            create_time: m.create_time,
            local_type: m.local_type,
            is_send: i64::from(m.is_send),
            sender_username: m.sender_username.clone(),
            content: m.content.clone(),
            text,
            image_md5: m.image_md5.clone(),
            image_dat_name: m.image_dat_name.clone(),
            video_md5: m.video_md5.clone(),
            emoji_md5: m.emoji_md5.clone(),
            emoji_cdn_url: m.emoji_cdn_url.clone(),
            xml_type: m.xml_type.clone(),
            file_name: m.file_name.clone(),
            file_ext: m.file_ext.clone(),
            file_md5: m.file_md5.clone(),
            file_size: m.file_size,
            location_lat: m.location_lat,
            location_lng: m.location_lng,
            location_poiname: m.location_poiname.clone(),
            location_label: m.location_label.clone(),
            app_msg_kind,
        }
    }
}

/// `chat message` without `parsed`: the row of WeChat's table, and the message as the query commands list it.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MessageLookup {
    /// The row, with its columns.
    pub row: serde_json::Value,
    /// Null when the row could not be read as a message.
    pub message: Option<MessageView>,
}

/// What a message of `chat resources` holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Image,
    Video,
    Voice,
    File,
}

/// A message of `chat resources`, newest first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    #[serde(flatten)]
    pub message: MessageView,
    /// Where the message stands in the listing (not in its conversation): `before` reads on from it.
    #[serde(rename = "_cursor")]
    pub cursor: String,
    pub session_display_name: String,
    pub session_id: String,
    pub resource_type: ResourceKind,
}

/// A page of `chat resources`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePage {
    pub items: Vec<Resource>,
    pub has_more: bool,
    /// The `_cursor` of the last item, to pass as `before` for the next page; null on the last page.
    pub next_cursor: Option<String>,
}

/// Whether a conversation is folded and muted, of `chat statuses`, which answers one for each username asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatStatus {
    pub is_folded: bool,
    pub is_muted: bool,
}

/// How many contacts of each kind there are (`chat tab-counts`), as the tabs of the desktop app count them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TabCounts {
    #[serde(default)]
    pub private: i64,
    #[serde(default)]
    pub group: i64,
    #[serde(default)]
    pub official: i64,
    #[serde(default)]
    pub former_friend: i64,
}

/// The days a conversation has messages on (`chat dates`), as `YYYY-MM-DD` in the server's time zone, oldest first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionDates {
    pub session_id: String,
    pub dates: Vec<String>,
}

/// How many messages a conversation has on each day it has any (`chat date-counts`), by `YYYY-MM-DD`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionDateCounts {
    pub session_id: String,
    pub counts: BTreeMap<String, i64>,
}

/// Who a conversation is with and how many messages it has (`chat detail --fast`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionFacts {
    /// The conversation's username.
    pub wxid: String,
    /// The remark, else the nickname, else the alias, else the username.
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nick_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub message_count: i64,
}

/// A conversation's details (`chat detail`): its facts, and where its messages are kept.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SessionDetail {
    #[serde(flatten)]
    pub facts: SessionFacts,
    #[serde(flatten)]
    pub tables: SessionTables,
}

/// Where a conversation's messages are kept (`chat detail --extra`, and `chat detail` without `fast`).
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionTables {
    /// The time of its first message; null when its tables do not say.
    pub first_message_time: Option<i64>,
    pub latest_message_time: Option<i64>,
    pub message_tables: Vec<MessageTable>,
}

/// One of WeChat's tables that holds messages of a conversation.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageTable {
    /// The database's file name, such as `message_0.db`.
    pub db_name: String,
    pub table_name: String,
    pub count: i64,
}

/// Which files of a message's image are on this computer (`chat image-info`), and the sizes the message gives.
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    /// The thumbnail a chat shows.
    pub thumb: bool,
    /// The image shown when it is opened.
    pub normal: bool,
    /// The original, once View original downloaded it.
    pub hd: bool,
    /// Bytes of the image; null when neither the message nor its file says.
    pub normal_bytes: Option<u64>,
    /// Bytes of the original, as the message gives them even before it is downloaded.
    pub hd_bytes: Option<u64>,
}

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

/// The members of a group (`group members`), as the members panel of the desktop app lists them: the owner first,
/// then friends, then by how many messages they sent.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupMembers {
    pub chatroom_id: String,
    pub count: usize,
    /// Whether the panel was kept from a request a short while before (`refresh=true` builds it again).
    pub from_cache: bool,
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
    /// Its name as the desktop app shows it, in Chinese.
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

/// Who has posts in Moments (`sns users`), the one who posted last first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SnsUsers {
    pub usernames: Vec<String>,
}

/// How many Moments posts there are (`sns stats`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnsStats {
    pub total_posts: i64,
    /// How many people posted them.
    pub total_friends: i64,
    /// How many of them are the account's own; null when the account's wxid is not known.
    pub my_posts: Option<i64>,
}

/// How many Moments posts one person has (`sns post-counts --user`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserPosts {
    pub username: String,
    pub total_posts: i64,
}

/// The years with private messages (`report annual years`), the latest first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ReportYears {
    pub years: Vec<i64>,
    /// How they were found: `native` (WeChat's own index), `table-scan` (the time span of each table) or
    /// `edge-scan` (each chat's first and last message).
    pub strategy: String,
}

/// The annual report (`report annual generate`) over the private chats of one year, or of all of them.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnnualReport {
    /// 0 for all the years.
    pub year: i32,
    pub total_messages: i64,
    /// The private chats with messages.
    pub total_friends: usize,
    /// The three private chats with the most messages.
    pub core_friends: Vec<CoreFriend>,
    /// Months 1–12.
    pub monthly_top_friends: Vec<MonthlyTopFriend>,
    pub peak_day: Option<PeakDay>,
    pub longest_streak: Option<LongestStreak>,
    pub activity_heatmap: ActivityHeatmap,
    pub midnight_king: Option<MidnightKing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_avatar_url: Option<String>,
    pub mutual_friend: Option<MutualFriend>,
    pub social_initiative: Option<SocialInitiative>,
    pub response_speed: Option<ResponseSpeed>,
    /// My phrases said at least twice, the most said first.
    pub top_phrases: Vec<PhraseCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sns_stats: Option<AnnualSnsStats>,
    pub lost_friend: Option<LostFriend>,
}

/// One of the three private chats with the most messages.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CoreFriend {
    pub username: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub message_count: i64,
    pub sent_count: i64,
    pub received_count: i64,
}

/// Who a month's most messages are with.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyTopFriend {
    /// 1–12.
    pub month: i64,
    /// 「暂无」 for a month without messages.
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub message_count: i64,
}

/// The day with the most messages, and who most of them are with.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PeakDay {
    /// `YYYY-MM-DD`.
    pub date: String,
    pub message_count: i64,
    pub top_friend: String,
    pub top_friend_count: i64,
}

/// The most days in a row talked with one friend.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LongestStreak {
    pub friend_name: String,
    pub days: i64,
    /// `YYYY-MM-DD`.
    pub start_date: String,
    /// `YYYY-MM-DD`.
    pub end_date: String,
}

/// Messages by weekday and hour.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ActivityHeatmap {
    /// Seven rows, Monday first, of 24 hours each.
    pub data: Vec<Vec<i64>>,
}

/// Who the most messages between 0:00 and 6:00 are with.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MidnightKing {
    pub display_name: String,
    pub count: i64,
    /// Of all the messages between 0:00 and 6:00, to one decimal.
    pub percentage: f64,
}

/// The friend whose sent and received messages, 50 or more each, are closest to even.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MutualFriend {
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub sent_count: i64,
    pub received_count: i64,
    /// Sent ÷ received, to two decimals.
    pub ratio: f64,
}

/// Conversations opened by me or by them.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SocialInitiative {
    pub initiated_chats: i64,
    pub received_chats: i64,
    /// The share opened by me, in percent to one decimal.
    pub initiative_rate: f64,
    /// Whom I opened the most conversations with; left out when I opened none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_initiated_friend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_initiated_count: Option<i64>,
}

/// How fast I answer, in seconds.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResponseSpeed {
    pub avg_response_time: i64,
    pub fastest_friend: String,
    pub fastest_time: i64,
}

/// My Moments in the annual report.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnnualSnsStats {
    pub total_posts: i64,
    /// My posts by WeChat's post type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_counts: Option<IndexMap<String, i64>>,
    /// Who liked my posts the most.
    pub top_likers: Vec<FriendCount>,
    /// Whose posts I liked the most.
    pub top_liked: Vec<FriendCount>,
}

/// A friend and a count.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FriendCount {
    pub username: String,
    pub count: i64,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// A friend talked to much more in an earlier period than later.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LostFriend {
    pub username: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub early_count: i64,
    pub late_count: i64,
    /// The earlier period, as the report says it: 「去年的这个时候」 and the like.
    pub period_desc: String,
}

/// The dual report (`report dual generate`): me and one friend, over one year or all of them.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DualReport {
    /// 0 for all the years.
    pub year: i32,
    pub self_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_avatar_url: Option<String>,
    pub friend_username: String,
    pub friend_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friend_avatar_url: Option<String>,
    /// The first message ever between the two.
    pub first_chat: Option<DualMessage>,
    /// The first messages ever between the two, at most 10.
    pub first_chat_messages: Vec<DualMessage>,
    /// The first message of the year; null for all the years.
    pub year_first_chat: Option<YearFirstChat>,
    pub stats: DualStats,
    /// Phrases said at least twice by either, the most said first.
    pub top_phrases: Vec<PhraseCount>,
    /// Phrases three quarters or more of which are mine.
    pub my_exclusive_phrases: Vec<PhraseCount>,
    /// Phrases three quarters or more of which are the friend's.
    pub friend_exclusive_phrases: Vec<PhraseCount>,
    /// Seven rows, Monday first, of 24 hours each.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heatmap: Option<Vec<Vec<i64>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiative: Option<DualInitiative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<DualResponse>,
    /// Messages by month, `YYYY-MM`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly: Option<IndexMap<String, i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streak: Option<DualStreak>,
}

/// A message of the dual report.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DualMessage {
    /// Its text; raw for anything but text.
    pub content: String,
    pub is_sent_by_me: bool,
    /// Who sent it; only on `firstChat`, where the row says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_username: Option<String>,
    /// Milliseconds.
    pub create_time: i64,
    /// `YYYY-MM-DD HH:MM:SS`, local time.
    pub create_time_str: String,
    pub local_type: i64,
    /// A sticker's MD5.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_md5: Option<String>,
    /// A sticker's address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_cdn_url: Option<String>,
}

/// The first message of the year, with the first messages that follow it.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct YearFirstChat {
    /// Milliseconds.
    pub create_time: i64,
    pub create_time_str: String,
    pub content: String,
    pub is_sent_by_me: bool,
    pub friend_name: String,
    /// The year's first messages, at most 10.
    pub first_three_messages: Vec<DualMessage>,
    pub local_type: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_md5: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_cdn_url: Option<String>,
}

/// The counts of the dual report, and each one's most sent sticker.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DualStats {
    pub total_messages: i64,
    /// The characters of the text messages.
    pub total_words: i64,
    pub image_count: i64,
    pub voice_count: i64,
    pub emoji_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_top_emoji_md5: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_top_emoji_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friend_top_emoji_md5: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friend_top_emoji_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_top_emoji_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friend_top_emoji_count: Option<i64>,
}

/// Conversations opened by me, or by them.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DualInitiative {
    pub initiated: i64,
    pub received: i64,
}

/// How fast I answer the friend, in seconds.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DualResponse {
    pub avg: i64,
    pub fastest: i64,
    pub slowest: i64,
    /// The answers counted.
    pub count: i64,
}

/// The most days in a row the two talked.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DualStreak {
    pub days: i64,
    pub start_date: String,
    pub end_date: String,
}

/// A page of Moments posts (`sns timeline`), the newest first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SnsTimeline {
    pub timeline: Vec<SnsPost>,
}

impl SnsTimeline {
    /// The posts as the service builds them (`sns::enrich_post`, with `with-media` `sns_enrich_timeline_media`), read
    /// into the contract's types. A field they do not know fails here rather than leave the answer unseen; a test
    /// reads every field back out the same.
    pub fn of(posts: Vec<serde_json::Value>) -> crate::error::AppResult<Self> {
        let timeline = serde_json::from_value(serde_json::Value::Array(posts))
            .map_err(|err| crate::error::AppError::runtime(format!("a Moments post: {err}")))?;
        Ok(Self { timeline })
    }
}

/// A Moments post.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsPost {
    /// WeChat's id of the post.
    pub id: String,
    /// The row's id in WeChat's table, in decimal.
    pub tid: String,
    pub username: String,
    /// The poster's name: the post's own, else the contact's, else the username.
    pub nickname: String,
    pub create_time: i64,
    /// The text of the post.
    pub content_desc: String,
    /// WeChat's type of the post: 1 pictures, 15 a video, others links and such.
    #[serde(rename = "type")]
    pub kind: i64,
    /// The post's XML as WeChat keeps it.
    pub raw_xml: String,
    pub media: Vec<SnsMedia>,
    /// The names of those who liked it.
    pub likes: Vec<String>,
    pub comments: Vec<SnsComment>,
    /// The key of the post's video, which opens its cover too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SnsLocation>,
}

/// A picture or video of a post: addresses on WeChat's servers, fetched and decrypted by `sns media` with `key`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsMedia {
    pub url: String,
    pub thumb: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enc_idx: Option<String>,
    /// With `with-media`: the server's addresses, `url` and `thumb` being those of the files it fetched, and where
    /// it fetched them or a proxy of the server serves them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_thumb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_thumb_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_thumb_url: Option<String>,
    /// The moving part of a live photo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_photo: Option<SnsLivePhoto>,
}

/// The video of a live photo.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsLivePhoto {
    pub url: String,
    pub thumb: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enc_idx: Option<String>,
    /// With `with-media`: as for the picture it moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_thumb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_thumb_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_thumb_url: Option<String>,
}

/// A comment on a post.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsComment {
    pub id: String,
    pub nickname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub content: String,
    /// The comment it answers; empty when none.
    pub ref_comment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emojis: Vec<SnsEmoji>,
}

/// A sticker in a comment.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsEmoji {
    pub url: String,
    pub md5: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypt_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aes_key: Option<String>,
}

/// Where a post was made, as much as it says.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsLocation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poi_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poi_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poi_address_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The pushes of an official account (`biz messages`), as the desktop app lists them.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BizMessages {
    pub username: String,
    pub messages: Vec<BizMessage>,
}

/// One push of an official account: its lead article and the others sent with it.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BizMessage {
    /// The message's id in its table; null when its row does not say.
    pub local_id: Option<i64>,
    pub create_time: Option<i64>,
    pub title: String,
    pub description: String,
    pub url: String,
    pub cover: String,
    /// The other articles of the push.
    pub content_list: Vec<BizArticle>,
}

/// An article sent with a push.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BizArticle {
    pub title: String,
    pub url: String,
    pub cover: String,
    pub summary: String,
}

/// A page of WeChat Pay notices (`biz pay-records`), newest first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PayRecords {
    pub records: Vec<PayRecord>,
    /// Whether the WeChat Pay account has messages past this page; a page may hold fewer notices than `limit`, as
    /// its other messages are left out.
    pub has_more: bool,
}

/// A WeChat Pay notice.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PayRecord {
    pub title: String,
    /// Its lines, as WeChat writes them (amount, method, state…).
    pub description: String,
    /// Who was paid; `WeChat Pay` (in the server's language) when the notice does not say.
    pub merchant_name: String,
    pub merchant_icon: String,
    /// When it was published, in Unix seconds; 0 when the notice does not say.
    pub timestamp: i64,
    pub local_id: Option<i64>,
    pub create_time: Option<i64>,
}

/// A profile of the configuration (`config profiles`): its account and how far it is set up, without its keys.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub name: String,
    /// Whether the commands read it.
    pub current: bool,
    pub wxid: Option<String>,
    /// The WeChat data directory.
    pub db_path: Option<String>,
    /// Whether the database key is set.
    pub decrypt_key: bool,
    /// Whether both image keys are set.
    pub image_keys: bool,
    /// Whether the key of the AI model API is set.
    pub ai_key: bool,
    /// Whether it can open the databases: the data directory and the database key are set.
    pub complete: bool,
}

/// Where `weflow` keeps its files and what it is (`runtime info`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub home_dir: std::path::PathBuf,
    pub config_path: std::path::PathBuf,
    pub runtime_dir: std::path::PathBuf,
    pub version: String,
    /// The target it was built for, such as `x86_64-pc-windows-msvc`.
    pub target: String,
}

/// One part of WeFlow's cache (`cache list`), and how much of the disk it takes.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct CacheUsage {
    /// Its name, as `cache clear` takes it.
    pub part: crate::services::CachePart,
    pub paths: Vec<std::path::PathBuf>,
    pub bytes: u64,
    pub files: u64,
}

/// The default places of WeChat's data directory (`db detect`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbCandidates {
    pub candidates: Vec<DbCandidate>,
}

/// A place WeChat's data directory may be.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbCandidate {
    pub path: std::path::PathBuf,
    pub exists: bool,
}

/// The accounts in WeChat's data directory (`db wxid`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbAccounts {
    pub accounts: Vec<DbAccount>,
}

/// An account of WeChat's data directory.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbAccount {
    /// As `config set wxid` takes it, without the suffix of its folder's name.
    pub wxid: String,
    /// Its folder.
    pub path: std::path::PathBuf,
}

/// That the databases open with the configured key (`db test`); a failure is an error instead.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DbConnection {
    pub account_dir: std::path::PathBuf,
    /// Always true.
    pub connected: bool,
}

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

/// One insight record (`insight get`), with `success` inside as the desktop app has it. A record not found is a
/// failure of the command instead, so an answer that succeeds holds `success: true` and the record.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InsightLookup {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<crate::insight::InsightRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What `image auto-download status` answers: the hook the desktop app used is not part of WeFlow.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutoDownloadStatus {
    /// Always false.
    pub is_hooked: bool,
    /// Always null.
    pub pid: Option<u32>,
    /// Whether this computer can download WeChat's images on its own.
    pub supported: bool,
}

/// What `image auto-download start` answers once stopped (Ctrl+C).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct AutoDownloadStopped {
    /// Always true.
    pub stopped: bool,
}

/// The MD5 a video message names (`video parse-md5`); null when its XML has none.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct VideoMd5 {
    pub md5: Option<String>,
}

/// A conversation's messages written in one format (`export messages`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MessagesExported {
    /// The file; in a zip of several formats, its name inside the zip.
    pub out: PathBuf,
    /// The messages written.
    pub count: usize,
    pub session: String,
    pub format: String,
    /// The media copied beside it, with `media`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaCopied>,
}

/// One conversation in several formats (`export messages` with several `format`s), or packed (`zip`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum MessagesBundle {
    Folder(MessagesFolder),
    Zip(MessagesZip),
}

/// The formats of one conversation, each written to its file in one folder.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MessagesFolder {
    /// The folder.
    pub out: PathBuf,
    pub formats: Vec<String>,
    pub files: Vec<MessagesExported>,
}

/// The formats of one conversation packed into a zip, with their media.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MessagesZip {
    /// The zip.
    pub out: PathBuf,
    pub formats: Vec<String>,
    /// Each format's file, named as it is inside the zip.
    pub files: Vec<MessagesExported>,
    /// The media packed with them; null without `media`.
    pub media: Option<MediaCopied>,
}

/// The media an export of messages copied to its `media/` folder.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MediaCopied {
    /// The media messages asked for.
    pub requested: usize,
    pub exported: usize,
    /// The ones whose file is not on disk, or could not be read.
    pub missing: usize,
    pub dir: PathBuf,
    #[serde(flatten)]
    pub ffmpeg: Option<FfmpegMissing>,
}

/// What `export media` (and `chat voice`) copied.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MediaExport {
    pub exported: usize,
    /// The media messages found.
    pub found: usize,
    pub missing: usize,
    /// The missing ones by kind: `image`, `voice`, `video`, `emoji`.
    pub missing_by_kind: BTreeMap<String, usize>,
    /// The exported images that are only the thumbnail.
    pub thumb_only: usize,
    /// What `missing` and `thumbOnly` mean, in English.
    pub note: String,
    /// The conversations looked through.
    pub sessions: usize,
    pub out: PathBuf,
    pub files: Vec<MediaFileExported>,
    #[serde(flatten)]
    pub ffmpeg: Option<FfmpegMissing>,
}

/// A media file `export media` wrote.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MediaFileExported {
    pub session: String,
    /// `image`, `voice`, `video` or `emoji`.
    pub kind: String,
    pub file_name: String,
    pub path: String,
    pub local_id: i64,
    pub create_time: i64,
    /// An image's: the file is only its thumbnail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_thumb: Option<bool>,
}

/// The images an export left out because ffmpeg, which converts them, was not found.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegMissing {
    pub ffmpeg_missing: usize,
    pub ffmpeg_missing_images: Vec<MissingImage>,
    /// What to do, with the dates of the first ones.
    pub hint: String,
}

/// An image left out for lack of ffmpeg: where to find it in WeChat.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MissingImage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time: Option<i64>,
    /// The message's local time; `?` without one.
    pub time: String,
}

/// The file an export wrote (`export sessions`, `contacts`, `footprint`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ExportWritten {
    pub out: PathBuf,
    /// The format it was written in: `json`, `csv`, `excel`, …
    pub format: String,
}

/// What `group export-members` or `export-member-messages` wrote.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct GroupExported {
    /// Always true.
    pub success: bool,
    /// The members, or the messages, written.
    pub count: usize,
}

/// What `sns export` wrote.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnsExported {
    /// Always true.
    pub success: bool,
    /// Empty when no post matched, and nothing was written.
    pub file_path: PathBuf,
    pub post_count: usize,
    /// The media files copied beside it.
    pub media_count: usize,
}

/// The backup `backup create` wrote.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackupCreated {
    pub out: PathBuf,
    /// The files in it.
    pub entries: usize,
    pub wxid: String,
    /// Unix seconds.
    pub created_at: u64,
}

/// Where `backup restore` put the files.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackupRestored {
    pub restored_to: PathBuf,
}

/// What `cache clear` removed.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CacheCleared {
    pub removed_paths: Vec<String>,
    /// What the removed files took on disk.
    pub freed_bytes: u64,
    /// What could not be removed, and why; left out when everything was.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// What `cache clear-account` removed.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountCleared {
    pub removed_paths: Vec<String>,
    /// Whether the account was taken out of the profile (`db_path`, `wxid`, the keys).
    pub profile_reset: bool,
    /// What could not be removed, and why; left out when everything was.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// What `cache clear` or `clear-account` would remove (`--dry-run`), for a page that asks first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemovalPlan {
    /// Always true.
    pub dry_run: bool,
    pub paths: Vec<PlannedRemoval>,
    /// What they all take on disk.
    pub bytes: u64,
    /// The profile keys it would unset (`clear-account`).
    pub profile_keys: Vec<String>,
}

/// A file or folder a removal would take.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PlannedRemoval {
    pub path: PathBuf,
    pub bytes: u64,
}

/// A removal the terminal was asked to confirm and was not.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Cancelled {
    /// Always true.
    pub cancelled: bool,
}

/// The config file a `config` change was written to.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfigWritten {
    /// Null when `config unset config_path` finds no default place for it.
    pub config_path: Option<PathBuf>,
}

/// What `config import` took from the desktop app's config.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfigImported {
    pub imported_from: PathBuf,
    pub config_path: PathBuf,
    /// The keys left out because the desktop app keeps them encrypted.
    pub skipped_encrypted_keys: Vec<String>,
}

/// The language `weflow lang` saved: `en` or `zh`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct LangSaved {
    pub lang: String,
}

/// What the export window shows of each conversation asked for (`chat export-stats`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ExportStats {
    /// By username, in the order asked for.
    pub data: IndexMap<String, SessionExportStats>,
}

/// One conversation's messages by type, within the range asked for, and its relations.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionExportStats {
    pub total_messages: i64,
    pub voice_messages: i64,
    pub image_messages: i64,
    pub video_messages: i64,
    pub emoji_messages: i64,
    pub transfer_messages: i64,
    pub red_packet_messages: i64,
    pub call_messages: i64,
    /// Left out without messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_timestamp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_timestamp: Option<i64>,
    /// A group's: my messages in it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_my_messages: Option<i64>,
    /// A group's: the members who said something.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_active_speakers: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_member_count: Option<i64>,
    /// A group's, with relations: its members who are my friends.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_mutual_friends: Option<i64>,
    /// A private chat's, with relations: the groups we are both in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_mutual_groups: Option<i64>,
}

/// The names a group's transfer message shows for its two sides (`chat transfer-names`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TransferNames {
    pub payer_name: String,
    pub receiver_name: String,
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
    /// Always true: the desktop app's shape.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A post as `sns timeline` builds it reads into [`SnsPost`] and writes back out the same, every field of it: a
    /// field `sns::enrich_post` gains or loses fails here before it is lost from the answer.
    #[test]
    fn a_moments_post_reads_into_the_contract_and_back_the_same() {
        let xml = r#"<TimelineObject><id>1395</id><createTime>1700000000</createTime>
            <contentDesc>at the river</contentDesc>
            <location city="Shanghai" poiName="Bund" latitude="31.2" longitude="121"/>
            <ContentObject><type>1</type><mediaList>
            <media><url md5="aa11" token="t1" key="k1" enc_idx="1">http://mmsns.qpic.cn/a</url><thumb token="t1">http://mmsns.qpic.cn/a/150</thumb>
              <livePhoto><url token="t2" key="k2" md5="bb22" enc_idx="1">http://mmsns.qpic.cn/v</url><thumb>http://mmsns.qpic.cn/v/150</thumb></livePhoto></media>
            </mediaList></ContentObject><enc key="987"/>
            <like_user_list><user_comment><nickname>Carol</nickname></user_comment></like_user_list>
            <comment_user_list>
              <user_comment><comment_id>1</comment_id><username>wxid_carol</username><nickname>Carol</nickname><content>nice</content><ref_comment_id>0</ref_comment_id></user_comment>
              <user_comment><comment_id>2</comment_id><username>wxid_bob</username><nickname>Bob</nickname><content></content><ref_comment_id>1</ref_comment_id><ref_username>wxid_carol</ref_username>
                <emojiinfo><url>http://emoji.qpic.cn/e</url><md5>cc33</md5><width>120</width></emojiinfo></user_comment>
            </comment_user_list></TimelineObject>"#;
        let contact = crate::sns::CachedContact {
            display_name: Some("Bobby".into()),
            avatar_url: Some("https://wx.qlogo.cn/b".into()),
        };
        let post = crate::sns::enrich_post(
            &weflow_native::native_sns::parse_post(2001, "wxid_bob", xml),
            Some(&contact),
        );
        let typed: SnsPost = serde_json::from_value(post.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), post);
        // the test has every kind of field to lose
        assert!(typed.video_key.is_some() && typed.avatar_url.is_some());
        let media = &typed.media[0];
        assert!(media.key.is_some() && media.enc_idx.is_some() && media.live_photo.is_some());
        let location = typed.location.as_ref().unwrap();
        assert_eq!(location.longitude.as_ref().unwrap().as_i64(), Some(121));
        assert_eq!(typed.comments.len(), 2);
        assert!(
            !typed.comments[1].emojis.is_empty() && typed.comments[1].emojis[0].height.is_none()
        );
        assert_eq!(typed.likes, ["Carol"]);
        // what `with-media` adds to a picture and its live photo
        let fetched = json!({
            "url": "/sns/media/a", "thumb": "/sns/media/a.t", "key": "k1", "rawUrl": "http://mmsns.qpic.cn/a",
            "rawThumb": "http://mmsns.qpic.cn/a/150", "resolvedUrl": "/sns/media/a", "proxyThumbUrl": "/p/a.t",
            "livePhoto": { "url": "/sns/media/v", "thumb": "", "rawUrl": "http://mmsns.qpic.cn/v", "proxyUrl": "/p/v" },
        });
        let media: SnsMedia = serde_json::from_value(fetched.clone()).unwrap();
        assert_eq!(serde_json::to_value(media).unwrap(), fetched);
        // and a field none of them knows fails rather than goes missing from the answer
        assert!(
            serde_json::from_value::<SnsMedia>(json!({ "url": "", "thumb": "", "size": 1 }))
                .is_err()
        );
    }

    #[test]
    fn a_server_id_past_2_53_keeps_every_digit_and_only_the_fields_of_the_contract_are_written() {
        let rows = [json!({
            "local_id": 5, "server_id": "9007199254740993", "create_time": 1700000000, "local_type": 49,
            "sort_seq": 1700000000000i64, "message_content":
                "<msg><appmsg><type>6</type><title>a.pdf</title></appmsg></msg>",
            "_db_path": "/a/message_0.db",
        })];
        let message = crate::chat_msg::map_rows(&rows, "wxid_me").remove(0);
        let doc = json!(ParsedMessage {
            message,
            cursor: Some("1.2.3".into()),
            session_id: None,
        });
        assert_eq!(doc["serverId"], "9007199254740993");
        assert_eq!(doc["_cursor"], "1.2.3");
        for key in ["serverIdRaw", "dbPath", "tableName", "t49", "_session_id"] {
            assert!(doc.get(key).is_none(), "{key} in {doc}");
        }
        // what the message says, without what it does not
        assert_eq!(doc["fileName"], "a.pdf");
        assert!(doc.get("linkUrl").is_none() && doc.get("quotedContent").is_none());
        // the nullable fields are there
        assert!(doc["isSend"].is_null() || doc["isSend"].is_number());
        assert!(doc.as_object().unwrap().contains_key("senderUsername"));
    }
}
