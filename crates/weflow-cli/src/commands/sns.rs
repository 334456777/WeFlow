//! `weflow sns`: Moments, their media and the block-delete trigger.

use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

use serde_json::{json, Value};
use weflow_core::error::{AppError, AppResult};
use weflow_core::services::ServiceHub;

#[derive(Args, Debug)]
pub(crate) struct SnsCommand {
    #[command(subcommand)]
    pub(crate) command: SnsSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum SnsSubcommand {
    /// Moments timeline (newest first)
    Timeline {
        /// Number of posts
        #[arg(long, default_value_t = 20)]
        limit: i32,
        /// Number of posts to skip
        #[arg(long, default_value_t = 0)]
        offset: i32,
        /// Only posts of these users (repeatable)
        #[arg(long = "user")]
        users: Vec<String>,
        /// Only posts containing this text
        #[arg(long)]
        keyword: Option<String>,
        /// Only posts from this time on (Unix seconds, 0 = no limit)
        #[arg(long, default_value_t = 0)]
        start: i64,
        /// Only posts up to this time (Unix seconds, 0 = no limit)
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
    /// Export statistics (total posts / friends / mine); --fast reads the cached counts only
    Stats {
        /// Only read the cached counts
        #[arg(long)]
        fast: bool,
    },
    /// Post counts per user, or the statistics of one user
    PostCounts {
        /// Statistics of this user only
        #[arg(long)]
        user: Option<String>,
        /// Prefer the cached counts
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
        /// Only posts of these users (repeatable)
        #[arg(long = "user")]
        users: Vec<String>,
        /// Only posts containing this text
        #[arg(long)]
        keyword: Option<String>,
        /// Only posts from this time on (Unix seconds, 0 = no limit)
        #[arg(long, default_value_t = 0)]
        start: i64,
        /// Only posts up to this time (Unix seconds, 0 = no limit)
        #[arg(long, default_value_t = 0)]
        end: i64,
        /// Also save images / live photos / videos next to the export
        #[arg(long)]
        media: bool,
    },
    /// Fetch (and decrypt, with --key) a Moments image or video
    Media {
        /// Resource URL
        url: String,
        /// Decryption key
        #[arg(long)]
        key: Option<String>,
        /// Output file (default: print base64 in the result)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Download a Moments sticker (plain or AES-GCM encrypted)
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
        /// Output file (default: print base64 in the result)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Moments block-delete trigger (installing and removing are refused: the database is read-only)
    BlockDelete {
        /// check, install or uninstall
        action: TriggerAction,
    },
    /// Delete a Moments post (always refused: the database is read-only)
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
            let data = fetched.data.unwrap_or_default();
            match out {
                Some(path) => {
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            AppError::runtime(format!("failed to create {}: {e}", parent.display()))
                        })?;
                    }
                    std::fs::write(&path, &data).map_err(|e| {
                        AppError::runtime(format!("failed to write {}: {e}", path.display()))
                    })?;
                    Ok(json!(weflow_core::contract::SnsMediaFetched {
                        out: Some(path.to_string_lossy().into_owned()),
                        content_type: fetched.content_type,
                        bytes: data.len(),
                        cache_path: fetched.cache_path,
                    }))
                }
                None => Ok(json!(weflow_core::contract::SnsMediaFetched {
                    out: None,
                    content_type: fetched.content_type,
                    bytes: data.len(),
                    cache_path: fetched.cache_path,
                })),
            }
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
