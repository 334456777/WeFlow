//! The WebUI versions the `web` commands list, download and choose.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Serialize;

/// What `web info` answers: the WebUI `serve` shows, and the versions downloaded.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebInfo {
    /// The version `serve` shows; null when none is downloaded.
    pub in_use: Option<String>,
    pub path: Option<PathBuf>,
    /// The version chosen with `web use`.
    pub picked: Option<String>,
    /// The version this weflow was built for.
    pub built_in: String,
    /// The version `serve` would show.
    pub wanted: Option<String>,
    pub downloaded: bool,
    /// Where the versions are kept.
    pub store: PathBuf,
    pub installed: Vec<String>,
}

/// A downloaded WebUI version (`web list`, `web install`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebInstalled {
    pub version: String,
    pub api_version: String,
    pub min_backend_version: String,
    pub path: PathBuf,
    /// The one `serve` shows.
    pub in_use: bool,
    /// The one chosen with `web use`.
    pub picked: bool,
    /// Whether it suits this weflow.
    pub suits: bool,
}

/// A published WebUI version (`web list --remote`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebPublished {
    pub version: String,
    /// Bytes.
    pub size: u64,
    pub api_version: Option<String>,
    pub min_backend_version: Option<String>,
    pub suits: bool,
    /// Why it does not suit; null when it does.
    pub why: Option<String>,
    pub downloaded: bool,
}

/// The WebUI version `web use` chose.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WebChosen {
    pub version: String,
    pub path: PathBuf,
}

/// The WebUI version `web update` moved to.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WebUpdated {
    pub version: String,
    /// The version wanted before; null when there was none.
    pub before: Option<String>,
    pub path: PathBuf,
}

/// What `web prune` did.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WebPruned {
    pub removed: Vec<String>,
    pub kept: Vec<String>,
    /// Versions a running `serve` shows, kept until it stops.
    pub shown: Vec<String>,
}
