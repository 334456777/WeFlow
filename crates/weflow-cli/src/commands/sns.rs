//! `weflow sns`: Moments, their media and the block-delete trigger.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};

use serde_json::{json, Value};
use weflow_core::error::{AppError, AppResult};
use weflow_core::services::{ServiceHub, SnsMediaFetch};

#[derive(Args, Debug)]
pub(crate) struct SnsCommand {
    #[command(subcommand)]
    pub(crate) command: SnsSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum SnsSubcommand {
    /// Moments timeline, newest first
    Timeline {
        /// Number of posts
        #[arg(long, default_value_t = 20)]
        limit: i32,
        /// Number of posts to skip
        #[arg(long, default_value_t = 0)]
        offset: i32,
        /// Only posts of these users; repeat for several
        #[arg(long = "user")]
        users: Vec<String>,
        /// Only posts containing this text
        #[arg(long)]
        keyword: Option<String>,
        /// Only posts from this time on, in Unix seconds; 0 means no limit
        #[arg(long, default_value_t = 0)]
        start: i64,
        /// Only posts up to this time, in Unix seconds; 0 means no limit
        #[arg(long, default_value_t = 0)]
        end: i64,
        /// Download and decrypt images / videos into the cache and inline them as data URLs
        #[arg(long)]
        with_media: bool,
    },
    /// Users that have posted
    Users,
    /// Ask the server for the first bytes of a Moments resource and show the status and decryption headers
    DebugResource {
        /// Resource URL
        url: String,
    },
    /// Moments statistics for the export page: total posts, users who posted, your posts
    Stats {
        /// Use the counts kept from an earlier read while they have not expired, and skip the slow count over the timeline
        #[arg(long)]
        prefer_cache: bool,
    },
    /// Post counts per user, or the statistics of one user
    PostCounts {
        /// Statistics of this user only
        #[arg(long)]
        user: Option<String>,
        /// Use the counts kept from an earlier read while they have not expired
        #[arg(long)]
        prefer_cache: bool,
    },
    /// Export the timeline as json, html or arkmejson
    Export {
        /// Output file
        #[arg(long)]
        out: PathBuf,
        /// Export format: json, html or arkmejson
        #[arg(long, default_value = "json")]
        format: String,
        /// Only posts of these users; repeat for several
        #[arg(long = "user")]
        users: Vec<String>,
        /// Only posts containing this text
        #[arg(long)]
        keyword: Option<String>,
        /// Only posts from this time on, in Unix seconds; 0 means no limit
        #[arg(long, default_value_t = 0)]
        start: i64,
        /// Only posts up to this time, in Unix seconds; 0 means no limit
        #[arg(long, default_value_t = 0)]
        end: i64,
        /// Also save images / live photos / videos next to the export
        #[arg(long)]
        media: bool,
    },
    /// Fetch a Moments image or video, and decrypt it with --key
    Media {
        /// Resource URL
        url: String,
        /// Decryption key
        #[arg(long)]
        key: Option<String>,
        /// Output file; without it only the size and the file in the cache are given
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Download a Moments sticker, plain or AES-GCM encrypted
    DownloadEmoji {
        /// Sticker URL
        url: String,
        /// URL of the encrypted sticker
        #[arg(long)]
        encrypt_url: Option<String>,
        /// AES key of the encrypted sticker
        #[arg(long)]
        aes_key: Option<String>,
    },
    /// Download an arbitrary image URL and decrypt it if it is a .dat payload
    DownloadImage {
        /// Image URL
        url: String,
        /// Output file; default: print base64 in the result
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Moments block-delete trigger; checking, installing and removing are refused, as the database is read-only
    BlockDelete {
        /// check, install or uninstall
        action: TriggerAction,
    },
    /// Delete a Moments post; always refused, as the database is read-only
    Delete {
        /// Post id
        post_id: String,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum TriggerAction {
    Check,
    Install,
    Uninstall,
}

pub(crate) async fn handle_sns(command: SnsCommand, hub: &ServiceHub) -> AppResult<Value> {
    use weflow_core::services::{SnsExportOptions, SnsTimelineQuery};
    match command.command {
        SnsSubcommand::DebugResource { url } => Ok(json!(hub.sns_debug_resource(&url).await?)),
        // with its media: without them the timeline is a read
        SnsSubcommand::Timeline {
            limit,
            offset,
            users,
            keyword,
            start,
            end,
            with_media: _,
        } => {
            let posts = hub.sns_timeline_query(&SnsTimelineQuery {
                limit,
                offset,
                usernames: users,
                keyword,
                start,
                end,
            })?;
            let posts = hub.sns_enrich_timeline_media(posts).await;
            Ok(json!(weflow_core::contract::SnsTimeline {
                timeline: posts
            }))
        }
        SnsSubcommand::Users | SnsSubcommand::Stats { .. } | SnsSubcommand::PostCounts { .. } => {
            unreachable!("a read")
        }
        SnsSubcommand::Export {
            out,
            format,
            users,
            keyword,
            start,
            end,
            media,
        } => {
            let exported = hub
                .sns_export_timeline(&SnsExportOptions {
                    output_dir: out,
                    format,
                    usernames: users,
                    keyword,
                    export_media: media,
                    start,
                    end,
                })
                .await?;
            Ok(json!(exported))
        }
        SnsSubcommand::Media { url, key, out } => {
            let fetched = hub.sns_fetch_media(&url, key.as_deref()).await?;
            let bytes = match &out {
                Some(path) => save_media(&fetched, path)?,
                None => media_size(&fetched)?,
            };
            Ok(json!(weflow_core::contract::SnsMediaFetched {
                out: out.map(|path| path.to_string_lossy().into_owned()),
                content_type: fetched.content_type,
                bytes,
                cache_path: fetched.cache_path,
            }))
        }
        SnsSubcommand::DownloadEmoji {
            url,
            encrypt_url,
            aes_key,
        } => Ok(json!(
            hub.sns_download_emoji(&url, encrypt_url.as_deref(), aes_key.as_deref())
                .await?
        )),
        SnsSubcommand::DownloadImage { url, out } => {
            Ok(json!(hub.sns_download_image(&url, out.as_deref()).await?))
        }
        SnsSubcommand::BlockDelete { action } => match action {
            TriggerAction::Check => hub.sns_block_delete_status(),
            TriggerAction::Install => hub.sns_block_delete_install(),
            TriggerAction::Uninstall => hub.sns_block_delete_uninstall(),
        },
        SnsSubcommand::Delete { post_id } => hub.sns_delete_post(&post_id),
    }
}

/// Writes what `sns media` fetched to `path`: the bytes it holds, or a copy of its file in the cache when only the
/// file was kept (a video found in the cache). Returns the size written.
fn save_media(fetched: &SnsMediaFetch, path: &Path) -> AppResult<usize> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::runtime(format!("failed to create {}: {e}", parent.display()))
        })?;
    }
    let written =
        |e: std::io::Error| AppError::runtime(format!("failed to write {}: {e}", path.display()));
    match (&fetched.data, &fetched.cache_path) {
        (Some(data), _) => {
            std::fs::write(path, data).map_err(written)?;
            Ok(data.len())
        }
        (None, Some(cached)) => {
            let copied = std::fs::copy(cached, path).map_err(written)?;
            Ok(usize::try_from(copied).unwrap_or(usize::MAX))
        }
        (None, None) => Err(AppError::runtime("the media came back empty")),
    }
}

/// The size of what `sns media` fetched: the bytes it holds, or its file in the cache.
fn media_size(fetched: &SnsMediaFetch) -> AppResult<usize> {
    match (&fetched.data, &fetched.cache_path) {
        (Some(data), _) => Ok(data.len()),
        (None, Some(cached)) => {
            let len = std::fs::metadata(cached)
                .map_err(|e| {
                    AppError::runtime(format!("failed to read {}: {e}", cached.display()))
                })?
                .len();
            Ok(usize::try_from(len).unwrap_or(usize::MAX))
        }
        (None, None) => Ok(0),
    }
}
