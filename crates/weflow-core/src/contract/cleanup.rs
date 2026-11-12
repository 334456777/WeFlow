//! WeFlow's cache: what it takes and what clearing it removes.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Serialize;

/// One part of WeFlow's cache (`cache list`), and how much of the disk it takes.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct CacheUsage {
    /// Its name, as `cache clear` takes it.
    pub part: crate::services::CachePart,
    /// What `cache clear` removes; for `runtime`, the folders of other versions and earlier ffmpeg builds found.
    pub paths: Vec<PathBuf>,
    /// What `paths` take on disk.
    pub bytes: u64,
    pub files: u64,
    /// For `runtime`: the running version's folder, which `cache clear` leaves and `bytes` does not count.
    #[serde(rename = "inUse", skip_serializing_if = "Option::is_none")]
    pub in_use: Option<PathBuf>,
}

/// What `cache clear` removed.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CacheCleared {
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
    pub paths: Vec<PlannedRemoval>,
    /// What they all take on disk.
    pub bytes: u64,
    /// The profile keys it would unset (`clear-account`).
    pub profile_keys: Vec<String>,
}

/// A file or folder a removal would take.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PlannedRemoval {
    pub path: PathBuf,
    pub bytes: u64,
}

/// A removal the terminal was asked to confirm and was not.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Cancelled {
    /// Always true.
    pub cancelled: bool,
}
