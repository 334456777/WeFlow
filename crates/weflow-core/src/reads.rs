//! The reads the CLI and the command API share (#397): one typed request each, run on a [`ServiceHub`]. The CLI builds
//! them from its arguments and the command API from the parameters of a request, so neither depends on how the other
//! parses; both answer what [`Read::run`] returns, serialized at their own edge.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::config::ConfigStore;
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
    /// Whether each conversation is folded and muted.
    ChatStatuses {
        usernames: Vec<String>,
    },
    ChatTabCounts,
    GroupList,
    /// `group ranking`; like every `start` and `end` below, local dates (`YYYY-MM-DD`), both included.
    GroupRanking {
        chatroom_id: String,
        limit: usize,
        start: Option<String>,
        end: Option<String>,
    },
    GroupHours {
        chatroom_id: String,
        start: Option<String>,
        end: Option<String>,
    },
    GroupMedia {
        chatroom_id: String,
        start: Option<String>,
        end: Option<String>,
    },
    GroupMember {
        chatroom_id: String,
        username: String,
        start: Option<String>,
        end: Option<String>,
    },
    GroupMemberMessages {
        chatroom_id: String,
        username: String,
        limit: usize,
        cursor: usize,
        start: Option<String>,
        end: Option<String>,
    },
    /// `analytics overall`; `force` reads every chat again rather than the aggregate kept.
    AnalyticsOverall {
        force: bool,
    },
    AnalyticsRankings {
        limit: usize,
        start: Option<String>,
        end: Option<String>,
    },
    AnalyticsTime,
    /// `analytics excluded` without `--set`, which saves the list.
    AnalyticsExcluded,
    AnalyticsExcludeCandidates,
    BizAccounts,
    BizMessages {
        username: String,
        limit: i32,
        offset: i32,
    },
    BizPayRecords {
        limit: i32,
        offset: i32,
    },
    InsightFootprint {
        start: Option<String>,
        end: Option<String>,
    },
    InsightGet {
        id: String,
    },
    ReportAnnualYears,
    CacheList,
    /// `config get`: a key of the current profile, or the whole profile; read from the file, as the CLI does.
    ConfigGet {
        key: Option<String>,
    },
    ConfigProfiles,
    RuntimeInfo,
    DbDetect,
    /// `db test` (and `db open`): opens the configured account with its key.
    DbTest,
    DbWxid {
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
pub fn profiles_summary(config: &ConfigStore) -> Vec<crate::contract::ProfileSummary> {
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
    ConfigStore::load(hub.config_path()).map_err(|err| AppError::config(err.to_string()))
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
            Read::ChatContacts { parsed: true } => Ok(json!(hub.chat_contacts_parsed()?)),
            Read::ChatCounts { sessions } => Ok(json!(hub.chat_counts(sessions)?)),
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
                msg_id,
                create_time,
            } => Ok(json!(hub.image_info_for_message(
                session_id,
                msg_id,
                *create_time
            )?)),
            Read::GroupMembers {
                chatroom_id,
                counts,
                refresh,
            } => {
                let (members, from_cache, updated_at) =
                    hub.group_members_panel(chatroom_id, *refresh, *counts)?;
                Ok(json!(crate::contract::GroupMembers {
                    chatroom_id: chatroom_id.clone(),
                    count: members.len(),
                    from_cache,
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
                Ok(json!(crate::contract::SnsTimeline::of(posts)?))
            }
            Read::SnsUsers => Ok(json!(crate::contract::SnsUsers {
                usernames: hub.sns_usernames_list()?
            })),
            Read::SnsStats { fast } => Ok(json!(hub.sns_export_stats(*fast)?)),
            Read::SnsPostCounts { user, prefer_cache } => match user {
                Some(u) => Ok(json!(hub.sns_user_post_stats(u)?)),
                None => Ok(json!(hub.sns_user_post_counts(*prefer_cache)?)),
            },
            Read::ChatStatuses { usernames } => Ok(json!(hub.chat_statuses(usernames)?)),
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
            Read::AnalyticsOverall { force } => {
                Ok(json!(hub.analytics_overall_statistics(*force)?))
            }
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
            Read::InsightGet { id } => Ok(json!(hub.insight_get_record(id))),
            Read::ReportAnnualYears => Ok(json!(hub.report_available_years()?)),
            Read::CacheList => Ok(json!(hub.cache_list())),
            Read::ConfigGet { key } => {
                if matches!(key.as_deref(), Some("config_path" | "configPath")) {
                    return Ok(json!(hub.config_path().to_string_lossy()));
                }
                let config = config_of(hub)?;
                Ok(match key {
                    Some(key) => config.get_key(None, key),
                    None => serde_json::to_value(config.profile(None)).unwrap(),
                })
            }
            Read::ConfigProfiles => Ok(json!(profiles_summary(&config_of(hub)?))),
            Read::RuntimeInfo => {
                let ctx = hub.context();
                Ok(json!(crate::contract::RuntimeInfo {
                    home_dir: ctx.home_dir.clone(),
                    config_path: ctx.config_path.clone(),
                    runtime_dir: ctx.runtime_dir.clone(),
                    version: ctx.version.clone(),
                    target: weflow_assets::target_triple().to_string(),
                }))
            }
            Read::DbDetect => Ok(json!(hub.db_detect())),
            Read::DbTest => Ok(json!(hub.db_test()?)),
            Read::DbWxid { root } => Ok(json!(hub.db_wxid(root.as_deref())?)),
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
