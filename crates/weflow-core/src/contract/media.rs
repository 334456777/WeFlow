//! Images, voices, videos and stickers: their lists and what the media commands decode or write.

use schemars::JsonSchema;
use serde::Serialize;

use super::chat::MessageView;

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

/// The images of a conversation (`chat images`), the newest first, each once.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ChatImages {
    pub images: Vec<ImageRef>,
}

/// What names an image message's file.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageRef {
    pub image_md5: Option<String>,
    pub image_dat_name: Option<String>,
    /// Null when the message has no time.
    pub create_time: Option<i64>,
}

/// The voice messages of a conversation (`chat voice-messages`), the newest first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct VoiceMessages {
    pub messages: Vec<MessageView>,
}

/// A page of image and video messages across conversations (`chat media-stream`), the newest first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MediaStream {
    pub items: Vec<MediaStreamItem>,
    pub has_more: bool,
    /// The `offset` of the next page.
    pub next_offset: usize,
}

/// A message of `chat media-stream`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MediaStreamItem {
    #[serde(flatten)]
    pub message: MessageView,
    pub session_id: String,
    pub session_display_name: String,
    /// `image` or `video`.
    pub media_type: String,
}

/// A voice already decoded (`chat voice-cache`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCache {
    /// Always true.
    pub success: bool,
    pub has_cache: bool,
    /// The WAV, in base64; left out without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// The voices `chat voice-preload` decoded ahead.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct VoicesPrepared {
    /// Always true.
    pub success: bool,
    pub prepared: usize,
}

/// The stickers `chat emoji` downloaded.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct EmojisDownloaded {
    /// The sticker messages found.
    pub found: usize,
    /// The ones downloaded now, not already there.
    pub downloaded: usize,
    /// Left out when none was found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<EmojiFile>>,
}

/// A sticker `chat emoji` has on disk.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct EmojiFile {
    pub md5: String,
    /// Already there before.
    pub cached: bool,
    pub path: String,
    /// Bytes downloaded; left out for one already there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<usize>,
}

/// A media file a command wrote (`chat image-data`, `video-data`, `voice-data` with `out`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MediaWritten {
    /// Always true.
    pub success: bool,
    pub path: String,
    pub bytes: u64,
}

/// A voice as WAV in base64 (`chat voice-data` without `out`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct VoiceData {
    /// Always true.
    pub success: bool,
    pub data: String,
}

/// What `image auto-download status` answers: the hook is not part of WeFlow.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutoDownloadStatus {
    /// Always false.
    pub is_hooked: bool,
    /// Always null.
    pub pid: Option<u32>,
    /// Whether this computer can download WeChat's images on its own.
    pub supported: bool,
}

/// What `image auto-download start` answers once stopped (Ctrl+C).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct AutoDownloadStopped {
    /// Always true.
    pub stopped: bool,
}

/// The MD5 a video message names (`video parse-md5`); null when its XML has none.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct VideoMd5 {
    pub md5: Option<String>,
}
