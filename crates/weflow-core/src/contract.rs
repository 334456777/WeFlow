//! The command API's contract (#399): what the `data` of its key reads holds, as Rust types that serialize to the
//! answer and describe it as JSON Schema. `/openapi.json` carries the schemas, and the WebUI's types are generated
//! from them, so a change here that its clients would not read the same shows in their types.
//!
//! Throughout: times are Unix seconds unless a field says otherwise; an id of WeChat's that may pass 2^53 (a
//! message's server id) is a decimal string; an optional field is left out when there is nothing to say, a nullable
//! one is `null`. The rows of WeChat's own tables (`chat messages` without `parsed`, `chat contacts` without
//! `parsed`…) have the columns of those tables and are not described further.

use schemars::JsonSchema;
use serde::Serialize;

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
    /// WeChat's type of the last message (see [`ChatMessage::local_type`]).
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
    /// WeChat's type of the message (see [`ChatMessage::local_type`]).
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
