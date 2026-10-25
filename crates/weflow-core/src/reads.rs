//! The reads the CLI and the command API share (#397): one typed request each, run on a [`ServiceHub`]. The CLI builds
//! them from its arguments and the command API from the parameters of a request, so neither depends on how the other
//! parses; both answer what [`Read::run`] returns, serialized at their own edge.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::services::{ImageVariant, ResourceQuery, ServiceHub, SnsTimelineQuery};

/// A page of a conversation's messages (`chat messages`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessagePage {
    pub session_id: String,
    pub limit: i32,
    pub offset: i32,
    /// The `_cursor` of the message the page ends above.
    pub before: Option<String>,
    /// The `_cursor` of the message the page starts below.
    pub after: Option<String>,
    /// Unix seconds: the page from where this time falls in the chat's order.
    pub since: Option<i64>,
    pub parsed: bool,
}

/// A read that answers a JSON value: the reads of the command API's in-process routes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// `chat sessions`; a `limit` of 0 is all of them.
    ChatSessions {
        limit: usize,
    },
    ChatMessages(MessagePage),
    ChatLatest {
        session_id: String,
        limit: i32,
    },
    /// `chat message`: by exactly one of `local_id` and `server_id`.
    ChatMessage {
        session_id: String,
        local_id: Option<i64>,
        server_id: Option<String>,
        create_time: Option<i64>,
        parsed: bool,
    },
    ChatContact {
        username: String,
    },
    ChatContacts {
        parsed: bool,
    },
    ChatCounts {
        sessions: Vec<String>,
    },
    ChatSearch {
        keyword: String,
        session_id: Option<String>,
        limit: i32,
        offset: i32,
        start: i32,
        end: i32,
        parsed: bool,
    },
    ChatDates {
        session_id: String,
    },
    ChatDateCounts {
        session_id: String,
    },
    /// `chat detail --fast`: the contact and the message count.
    ChatDetailFast {
        session_id: String,
    },
    /// `chat resources`; `start` and `end` are local dates (`YYYY-MM-DD`), both included.
    ChatResources {
        session: Option<String>,
        types: Vec<String>,
        start: Option<String>,
        end: Option<String>,
        limit: usize,
        before: Option<String>,
    },
    ChatImageInfo {
        session_id: String,
        msg_id: String,
        create_time: Option<i64>,
    },
    GroupMembers {
        chatroom_id: String,
        counts: bool,
        refresh: bool,
    },
    /// `sns timeline` without its media, which are downloaded.
    SnsTimeline {
        limit: i32,
        offset: i32,
        users: Vec<String>,
        keyword: Option<String>,
        start: i64,
        end: i64,
    },
    SnsUsers,
    SnsStats {
        fast: bool,
    },
    SnsPostCounts {
        user: Option<String>,
        prefer_cache: bool,
    },
}

impl Read {
    /// What the read answers: the `data` of its `--json` document.
    pub fn run(&self, hub: &ServiceHub) -> AppResult<Value> {
        match self {
            Read::ChatSessions { limit } => {
                let mut sessions = hub.chat_sessions_list()?;
                if *limit > 0 {
                    sessions.truncate(*limit);
                }
                Ok(Value::Array(sessions))
            }
            Read::ChatMessages(page) => {
                let found = match page.since {
                    Some(seconds) => {
                        hub.message_page_since(&page.session_id, page.limit, seconds)?
                    }
                    None => hub.message_page(
                        &page.session_id,
                        page.limit,
                        page.offset,
                        page.before.as_deref(),
                        page.after.as_deref(),
                    )?,
                };
                Ok(if page.parsed {
                    hub.parsed_message_page(found)
                } else {
                    found
                })
            }
            Read::ChatLatest { session_id, limit } => {
                hub.message_page(session_id, *limit, 0, None, None)
            }
            Read::ChatMessage {
                session_id,
                local_id,
                server_id,
                create_time,
                parsed,
            } => {
                let found = match (local_id, server_id) {
                    (Some(id), None) => hub.chat_message_by_id(session_id, *id, *create_time),
                    (None, Some(svr)) => hub.chat_message_by_server_id(session_id, svr),
                    _ => Err(AppError::usage(
                        "pass exactly one of --local-id or --server-id",
                    )),
                }?;
                Ok(if *parsed {
                    hub.parsed_lookup(found)
                } else {
                    found
                })
            }
            Read::ChatContact { username } => hub.contact(username),
            Read::ChatContacts { parsed: false } => hub.contacts(),
            Read::ChatContacts { parsed: true } => Ok(Value::Array(hub.chat_contacts_parsed()?)),
            Read::ChatCounts { sessions } => hub.chat_counts(sessions),
            Read::ChatSearch {
                keyword,
                session_id,
                limit,
                offset,
                start,
                end,
                parsed,
            } => {
                let hits = hub.search(
                    keyword,
                    session_id.as_deref(),
                    *limit,
                    *offset,
                    *start,
                    *end,
                )?;
                Ok(if *parsed {
                    hub.parsed_message_page(hits)
                } else {
                    hits
                })
            }
            Read::ChatDates { session_id } => hub.chat_dates(session_id),
            Read::ChatDateCounts { session_id } => hub.chat_date_counts(session_id),
            Read::ChatDetailFast { session_id } => hub.chat_detail_fast(session_id),
            Read::ChatResources {
                session,
                types,
                start,
                end,
                limit,
                before,
            } => {
                let (b, e) = crate::dates::date_range_args(start.as_deref(), end.as_deref())?;
                hub.chat_resources(&ResourceQuery {
                    session_id: session.clone(),
                    types: types.clone(),
                    begin: b.unwrap_or(0),
                    end: e.map(|x| x - 1).unwrap_or(0),
                    limit: *limit,
                    before: before.clone(),
                })
            }
            Read::ChatImageInfo {
                session_id,
                msg_id,
                create_time,
            } => hub.image_info_for_message(session_id, msg_id, *create_time),
            Read::GroupMembers {
                chatroom_id,
                counts,
                refresh,
            } => {
                let (members, from_cache, updated_at) =
                    hub.group_members_panel(chatroom_id, *refresh, *counts)?;
                Ok(
                    json!({ "chatroomId": chatroom_id, "count": members.len(), "fromCache": from_cache, "updatedAt": updated_at, "members": members }),
                )
            }
            Read::SnsTimeline {
                limit,
                offset,
                users,
                keyword,
                start,
                end,
            } => {
                let posts = hub.sns_timeline_query(&SnsTimelineQuery {
                    limit: *limit,
                    offset: *offset,
                    usernames: users.clone(),
                    keyword: keyword.clone(),
                    start: *start,
                    end: *end,
                })?;
                Ok(json!({ "timeline": posts }))
            }
            Read::SnsUsers => Ok(json!({ "usernames": hub.sns_usernames_list()? })),
            Read::SnsStats { fast } => hub.sns_export_stats(*fast),
            Read::SnsPostCounts { user, prefer_cache } => match user {
                Some(u) => hub.sns_user_post_stats(u),
                None => Ok(json!(hub.sns_user_post_counts(*prefer_cache)?)),
            },
        }
    }
}

/// The image file `chat image-data` asks for: `--thumb`, or `--variant` (`best`, `thumb`, `normal`, `hd`).
pub fn image_variant(thumb: bool, variant: &str) -> ImageVariant {
    match (thumb, variant) {
        (true, _) | (_, "thumb") => ImageVariant::Thumb,
        (_, "normal") => ImageVariant::Normal,
        (_, "hd") => ImageVariant::Hd,
        _ => ImageVariant::Best,
    }
}

/// The file of a message the command API shows a page as it is (`<img>`, `<audio>`, `<video>`), and that
/// `chat image-data`, `voice-data` and `video-data` write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaRead {
    ChatImage {
        session_id: String,
        msg_id: String,
        create_time: Option<i64>,
        variant: ImageVariant,
    },
    /// Decoded to WAV.
    ChatVoice {
        session_id: String,
        msg_id: String,
        create_time: Option<i64>,
        server_id: Option<String>,
        sender: Option<String>,
    },
    /// The video as WeChat keeps it, or with `thumb` its thumbnail, else its cover.
    ChatVideo {
        session_id: String,
        msg_id: String,
        create_time: Option<i64>,
        thumb: bool,
    },
}

/// A media file and its content type.
#[derive(Debug)]
pub struct MediaFile {
    pub body: MediaBody,
    pub mime: &'static str,
}

/// What a media file is made of: bytes decoded in memory, or a file on disk, read as it is sent (a video).
#[derive(Debug)]
pub enum MediaBody {
    Bytes(Vec<u8>),
    File(PathBuf),
}

/// The content type of an image, from its first bytes.
pub fn image_mime(bytes: &[u8]) -> &'static str {
    let ext = crate::decrypt::detect_image_extension(bytes);
    mime_guess::from_ext(ext.trim_start_matches('.'))
        .first_raw()
        .filter(|m| m.starts_with("image/"))
        .unwrap_or("application/octet-stream")
}

impl MediaRead {
    /// The file, read on a blocking thread: decrypting an image or decoding a voice takes a few milliseconds.
    pub async fn fetch(self, hub: ServiceHub) -> AppResult<MediaFile> {
        tokio::task::spawn_blocking(move || self.fetch_blocking(&hub))
            .await
            .unwrap_or_else(|err| Err(AppError::runtime(format!("the command failed: {err}"))))
    }

    fn fetch_blocking(&self, hub: &ServiceHub) -> AppResult<MediaFile> {
        match self {
            MediaRead::ChatImage {
                session_id,
                msg_id,
                create_time,
                variant,
            } => {
                let bytes =
                    hub.image_variant_for_message(session_id, msg_id, *create_time, *variant)?;
                Ok(MediaFile {
                    mime: image_mime(&bytes),
                    body: MediaBody::Bytes(bytes),
                })
            }
            MediaRead::ChatVoice {
                session_id,
                msg_id,
                create_time,
                server_id,
                sender,
            } => Ok(MediaFile {
                body: MediaBody::Bytes(hub.voice_data(
                    session_id,
                    msg_id,
                    *create_time,
                    server_id.as_deref(),
                    sender.as_deref(),
                )?),
                mime: "audio/wav",
            }),
            MediaRead::ChatVideo {
                session_id,
                msg_id,
                create_time,
                thumb,
            } => {
                let files = hub.video_files_for_message(session_id, msg_id, *create_time)?;
                let path = video_file(&files, *thumb)?;
                if !*thumb {
                    return Ok(MediaFile {
                        body: MediaBody::File(path.to_path_buf()),
                        mime: "video/mp4",
                    });
                }
                // a poster is small: read, to tell its type
                let bytes = std::fs::read(path).map_err(|e| {
                    AppError::runtime(format!("failed to read {}: {e}", path.display()))
                })?;
                Ok(MediaFile {
                    mime: image_mime(&bytes),
                    body: MediaBody::Bytes(bytes),
                })
            }
        }
    }
}

/// The file `chat video-data` answers: the video, or with `thumb` its thumbnail, else its cover.
fn video_file(files: &crate::video::VideoFiles, thumb: bool) -> AppResult<&std::path::Path> {
    if !thumb {
        return Ok(&files.video);
    }
    [&files.thumb, &files.cover]
        .into_iter()
        .flatten()
        .find(|p| p.exists())
        .map(PathBuf::as_path)
        .ok_or_else(|| {
            AppError::runtime("the video has no thumbnail or cover")
                .with_details(json!({ "failureKind": "not_found" }))
        })
}
