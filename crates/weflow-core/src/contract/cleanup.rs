//! WeFlow's cache: what it takes and what clearing it removes.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Serialize;

/// One part of WeFlow's cache (`cache list`), and how much of the disk it takes.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct CacheUsage {
    /// Its name, as `cache clear` takes it.
    pub part: crate::services::CachePart,
    /// What `cache clear` removes; for `runtime`, the runtime directories and ffmpeg builds earlier versions left.
    pub paths: Vec<PathBuf>,
    /// What `paths` take on disk.
    pub bytes: u64,
    /// How many files `paths` hold.
    pub files: u64,
}

/// What `cache clear` removed.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CacheCleared {
    /// The files and folders removed.
    pub removed_paths: Vec<String>,
    /// What the removed files took on disk.
    pub freed_bytes: u64,
    /// What could not be removed, and why; left out when everything was.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// What `cache clear-account` removed.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountCleared {
    /// The files and folders removed: WeFlow's caches of the account, and the exports named after it.
    pub removed_paths: Vec<String>,
    /// Whether the account was taken out of the profile (`db_path`, `wxid`, the keys).
    pub profile_reset: bool,
    /// What could not be removed, and why; left out when everything was.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// What `cache clear` or `clear-account` would remove (`--dry-run`), for a page that asks first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemovalPlan {
    /// Always true.
    pub dry_run: bool,
    /// The files and folders it would remove.
    pub paths: Vec<PlannedRemoval>,
    /// What they all take on disk.
    pub bytes: u64,
    /// The profile keys it would unset (`clear-account`).
    pub profile_keys: Vec<String>,
}

/// A file or folder a removal would take.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PlannedRemoval {
    /// The file or folder.
    pub path: PathBuf,
    /// What it takes on disk; a folder's is that of the files under it.
    pub bytes: u64,
}

/// A removal the terminal was asked to confirm and was not.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Cancelled {
    /// Always true.
    pub cancelled: bool,
}
