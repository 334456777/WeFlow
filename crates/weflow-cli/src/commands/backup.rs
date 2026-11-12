//! `weflow backup`: create, inspect and restore backups of the current account.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use serde_json::{json, Value};
use weflow_core::error::AppResult;
use weflow_core::services::ServiceHub;

#[derive(Args, Debug)]
pub(crate) struct BackupCommand {
    #[command(subcommand)]
    command: BackupSubcommand,
}

#[derive(Subcommand, Debug)]
enum BackupSubcommand {
    /// Create a backup of the current account
    Create {
        /// Output file
        #[arg(long)]
        out: PathBuf,
        /// Also back up the images WeChat keeps (msg/attach)
        #[arg(long)]
        images: bool,
        /// Also back up the videos WeChat keeps (msg/video)
        #[arg(long)]
        videos: bool,
        /// Also back up the files sent and received (msg/file)
        #[arg(long)]
        files: bool,
        /// Also back up the stickers WeFlow downloaded (its sticker cache)
        #[arg(long)]
        emojis: bool,
    },
    /// Show what a backup contains
    Inspect {
        /// Backup file
        path: PathBuf,
    },
    /// Restore a backup into a folder
    Restore {
        /// Backup file
        path: PathBuf,
        /// Folder to restore into
        #[arg(long)]
        target: Option<PathBuf>,
    },
}

pub(crate) fn handle_backup(command: &BackupCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        BackupSubcommand::Create {
            out,
            images,
            videos,
            files,
            emojis,
        } => {
            let options = weflow_core::backup::BackupOptions {
                images: *images,
                videos: *videos,
                files: *files,
                emojis: *emojis,
            };
            Ok(json!(hub.backup_create(out, &options)?))
        }
        BackupSubcommand::Inspect { path } => Ok(json!(hub.backup_inspect(path)?)),
        BackupSubcommand::Restore { path, target } => {
            let target_dir = target.as_deref().unwrap_or_else(|| Path::new("."));
            Ok(json!(hub.backup_restore(path, target_dir)?))
        }
    }
}
