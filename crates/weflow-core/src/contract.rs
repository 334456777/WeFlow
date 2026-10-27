//! The command API's contract (#399): what the `data` of its key reads holds, as Rust types that serialize to the
//! answer and describe it as JSON Schema. `/openapi.json` carries the schemas, and the WebUI's types are generated
//! from them, so a change here that its clients would not read the same shows in their types.
//!
//! Throughout: times are Unix seconds unless a field says otherwise; an id of WeChat's that may pass 2^53 (a
//! message's server id) is a decimal string; an optional field is left out when there is nothing to say, a nullable
//! one is `null`. The rows of WeChat's own tables (`chat messages` without `parsed`, `chat contacts` without
//! `parsed`…) have the columns of those tables and are not described further.

use std::collections::BTreeMap;

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
#[derive(Clone, Debug, Serialize, JsonSchema)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
