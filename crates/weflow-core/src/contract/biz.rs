//! Official accounts' pushes and WeChat Pay notices.

use schemars::JsonSchema;
use serde::Serialize;

/// The pushes of an official account (`biz messages`).
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
