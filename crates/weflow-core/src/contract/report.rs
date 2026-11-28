//! 年度报告和双人报告。

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::group::PhraseCount;

/// 私聊中有消息的年份（`report annual years`），最近的在前。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ReportYears {
    /// 这些年份，按服务端所在的时区。
    pub years: Vec<i64>,
    /// 找出年份的方式：`native`（只读数据库层按每条消息的时间查出）、`table-scan`（各表的时间范围）或
    /// `edge-scan`（各私聊的第一条和最后一条消息）。
    pub strategy: String,
}

/// 年度报告（`report annual generate`），统计一年或所有年份的私聊。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnnualReport {
    /// 报告的年份；所有年份时为 0。
    pub year: i32,
    /// 私聊的消息数。
    pub total_messages: i64,
    /// 有消息的私聊数。
    pub total_friends: usize,
    /// 消息最多的三个私聊。
    pub core_friends: Vec<CoreFriend>,
    /// 1 到 12 月每个月消息最多的好友。
    pub monthly_top_friends: Vec<MonthlyTopFriend>,
    /// 消息最多的一天；没有消息时为 `null`。
    pub peak_day: Option<PeakDay>,
    /// 与一位好友连续聊天最多的天数；没有消息时为 `null`。
    pub longest_streak: Option<LongestStreak>,
    /// 按星期和小时的消息数。
    pub activity_heatmap: ActivityHeatmap,
    /// 0:00 到 6:00 之间消息最多的好友；这段时间没有消息时为 `null`。
    pub midnight_king: Option<MidnightKing>,
    /// 账号所有者头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_avatar_url: Option<String>,
    /// 双方来往最均衡的好友；没有一位好友双方各发了 50 条以上时为 `null`。
    pub mutual_friend: Option<MutualFriend>,
    /// 一轮轮聊天由谁开始；没有聊天时为 `null`。
    pub social_initiative: Option<SocialInitiative>,
    /// 账号所有者回复的速度；没有一个私聊有 10 次以上回复时为 `null`。
    pub response_speed: Option<ResponseSpeed>,
    /// 账号所有者说过至少 2 次的常用语，说得多的在前。
    pub top_phrases: Vec<PhraseCount>,
    /// 账号所有者的朋友圈；读不了朋友圈时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sns_stats: Option<AnnualSnsStats>,
    /// 前一段时间聊得多、后来很少联系的好友；没有符合条件的好友时为 `null`。
    pub lost_friend: Option<LostFriend>,
}

/// 消息最多的三个私聊之一。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CoreFriend {
    /// 好友的 wxid。
    pub username: String,
    /// 备注，没有时依次用昵称、微信号和用户名本身。
    pub display_name: String,
    /// 头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// 这个私聊的消息数。
    pub message_count: i64,
    /// 其中账号所有者发出的消息数。
    pub sent_count: i64,
    /// 其中账号所有者收到的消息数。
    pub received_count: i64,
}

/// 一个月中消息最多的好友。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyTopFriend {
    /// 月份，1 到 12。
    pub month: i64,
    /// 好友的名字；没有消息的月份为「暂无」。
    pub display_name: String,
    /// 头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// 这个月与好友的消息数；没有消息的月份为 0。
    pub message_count: i64,
}

/// 消息最多的一天，以及这天消息最多的好友。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PeakDay {
    /// 日期，`YYYY-MM-DD`。
    pub date: String,
    /// 这天的消息数。
    pub message_count: i64,
    /// 这天消息最多的好友的名字；读不出时为空。
    pub top_friend: String,
    /// 这天与那位好友的消息数。
    pub top_friend_count: i64,
}

/// 与一位好友连续聊天最多的天数。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LongestStreak {
    /// 好友的名字。
    pub friend_name: String,
    /// 连续的天数。
    pub days: i64,
    /// 第一天，`YYYY-MM-DD`。
    pub start_date: String,
    /// 最后一天，`YYYY-MM-DD`。
    pub end_date: String,
}

/// 按星期和小时的消息数。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ActivityHeatmap {
    /// 七行，星期一在前，每行 24 个小时。
    pub data: Vec<Vec<i64>>,
}

/// 0:00 到 6:00 之间消息最多的好友。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MidnightKing {
    /// 好友的名字。
    pub display_name: String,
    /// 0:00 到 6:00 之间与这位好友的消息数。
    pub count: i64,
    /// 占 0:00 到 6:00 之间全部消息的百分比，保留一位小数。
    pub percentage: f64,
}

/// 发出和收到的消息各有 50 条以上、两者最接近相等的好友。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MutualFriend {
    /// 好友的名字。
    pub display_name: String,
    /// 头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// 账号所有者发给对方的消息数。
    pub sent_count: i64,
    /// 对方发给账号所有者的消息数。
    pub received_count: i64,
    /// 发出数除以收到数，保留两位小数。
    pub ratio: f64,
}

/// 由账号所有者或对方开始的一轮轮聊天。
///
/// 静默超过 1 小时后的第一条消息开始新的一轮。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SocialInitiative {
    /// 账号所有者开始的轮数。
    pub initiated_chats: i64,
    /// 对方开始的轮数。
    pub received_chats: i64,
    /// 账号所有者开始的比例，百分数，保留一位小数。
    pub initiative_rate: f64,
    /// 账号所有者主动开始聊天最多的好友；一轮都没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_initiated_friend: Option<String>,
    /// 与那位好友主动开始的轮数；一轮都没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_initiated_count: Option<i64>,
}

/// 账号所有者回复的速度，单位为秒。
///
/// 回复是同一轮聊天中对方消息之后账号所有者的消息，间隔在一天之内；只计入回复至少 10 次的私聊。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResponseSpeed {
    /// 全部回复的平均时间。
    pub avg_response_time: i64,
    /// 平均回复最快的好友的名字。
    pub fastest_friend: String,
    /// 对那位好友的平均回复时间。
    pub fastest_time: i64,
}

/// 年度报告中账号所有者的朋友圈。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnnualSnsStats {
    /// 账号所有者的动态数。
    pub total_posts: i64,
    /// 按微信的动态类型计数；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_counts: Option<IndexMap<String, i64>>,
    /// 给账号所有者的动态点赞最多的好友。
    pub top_likers: Vec<FriendCount>,
    /// 账号所有者点赞最多的好友。
    pub top_liked: Vec<FriendCount>,
}

/// 一位好友和一个计数。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FriendCount {
    /// 好友的 wxid。
    pub username: String,
    /// 点赞的次数。
    pub count: i64,
    /// 备注，没有时依次用昵称、微信号和用户名本身。
    pub display_name: String,
    /// 头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// 前一段时间比后一段聊得多得多的好友。
///
/// 前一段超过 80 条消息（所有年份时超过 100 条），并且是后一段的 5 倍以上。
/// 所有年份时前后两段以最早和最晚一天的中点分开；今年为过去 12 个月的前 6 个月和后 6 个月；往年为上半年和下半年。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LostFriend {
    /// 好友的 wxid。
    pub username: String,
    /// 备注，没有时依次用昵称、微信号和用户名本身。
    pub display_name: String,
    /// 头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// 前一段的消息数。
    pub early_count: i64,
    /// 后一段的消息数。
    pub late_count: i64,
    /// 报告对前一段的说法：所有年份为「这段时间以来」，今年为「去年的这个时候」，往年为「<年份>年上半年」。
    pub period_desc: String,
}

/// 双人报告（`report dual generate`）：账号所有者和一位好友，统计一年或所有年份。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DualReport {
    /// 报告的年份；所有年份时为 0。
    pub year: i32,
    /// 账号所有者的名字：联系人中的显示名，没有时为 wxid。
    pub self_name: String,
    /// 账号所有者头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_avatar_url: Option<String>,
    /// 好友的 wxid。
    pub friend_username: String,
    /// 备注，没有时依次用昵称、微信号和用户名本身。
    pub friend_name: String,
    /// 好友头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friend_avatar_url: Option<String>,
    /// 两人之间的第一条消息；没有消息时为 `null`。
    pub first_chat: Option<DualMessage>,
    /// 两人之间最早的几条消息，最多 10 条。
    pub first_chat_messages: Vec<DualMessage>,
    /// 这一年的第一条消息；所有年份或这一年没有消息时为 `null`。
    pub year_first_chat: Option<YearFirstChat>,
    /// 各项计数。
    pub stats: DualStats,
    /// 两人中任一方说过至少 2 次的常用语，说得多的在前；不含 `exclude-word` 给出的词。
    pub top_phrases: Vec<PhraseCount>,
    /// 至少四分之三出自账号所有者、且账号所有者说过至少 2 次的常用语，最多 20 条。
    pub my_exclusive_phrases: Vec<PhraseCount>,
    /// 至少四分之三出自好友、且好友说过至少 2 次的常用语，最多 20 条。
    pub friend_exclusive_phrases: Vec<PhraseCount>,
    /// 按星期和小时的消息数：七行，星期一在前，每行 24 个小时。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heatmap: Option<Vec<Vec<i64>>>,
    /// 一轮轮聊天由谁开始。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiative: Option<DualInitiative>,
    /// 账号所有者回复的速度；一次都没有回复时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<DualResponse>,
    /// 按月份的消息数，键为 `YYYY-MM`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly: Option<IndexMap<String, i64>>,
    /// 两人连续聊天最多的天数；没有消息时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streak: Option<DualStreak>,
}

/// 双人报告中的一条消息。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DualMessage {
    /// 微信保存的内容：文字消息为文字，其他消息为原始内容（常为 XML）。
    pub content: String,
    /// 是否为账号所有者发的。
    pub is_sent_by_me: bool,
    /// 发送者；只在 `firstChat` 中、消息行写了发送者时才有。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_username: Option<String>,
    /// 发送时间，Unix 毫秒。
    pub create_time: i64,
    /// 发送时间，服务端所在时区的 `YYYY-MM-DD HH:MM:SS`。
    pub create_time_str: String,
    /// 消息类型，与解析过的消息的 `localType` 相同。
    pub local_type: i64,
    /// app 消息的种类，与解析过的消息的 `appMsgKind` 相同；其他消息省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_kind: Option<String>,
    /// 表情的 MD5。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_md5: Option<String>,
    /// 表情的地址。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_cdn_url: Option<String>,
}

/// 这一年的第一条消息，以及它之后最早的几条消息。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct YearFirstChat {
    /// 发送时间，Unix 毫秒。
    pub create_time: i64,
    /// 发送时间，服务端所在时区的 `YYYY-MM-DD HH:MM:SS`。
    pub create_time_str: String,
    /// 微信保存的内容：文字消息为文字，其他消息为原始内容（常为 XML）。
    pub content: String,
    /// 是否为账号所有者发的。
    pub is_sent_by_me: bool,
    /// 好友的名字。
    pub friend_name: String,
    /// 这一年最早的几条消息，最多 10 条。
    pub first_three_messages: Vec<DualMessage>,
    /// 消息类型，与解析过的消息的 `localType` 相同。
    pub local_type: i64,
    /// app 消息的种类，与解析过的消息的 `appMsgKind` 相同；其他消息省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_kind: Option<String>,
    /// 表情的 MD5。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_md5: Option<String>,
    /// 表情的地址。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_cdn_url: Option<String>,
}

/// 双人报告的计数，以及双方各自发得最多的表情。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DualStats {
    /// 两人之间的全部消息数。
    pub total_messages: i64,
    /// 文字消息的字数。
    pub total_words: i64,
    /// 图片数。
    pub image_count: i64,
    /// 语音消息数。
    pub voice_count: i64,
    /// 表情数。
    pub emoji_count: i64,
    /// 账号所有者发得最多的表情的 MD5；一个都没发时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_top_emoji_md5: Option<String>,
    /// 这个表情的地址；消息和表情数据库中都没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_top_emoji_url: Option<String>,
    /// 好友发得最多的表情的 MD5；一个都没发时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friend_top_emoji_md5: Option<String>,
    /// 这个表情的地址；消息和表情数据库中都没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friend_top_emoji_url: Option<String>,
    /// 账号所有者发这个表情的次数；一个都没发时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_top_emoji_count: Option<i64>,
    /// 好友发那个表情的次数；一个都没发时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friend_top_emoji_count: Option<i64>,
}

/// 由账号所有者或好友开始的一轮轮聊天。
///
/// 静默超过 1 小时后的第一条消息开始新的一轮。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DualInitiative {
    /// 账号所有者开始的轮数。
    pub initiated: i64,
    /// 好友开始的轮数。
    pub received: i64,
}

/// 账号所有者回复好友的速度，单位为秒。
///
/// 回复是同一轮聊天中好友消息之后账号所有者的消息，间隔在一天之内。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DualResponse {
    /// 平均时间，取整。
    pub avg: i64,
    /// 最快的一次。
    pub fastest: i64,
    /// 最慢的一次。
    pub slowest: i64,
    /// 计入的回复次数。
    pub count: i64,
}

/// 两人连续聊天最多的天数。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DualStreak {
    /// 连续的天数。
    pub days: i64,
    /// 第一天，`YYYY-MM-DD`。
    pub start_date: String,
    /// 最后一天，`YYYY-MM-DD`。
    pub end_date: String,
}
