//! 图片、语音、视频和表情：它们的列表，以及媒体命令解码或写出的内容。

use schemars::JsonSchema;
use serde::Serialize;

use super::chat::MessageView;

/// 消息中的图片有哪些版本在本机上（`chat image-info`），以及消息给出的大小。
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    /// 是否有缩略图，即聊天窗口中显示的那张。
    pub thumb: bool,
    /// 是否有点开时显示的图片。
    pub normal: bool,
    /// 是否有原图：在微信中点「查看原图」下载后才有。
    pub hd: bool,
    /// 图片的字节数：消息中写的 `length`，没有时为本机文件的大小；都没有时为 `null`。
    pub normal_bytes: Option<u64>,
    /// 原图的字节数：消息中写的 `hdlength`（原图还没下载时也有），没有时为本机原图文件的大小；都没有时为 `null`。
    pub hd_bytes: Option<u64>,
}

/// 会话中的图片（`chat images`），从新到旧，每张只列一次。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ChatImages {
    /// 这些图片消息。
    pub images: Vec<ImageRef>,
}

/// 用来找到图片消息的文件的标识。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageRef {
    /// 图片的 MD5，与消息的 `imageMd5` 相同；消息中没有时为 `null`。
    pub image_md5: Option<String>,
    /// 图片的 `.dat` 文件名，与消息的 `imageDatName` 相同；消息中没有时为 `null`。
    pub image_dat_name: Option<String>,
    /// 发送时间；消息没有时间时为 `null`。
    pub create_time: Option<i64>,
}

/// 会话中的语音消息（`chat voice-messages`），从新到旧，每条只列一次。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct VoiceMessages {
    /// 这些语音消息。
    pub messages: Vec<MessageView>,
}

/// 各会话的图片和视频消息中的一页（`chat media-stream`），从新到旧。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MediaStream {
    /// 这一页的消息。
    pub items: Vec<MediaStreamItem>,
    /// 这一页之后是否还有消息。
    pub has_more: bool,
    /// 下一页的 `offset`。
    pub next_offset: usize,
}

/// `chat media-stream` 中的一条消息。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MediaStreamItem {
    /// 消息本身，它的字段放在顶层。
    #[serde(flatten)]
    pub message: MessageView,
    /// 会话的用户名。
    pub session_id: String,
    /// 会话的显示名，与会话列表相同。
    pub session_display_name: String,
    /// `image` 或 `video`。
    pub media_type: String,
}

/// 缓存中已解码的语音（`chat voice-cache`）；这个命令不做解码。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCache {
    /// 缓存中是否有这条语音解码后的 WAV。
    pub has_cache: bool,
    /// base64 编码的 WAV；没有时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// `chat voice-preload` 预先解码的语音。
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
pub struct VoicesPrepared {
    /// 这次解码的和缓存中已有的语音数。
    pub prepared: usize,
}

/// `chat emoji` 下载的表情：会话最近 500 条消息中的表情。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct EmojisDownloaded {
    /// 找到的表情数。
    pub found: usize,
    /// 这次下载的表情数，不含原来就有的。
    pub downloaded: usize,
    /// 请求或写入失败的表情数；它们不在 `files` 中。
    pub failed: usize,
    /// 磁盘上的表情，即原来就有的和这次下载的，按消息的顺序；一个都没找到时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<EmojiFile>>,
}

/// `chat emoji` 在磁盘上的一个表情。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct EmojiFile {
    /// 表情的 MD5。
    pub md5: String,
    /// 下载之前是否就已经有了。
    pub cached: bool,
    /// 文件的路径。
    pub path: String,
    /// 下载的字节数；原来就有的省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<usize>,
}

/// 命令写出的一个媒体文件（带 `out` 的 `chat image-data`、`video-data`、`voice-data`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MediaWritten {
    /// 写出的文件。
    pub path: String,
    /// 文件的字节数。
    pub bytes: u64,
}

/// base64 编码的 WAV 语音（不带 `out` 的 `chat voice-data`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct VoiceData {
    /// base64 编码的 WAV。
    pub data: String,
}

/// `image auto-download status` 的回答：原图自动下载的钩子是否在微信中。
///
/// `weflow serve` 为它自己运行的钩子回答 `--json` 文档（在 Windows x64 上，`serve_image_auto_download` 不为
/// `false` 时运行）；单独运行的命令，以及经 `serve` 以作业或带 `std` 运行的命令，回答的是自己的进程，而它不运行钩子。
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutoDownloadStatus {
    /// 钩子现在是否在微信中。
    pub is_hooked: bool,
    /// 挂钩的微信进程的 ID；没有挂钩时为 `null`。
    pub pid: Option<u32>,
    /// 这台计算机是否支持原图自动下载（只有 Windows x64 支持）。
    pub supported: bool,
}

/// `image auto-download start` 收到停止请求（如 Ctrl+C）、撤除钩子后的回答。
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
pub struct AutoDownloadStopped {
    /// 总是 `true`。
    pub stopped: bool,
}

/// 视频消息中写的 MD5（`video parse-md5`）；XML 中没有时为 `null`。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct VideoMd5 {
    /// 视频的 MD5，与消息的 `videoMd5` 相同。
    pub md5: Option<String>,
}
