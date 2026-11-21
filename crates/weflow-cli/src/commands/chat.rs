//! `weflow chat`: conversations, contacts, messages and the media of messages.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::reads::{media_of, write_media};

use serde_json::{json, Value};
use weflow_core::dates::date_range_args;
use weflow_core::error::{AppError, AppResult};
use weflow_core::reads::MediaBody;
use weflow_core::services::ServiceHub;

#[derive(Args, Debug)]
pub(crate) struct ChatCommand {
    #[command(subcommand)]
    pub(crate) command: ChatSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ChatSubcommand {
    /// List conversations, newest first
    Sessions {
        /// Maximum number of sessions; 0 means all
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },
    /// A page of a conversation's messages
    Messages(PageArgs),
    /// Search the messages of one conversation or of all of them
    Search {
        /// Text to search for
        keyword: String,
        /// Only search this conversation; default: all conversations
        #[arg(long)]
        session: Option<String>,
        /// Maximum number of results
        #[arg(long, default_value_t = 50)]
        limit: i32,
        /// Number of results to skip
        #[arg(long, default_value_t = 0)]
        offset: i32,
        /// Only messages from this time on, in Unix seconds; 0 means no limit
        #[arg(long, default_value_t = 0)]
        start: i32,
        /// Only messages up to this time, in Unix seconds; 0 means no limit
        #[arg(long, default_value_t = 0)]
        end: i32,
        /// Each hit parsed into type, text, quote, media, link, file and so on, instead of the database row
        #[arg(long)]
        parsed: bool,
    },
    /// List all contacts
    Contacts {
        /// The contact list sorted and classified into friends, groups, official accounts and former friends, with names, labels, region, signature and avatar, instead of the database rows
        #[arg(long)]
        parsed: bool,
    },
    /// Show one contact
    Contact {
        /// Contact username, the wxid
        username: String,
    },
    /// Look up one message by local id or server id
    Message {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        #[arg(long)]
        local_id: Option<i64>,
        /// Server message id
        #[arg(long)]
        server_id: Option<String>,
        /// Create time of the message in seconds, with --local-id: tells apart messages of different message databases that share the local id
        #[arg(long, requires = "local_id")]
        create_time: Option<i64>,
        /// The message alone, parsed as `chat messages --parsed` gives it, with its `_cursor`
        #[arg(long)]
        parsed: bool,
    },
    /// Dates that have messages in a session, as YYYY-MM-DD
    Dates {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
    },
    /// Message count per day for a session
    DateCounts {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
    },
    /// Total message counts for several sessions
    Counts {
        /// Conversation ids
        sessions: Vec<String>,
    },
    /// Folded / muted / pinned state of sessions
    Statuses {
        /// Conversation ids
        sessions: Vec<String>,
    },
    /// Session details: contact info, message count, message tables, and the first and latest time
    Detail {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Only the fast part: contact info and message count
        #[arg(long, conflicts_with = "extra")]
        fast: bool,
        /// Only the extra part: tables and the first and latest time
        #[arg(long)]
        extra: bool,
    },
    /// Contact counts per tab: friends, groups, official accounts and former friends
    TabCounts,
    /// Per-session statistics used by the export page
    ExportStats {
        /// Conversation ids
        sessions: Vec<String>,
        /// Start date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        start: Option<String>,
        /// End date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        end: Option<String>,
        /// Skip mutual group / friend relations
        #[arg(long)]
        no_relations: bool,
    },
    /// Read or set the cached "my message count" of a group chat
    GroupHint {
        /// Group chat id, such as xxx@chatroom
        chatroom_id: String,
        /// Store this value as the "my message count"
        #[arg(long)]
        set: Option<i64>,
    },
    /// List image / video / voice / file messages across sessions
    Resources {
        /// Only this conversation; default: all conversations
        #[arg(long)]
        session: Option<String>,
        /// image, video, voice or file; repeat for several
        #[arg(long = "type")]
        types: Vec<String>,
        /// Start date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        start: Option<String>,
        /// End date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        end: Option<String>,
        /// Maximum number of results
        #[arg(long, default_value_t = 300)]
        limit: usize,
        /// Cursor returned as `nextCursor` by the previous page
        #[arg(long)]
        before: Option<String>,
    },
    /// All image identifiers of a session, md5 and dat name, newest first
    Images {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
    },
    /// All voice messages of a session
    VoiceMessages {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
    },
    /// Image and video messages across sessions, newest first, paged by offset
    MediaStream {
        /// Only this conversation; default: all conversations
        #[arg(long)]
        session: Option<String>,
        /// image, video or all
        #[arg(long = "type", value_name = "TYPE", default_value = "all")]
        media_type: String,
        /// Start date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        start: Option<String>,
        /// End date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        end: Option<String>,
        /// Maximum number of results
        #[arg(long, default_value_t = 200)]
        limit: i32,
        /// Number of results to skip
        #[arg(long, default_value_t = 0)]
        offset: i32,
    },
    /// Resolve payer / receiver display names of a transfer message
    TransferNames {
        /// Group chat id, such as xxx@chatroom
        chatroom_id: String,
        /// Payer username, the wxid
        payer: String,
        /// Receiver username, the wxid
        receiver: String,
    },
    /// Decode one SILK voice message into a 24 kHz WAV file
    VoiceData {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        local_id: String,
        /// Message create time in seconds: tells apart messages of different message databases that share the local id
        #[arg(long)]
        create_time: Option<i64>,
        /// Server message id
        #[arg(long)]
        server_id: Option<String>,
        /// Sender wxid, which matters in group chats
        #[arg(long)]
        sender: Option<String>,
        /// Output WAV path; default: print base64 in the JSON result
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Decrypt the image of one message and write it to a file
    ImageData {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        local_id: String,
        /// Message create time in seconds: tells apart messages of different message databases that share the local id
        #[arg(long)]
        create_time: Option<i64>,
        /// Which file of the image: best is the best on this computer; thumb is WeChat's own thumbnail of a few KB, or the image when there is none; normal is the image as opened; hd is the original WeChat downloads on View original, only once it has
        #[arg(long, value_parser = ["best", "thumb", "normal", "hd"], default_value = "best")]
        variant: String,
        /// Output file
        #[arg(long)]
        out: PathBuf,
    },
    /// Which files of a message's image are on this computer, among the thumbnail, the image and the HD original, and their sizes
    ImageInfo {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        local_id: String,
        /// Message create time in seconds: tells apart messages of different message databases that share the local id
        #[arg(long)]
        create_time: Option<i64>,
    },
    /// Copy the video of one message to a file, the MP4 as WeChat keeps it
    VideoData {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        local_id: String,
        /// Message create time in seconds: tells apart messages of different message databases that share the local id
        #[arg(long)]
        create_time: Option<i64>,
        /// WeChat's own thumbnail of the video, or its cover when there is none, instead of the video
        #[arg(long)]
        thumb: bool,
        /// Output file
        #[arg(long)]
        out: PathBuf,
    },
    /// Check whether a decoded voice WAV is already cached for a message id
    VoiceCache {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        local_id: String,
    },
    /// Decode and cache many voice messages
    VoicePreload {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// JSON array of {localId, createTime, serverId?, senderWxid?}
        messages_json: String,
    },
    /// Download the stickers of a conversation
    Emoji {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Output directory
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Args, Debug)]
pub(crate) struct PageArgs {
    /// Conversation id: the other party's wxid, or xxx@chatroom for a group
    pub(crate) session_id: String,
    /// Maximum number of results
    #[arg(long, default_value_t = 50)]
    pub(crate) limit: i32,
    /// Number of results to skip
    #[arg(long, default_value_t = 0)]
    pub(crate) offset: i32,
    /// Only messages older than this one: the `_cursor` of the last message of the previous page, as fast however far back
    #[arg(long, conflicts_with_all = ["offset", "after"])]
    pub(crate) before: Option<String>,
    /// Only messages newer than this one, given by its `_cursor`: the closest ones, still newest first
    #[arg(long, conflicts_with = "offset")]
    pub(crate) after: Option<String>,
    /// The messages from where this time, in Unix seconds, falls in the chat's order: the earliest ones, still newest first, as when a chat window jumps to a day
    #[arg(long, conflicts_with_all = ["offset", "before", "after"])]
    pub(crate) since: Option<i64>,
    /// Each message parsed into type, text, quote, media, link, file and so on, instead of the database row
    #[arg(long)]
    pub(crate) parsed: bool,
}

pub(crate) async fn handle_chat(command: ChatCommand, hub: &ServiceHub) -> AppResult<Value> {
    use weflow_core::contract::MediaWritten;
    let command = match media_of(command.command) {
        Ok((read, out)) => {
            let file = read.fetch(hub.clone()).await?;
            let Some(path) = out else {
                // only `voice-data` goes without `--out`: the WAV in base64
                use base64::Engine;
                let MediaBody::Bytes(wav) = file.body else {
                    unreachable!("a voice is decoded in memory")
                };
                return Ok(json!(weflow_core::contract::VoiceData {
                    data: base64::engine::general_purpose::STANDARD.encode(&wav),
                }));
            };
            let bytes = write_media(file, &path)?;
            return Ok(json!(MediaWritten {
                path: path.to_string_lossy().into_owned(),
                bytes,
            }));
        }
        Err(command) => command,
    };
    match &command {
        // taken by `media_of` above
        ChatSubcommand::VoiceData { .. }
        | ChatSubcommand::ImageData { .. }
        | ChatSubcommand::VideoData { .. } => unreachable!("a media command"),
        // run as a `Read` by `run`
        ChatSubcommand::Sessions { .. }
        | ChatSubcommand::Messages(_)
        | ChatSubcommand::Search { .. }
        | ChatSubcommand::Contacts { .. }
        | ChatSubcommand::Contact { .. }
        | ChatSubcommand::Message { .. }
        | ChatSubcommand::Dates { .. }
        | ChatSubcommand::DateCounts { .. }
        | ChatSubcommand::Counts { .. }
        | ChatSubcommand::Resources { .. }
        | ChatSubcommand::ImageInfo { .. }
        | ChatSubcommand::Statuses { .. }
        | ChatSubcommand::TabCounts => unreachable!("a read"),
        ChatSubcommand::Detail {
            session_id,
            fast,
            extra,
        } => match (fast, extra) {
            (false, true) => hub.chat_detail_extra(session_id).map(|t| json!(t)),
            _ => hub.chat_detail(session_id).map(|d| json!(d)),
        },
        ChatSubcommand::ExportStats {
            sessions,
            start,
            end,
            no_relations,
        } => {
            let (b, e) = date_range_args(start.as_deref(), end.as_deref())?;
            Ok(json!(hub.chat_export_stats(
                sessions,
                b.unwrap_or(0),
                e.map(|x| x - 1).unwrap_or(0),
                !no_relations,
            )?))
        }
        ChatSubcommand::GroupHint { chatroom_id, set } => Ok(json!(match set {
            Some(count) => hub.set_group_hint(chatroom_id, *count)?,
            None => hub.group_hint(chatroom_id)?,
        })),
        ChatSubcommand::Images { session_id } => Ok(json!(hub.chat_all_images(session_id)?)),
        ChatSubcommand::VoiceMessages { session_id } => Ok(json!(hub.chat_all_voices(session_id)?)),
        ChatSubcommand::MediaStream {
            session,
            media_type,
            start,
            end,
            limit,
            offset,
        } => {
            let (b, e) = date_range_args(start.as_deref(), end.as_deref())?;
            Ok(json!(hub.chat_media_stream(
                session.as_deref(),
                media_type,
                b.unwrap_or(0),
                e.map(|x| x - 1).unwrap_or(0),
                *limit,
                *offset,
            )?))
        }
        ChatSubcommand::TransferNames {
            chatroom_id,
            payer,
            receiver,
        } => Ok(json!(hub.chat_transfer_names(
            chatroom_id,
            payer,
            receiver
        )?)),
        ChatSubcommand::VoiceCache {
            session_id,
            local_id,
        } => Ok(json!(hub.voice_resolve_cache(session_id, local_id))),
        ChatSubcommand::VoicePreload {
            session_id,
            messages_json,
        } => {
            let messages: Vec<Value> = serde_json::from_str(messages_json)
                .map_err(|e| AppError::usage(format!("messages_json must be a JSON array: {e}")))?;
            Ok(json!(hub.voice_preload(session_id, &messages)?))
        }
        ChatSubcommand::Emoji { session_id, out } => {
            Ok(json!(hub.emoji_download(session_id, out).await?))
        }
    }
}
