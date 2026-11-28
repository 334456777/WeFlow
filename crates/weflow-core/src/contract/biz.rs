//! Official accounts' pushes and WeChat Pay notices.

use schemars::JsonSchema;
use serde::Serialize;

/// The pushes of an official account (`biz messages`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BizMessages {
    /// The official account, `gh_…`.
    pub username: String,
    /// Its pushes; messages without content are left out.
    pub messages: Vec<BizMessage>,
}

/// One push of an official account: its lead article and the others sent with it, or its text.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BizMessage {
    /// The message's id in its table; null when its row does not say.
    pub local_id: Option<i64>,
    /// When it was sent; null when its row does not say.
    pub create_time: Option<i64>,
    /// The push's text when it is not an article (a plain text message); empty for an article.
    pub text: String,
    /// The lead article's title; empty when the push has none.
    pub title: String,
    /// The lead article's summary.
    pub description: String,
    /// The lead article's address.
    pub url: String,
    /// The lead article's cover image: its `cover`, else its `thumburl`.
    pub cover: String,
    /// The other articles of the push.
    pub content_list: Vec<BizArticle>,
}

/// An article sent with a push.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BizArticle {
    /// Never empty: an article without a title is left out.
    pub title: String,
    /// The article's address.
    pub url: String,
    /// Its cover image: its `cover`, else its `thumburl`.
    pub cover: String,
    /// Its `summary`, else its `digest`.
    pub summary: String,
}

/// A page of WeChat Pay notices (`biz pay-records`), newest first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PayRecords {
    /// The notices of the page.
    pub records: Vec<PayRecord>,
    /// Whether the WeChat Pay account has messages past this page; a page may hold fewer notices than `limit`, as
    /// its other messages are left out.
    pub has_more: bool,
}

/// A WeChat Pay notice.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PayRecord {
    /// The notice's title, such as 「已支付¥9.99」.
    pub title: String,
    /// Its lines, as WeChat writes them (amount, method, state…).
    pub description: String,
    /// How it was paid, as the card's top line 「使用零钱支付」 names it (「零钱」); null when the card has no top line of that form.
    pub method: Option<String>,
    /// Its state, the words before the amount in the title (「已支付」 of 「已支付¥9.99」); null when the title holds no
    /// amount.
    pub status: Option<String>,
    /// Who was paid; `WeChat Pay` (in the server's language) when the notice does not say.
    pub merchant_name: String,
    /// The address of the payee's icon; empty when the notice has none.
    pub merchant_icon: String,
    /// When it was published, in Unix seconds; 0 when the notice does not say.
    pub timestamp: i64,
    /// The message's id in its table.
    pub local_id: Option<i64>,
    /// When the message was received; null when its row does not say.
    pub create_time: Option<i64>,
}
