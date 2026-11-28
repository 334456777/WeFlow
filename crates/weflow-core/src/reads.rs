//! 命令行和命令 API 共用的读取（#397），每种读取是一个类型化的请求。
//!
//! 每个读取都在一个 [`ServiceHub`] 上运行。命令行从自己的参数构造读取，命令 API 从请求的参数构造，
//! 两边互不依赖对方的解析方式；两边都回答 [`Read::run`] 的结果，各自在自己的边界上序列化。
//! 返回文件本身的媒体读取是 [`MediaRead`]。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::config::ConfigStore;
use crate::error::{AppError, AppResult};
use crate::services::{Freshness, ImageVariant, ResourceQuery, ServiceHub, SnsTimelineQuery};

/// 会话消息中的一页（`chat messages`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessagePage {
    /// 会话：私聊为对方的 wxid，群聊为 `…@chatroom`。
    pub session_id: String,
    /// 这一页最多的消息数。
    pub limit: i32,
    /// 不带 `before`、`after`、`since` 时，从最新的消息起跳过的条数。
    pub offset: i32,
    /// 读取这条消息之前（更早）的一页：那条消息的 `_cursor`。
    pub before: Option<String>,
    /// 读取这条消息之后（更新）的一页：那条消息的 `_cursor`。
    pub after: Option<String>,
    /// Unix 秒：从这个时间在聊天顺序中的位置开始的一页。
    pub since: Option<i64>,
    /// 按聊天窗口显示的样子回答消息，而不是微信保存的原样。
    pub parsed: bool,
}

/// 回答一个 JSON 值的读取，即命令 API 进程内路由的读取。
///
/// `session_id` 是会话：私聊为对方的 wxid，群聊为 `…@chatroom`。类型为 `Option<String>` 的 `start`、`end`
/// 是本地日期（`YYYY-MM-DD`），两端都包含；`parsed` 让消息按聊天窗口显示的样子回答，而不是微信保存的原样。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// `chat sessions`；`limit` 为 0 时列出全部。
    ChatSessions {
        /// 最多列出的会话数。
        limit: usize,
    },
    /// `chat messages`：会话消息中的一页。
    ChatMessages(MessagePage),
    /// `chat message`：按 `local_id`、`server_id` 两者之一查找，只能给一个。
    ChatMessage {
        /// 会话。
        session_id: String,
        /// 消息的本地 ID。
        local_id: Option<i64>,
        /// 服务器 ID，十进制。
        server_id: Option<String>,
        /// 与 `local_id` 一起给出时：消息的时间，用来区分不同消息库中本地 ID 相同的消息。
        create_time: Option<i64>,
        /// 按聊天窗口显示的样子回答。
        parsed: bool,
    },
    /// `chat contact`：一个联系人的行。
    ChatContact {
        /// 联系人的 wxid。
        username: String,
    },
    /// `chat contacts`：全部联系人。
    ChatContacts {
        /// 排序并分类，带名字、标签和头像，而不是微信保存的原样。
        parsed: bool,
    },
    /// `chat counts`：每个会话的消息数。
    ChatCounts {
        /// 这些会话。
        sessions: Vec<String>,
    },
    /// `chat search`：在会话 `session` 中搜索，没有给出时搜索全部会话。
    ChatSearch {
        /// 要找的文字。
        keyword: String,
        /// 会话；没有时为全部会话。
        session: Option<String>,
        /// 最多的结果数。
        limit: i32,
        /// 从最新的结果起跳过的条数。
        offset: i32,
        /// Unix 秒：只要从这时起的消息；0 表示不限。
        start: i32,
        /// Unix 秒：只要到这时为止的消息；0 表示不限。
        end: i32,
        /// 每条结果按聊天窗口显示的样子回答。
        parsed: bool,
    },
    /// `chat dates`：会话有消息的日期。
    ChatDates {
        /// 会话。
        session_id: String,
    },
    /// `chat date-counts`：会话每天的消息数。
    ChatDateCounts {
        /// 会话。
        session_id: String,
    },
    /// `chat detail --fast`：联系人和消息数。
    ChatDetailFast {
        /// 会话。
        session_id: String,
    },
    /// `chat resources`；`start`、`end` 是本地日期（`YYYY-MM-DD`），两端都包含。
    ChatResources {
        /// 会话；没有时为全部会话。
        session: Option<String>,
        /// 要列出的种类：`image`、`video`、`voice`、`file`；为空时列出全部种类。
        types: Vec<String>,
        /// 第一天。
        start: Option<String>,
        /// 最后一天。
        end: Option<String>,
        /// 这一页最多的消息数：0 为 300，最多 2000。
        limit: usize,
        /// 上一页的 `nextCursor`。
        before: Option<String>,
    },
    /// `chat image-info`：消息中的图片有哪些版本在本机上。
    ChatImageInfo {
        /// 会话。
        session_id: String,
        /// 消息的本地 ID。
        local_id: String,
        /// 消息的时间，用来区分不同消息库中本地 ID 相同的消息。
        create_time: Option<i64>,
    },
    /// `group members`：群的成员面板。
    GroupMembers {
        /// 群，`…@chatroom`。
        chatroom_id: String,
        /// 同时统计每个成员的消息数。
        counts: bool,
    },
    /// 不带 `with-media` 的 `sns timeline`（带它时要下载媒体）。
    SnsTimeline {
        /// 最多的动态数。
        limit: i32,
        /// 从最新的动态起跳过的条数。
        offset: i32,
        /// 只要这些 wxid 的动态；为空时不限。
        users: Vec<String>,
        /// 只要含这段文字的动态。
        keyword: Option<String>,
        /// Unix 秒：只要从这时起的动态；0 表示不限。
        start: i64,
        /// Unix 秒：只要到这时为止的动态；0 表示不限。
        end: i64,
    },
    /// `sns users`：发过动态的人。
    SnsUsers,
    /// `sns stats`：朋友圈的动态数。
    SnsStats {
        /// 之前读到的计数没有过期时直接用它回答。
        prefer_cache: bool,
    },
    /// `sns post-counts`：每个人的动态数。
    SnsPostCounts {
        /// 只要这个 wxid 的动态数。
        user: Option<String>,
        /// 之前读到的计数没有过期时直接用它回答。
        prefer_cache: bool,
    },
    /// `chat statuses`：每个会话是否折叠、免打扰和置顶。
    ChatStatuses {
        /// 这些会话。
        sessions: Vec<String>,
    },
    /// `chat tab-counts`：每类联系人的个数。
    ChatTabCounts,
    /// `group list`：账号所在的群。
    GroupList,
    /// `group ranking`；与下面每个 `start`、`end` 一样，日期是本地的 `YYYY-MM-DD`，两端都包含。
    GroupRanking {
        /// 群，`…@chatroom`。
        chatroom_id: String,
        /// 最多的成员数；0 表示全部。
        limit: usize,
        /// 第一天。
        start: Option<String>,
        /// 最后一天。
        end: Option<String>,
    },
    /// `group hours`：群消息按小时的分布。
    GroupHours {
        /// 群，`…@chatroom`。
        chatroom_id: String,
        /// 第一天。
        start: Option<String>,
        /// 最后一天。
        end: Option<String>,
    },
    /// `group media`：群里各种消息的数量。
    GroupMedia {
        /// 群，`…@chatroom`。
        chatroom_id: String,
        /// 第一天。
        start: Option<String>,
        /// 最后一天。
        end: Option<String>,
    },
    /// `group member`：一个成员发了什么。
    GroupMember {
        /// 群，`…@chatroom`。
        chatroom_id: String,
        /// 成员的 wxid。
        username: String,
        /// 第一天。
        start: Option<String>,
        /// 最后一天。
        end: Option<String>,
    },
    /// `group member-messages`：一个成员的消息中的一页。
    GroupMemberMessages {
        /// 群，`…@chatroom`。
        chatroom_id: String,
        /// 成员的 wxid。
        username: String,
        /// 这一页最多的消息数，在 [`MEMBER_MESSAGES_LIMIT`](crate::services::MEMBER_MESSAGES_LIMIT) 之内。
        limit: usize,
        /// 上一页的 `nextCursor`；第一页为 0。
        cursor: usize,
        /// 第一天。
        start: Option<String>,
        /// 最后一天。
        end: Option<String>,
    },
    /// `analytics overall`：私聊的总体统计。
    AnalyticsOverall,
    /// `analytics rankings`：消息最多的私聊。
    AnalyticsRankings {
        /// 最多的私聊数；0 表示全部。
        limit: usize,
        /// 第一天。
        start: Option<String>,
        /// 最后一天。
        end: Option<String>,
    },
    /// `analytics time`：私聊消息的时间分布。
    AnalyticsTime,
    /// 不带 `--set` 的 `analytics excluded`（带 `--set` 时保存名单）。
    AnalyticsExcluded,
    /// `analytics exclude-candidates`：可以从统计中排除的人。
    AnalyticsExcludeCandidates,
    /// `biz accounts`：关注的公众号。
    BizAccounts,
    /// `biz messages`：一个公众号的推送中的一页。
    BizMessages {
        /// 公众号的用户名，`gh_…`。
        username: String,
        /// 最多读取的消息数。
        limit: i32,
        /// 从最新的消息起跳过的条数。
        offset: i32,
    },
    /// `biz pay-records`：微信支付通知中的一页。
    BizPayRecords {
        /// 最多读取的消息数。
        limit: i32,
        /// 从最新的消息起跳过的条数。
        offset: i32,
    },
    /// `insight footprint`：一段日期内的足迹。
    InsightFootprint {
        /// 第一天。
        start: Option<String>,
        /// 最后一天。
        end: Option<String>,
    },
    /// `insight get`：一条见解记录。
    InsightGet {
        /// 记录的 ID。
        id: String,
    },
    /// `report annual years`：可以生成年度报告的年份。
    ReportAnnualYears,
    /// `cache list`：缓存各部分占用的空间。
    CacheList,
    /// `config get`：当前档案的一个键，或整个档案；与命令行一样从文件读取。
    ConfigGet {
        /// 键；没有时为整个档案。
        key: Option<String>,
    },
    /// `config profiles`：配置中的档案。
    ConfigProfiles,
    /// `db detect`：微信数据目录的默认位置。
    DbDetect,
    /// `db test`：用配置的密钥打开配置的账号。
    DbTest,
    /// `db wxid`：数据目录中的账号。
    DbWxid {
        /// 数据目录；没有时取配置的 `db_path`，也没有配置时取存在的各个默认位置。
        root: Option<String>,
    },
}

/// 把 `--start`、`--end` 换成 Unix 秒，两端都包含（结束为那一天的最后一秒）；没有给出的一端为 0，表示不限。
///
/// # Errors
///
/// 返回用法错误：日期不是有效的 `YYYY-MM-DD`（年份在 1970 到 2100 之间）、在本地时区中不存在，
/// 或者 `start` 晚于 `end`。
pub fn day_span(start: Option<&str>, end: Option<&str>) -> AppResult<(i64, i64)> {
    let (b, e) = crate::dates::date_range_args(start, end)?;
    Ok((b.unwrap_or(0), e.map(|e| e - 1).unwrap_or(0)))
}

/// `config profiles`（#215、#216）：每个档案设置了什么；密钥只说明设置了没有，客户端不读密钥也能知道哪个账号可以用。
/// `complete` 表示足以打开数据库。
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
            // AI 模型接口的密钥（#213）与数据库密钥一样，只说明设置了没有
            ai_key: set(&p.ai_model_api_key),
            complete: set(&p.db_path) && set(&p.decrypt_key),
        })
        .collect();
    profiles
}

/// 重新读取 `hub` 的配置文件：配置的读取回答文件现在的内容。
fn config_of(hub: &ServiceHub) -> AppResult<ConfigStore> {
    ConfigStore::load(hub.config_path()).map_err(|err| AppError::config(format!("{err:#}")))
}

impl Read {
    /// 读取的回答，即它的 `--json` 文档中的 `data`。
    ///
    /// # Errors
    ///
    /// 返回所调用的服务的错误：参数不对时为用法错误，账号没有配置好时为配置错误，读取数据库失败时为数据库层的错误，
    /// 要找的消息或记录不存在时为 `not_found`。
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
                // 类型化的行（#492），只序列化一次：按聊天窗口显示的样子解析，或者保持微信保存的原样
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

/// `chat image-data --variant` 要的图片版本（`best`、`thumb`、`normal`、`hd`）；其他写法为 `best`。
pub fn image_variant(variant: &str) -> ImageVariant {
    match variant {
        "thumb" => ImageVariant::Thumb,
        "normal" => ImageVariant::Normal,
        "hd" => ImageVariant::Hd,
        _ => ImageVariant::Best,
    }
}

/// 消息中的媒体文件：命令 API 把它原样交给页面（`<img>`、`<audio>`、`<video>`），
/// `chat image-data`、`voice-data`、`video-data` 把它写出。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaRead {
    /// 解密后的图片。
    ChatImage {
        /// 会话。
        session_id: String,
        /// 消息的本地 ID。
        local_id: String,
        /// 消息的时间，用来区分不同消息库中本地 ID 相同的消息。
        create_time: Option<i64>,
        /// 取微信保存的哪个版本。
        variant: ImageVariant,
    },
    /// 解码成 WAV 的语音。
    ChatVoice {
        /// 会话。
        session_id: String,
        /// 消息的本地 ID。
        local_id: String,
        /// 消息的时间，用来区分不同消息库中本地 ID 相同的消息。
        create_time: Option<i64>,
        /// 服务器 ID，十进制。
        server_id: Option<String>,
        /// 发送者，在群聊中用来找到语音。
        sender: Option<String>,
    },
    /// 微信保存的视频本身；带 `thumb` 时为缩略图，没有时为封面。
    ChatVideo {
        /// 会话。
        session_id: String,
        /// 消息的本地 ID。
        local_id: String,
        /// 消息的时间，用来区分不同消息库中本地 ID 相同的消息。
        create_time: Option<i64>,
        /// 取缩略图（没有时为封面），而不是视频。
        thumb: bool,
    },
}

/// 一个媒体文件及其内容类型。
#[derive(Debug)]
pub struct MediaFile {
    /// 文件的内容。
    pub body: MediaBody,
    /// 内容类型（MIME）。
    pub mime: &'static str,
}

/// 媒体文件由什么组成：在内存中解码出的字节，或者磁盘上边读边发的文件（视频）。
#[derive(Debug)]
pub enum MediaBody {
    /// 在内存中解码出的字节。
    Bytes(Vec<u8>),
    /// 磁盘上的文件。
    File(PathBuf),
}

/// 按开头的字节判断图片的内容类型；认不出时为 `application/octet-stream`。
pub fn image_mime(bytes: &[u8]) -> &'static str {
    crate::image::ImageFormat::of(bytes)
        .map_or("application/octet-stream", crate::image::ImageFormat::mime)
}

impl MediaRead {
    /// 在阻塞线程上读取文件：解密图片或解码语音要花几毫秒。
    ///
    /// # Errors
    ///
    /// 同各媒体所用的服务方法（[`ServiceHub::image_variant_for_message`]、[`ServiceHub::voice_data`]、
    /// [`ServiceHub::video_files_for_message`]）；要视频的缩略图而缩略图和封面都不在本机时，返回 `failureKind` 为
    /// `not_found` 的运行时错误；阻塞线程失败时返回运行时错误。
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
                // 封面不大：读进内存，好判断它的类型
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

/// `chat video-data` 回答的文件：视频本身；带 `thumb` 时为缩略图，没有时为封面。
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
