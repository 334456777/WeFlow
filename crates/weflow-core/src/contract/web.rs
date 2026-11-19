//! The WebUI versions the `web` commands list, download and choose.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Serialize;

/// What `web info` answers: the WebUI `serve` shows, and the versions downloaded.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebInfo {
    /// The version `serve` shows when it starts: one kept here that suits this weflow; null when there is none.
    pub in_use: Option<String>,
    /// The folder of `in_use`; null when there is none.
    pub path: Option<PathBuf>,
    /// The version chosen with `web use`.
    pub picked: Option<String>,
    /// The version this weflow was built for.
    pub built_in: String,
    /// The version `serve` would show: downloaded beside it when it is not kept here.
    pub wanted: Option<String>,
    /// Whether `wanted` is kept here.
    pub downloaded: bool,
    /// Where the versions are kept.
    pub store: PathBuf,
    /// The versions kept there.
    pub installed: Vec<String>,
    /// Why `serve` passes over the version picked (it does not suit this weflow) and what to do about it; null when
    /// it does not.
    pub note: Option<String>,
}

/// A downloaded WebUI version (`web list`, `web install`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebInstalled {
    /// The WebUI's version, as `web use` takes it.
    pub version: String,
    /// The API version it was built for (`weflow.apiVersion` of its `package.json`).
    pub api_version: String,
    /// The oldest weflow it works with (`weflow.minBackendVersion`).
    pub min_backend_version: String,
    /// Its folder.
    pub path: PathBuf,
    /// The one `serve` shows when it starts.
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
    /// The WebUI's version, as `web install` takes it.
    pub version: String,
    /// The size of its download, in bytes.
    pub size: u64,
    /// The API version it was built for; null when its metadata leaves it out.
    pub api_version: Option<String>,
    /// The oldest weflow it works with; null when its metadata leaves it out.
    pub min_backend_version: Option<String>,
    /// Whether it suits this weflow.
    pub suits: bool,
    /// Why it does not suit; null when it does.
    pub why: Option<String>,
    /// Whether it is kept here already.
    pub downloaded: bool,
}

/// The WebUI version `web use` chose.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WebChosen {
    /// The version, downloaded first when it was not kept here.
    pub version: String,
    /// Its folder.
    pub path: PathBuf,
}

/// The WebUI version `web update` moved to.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WebUpdated {
    /// The newest published release that suits this weflow, else the newest pre-release that does.
    pub version: String,
    /// The version wanted before; null when there was none.
    pub before: Option<String>,
    /// Its folder.
    pub path: PathBuf,
}

/// What `web prune` did.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WebPruned {
    /// The versions removed: all but the one chosen and the one this weflow was built for.
    pub removed: Vec<String>,
    /// The versions kept after it.
    pub kept: Vec<String>,
    /// Versions a running `serve` shows, kept until it stops.
    pub shown: Vec<String>,
}
