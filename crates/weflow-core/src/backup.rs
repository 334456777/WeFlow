//! Backups of an account: `backup create` ([`create_backup`]), `backup inspect` ([`inspect_backup`]) and
//! `backup restore` ([`restore_backup`]), and the manifest a backup carries.

use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

mod create;
mod restore;

pub use create::create_backup;
pub use restore::restore_backup;

const MANIFEST_NAME: &str = "weflow_backup_manifest.json";

/// Where WeFlow's sticker cache is kept in a backup.
const EMOJI_FOLDER: &str = "emojis";

/// What a backup holds besides the databases (`db_storage/`, always there; the voice messages are in them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackupOptions {
    /// The images WeChat keeps: `msg/attach/`.
    pub images: bool,
    /// The videos WeChat keeps: `msg/video/`.
    pub videos: bool,
    /// The files sent and received: `msg/file/`.
    pub files: bool,
    /// WeFlow's cache of the stickers it downloaded, kept in the backup as `emojis/`.
    pub emojis: bool,
}

/// A file in a backup.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BackupEntry {
    /// Its path in the backup: `db_storage/…`, `msg/attach/…`, `msg/video/…`, `msg/file/…` or `emojis/…`; a backup
    /// of an older WeFlow can also hold `FileStorage/…`.
    pub path: String,
    pub sha256: String,
    /// Bytes.
    pub size: u64,
}

/// What a backup holds (`backup inspect`): the manifest stored in the archive.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BackupManifest {
    /// The manifest's own format.
    pub version: String,
    /// Unix seconds.
    pub created_at: u64,
    pub wxid: String,
    /// The WeFlow that wrote it.
    pub weflow_version: String,
    pub entries: Vec<BackupEntry>,
    /// Files of the media and stickers that could not be read and are not in the backup. Left out when there are
    /// none, and in a backup of an older WeFlow.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SkippedFile>,
    /// Databases (their path in the backup) WeChat kept writing to while they were copied: their main file and WAL
    /// in the backup may be of two moments. Left out when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsettled: Vec<String>,
}

/// A file left out of a backup because it could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SkippedFile {
    /// Its path in the backup, as it would have been.
    pub path: String,
    /// What the system said.
    pub reason: String,
}

pub fn inspect_backup(archive_path: &Path) -> Result<BackupManifest> {
    let file =
        fs::File::open(archive_path).with_context(|| format!("open {}", archive_path.display()))?;
    let mut zip = ZipArchive::new(file).context("open zip archive")?;
    let mut entry = zip
        .by_name(MANIFEST_NAME)
        .context("manifest not found in archive")?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).context("read manifest")?;
    serde_json::from_slice(&buf).context("parse manifest")
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex_digest(&h.finalize())
}

fn hex_digest(digest: &[u8]) -> String {
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
