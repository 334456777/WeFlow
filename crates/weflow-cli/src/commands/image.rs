//! `weflow image`: decrypting images and the automatic download hook.

use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub(crate) struct ImageCommand {
    #[command(subcommand)]
    pub(crate) command: ImageSubcommand,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct ImageTarget {
    /// Conversation the image belongs to (locates msg/attach/<md5(session)>/…)
    #[arg(long)]
    session: Option<String>,
    /// Image md5 from the message
    #[arg(long)]
    md5: Option<String>,
    /// Image .dat file name
    #[arg(long)]
    dat_name: Option<String>,
    /// Message create time (seconds); selects the year-month folder
    #[arg(long)]
    create_time: Option<i64>,
    /// Return a file path instead of a base64 data URL
    #[arg(long)]
    prefer_file_path: bool,
    /// Do not fall back to scanning by .dat name
    #[arg(long)]
    no_cache_index: bool,
}

impl ImageTarget {
    pub(crate) fn payload(&self, force: bool) -> weflow_core::services::ImagePayload {
        weflow_core::services::ImagePayload {
            session_id: self.session.clone(),
            image_md5: self.md5.clone(),
            image_dat_name: self.dat_name.clone(),
            create_time: self.create_time,
            prefer_file_path: self.prefer_file_path,
            allow_cache_index: if self.no_cache_index {
                Some(false)
            } else {
                None
            },
            force,
        }
    }
}

#[derive(Subcommand, Debug)]
pub(crate) enum ImageSubcommand {
    /// Decrypt an image into the image cache. Without --force a cached thumbnail may be returned; with --force the HD original is used when it is on disk (`export media` always behaves like --force)
    Decrypt {
        #[command(flatten)]
        target: ImageTarget,
        /// Prefer the HD rendition
        #[arg(long)]
        force: bool,
    },
    /// Look an image up in the cache without decrypting
    ResolveCache {
        #[command(flatten)]
        target: ImageTarget,
    },
    /// Resolve many images; takes a JSON array of payloads (sessionId, imageMd5, imageDatName, createTime, …)
    ResolveBatch {
        /// JSON array of image payloads
        payloads_json: String,
    },
    /// Windows only: make WeChat download original-size images (image auto-download hook)
    AutoDownload {
        #[command(subcommand)]
        command: AutoDownloadSubcommand,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum AutoDownloadSubcommand {
    /// Hook WeChat and keep the hook alive until interrupted (re-hooks after a WeChat restart)
    Start {
        /// Conversation ids to download originals for (default: `autoDownloadWhitelist` from the config)
        #[arg(long, value_delimiter = ',')]
        whitelist: Vec<String>,
    },
    /// Show whether the image auto-download hook is running
    Status,
}
