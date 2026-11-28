//! 公众号的推送和微信支付的通知。

use schemars::JsonSchema;
use serde::Serialize;

/// 一个公众号的推送（`biz messages`），从新到旧的一页。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BizMessages {
    /// 公众号的用户名，`gh_…`。
    pub username: String,
    /// 它的推送；没有内容的消息不列出。
    pub messages: Vec<BizMessage>,
}

/// 公众号的一次推送：头条文章和随它一起发出的其他文章。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BizMessage {
    /// 消息的本地 ID，只在所在分片中唯一。
    pub local_id: Option<i64>,
    /// 发出的时间；消息行中没有时为 `null`。
    pub create_time: Option<i64>,
    /// 头条文章的标题；推送中没有时为空。
    pub title: String,
    /// 头条文章的摘要（`<des>`）。
    pub description: String,
    /// 头条文章的地址。
    pub url: String,
    /// 头条文章的封面图：`<cover>`，没有时为 `<thumburl>`。
    pub cover: String,
    /// 推送中的其他文章。
    pub content_list: Vec<BizArticle>,
}

/// 随推送一起发出的一篇文章。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BizArticle {
    /// 标题，不会为空：没有标题的文章不列出。
    pub title: String,
    /// 文章的地址。
    pub url: String,
    /// 封面图：`<cover>`，没有时为 `<thumburl>`。
    pub cover: String,
    /// 摘要：`<summary>`，没有时为 `<digest>`。
    pub summary: String,
}

/// 微信支付通知的一页（`biz pay-records`），从新到旧。
///
/// 通知来自微信支付账号的会话，以及微信存放支付卡片的服务通知。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PayRecords {
    /// 这一页的通知。
    pub records: Vec<PayRecord>,
    /// 这一页之后是否还有消息：这一页读满 `limit` 条消息时为 `true`。两个会话中不是微信支付通知的消息不列出，
    /// 所以一页的通知可能少于 `limit` 条。
    pub has_more: bool,
}

/// 一条微信支付通知。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PayRecord {
    /// 通知的标题，如「已支付¥9.99」。
    pub title: String,
    /// 通知的各行，即微信写的原文（金额、支付方式、状态等）。
    pub description: String,
    /// 支付方式，取自卡片顶行「使用零钱支付」这样的写法（「零钱」）；卡片没有这样的顶行时为 `null`。
    pub method: Option<String>,
    /// 状态，即标题中金额前面的文字（「已支付¥9.99」中的「已支付」）；标题中没有金额时为 `null`。
    pub status: Option<String>,
    /// 收款方（`<display_name>`）；通知中没有时为服务端语言的「微信支付」（英文为 `WeChat Pay`）。
    pub merchant_name: String,
    /// 收款方图标的地址；通知中没有时为空。
    pub merchant_icon: String,
    /// 通知发布的时间（`<pub_time>`），Unix 秒；通知中没有时为 0。
    pub timestamp: i64,
    /// 消息的本地 ID，只在所在分片中唯一。
    pub local_id: Option<i64>,
    /// 收到消息的时间；消息行中没有时为 `null`。
    pub create_time: Option<i64>,
}
