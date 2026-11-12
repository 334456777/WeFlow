//! The annual report and the dual report.

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::group::PhraseCount;

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
