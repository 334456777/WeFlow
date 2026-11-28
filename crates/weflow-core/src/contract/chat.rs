//! 聊天的读取：会话、联系人、消息及其分页、资源，以及会话的状态和详情。

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::chat_msg::ChatMessage;
use crate::message::appmsg::app_message_kind;
use crate::message::{parse_message_content, ExportMsg};

/// 聊天窗口显示的一条消息（带 `parsed` 的 `chat messages`、`chat message` 和 `chat search`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ParsedMessage {
    /// 消息本身，它的字段放在顶层。
    #[serde(flatten)]
    pub message: ChatMessage,
    /// 消息在会话中的位置（游标）：作为 `before` 或 `after` 传入，就能读取它前后的消息。
    /// 它是不透明的，只有服务端解读它。
    #[serde(rename = "_cursor", skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// 搜到的消息所在的会话，只在 `chat search` 中有。
    #[serde(rename = "_session_id", skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// `chat sessions` 中的一个会话，从新到旧排列。
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// 对方的 wxid；群聊为 `…@chatroom`，公众号为 `gh_…`。
    pub username: String,
    /// 会话表中这一行的类型；从联系人补上的会话（会话表中没有的公众号等）为 0。
    #[serde(rename = "type")]
    pub kind: i64,
    /// 在微信中还没读的消息数；公众号没有这一列，按最近收到的消息估算。
    pub unread_count: i64,
    /// 列表显示的最后一条消息；会话表中没有摘要时为消息类型的标签（如 `[图片]`）。
    pub summary: String,
    /// 列表排序所依据的时间（公众号不早于最后一条消息的时间）；从联系人补上、没有消息的会话为 0。
    pub sort_timestamp: i64,
    /// 最后一条消息的时间；会话表的行中没有时取 `sortTimestamp`，从联系人补上、没有消息的会话为 0。
    pub last_timestamp: i64,
    /// 最后一条消息的类型，与消息的 `localType` 相同。
    pub last_msg_type: i64,
    /// 会话大约有多少条消息，会话表的行或消息表给出时才有。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_count_hint: Option<i64>,
    /// 备注，没有时依次用昵称、微信号和用户名本身。
    pub display_name: String,
    /// 群聊中最后一条消息的发送者。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_msg_sender: Option<String>,
    /// 群聊中最后一条消息的发送者的名字。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sender_display_name: Option<String>,
    /// 账号自己的 wxid（裸 wxid）。
    pub self_wxid: String,
    /// 会话头像的地址；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// `chat contacts --parsed` 中联系人的种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContactKind {
    /// 好友。
    Friend,
    /// 群聊。
    Group,
    /// 公众号。
    Official,
    /// 曾经的好友。
    FormerFriend,
}

impl ContactKind {
    /// 序列化后的名字，`export contacts --kind` 接受同样的写法。
    pub fn name(self) -> &'static str {
        match self {
            Self::Friend => "friend",
            Self::Group => "group",
            Self::Official => "official",
            Self::FormerFriend => "former_friend",
        }
    }
}

/// `chat contacts --parsed` 中的一个联系人：最近聊过的在前，其余按名字排列。列表不含账号自己。
///
/// 文字字段为空时省略。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    /// 联系人的 wxid；群聊为 `…@chatroom`，公众号为 `gh_…`。
    pub username: String,
    /// 备注，没有时依次用昵称、微信号和用户名本身。
    pub display_name: String,
    /// 账号所有者给对方的备注。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
    /// 对方自己设的昵称。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    /// 微信号。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// 联系人所在标签的名称。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    /// 个性签名。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_description: Option<String>,
    /// 对方填写的地区。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// 联系人的种类。
    #[serde(rename = "type")]
    pub kind: ContactKind,
    /// 头像的地址：大头像，没有时为小头像。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// 查询命令列出的一条消息（`chat resources`、`chat voice-messages`、`chat media-stream`，以及不带 `parsed` 的
/// `chat message`）。
///
/// 含消息行的主要字段、消息的文字和媒体的标识。可选字段在消息没有时省略。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    /// 本地 ID，只在所在分片中唯一；与 `createTime` 一起是 `chat message` 查找消息所用的键。
    pub local_id: i64,
    /// 服务器 ID 的十进制字符串（可能超过 2^53）；消息没有服务器 ID 时为 `"0"`。
    pub server_id: String,
    /// 发送时间。
    pub create_time: i64,
    /// 消息类型，与解析过的消息的 `localType` 相同。
    pub local_type: i64,
    /// 账号所有者发的为 1，否则为 0。
    pub is_send: i64,
    /// 发送者，与导出相同：系统消息和拍一拍中，撤回提示为撤回者，其他为会话；其余消息中，账号自己发的为账号，
    /// 其他为行中的发送者，没有时为会话。
    pub sender_username: String,
    /// 微信保存的消息内容（常为 XML）。
    pub content: String,
    /// 消息的文字，写法与导出相同；没有文字时为 `null`。
    pub text: Option<String>,
    /// 图片的 MD5，与解析过的消息的 `imageMd5` 相同。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_md5: Option<String>,
    /// 图片的 `.dat` 文件名，与解析过的消息的 `imageDatName` 相同。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_dat_name: Option<String>,
    /// 视频的 MD5，视频文件以它命名，与解析过的消息的 `videoMd5` 相同。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_md5: Option<String>,
    /// 表情的 MD5。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_md5: Option<String>,
    /// 表情在微信服务器上的地址。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_cdn_url: Option<String>,
    /// app 消息的 `<type>`，与解析过的消息的 `xmlType` 相同。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xml_type: Option<String>,
    /// 文件名。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// 文件的扩展名，不带点。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_ext: Option<String>,
    /// 文件的 MD5。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_md5: Option<String>,
    /// 文件大小，单位为字节。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size: Option<i64>,
    /// 位置的纬度。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_lat: Option<f64>,
    /// 位置的经度。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_lng: Option<f64>,
    /// 位置的地点名称（`poiname`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_poiname: Option<String>,
    /// 位置的标签（`label`），即微信显示的地址。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_label: Option<String>,
    /// app 消息的种类（`link`、`file`、`music` 等），与导出的种类相同；不是 app 消息的消息（包括图片、语音、视频、
    /// 表情和位置）省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_msg_kind: Option<String>,
}

impl MessageView {
    /// 把 `m` 写成查询命令列出的样子；`my_wxid` 是账号的 wxid，文字中与账号有关的部分（如转账的方向）按它来写。
    pub fn of(m: &ExportMsg, my_wxid: &str) -> Self {
        let text = parse_message_content(
            &m.content,
            m.local_type,
            Some(my_wxid),
            Some(&m.sender_username),
            m.emoji_caption.as_deref(),
        );
        let app_msg_kind = app_message_kind(&m.content, m.local_type).map(String::from);
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

/// 微信保存的一条消息（不带 `parsed` 的 `chat messages`，`chat message` 的 `row`）。
///
/// 各列按类型读出，内容已解压并解码成文字，发送者已换成用户名（#257、#492）。
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageRow {
    /// 本地 ID，只在所在的消息库（`shard`）中唯一，不同的消息库之间可能相同。
    pub local_id: i64,
    /// 服务器 ID 的十进制字符串（可能超过 2^53）；消息没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    /// 消息类型，app 消息的子类型在高 32 位（`(sub << 32) | 49`）。
    pub local_type: i64,
    /// 在聊天顺序中的位置（毫秒时间加一个计数）；微信没有保存时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_seq: Option<i64>,
    /// 发送时间；微信没有保存时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time: Option<i64>,
    /// 微信的 `status` 列；微信没有保存时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<i64>,
    /// 发送者的用户名；微信的用户名表中没有对应的行时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    /// 是否为账号所有者发的。
    pub is_send: bool,
    /// 文字；不是文字的消息为它的 XML。已解压并解码。
    pub content: String,
    /// `msgsource` 的 XML（@ 了谁、群的人数等）；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// 微信的 `packed_info_data` 列（protobuf），写成小写十六进制；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packed_info: Option<String>,
    /// 所在消息库的分片号：`message_<n>.db` 为 n，`biz_message_<n>.db` 为 16777216 + n。
    pub shard: u32,
    /// 消息在会话中的位置（游标）：作为 `before` 或 `after` 传入，就能读取它前后的消息。
    /// 它是不透明的，只有服务端解读它。
    #[serde(rename = "_cursor")]
    pub cursor: String,
}

impl From<weflow_native::msg_row::MsgRow> for MessageRow {
    fn from(row: weflow_native::msg_row::MsgRow) -> Self {
        let cursor = row.pos().token();
        Self {
            local_id: row.local_id,
            server_id: row.server_id.map(|id| id.to_string()),
            local_type: row.local_type,
            sort_seq: row.sort_seq,
            create_time: row.create_time,
            status: row.status,
            sender: row.sender,
            is_send: row.is_send,
            content: row.content,
            source: row.source,
            packed_info: row.packed_info.as_deref().map(weflow_native::db::hex),
            shard: row.shard,
            cursor,
        }
    }
}

/// 不带 `parsed` 的 `chat search` 搜到的一条：微信保存的消息，以及它所在的会话。
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SearchHit {
    /// 消息本身，它的字段放在顶层。
    #[serde(flatten)]
    pub row: MessageRow,
    /// 消息所在的会话。
    #[serde(rename = "_session_id")]
    pub session_id: String,
}

impl From<weflow_native::media::SessionRow> for SearchHit {
    fn from(hit: weflow_native::media::SessionRow) -> Self {
        Self {
            row: hit.row.into(),
            session_id: hit.session_id,
        }
    }
}

/// 不带 `parsed` 的 `chat message`：微信保存的这条消息，以及查询命令列出它时的样子。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MessageLookup {
    /// 微信保存的这条消息。
    pub row: MessageRow,
    /// 查询命令列出它时的样子。
    pub message: Option<MessageView>,
}

/// `chat resources` 中一条消息的资源种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    /// 图片（类型 3）。
    Image,
    /// 视频（类型 43）。
    Video,
    /// 语音（类型 34）。
    Voice,
    /// 文件消息。
    File,
}

/// `chat resources` 中的一条消息，从新到旧排列。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    /// 消息本身，它的字段放在顶层。
    #[serde(flatten)]
    pub message: MessageView,
    /// 消息在这个列表中（而不是在它的会话中）的位置：作为 `before` 传入时从它之后接着读。
    #[serde(rename = "_cursor")]
    pub cursor: String,
    /// 会话的显示名，与会话列表相同。
    pub session_display_name: String,
    /// 会话的用户名。
    pub session_id: String,
    /// 消息中的资源种类。
    pub resource_type: ResourceKind,
}

/// `chat resources` 的一页。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePage {
    /// 这一页的消息。
    pub items: Vec<Resource>,
    /// 这一页之后是否还有消息。
    pub has_more: bool,
    /// 最后一项的 `_cursor`，作为 `before` 传入就能读下一页；最后一页为 `null`。
    pub next_cursor: Option<String>,
}

/// 会话是否折叠、免打扰和置顶（`chat statuses` 为请求的每个用户名回答一个）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatStatus {
    /// 是否折叠进了微信的「折叠的聊天」。
    pub is_folded: bool,
    /// 是否设了消息免打扰。
    pub is_muted: bool,
    /// 是否在微信的会话列表中置顶（#885）。
    pub is_pinned: bool,
}

/// 每类联系人各有多少（`chat tab-counts`），按联系人列表的各个标签页计数，不含账号自己。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TabCounts {
    /// 好友。
    #[serde(default)]
    pub private: i64,
    /// 群聊。
    #[serde(default)]
    pub group: i64,
    /// 公众号。
    #[serde(default)]
    pub official: i64,
    /// 曾经的好友。
    #[serde(default)]
    pub former_friend: i64,
}

/// 会话有消息的日期（`chat dates`），写成服务端所在时区的 `YYYY-MM-DD`，从早到晚。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionDates {
    /// 会话的用户名。
    pub session_id: String,
    /// 这些日期。
    pub dates: Vec<String>,
}

/// 会话在每个有消息的日期的消息数（`chat date-counts`），以 `YYYY-MM-DD` 为键。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionDateCounts {
    /// 会话的用户名。
    pub session_id: String,
    /// 每天的消息数，日期按服务端所在的时区。
    pub counts: BTreeMap<String, i64>,
}

/// 会话的对方是谁、有多少条消息（`chat detail --fast`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionFacts {
    /// 会话的用户名。
    pub wxid: String,
    /// 备注，没有时依次用昵称、微信号和用户名本身。
    pub display_name: String,
    /// 账号所有者给对方的备注。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
    /// 对方自己设的昵称，群聊为群名。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nick_name: Option<String>,
    /// 微信号。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// 头像的地址（`http` 地址或 `data:` URL）；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// 会话的消息数。
    pub message_count: i64,
}

/// 会话的详情（`chat detail`）：对方是谁，以及消息存放在哪里。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SessionDetail {
    /// 会话的对方是谁，字段放在顶层。
    #[serde(flatten)]
    pub facts: SessionFacts,
    /// 消息存放在哪里，字段放在顶层；读不出消息表时各项为空。
    #[serde(flatten)]
    pub tables: SessionTables,
}

/// 会话的消息存放在哪里（`chat detail --extra`，以及不带 `fast` 的 `chat detail`）。
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionTables {
    /// 第一条消息的时间；消息表中读不出时为 `null`。
    pub first_message_time: Option<i64>,
    /// 最后一条消息的时间；消息表中读不出时为 `null`。
    pub latest_message_time: Option<i64>,
    /// 存放这个会话的消息的表。
    pub message_tables: Vec<MessageTable>,
}

/// 微信中存放一个会话的消息的一张表。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageTable {
    /// 数据库的文件名，如 `message_0.db`。
    pub db_name: String,
    /// 表名，`Msg_<用户名的 MD5>`。
    pub table_name: String,
    /// 表中的消息数。
    pub count: i64,
}

/// `chat group-hint` 保存的自己在这个群里的消息数；没有保存过时各项都省略。带 `set` 时只有保存的时间
/// `updatedAt`。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupHint {
    /// 账号所有者在这个群里的消息数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
    /// 保存的时间，Unix 毫秒。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// 记录从哪里读到，总是 `disk`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// 群里的转账消息为双方显示的名字（`chat transfer-names`）。
///
/// 名字依次取群昵称、联系人的备注、昵称和微信号，都没有时为用户名；账号自己没有群昵称时为服务端语言的「我」
/// （英文为 `Me`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TransferNames {
    /// 付款方的名字。
    pub payer_name: String,
    /// 收款方的名字。
    pub receiver_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_server_id_past_2_53_keeps_every_digit_and_only_the_fields_of_the_contract_are_written() {
        let row = weflow_native::msg_row::MsgRow {
            local_id: 5,
            server_id: Some(9_007_199_254_740_993),
            create_time: Some(1700000000),
            local_type: 49,
            sort_seq: Some(1700000000000),
            content: "<msg><appmsg><type>6</type><title>a.pdf</title></appmsg></msg>".into(),
            ..Default::default()
        };
        let message = crate::chat_msg::map_msg_rows(vec![row], "wxid_bob", "wxid_me").remove(0);
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
        // 只写出消息中有的字段
        assert_eq!(doc["fileName"], "a.pdf");
        assert!(doc.get("linkUrl").is_none() && doc.get("quotedContent").is_none());
        // 可为空的字段总是写出
        assert!(doc["isSend"].is_null() || doc["isSend"].is_number());
        assert!(doc.as_object().unwrap().contains_key("senderUsername"));
    }
}
