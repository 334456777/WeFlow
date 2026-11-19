//! What the exports write.

use std::collections::BTreeMap;
use std::path::PathBuf;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::Serialize;

/// A conversation's messages written in one format (`export messages`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MessagesExported {
    /// The file; in a zip of several formats, its name inside the zip.
    pub out: PathBuf,
    /// The messages written.
    pub count: usize,
    /// The conversation's username.
    pub session: String,
    /// The format written: `json`, `html`, `txt`, …
    pub format: String,
    /// The media copied beside it, with `media`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaCopied>,
}

/// One conversation in several formats (`export messages` with several `format`s), or packed (`zip`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum MessagesBundle {
    /// Each format written to its file in one folder.
    Folder(MessagesFolder),
    /// The formats packed into a zip.
    Zip(MessagesZip),
}

/// The formats of one conversation, each written to its file in one folder.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MessagesFolder {
    /// The folder.
    pub out: PathBuf,
    /// The formats written, in the order asked for.
    pub formats: Vec<String>,
    /// Each format's file, in the same order.
    pub files: Vec<MessagesExported>,
}

/// The formats of one conversation packed into a zip, with their media.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MessagesZip {
    /// The zip.
    pub out: PathBuf,
    /// The formats packed, in the order asked for.
    pub formats: Vec<String>,
    /// Each format's file, named as it is inside the zip.
    pub files: Vec<MessagesExported>,
    /// The media packed with them; null without `media`.
    pub media: Option<MediaCopied>,
}

/// The media an export of messages copied to its `media/` folder.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MediaCopied {
    /// The media messages asked for.
    pub requested: usize,
    /// The ones whose file was copied.
    pub exported: usize,
    /// The ones whose file is not on disk, or could not be read or converted.
    pub missing: usize,
    /// The folder they went to, `media/<name of the export>` beside the export.
    pub dir: PathBuf,
}

/// What `export media` copied.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MediaExport {
    /// The files written; as many as `files`.
    pub exported: usize,
    /// The media messages found.
    pub found: usize,
    /// The media messages whose file could not be written: `found` less `exported`.
    pub missing: usize,
    /// The missing ones by kind: `image`, `voice`, `video`, `emoji`.
    pub missing_by_kind: BTreeMap<String, usize>,
    /// The exported images that are only the thumbnail.
    pub thumb_only: usize,
    /// What `missing` and `thumbOnly` mean, in English.
    pub note: String,
    /// The conversations looked through.
    pub sessions: usize,
    /// The folder written to; each conversation's files are in a subfolder named after it.
    pub out: PathBuf,
    /// The files written.
    pub files: Vec<MediaFileExported>,
}

/// A media file `export media` wrote.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MediaFileExported {
    /// The conversation's username.
    pub session: String,
    /// `image`, `voice`, `video` or `emoji`.
    pub kind: String,
    /// The file's name.
    pub file_name: String,
    /// The file.
    pub path: String,
    /// The message's `localId`.
    pub local_id: i64,
    /// When the message was sent.
    pub create_time: i64,
    /// An image's: the file is only its thumbnail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_thumb: Option<bool>,
}

/// The file an export wrote (`export sessions`, `contacts`, `footprint`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ExportWritten {
    /// The file.
    pub out: PathBuf,
    /// The format it was written in: `json`, `csv`, `excel`, …
    pub format: String,
}

/// What `group export-members` or `export-member-messages` wrote.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct GroupExported {
    /// Always true.
    pub success: bool,
    /// The members, or the messages, written.
    pub count: usize,
}

/// What `sns export` wrote.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnsExported {
    /// Always true.
    pub success: bool,
    /// Empty when no post matched, and nothing was written.
    pub file_path: PathBuf,
    /// The posts written.
    pub post_count: usize,
    /// The media files copied beside it.
    pub media_count: usize,
}

/// What the export window shows of each conversation asked for (`chat export-stats`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ExportStats {
    /// By username, in the order asked for.
    pub data: IndexMap<String, SessionExportStats>,
}

/// One conversation's messages by type, within the range asked for, and its relations.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionExportStats {
    /// All the messages.
    pub total_messages: i64,
    /// Voice messages (type 34).
    pub voice_messages: i64,
    /// Images (type 3).
    pub image_messages: i64,
    /// Videos (type 43).
    pub video_messages: i64,
    /// Stickers (type 47).
    pub emoji_messages: i64,
    /// Transfers (app messages of sub-type 2000).
    pub transfer_messages: i64,
    /// Red packets (app messages of sub-type 2001).
    pub red_packet_messages: i64,
    /// Voice and video calls (type 50).
    pub call_messages: i64,
    /// The time of the first message; left out without messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_timestamp: Option<i64>,
    /// The time of the last message; left out without messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_timestamp: Option<i64>,
    /// A group's: my messages in it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_my_messages: Option<i64>,
    /// A group's: the members who said something.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_active_speakers: Option<i64>,
    /// A group's: its members.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_member_count: Option<i64>,
    /// A group's, with relations: its members who are my friends.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_mutual_friends: Option<i64>,
    /// A private chat's, with relations: the groups we are both in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_mutual_groups: Option<i64>,
}
