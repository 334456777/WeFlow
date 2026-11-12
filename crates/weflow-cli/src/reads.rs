//! What the command line and the command API of `weflow serve` share: the commands that are reads of
//! `weflow_core::reads` or write a message's media file, the hub they run on, and the `--json` document of an outcome.

use std::path::{Path, PathBuf};

use super::cli::Commands;
use super::commands::analytics::{AnalyticsCommand, AnalyticsSubcommand};
use super::commands::biz::BizSubcommand;
use super::commands::cache::{CacheCommand, CacheSubcommand};
use super::commands::chat::{ChatCommand, ChatSubcommand};
use super::commands::config::{ConfigCommand, ConfigSubcommand};
use super::commands::db::{DbCommand, DbSubcommand};
use super::commands::group::{GroupCommand, GroupSubcommand};
use super::commands::insight::{InsightCommand, InsightSubcommand};
use super::commands::report::{AnnualSubcommand, ReportCommand, ReportSubcommand};
use super::commands::runtime::{RuntimeCommand, RuntimeSubcommand};
use super::commands::sns::{SnsCommand, SnsSubcommand};
use super::{checked, exit_status, VERSION};
use serde_json::Value;
use weflow_core::config::{AppContext, ConfigStore};
use weflow_core::error::{AppError, AppResult};
use weflow_core::output::{failure, success};
use weflow_core::reads::{image_variant, MediaBody, MediaFile, MediaRead, MessagePage, Read};
use weflow_core::services::ServiceHub;

/// The hub `run` builds, and the config file it was built from: the command API's in-process routes keep one
/// open and build it again when that file changes.
pub(crate) fn open_hub() -> AppResult<(ServiceHub, PathBuf)> {
    let ctx = AppContext::new(VERSION)?;
    let config =
        ConfigStore::load(&ctx.config_path).map_err(|err| AppError::config(err.to_string()))?;
    let path = ctx.config_path.clone();
    Ok((ServiceHub::new(ctx, config), path))
}

/// The commands that are a [`Read`] of `weflow_core::reads` (#397): the CLI runs them as such, and the command API's
/// in-process routes (`/api/…`, issue #140) answer the same on a hub that stays open. Any other command is handed back.
#[expect(
    clippy::result_large_err,
    reason = "the Err is the command handed back to run as something else, not an error passed up with `?`"
)]
pub(crate) fn read_of(command: Commands) -> Result<Read, Commands> {
    Ok(match command {
        Commands::Chat(c) => match c.command {
            ChatSubcommand::Sessions { limit } => Read::ChatSessions { limit },
            ChatSubcommand::Messages(a) => Read::ChatMessages(MessagePage {
                session_id: a.session_id,
                limit: a.limit,
                offset: a.offset,
                before: a.before,
                after: a.after,
                since: a.since,
                parsed: a.parsed,
            }),
            ChatSubcommand::Latest { session_id, limit } => Read::ChatLatest { session_id, limit },
            ChatSubcommand::Message {
                session_id,
                local_id,
                server_id,
                create_time,
                parsed,
            } => Read::ChatMessage {
                session_id,
                local_id,
                server_id,
                create_time,
                parsed,
            },
            ChatSubcommand::Contact { username } => Read::ChatContact { username },
            ChatSubcommand::Contacts { parsed } => Read::ChatContacts { parsed },
            ChatSubcommand::Counts { sessions } => Read::ChatCounts { sessions },
            ChatSubcommand::Search {
                keyword,
                session_id,
                limit,
                offset,
                start,
                end,
                parsed,
            } => Read::ChatSearch {
                keyword,
                session_id,
                limit,
                offset,
                start,
                end,
                parsed,
            },
            ChatSubcommand::Dates { session_id } => Read::ChatDates { session_id },
            ChatSubcommand::DateCounts { session_id } => Read::ChatDateCounts { session_id },
            ChatSubcommand::Detail {
                session_id,
                fast: true,
                extra: false,
            } => Read::ChatDetailFast { session_id },
            ChatSubcommand::Resources {
                session,
                types,
                start,
                end,
                limit,
                before,
            } => Read::ChatResources {
                session,
                types,
                start,
                end,
                limit,
                before,
            },
            ChatSubcommand::ImageInfo {
                session_id,
                msg_id,
                create_time,
            } => Read::ChatImageInfo {
                session_id,
                msg_id,
                create_time,
            },
            ChatSubcommand::Statuses { usernames } => Read::ChatStatuses { usernames },
            ChatSubcommand::TabCounts => Read::ChatTabCounts,
            command => return Err(Commands::Chat(ChatCommand { command })),
        },
        Commands::Group(c) => match c.command {
            GroupSubcommand::List => Read::GroupList,
            GroupSubcommand::Members {
                chatroom_id,
                counts,
            } => Read::GroupMembers {
                chatroom_id,
                counts,
            },
            GroupSubcommand::Ranking {
                chatroom_id,
                limit,
                start,
                end,
            } => Read::GroupRanking {
                chatroom_id,
                limit,
                start,
                end,
            },
            GroupSubcommand::Hours {
                chatroom_id,
                start,
                end,
            } => Read::GroupHours {
                chatroom_id,
                start,
                end,
            },
            GroupSubcommand::Media {
                chatroom_id,
                start,
                end,
            } => Read::GroupMedia {
                chatroom_id,
                start,
                end,
            },
            GroupSubcommand::Member {
                chatroom_id,
                username,
                start,
                end,
            } => Read::GroupMember {
                chatroom_id,
                username,
                start,
                end,
            },
            GroupSubcommand::MemberMessages {
                chatroom_id,
                username,
                limit,
                cursor,
                start,
                end,
            } => Read::GroupMemberMessages {
                chatroom_id,
                username,
                limit,
                cursor,
                start,
                end,
            },
            command => return Err(Commands::Group(GroupCommand { command })),
        },
        Commands::Analytics(c) => match c.command {
            AnalyticsSubcommand::Overall => Read::AnalyticsOverall,
            AnalyticsSubcommand::Rankings { limit, start, end } => {
                Read::AnalyticsRankings { limit, start, end }
            }
            AnalyticsSubcommand::Time => Read::AnalyticsTime,
            // `--set` saves the list
            AnalyticsSubcommand::Excluded { set: None } => Read::AnalyticsExcluded,
            AnalyticsSubcommand::ExcludeCandidates => Read::AnalyticsExcludeCandidates,
            command => return Err(Commands::Analytics(AnalyticsCommand { command })),
        },
        Commands::Biz(c) => match c.command {
            BizSubcommand::Accounts => Read::BizAccounts,
            BizSubcommand::Messages {
                username,
                limit,
                offset,
            } => Read::BizMessages {
                username,
                limit,
                offset,
            },
            BizSubcommand::PayRecords { limit, offset } => Read::BizPayRecords { limit, offset },
        },
        Commands::Insight(c) => match c.command {
            InsightSubcommand::Footprint { start, end } => Read::InsightFootprint { start, end },
            InsightSubcommand::Get { id } => Read::InsightGet { id },
            command => return Err(Commands::Insight(InsightCommand { command })),
        },
        Commands::Report(c) => match c.command {
            ReportSubcommand::Annual {
                command: AnnualSubcommand::Years,
            } => Read::ReportAnnualYears,
            command => return Err(Commands::Report(ReportCommand { command })),
        },
        Commands::Cache(c) => match c.command {
            CacheSubcommand::List => Read::CacheList,
            command => return Err(Commands::Cache(CacheCommand { command })),
        },
        Commands::Config(c) => match c.command {
            ConfigSubcommand::Get { key } => Read::ConfigGet { key },
            ConfigSubcommand::Profiles => Read::ConfigProfiles,
            command => return Err(Commands::Config(ConfigCommand { command })),
        },
        Commands::Runtime(c) => match c.command {
            RuntimeSubcommand::Info => Read::RuntimeInfo,
            command => return Err(Commands::Runtime(RuntimeCommand { command })),
        },
        Commands::Db(c) => match c.command {
            DbSubcommand::Detect => Read::DbDetect,
            DbSubcommand::Test | DbSubcommand::Open => Read::DbTest,
            DbSubcommand::Wxid { root } => Read::DbWxid { root },
            command => return Err(Commands::Db(DbCommand { command })),
        },
        Commands::Sns(c) => match c.command {
            // the timeline without its media, which are downloaded
            SnsSubcommand::Timeline {
                limit,
                offset,
                users,
                keyword,
                start,
                end,
                with_media: false,
            } => Read::SnsTimeline {
                limit,
                offset,
                users,
                keyword,
                start,
                end,
            },
            SnsSubcommand::Users => Read::SnsUsers,
            SnsSubcommand::Stats { fast } => Read::SnsStats { fast },
            SnsSubcommand::PostCounts { user, prefer_cache } => {
                Read::SnsPostCounts { user, prefer_cache }
            }
            command => return Err(Commands::Sns(SnsCommand { command })),
        },
        command => return Err(command),
    })
}

/// The commands that write a message's file, as a [`MediaRead`], with the file to write (`--out`; image-data and
/// video-data require it): the `/api/` media routes answer the file itself. Any other command is handed back.
#[expect(
    clippy::result_large_err,
    reason = "the Err is the command handed back to run as something else, not an error passed up with `?`"
)]
pub(crate) fn media_of(
    command: ChatSubcommand,
) -> Result<(MediaRead, Option<PathBuf>), ChatSubcommand> {
    Ok(match command {
        ChatSubcommand::ImageData {
            session_id,
            msg_id,
            create_time,
            thumb,
            variant,
            out,
        } => (
            MediaRead::ChatImage {
                session_id,
                msg_id,
                create_time,
                variant: image_variant(if thumb { "thumb" } else { &variant }),
            },
            Some(out),
        ),
        ChatSubcommand::VoiceData {
            session_id,
            msg_id,
            create_time,
            server_id,
            sender,
            out,
        } => (
            MediaRead::ChatVoice {
                session_id,
                msg_id,
                create_time,
                server_id,
                sender,
            },
            out,
        ),
        ChatSubcommand::VideoData {
            session_id,
            msg_id,
            create_time,
            thumb,
            out,
        } => (
            MediaRead::ChatVideo {
                session_id,
                msg_id,
                create_time,
                thumb,
            },
            Some(out),
        ),
        command => return Err(command),
    })
}

/// Writes a media file to `out`, making its folder first; the number of bytes written.
pub(crate) fn write_media(file: MediaFile, out: &Path) -> AppResult<u64> {
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::runtime(format!("failed to create {}: {e}", parent.display()))
        })?;
    }
    let written = match file.body {
        MediaBody::Bytes(bytes) => std::fs::write(out, &bytes).map(|()| bytes.len() as u64),
        MediaBody::File(from) => std::fs::copy(from, out),
    };
    written.map_err(|e| AppError::runtime(format!("failed to write {}: {e}", out.display())))
}

/// The `--json` document of a command's outcome, and its exit code. The command API's in-process routes answer
/// with it too, so they match the child process byte for byte.
#[expect(
    clippy::expect_used,
    reason = "a document of JSON values and strings always serializes"
)]
pub(crate) fn json_document(outcome: AppResult<Value>) -> (Value, u8) {
    let (mut doc, exit) = match checked(outcome) {
        Ok(value) => (
            serde_json::to_value(success(value)).expect("a success document serializes"),
            0,
        ),
        Err(err) => (
            serde_json::to_value(failure(err.payload())).expect("a failure document serializes"),
            exit_status(err.exit_code),
        ),
    };
    weflow_core::locale::localize_json(&mut doc);
    (doc, exit)
}
