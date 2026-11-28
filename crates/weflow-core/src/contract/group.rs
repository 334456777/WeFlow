//! 群聊，以及私聊的统计：成员、排行、时段和消息的种类。

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::chat_msg::ChatMessage;

/// `group list` 中的一个群聊，成员多的在前。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupChat {
    /// 群的 ID，`…@chatroom`。
    pub username: String,
    /// 群的备注，没有时依次用群名和群的 ID。
    pub display_name: String,
    /// 成员数。
    pub member_count: i64,
    /// 群头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// 群的成员（`group members`）：群主在前，其次是好友，再按发的消息数从多到少，最后按名字排列。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupMembers {
    /// 群的 ID，`…@chatroom`。
    pub chatroom_id: String,
    /// 成员数。
    pub count: usize,
    /// 生成成员面板的时间，Unix 毫秒。
    pub updated_at: i64,
    /// 这些成员。
    pub members: Vec<GroupMember>,
}

/// 群成员面板中的一个成员。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupMember {
    /// 成员的 wxid。
    pub username: String,
    /// 成员在账号中的显示名：备注，没有时依次用昵称、微信号和用户名本身（群昵称见 `groupNickname`）。
    pub display_name: String,
    /// 联系人中的昵称；不在联系人中时为空。
    pub nickname: String,
    /// 联系人中的微信号；不在联系人中或没有设置时为空。
    pub alias: String,
    /// 账号所有者给对方的备注；不在联系人中或没有备注时为空。
    pub remark: String,
    /// 对方在这个群里给自己起的群昵称；没有时为空。
    pub group_nickname: String,
    /// 头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// 是否为群主。
    pub is_owner: bool,
    /// 是否为账号所有者的好友。
    pub is_friend: bool,
    /// 在群里发的消息数；不带 `counts=true` 时为 0。
    pub message_count: i64,
}

/// 群里谁发了多少条消息（`group ranking`），多的在前。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupRank {
    /// 成员。
    pub member: RankedMember,
    /// 在请求的时间范围内发的消息数。
    pub message_count: i64,
}

/// 排行中的一个成员。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RankedMember {
    /// 成员的 wxid。
    pub username: String,
    /// 成员在账号中的显示名：备注，没有时依次用昵称、微信号和用户名本身。
    pub display_name: String,
    /// 头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// 群在一天中每个小时的消息数（`group hours`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupHours {
    /// 按小时计数，键为 `"0"` 到 `"23"`，按服务端所在的时区。
    pub hourly_distribution: IndexMap<String, i64>,
}

/// 群里各种消息的数量（`group media`），多的在前。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupMedia {
    /// 各种消息，多的在前；没有消息的种类不列出。
    pub type_counts: Vec<MediaCount>,
    /// 计入的全部消息数。
    pub total: i64,
}

/// 一种消息及其数量。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MediaCount {
    /// 消息类型：文本 1、图片 3、语音 34、视频 43、表情 47、链接和文件 49；其余类型合在一起为 -1。
    #[serde(rename = "type")]
    pub kind: i64,
    /// 种类的中文名，如「文本」「其他」。
    pub name: String,
    /// 这种消息的数量。
    pub count: i64,
}

/// 群里一个成员发的消息（`group member`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemberAnalytics {
    /// 各类消息的数量。
    pub statistics: MemberStatistics,
    /// 按小时计数，键为 `"0"` 到 `"23"`，按服务端所在的时区。
    pub time_distribution: IndexMap<String, i64>,
    /// 说得最多的话，多的在前，最多 10 条。
    pub common_phrases: Vec<PhraseCount>,
    /// 文字中的微信表情（`[微笑]` 之类），用得多的在前，最多 10 个。
    pub common_emojis: Vec<EmojiCount>,
}

/// 一个成员各类消息的数量。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemberStatistics {
    /// 全部消息数。
    pub total_messages: i64,
    /// 文字消息数，包括引用。
    pub text_messages: i64,
    /// 图片数。
    pub image_messages: i64,
    /// 语音消息数。
    pub voice_messages: i64,
    /// 视频数。
    pub video_messages: i64,
    /// 表情数。
    pub emoji_messages: i64,
    /// 其他各种消息的数量。
    pub other_messages: i64,
    /// 第一条消息的时间；没有发过时为 `null`。
    pub first_message_time: Option<i64>,
    /// 最后一条消息的时间；没有发过时为 `null`。
    pub last_message_time: Option<i64>,
    /// 发过消息的天数，按服务端所在的时区。
    pub active_days: usize,
    /// 按消息类型（与 `localType` 相同）计数。
    pub message_type_counts: IndexMap<String, i64>,
}

/// 一句话和它被说了几次。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhraseCount {
    /// 一条完整的文字消息（不超过 20 个字符），按完全相同计数。
    pub phrase: String,
    /// 发了几次。
    pub count: i64,
}

/// 一个表情和它被发了几次。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct EmojiCount {
    /// 表情在文字中的写法，如 `[微笑]`。
    pub emoji: String,
    /// 用了几次。
    pub count: i64,
}

/// 一个成员在群里的消息中的一页（`group member-messages`），从新到旧。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemberMessages {
    /// 这一页的消息。
    pub messages: Vec<ChatMessage>,
    /// 这一页之后是否还有消息。
    pub has_more: bool,
    /// 作为 `cursor` 传入就能读下一页。
    pub next_cursor: usize,
}

/// 排除名单以外的全部私聊合在一起的统计（`analytics overall`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OverallStatistics {
    /// 全部消息数。
    pub total_messages: i64,
    /// 文字消息数，包括引用。
    pub text_messages: i64,
    /// 图片数。
    pub image_messages: i64,
    /// 语音消息数。
    pub voice_messages: i64,
    /// 视频数。
    pub video_messages: i64,
    /// 表情数。
    pub emoji_messages: i64,
    /// 其他各种消息的数量。
    pub other_messages: i64,
    /// 账号所有者发出的消息数。
    pub sent_messages: i64,
    /// 账号所有者收到的消息数。
    pub received_messages: i64,
    /// 第一条消息的时间；没有消息时为 `null`。
    pub first_message_time: Option<i64>,
    /// 最后一条消息的时间；没有消息时为 `null`。
    pub last_message_time: Option<i64>,
    /// 有消息的天数。
    pub active_days: i64,
    /// 按消息类型（与 `localType` 相同）计数。
    pub message_type_counts: IndexMap<String, i64>,
}

/// `analytics rankings` 中的一个私聊，消息最多的在前；不含排除名单中的人。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContactRank {
    /// 对方的 wxid。
    pub username: String,
    /// 备注，没有时依次用昵称、微信号和用户名本身。
    pub display_name: String,
    /// 头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// 对方设置的微信号；没有时，用户名不以 `wxid_` 开头则为用户名，否则为空。
    pub wechat_id: String,
    /// 请求的时间范围内这个私聊的消息数。
    pub message_count: i64,
    /// 其中账号所有者发出的消息数。
    pub sent_count: i64,
    /// 其中账号所有者收到的消息数。
    pub received_count: i64,
    /// 最后一条消息的时间；没有消息时为 `null`。
    pub last_message_time: Option<i64>,
}

/// 排除名单以外的私聊的消息在什么时候发出（`analytics time`），按服务端所在的时区。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TimeDistribution {
    /// 按小时计数，键为 `"0"` 到 `"23"`。
    pub hourly_distribution: IndexMap<String, i64>,
    /// 按星期计数，键为 `"1"`（星期一）到 `"7"`（星期日）。
    pub weekday_distribution: IndexMap<String, i64>,
    /// 按月份计数，键为 `YYYY-MM`。
    pub monthly_distribution: IndexMap<String, i64>,
}

/// 可以从统计中排除的人（`analytics exclude-candidates`）：各个私聊的对方，以及已经排除的人。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExcludeCandidate {
    /// 对方的 wxid。
    pub username: String,
    /// 备注，没有时依次用昵称、微信号和用户名本身。
    pub display_name: String,
    /// 头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// 写法与 `analytics rankings` 的 `wechatId` 相同。
    pub wechat_id: String,
}
