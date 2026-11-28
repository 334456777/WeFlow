//! 导出写出的内容，以及导出窗口显示的统计。

use std::collections::BTreeMap;
use std::path::PathBuf;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::Serialize;

/// 一个会话按一种格式写出的消息（`export messages`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MessagesExported {
    /// 写出的文件；打包成 zip 时为它在 zip 中的名字。
    pub out: PathBuf,
    /// 写出的消息数。
    pub count: usize,
    /// 会话的用户名。
    pub session: String,
    /// 写出的格式：`json`、`html`、`txt` 等。
    pub format: String,
    /// 带 `media` 时复制到它旁边的媒体；不带时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaCopied>,
}

/// 一个会话写成几种格式（`export messages` 给了几个 `format`），或者打包成 zip（`zip`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum MessagesBundle {
    /// 每种格式写成一个文件，放在同一个文件夹中。
    Folder(MessagesFolder),
    /// 各格式打包成一个 zip。
    Zip(MessagesZip),
}

/// 一个会话的几种格式，每种写成一个文件，放在同一个文件夹中。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MessagesFolder {
    /// 这个文件夹。
    pub out: PathBuf,
    /// 写出的格式，按请求的顺序。
    pub formats: Vec<String>,
    /// 每种格式的文件，顺序同上。
    pub files: Vec<MessagesExported>,
}

/// 一个会话的各格式和它们的媒体打包成的 zip。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MessagesZip {
    /// 这个 zip。
    pub out: PathBuf,
    /// 打包的格式，按请求的顺序。
    pub formats: Vec<String>,
    /// 每种格式的文件，名字为它在 zip 中的名字。
    pub files: Vec<MessagesExported>,
    /// 一起打包的媒体，`dir` 为 zip 中的文件夹；不带 `media` 时为 `null`。
    pub media: Option<MediaCopied>,
}

/// 消息的导出复制到 `media/` 文件夹中的媒体。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MediaCopied {
    /// `media` 选中的种类的消息数。
    pub requested: usize,
    /// 其中复制了文件的消息数。
    pub exported: usize,
    /// 其中文件不在磁盘上，或者读不了、转换不了的消息数。
    pub missing: usize,
    /// 媒体所在的文件夹，即导出文件旁边的 `media/<导出文件名>`；打包成 zip 时为它在 zip 中的路径。
    pub dir: PathBuf,
}

/// `export media` 复制了什么。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MediaExport {
    /// 写出的文件数，与 `files` 的项数相同。
    pub exported: usize,
    /// 找到的媒体消息数。
    pub found: usize,
    /// 文件写不出来的媒体消息数，即 `found` 减去 `exported`。
    pub missing: usize,
    /// 按种类分的 `missing`，键为 `image`、`voice`、`video`、`emoji`、`file`；没有缺失的种类不列出。
    pub missing_by_kind: BTreeMap<String, usize>,
    /// 导出的图片中只有缩略图的张数。
    pub thumb_only: usize,
    /// `missing` 和 `thumbOnly` 的含义，英文。
    pub note: String,
    /// 查看过的会话数。
    pub sessions: usize,
    /// 写入的文件夹；每个会话的文件在以会话命名的子文件夹中。
    pub out: PathBuf,
    /// 写出的文件。
    pub files: Vec<MediaFileExported>,
}

/// `export media` 写出的一个媒体文件。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MediaFileExported {
    /// 会话的用户名。
    pub session: String,
    /// `image`、`voice`、`video`、`emoji` 或 `file`。
    pub kind: String,
    /// 文件名。
    pub file_name: String,
    /// 文件的路径。
    pub path: String,
    /// 消息的本地 ID（`localId`）。
    pub local_id: i64,
    /// 消息的发送时间。
    pub create_time: i64,
    /// 图片才有：文件是否只是缩略图。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_thumb: Option<bool>,
}

/// 导出写出的文件（`export sessions`、`contacts`、`footprint`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ExportWritten {
    /// 写出的文件；带 `zip` 时为 zip 文件。
    pub out: PathBuf,
    /// 写出的格式：`json`、`csv`、`excel` 等。
    pub format: String,
}

/// `group export-members` 或 `export-member-messages` 写出了多少。
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
pub struct GroupExported {
    /// 写出的成员数或消息数。
    pub count: usize,
}

/// `sns export` 写出了什么。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnsExported {
    /// 写出的文件；没有符合条件的动态、什么都没写时为空。
    pub file_path: PathBuf,
    /// 写出的动态数。
    pub post_count: usize,
    /// 保存在它旁边 `media/` 文件夹中的媒体文件数。
    pub media_count: usize,
}

/// 导出窗口为请求的每个会话显示的统计（`chat export-stats`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ExportStats {
    /// 以会话的用户名为键，按请求的顺序。
    pub data: IndexMap<String, SessionExportStats>,
}

/// 一个会话在请求的时间范围内各类型的消息数，以及它的关系。
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionExportStats {
    /// 全部消息数。
    pub total_messages: i64,
    /// 语音消息数（类型 34）。
    pub voice_messages: i64,
    /// 图片数（类型 3）。
    pub image_messages: i64,
    /// 视频数（类型 43）。
    pub video_messages: i64,
    /// 表情数（类型 47）。
    pub emoji_messages: i64,
    /// 转账数（子类型为 2000 的 app 消息）。
    pub transfer_messages: i64,
    /// 红包数（子类型为 2001 的 app 消息）。
    pub red_packet_messages: i64,
    /// 语音和视频通话数（类型 50）。
    pub call_messages: i64,
    /// 第一条消息的时间；没有消息时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_timestamp: Option<i64>,
    /// 最后一条消息的时间；没有消息时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_timestamp: Option<i64>,
    /// 群聊才有：账号所有者在群里的消息数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_my_messages: Option<i64>,
    /// 群聊才有：发过言的成员数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_active_speakers: Option<i64>,
    /// 群聊才有：群的成员数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_member_count: Option<i64>,
    /// 群聊才有，带 `no-relations` 时省略：群成员中账号所有者的好友数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_mutual_friends: Option<i64>,
    /// 私聊才有，带 `no-relations` 时省略：双方共同所在的群数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_mutual_groups: Option<i64>,
}
