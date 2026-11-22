//! The reads the CLI and the command API share (#397), one typed request each.
//!
//! Each read runs on a [`ServiceHub`]. The CLI builds them from its arguments and the command API from the
//! parameters of a request, so neither depends on how the other parses; both answer what [`Read::run`] returns,
//! serialized at their own edge.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::config::ConfigStore;
use crate::error::{AppError, AppResult};
use crate::services::{Freshness, ImageVariant, ResourceQuery, ServiceHub, SnsTimelineQuery};

/// A page of a conversation's messages (`chat messages`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessagePage {
    /// The conversation: the other party's wxid, or `…@chatroom` for a group.
    pub session_id: String,
    /// The most messages of the page.
    pub limit: i32,
    /// The messages skipped, newest first, without `before`, `after` or `since`.
    pub offset: i32,
    /// The `_cursor` of the message the page ends above.
    pub before: Option<String>,
    /// The `_cursor` of the message the page starts below.
    pub after: Option<String>,
    /// Unix seconds: the page from where this time falls in the chat's order.
    pub since: Option<i64>,
    /// Answer the messages as the chat window shows them rather than as WeChat keeps them.
    pub parsed: bool,
}

/// A read that answers a JSON value: the reads of the command API's in-process routes.
///
/// A `session_id` is a conversation: the other party's wxid, or `…@chatroom` for a group. A `start` or `end` of type
/// `Option<String>` is a local date (`YYYY-MM-DD`), both included; `parsed` answers messages as the chat window shows
/// them rather than as WeChat keeps them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// `chat sessions`; a `limit` of 0 is all of them.
    ChatSessions {
        /// The most conversations.
        limit: usize,
    },
    /// `chat messages`.
    ChatMessages(MessagePage),
    /// `chat message`: by exactly one of `local_id` and `server_id`.
    ChatMessage {
        /// The conversation.
        session_id: String,
        /// The message's id in its table.
        local_id: Option<i64>,
        /// WeChat's id of the message, in decimal.
        server_id: Option<String>,
        /// With `local_id`: its time, which tells apart messages of different databases with the same id.
        create_time: Option<i64>,
        /// As the chat window shows it.
        parsed: bool,
    },
    /// `chat contact`: one contact's row.
    ChatContact {
        /// The contact's wxid.
        username: String,
    },
    /// `chat contacts`.
    ChatContacts {
        /// Sorted and classified into kinds, with names, labels and avatars, rather than as WeChat keeps them.
        parsed: bool,
    },
    /// `chat counts`: how many messages each conversation has.
    ChatCounts {
        /// The conversations.
        sessions: Vec<String>,
    },
    /// `chat search`: in the conversation `session`, or in all of them.
    ChatSearch {
        /// The text to find.
        keyword: String,
        /// The conversation; all of them when none.
        session: Option<String>,
        /// The most hits.
        limit: i32,
        /// The hits skipped, newest first.
        offset: i32,
        /// Unix seconds: only messages from then on; 0 for no bound.
        start: i32,
        /// Unix seconds: only messages up to then; 0 for no bound.
        end: i32,
        /// Each hit as the chat window shows it.
        parsed: bool,
    },
    /// `chat dates`: the days the conversation has messages on.
    ChatDates {
        /// The conversation.
        session_id: String,
    },
    /// `chat date-counts`: the conversation's messages by day.
    ChatDateCounts {
        /// The conversation.
        session_id: String,
    },
    /// `chat detail --fast`: the contact and the message count.
    ChatDetailFast {
        /// The conversation.
        session_id: String,
    },
    /// `chat resources`; `start` and `end` are local dates (`YYYY-MM-DD`), both included.
    ChatResources {
        /// The conversation; all of them when none.
        session: Option<String>,
        /// `image`, `video`, `voice` and `file`; all of them when empty.
        types: Vec<String>,
        /// The first day.
        start: Option<String>,
        /// The last day.
        end: Option<String>,
        /// The most messages of the page.
        limit: usize,
        /// The `nextCursor` of the page before.
        before: Option<String>,
    },
    /// `chat image-info`: which files of a message's image are on this computer.
    ChatImageInfo {
        /// The conversation.
        session_id: String,
        /// The message's id in its table.
        local_id: String,
        /// Its time, which tells apart messages of different databases with the same id.
        create_time: Option<i64>,
    },
    /// `group members`.
    GroupMembers {
        /// The group, `…@chatroom`.
        chatroom_id: String,
        /// Count each member's messages too.
        counts: bool,
    },
    /// `sns timeline` without its media, which are downloaded.
    SnsTimeline {
        /// The most posts.
        limit: i32,
        /// The posts skipped, newest first.
        offset: i32,
        /// Only the posts of these wxids; all when empty.
        users: Vec<String>,
        /// Only the posts holding this text.
        keyword: Option<String>,
        /// Unix seconds: only posts from then on; 0 for no bound.
        start: i64,
        /// Unix seconds: only posts up to then; 0 for no bound.
        end: i64,
    },
    /// `sns users`: who has posts.
    SnsUsers,
    /// `sns stats`.
    SnsStats {
        /// Answer the counts kept from an earlier read while they are fresh.
        prefer_cache: bool,
    },
    /// `sns post-counts`.
    SnsPostCounts {
        /// Only this wxid's posts.
        user: Option<String>,
        /// Answer the counts kept from an earlier read while they are fresh.
        prefer_cache: bool,
    },
    /// Whether each conversation is folded, muted and pinned.
    ChatStatuses {
        /// The conversations.
        sessions: Vec<String>,
    },
    /// `chat tab-counts`.
    ChatTabCounts,
    /// `group list`.
    GroupList,
    /// `group ranking`; like every `start` and `end` below, local dates (`YYYY-MM-DD`), both included.
    GroupRanking {
        /// The group, `…@chatroom`.
        chatroom_id: String,
        /// The most members; 0 for all of them.
        limit: usize,
        /// The first day.
        start: Option<String>,
        /// The last day.
        end: Option<String>,
    },
    /// `group hours`.
    GroupHours {
        /// The group, `…@chatroom`.
        chatroom_id: String,
        /// The first day.
        start: Option<String>,
        /// The last day.
        end: Option<String>,
    },
    /// `group media`.
    GroupMedia {
        /// The group, `…@chatroom`.
        chatroom_id: String,
        /// The first day.
        start: Option<String>,
        /// The last day.
        end: Option<String>,
    },
    /// `group member`: what one member sent.
    GroupMember {
        /// The group, `…@chatroom`.
        chatroom_id: String,
        /// The member's wxid.
        username: String,
        /// The first day.
        start: Option<String>,
        /// The last day.
        end: Option<String>,
    },
    /// `group member-messages`: a page of one member's messages.
    GroupMemberMessages {
        /// The group, `…@chatroom`.
        chatroom_id: String,
        /// The member's wxid.
        username: String,
        /// The most messages of the page, in [`MEMBER_MESSAGES_LIMIT`](crate::services::MEMBER_MESSAGES_LIMIT).
        limit: usize,
        /// The `nextCursor` of the page before; 0 for the first.
        cursor: usize,
        /// The first day.
        start: Option<String>,
        /// The last day.
        end: Option<String>,
    },
    /// `analytics overall`.
    AnalyticsOverall,
    /// `analytics rankings`.
    AnalyticsRankings {
        /// The most chats; 0 for all of them.
        limit: usize,
        /// The first day.
        start: Option<String>,
        /// The last day.
        end: Option<String>,
    },
    /// `analytics time`.
    AnalyticsTime,
    /// `analytics excluded` without `--set`, which saves the list.
    AnalyticsExcluded,
    /// `analytics exclude-candidates`.
    AnalyticsExcludeCandidates,
    /// `biz accounts`: the official accounts followed.
    BizAccounts,
    /// `biz messages`: a page of an official account's pushes.
    BizMessages {
        /// The official account, `gh_…`.
        username: String,
        /// The most pushes.
        limit: i32,
        /// The pushes skipped, newest first.
        offset: i32,
    },
    /// `biz pay-records`.
    BizPayRecords {
        /// The most messages read.
        limit: i32,
        /// The messages skipped, newest first.
        offset: i32,
    },
    /// `insight footprint`.
    InsightFootprint {
        /// The first day.
        start: Option<String>,
        /// The last day.
        end: Option<String>,
    },
    /// `insight get`.
    InsightGet {
        /// The record's id.
        id: String,
    },
    /// `report annual years`.
    ReportAnnualYears,
    /// `cache list`.
    CacheList,
    /// `config get`: a key of the current profile, or the whole profile; read from the file, as the CLI does.
    ConfigGet {
        /// The key; the whole profile when none.
        key: Option<String>,
    },
    /// `config profiles`.
    ConfigProfiles,
    /// `db detect`.
    DbDetect,
    /// `db test`: opens the configured account with its key.
    DbTest,
    /// `db wxid`: the accounts of a data directory.
    DbWxid {
        /// The data directory; the configured one when none.
        root: Option<String>,
    },
}

/// `--start` / `--end` as Unix seconds, both inclusive (the end is the last second of its day); 0 for no bound.
pub fn day_span(start: Option<&str>, end: Option<&str>) -> AppResult<(i64, i64)> {
    let (b, e) = crate::dates::date_range_args(start, end)?;
    Ok((b.unwrap_or(0), e.map(|e| e - 1).unwrap_or(0)))
}

/// `config profiles` (#215, #216): what each profile is set up with, the keys only as whether they are set, so a
/// client can tell which account is ready without reading a key. `complete`: enough to open the database.
fn profiles_summary(config: &ConfigStore) -> Vec<crate::contract::ProfileSummary> {
    let set = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.trim().is_empty());
    let profiles = config
        .profiles
        .iter()
        .map(|(name, p)| crate::contract::ProfileSummary {
            name: name.clone(),
            current: *name == config.current_profile,
            wxid: p.wxid.clone(),
            db_path: p.db_path.clone(),
            decrypt_key: set(&p.decrypt_key),
            image_keys: p.image_xor_key.is_some() && set(&p.image_aes_key),
            // the key of the AI model API (#213), as the database keys: whether it is set
            ai_key: set(&p.ai_model_api_key),
            complete: set(&p.db_path) && set(&p.decrypt_key),
        })
        .collect();
    profiles
}

/// The config file of `hub`, read again: a config read answers what the file holds now.
fn config_of(hub: &ServiceHub) -> AppResult<ConfigStore> {
    ConfigStore::load(hub.config_path()).map_err(|err| AppError::config(format!("{err:#}")))
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
                Ok(json!(sessions))
            }
            Read::ChatMessages(page) => {
                // typed rows (#492), serialized once: parsed as the chat window shows them, or as they are kept
                let rows = match page.since {
                    Some(seconds) => {
                        hub.message_page_since(&page.session_id, page.limit, seconds)?
                    }
                    None => hub.message_page_rows(
                        &page.session_id,
                        page.limit,
                        page.offset,
                        page.before.as_deref(),
                        page.after.as_deref(),
                    )?,
                };
                Ok(if page.parsed {
                    json!(hub.parsed_rows(rows, &page.session_id))
                } else {
                    json!(rows
                        .into_iter()
                        .map(crate::contract::MessageRow::from)
                        .collect::<Vec<_>>())
                })
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
                    json!(hub.parsed_lookup(found, session_id))
                } else {
                    json!(hub.message_lookup(found, session_id))
                })
            }
            Read::ChatContact { username } => hub.contact(username),
            Read::ChatContacts { parsed: false } => hub.contacts(),
            Read::ChatContacts { parsed: true } => Ok(json!(hub.chat_contacts_parsed()?)),
            Read::ChatCounts { sessions } => Ok(json!(hub.chat_counts(sessions)?)),
            Read::ChatSearch {
                keyword,
                session,
                limit,
                offset,
                start,
                end,
                parsed,
            } => {
                let hits =
                    hub.search(keyword, session.as_deref(), *limit, *offset, *start, *end)?;
                Ok(if *parsed {
                    json!(hub.parsed_hits(hits))
                } else {
                    json!(hits
                        .into_iter()
                        .map(crate::contract::SearchHit::from)
                        .collect::<Vec<_>>())
                })
            }
            Read::ChatDates { session_id } => Ok(json!(hub.chat_dates(session_id)?)),
            Read::ChatDateCounts { session_id } => Ok(json!(hub.chat_date_counts(session_id)?)),
            Read::ChatDetailFast { session_id } => Ok(json!(hub.chat_detail_fast(session_id)?)),
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
                local_id,
                create_time,
            } => Ok(json!(hub.image_info_for_message(
                session_id,
                local_id,
                *create_time
            )?)),
            Read::GroupMembers {
                chatroom_id,
                counts,
            } => {
                let (members, updated_at) = hub.group_members_panel(chatroom_id, *counts)?;
                Ok(json!(crate::contract::GroupMembers {
                    chatroom_id: chatroom_id.clone(),
                    count: members.len(),
                    updated_at,
                    members,
                }))
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
                Ok(json!(crate::contract::SnsTimeline { timeline: posts }))
            }
            Read::SnsUsers => Ok(json!(crate::contract::SnsUsers {
                usernames: hub.sns_usernames_list()?
            })),
            Read::SnsStats { prefer_cache } => Ok(json!(
                hub.sns_export_stats(Freshness::cached_if(*prefer_cache))?
            )),
            Read::SnsPostCounts { user, prefer_cache } => match user {
                Some(u) => Ok(json!(hub.sns_user_post_stats(u)?)),
                None => Ok(json!(
                    hub.sns_user_post_counts(Freshness::cached_if(*prefer_cache))?
                )),
            },
            Read::ChatStatuses { sessions } => Ok(json!(hub.chat_statuses(sessions)?)),
            Read::ChatTabCounts => Ok(json!(hub.chat_tab_counts()?)),
            Read::GroupList => Ok(json!(hub.group_chats()?)),
            Read::GroupRanking {
                chatroom_id,
                limit,
                start,
                end,
            } => {
                let (b, e) = day_span(start.as_deref(), end.as_deref())?;
                Ok(json!(hub.group_message_ranking(
                    chatroom_id,
                    *limit,
                    b,
                    e
                )?))
            }
            Read::GroupHours {
                chatroom_id,
                start,
                end,
            } => {
                let (b, e) = day_span(start.as_deref(), end.as_deref())?;
                Ok(json!(hub.group_active_hours(chatroom_id, b, e)?))
            }
            Read::GroupMedia {
                chatroom_id,
                start,
                end,
            } => {
                let (b, e) = day_span(start.as_deref(), end.as_deref())?;
                Ok(json!(hub.group_media_stats(chatroom_id, b, e)?))
            }
            Read::GroupMember {
                chatroom_id,
                username,
                start,
                end,
            } => {
                let (b, e) = day_span(start.as_deref(), end.as_deref())?;
                Ok(json!(hub.group_member_analytics(
                    chatroom_id,
                    username,
                    b,
                    e
                )?))
            }
            Read::GroupMemberMessages {
                chatroom_id,
                username,
                limit,
                cursor,
                start,
                end,
            } => {
                let (b, e) = day_span(start.as_deref(), end.as_deref())?;
                Ok(json!(hub.group_member_messages(
                    chatroom_id,
                    username,
                    b,
                    e,
                    *limit,
                    *cursor
                )?))
            }
            Read::AnalyticsOverall => Ok(json!(hub.analytics_overall_statistics()?)),
            Read::AnalyticsRankings { limit, start, end } => {
                let (b, e) = day_span(start.as_deref(), end.as_deref())?;
                Ok(json!(hub.analytics_contact_rankings(*limit, b, e)?))
            }
            Read::AnalyticsTime => Ok(json!(hub.analytics_time_distribution()?)),
            Read::AnalyticsExcluded => Ok(json!(hub.analytics_excluded_usernames()?)),
            Read::AnalyticsExcludeCandidates => Ok(json!(hub.analytics_exclude_candidates()?)),
            Read::BizAccounts => hub.biz_accounts(),
            Read::BizMessages {
                username,
                limit,
                offset,
            } => Ok(json!(hub.biz_messages(username, *limit, *offset)?)),
            Read::BizPayRecords { limit, offset } => {
                Ok(json!(hub.biz_pay_records(*limit, *offset)?))
            }
            Read::InsightFootprint { start, end } => {
                let (b, e) = day_span(start.as_deref(), end.as_deref())?;
                Ok(json!(hub.insight_footprint(b, e)?))
            }
            Read::InsightGet { id } => Ok(json!(hub.insight_get_record(id)?)),
            Read::ReportAnnualYears => Ok(json!(hub.report_available_years()?)),
            Read::CacheList => Ok(json!(hub.cache_list())),
            Read::ConfigGet { key } => {
                if key.as_deref() == Some("config_path") {
                    return Ok(json!(hub.config_path().to_string_lossy()));
                }
                let config = config_of(hub)?;
                Ok(match key {
                    Some(key) => {
                        crate::config::refuse_renamed(key)?;
                        config.get_key(None, key)
                    }
                    None => serde_json::to_value(config.profile(None))
                        .map_err(|err| AppError::runtime(format!("{err:#}")))?,
                })
            }
            Read::ConfigProfiles => Ok(json!(profiles_summary(&config_of(hub)?))),
            Read::DbDetect => Ok(json!(hub.db_detect())),
            Read::DbTest => Ok(json!(hub.db_test()?)),
            Read::DbWxid { root } => Ok(json!(hub.db_wxid(root.as_deref())?)),
        }
    }
}

/// The image file `chat image-data --variant` asks for (`best`, `thumb`, `normal`, `hd`).
pub fn image_variant(variant: &str) -> ImageVariant {
    match variant {
        "thumb" => ImageVariant::Thumb,
        "normal" => ImageVariant::Normal,
        "hd" => ImageVariant::Hd,
        _ => ImageVariant::Best,
    }
}

/// The file of a message the command API shows a page as it is (`<img>`, `<audio>`, `<video>`), and that
/// `chat image-data`, `voice-data` and `video-data` write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaRead {
    /// An image, decrypted.
    ChatImage {
        /// The conversation.
        session_id: String,
        /// The message's id in its table.
        local_id: String,
        /// Its time, which tells apart messages of different databases with the same id.
        create_time: Option<i64>,
        /// Which of WeChat's files of the image.
        variant: ImageVariant,
    },
    /// Decoded to WAV.
    ChatVoice {
        /// The conversation.
        session_id: String,
        /// The message's id in its table.
        local_id: String,
        /// Its time, which tells apart messages of different databases with the same id.
        create_time: Option<i64>,
        /// WeChat's id of the message, in decimal.
        server_id: Option<String>,
        /// Who sent it, which matters in a group.
        sender: Option<String>,
    },
    /// The video as WeChat keeps it, or with `thumb` its thumbnail, else its cover.
    ChatVideo {
        /// The conversation.
        session_id: String,
        /// The message's id in its table.
        local_id: String,
        /// Its time, which tells apart messages of different databases with the same id.
        create_time: Option<i64>,
        /// The thumbnail, else the cover, rather than the video.
        thumb: bool,
    },
}

/// A media file and its content type.
#[derive(Debug)]
pub struct MediaFile {
    /// What it holds.
    pub body: MediaBody,
    /// Its content type.
    pub mime: &'static str,
}

/// What a media file is made of: bytes decoded in memory, or a file on disk, read as it is sent (a video).
#[derive(Debug)]
pub enum MediaBody {
    /// Bytes decoded in memory.
    Bytes(Vec<u8>),
    /// A file on disk.
    File(PathBuf),
}

/// The content type of an image, from its first bytes.
pub fn image_mime(bytes: &[u8]) -> &'static str {
    crate::image::ImageFormat::of(bytes)
        .map_or("application/octet-stream", crate::image::ImageFormat::mime)
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
                local_id,
                create_time,
                variant,
            } => {
                let bytes =
                    hub.image_variant_for_message(session_id, local_id, *create_time, *variant)?;
                Ok(MediaFile {
                    mime: image_mime(&bytes),
                    body: MediaBody::Bytes(bytes),
                })
            }
            MediaRead::ChatVoice {
                session_id,
                local_id,
                create_time,
                server_id,
                sender,
            } => Ok(MediaFile {
                body: MediaBody::Bytes(hub.voice_data(
                    session_id,
                    local_id,
                    *create_time,
                    server_id.as_deref(),
                    sender.as_deref(),
                )?),
                mime: "audio/wav",
            }),
            MediaRead::ChatVideo {
                session_id,
                local_id,
                create_time,
                thumb,
            } => {
                let files = hub.video_files_for_message(session_id, local_id, *create_time)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_say_whether_keys_are_set_but_not_what_they_are() {
        let mut config = ConfigStore::default();
        config.set_key(None, "db_path", json!("/data")).unwrap();
        config
            .set_key(None, "decrypt_key", json!("ab".repeat(32)))
            .unwrap();
        config
            .set_key(None, "image_aes_key", json!("0123456789abcdef"))
            .unwrap();
        config
            .set_key(Some("work"), "wxid", json!("wxid_w"))
            .unwrap();
        config
            .set_key(Some("work"), "ai_model_api_key", json!("sk-secret"))
            .unwrap();
        let summary = json!(profiles_summary(&config));
        assert_eq!(
            summary,
            json!([
                { "name": "default", "current": true, "wxid": null, "dbPath": "/data",
                  "decryptKey": true, "imageKeys": false, "aiKey": false, "complete": true },
                { "name": "work", "current": false, "wxid": "wxid_w", "dbPath": null,
                  "decryptKey": false, "imageKeys": false, "aiKey": true, "complete": false },
            ])
        );
        assert!(!summary.to_string().contains("abab"));
        assert!(!summary.to_string().contains("sk-secret"));
    }
}
