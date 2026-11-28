//! WeFlow's cache: what it takes and what clearing it removes.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Serialize;

/// A part of WeFlow's cache that is listed and cleared on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CachePart {
    /// The images decrypted (`Images`).
    Images,
    /// The voices decoded (`Voices`).
    Voices,
    /// The stickers downloaded (`Emojis`).
    Emojis,
    /// The Moments media fetched (`sns_cache`).
    Sns,
    /// The analytics' saved results (`analytics_cache.json`).
    Analytics,
    /// The files `weflow serve` and the message push wrote for clients (`api-media`, `push-avatar-files` and the
    /// served files).
    Api,
    /// The fingerprints of the keys that opened the databases (`verified-keys`).
    Keys,
    /// Runtime directories and ffmpeg builds earlier versions left in the home folder.
    Runtime,
    /// The WebUI versions downloaded, but the ones a running `weflow serve` shows.
    Web,
}

impl CachePart {
    /// Every part, in the order they are listed; `cache clear --all` clears them all.
    pub const ALL: [CachePart; 9] = [
        CachePart::Images,
        CachePart::Voices,
        CachePart::Emojis,
        CachePart::Sns,
        CachePart::Analytics,
        CachePart::Api,
        CachePart::Keys,
        CachePart::Runtime,
        CachePart::Web,
    ];

    /// Name in the JSON output; also the option of `weflow cache clear` (`--images`, …).
    pub fn name(self) -> &'static str {
        match self {
            CachePart::Images => "images",
            CachePart::Voices => "voices",
            CachePart::Emojis => "emojis",
            CachePart::Sns => "sns",
            CachePart::Analytics => "analytics",
            CachePart::Api => "api",
            CachePart::Keys => "keys",
            CachePart::Runtime => "runtime",
            CachePart::Web => "web",
        }
    }
}

/// One part of WeFlow's cache (`cache list`), and how much of the disk it takes.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct CacheUsage {
    /// Its name, as `cache clear` takes it.
    pub part: CachePart,
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
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
pub struct Cancelled {
    /// Always true.
    pub cancelled: bool,
}
