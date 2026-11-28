//! `web` 命令列出、下载和选定的网页版本。

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Serialize;

/// `web info` 的回答：`serve` 显示的网页版本，以及本机已有的版本。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebInfo {
    /// `serve` 启动时显示的版本，即本机已有、适用于这个 weflow 的一个版本；没有时为 `null`。
    pub in_use: Option<String>,
    /// `inUse` 的文件夹；没有时为 `null`。
    pub path: Option<PathBuf>,
    /// 用 `web use` 选定的版本；没有选定时为 `null`。
    pub picked: Option<String>,
    /// 这个 weflow 构建时对应的网页版本（默认版本）。
    pub built_in: String,
    /// `serve` 最终显示的版本：本机没有时，`serve` 在运行时下载它，下载好后换成它。
    pub wanted: Option<String>,
    /// `wanted` 是否已在本机。
    pub downloaded: bool,
    /// 存放网页版本的文件夹。
    pub store: PathBuf,
    /// 其中已有的版本。
    pub installed: Vec<String>,
    /// `serve` 不显示选定的版本（它不适用于这个 weflow）时的原因和处理办法；其他情况为 `null`。
    pub note: Option<String>,
}

/// 本机已下载的一个网页版本（`web list`、`web install`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebInstalled {
    /// 网页的版本，即 `web use` 接受的写法。
    pub version: String,
    /// 构建时对应的 API 版本（它的 `package.json` 中的 `weflow.apiVersion`）。
    pub api_version: String,
    /// 能配合使用的最低 weflow 版本（`weflow.minBackendVersion`）。
    pub min_backend_version: String,
    /// 这个版本的文件夹。
    pub path: PathBuf,
    /// 是否为 `serve` 启动时显示的版本。
    pub in_use: bool,
    /// 是否为用 `web use` 选定的版本。
    pub picked: bool,
    /// 是否适用于这个 weflow。
    pub suits: bool,
}

/// 已发布的一个网页版本（`web list --remote`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebPublished {
    /// 网页的版本，即 `web install` 接受的写法。
    pub version: String,
    /// 下载的大小，单位为字节。
    pub size: u64,
    /// 构建时对应的 API 版本；元数据中没有时为 `null`。
    pub api_version: Option<String>,
    /// 能配合使用的最低 weflow 版本；元数据中没有时为 `null`。
    pub min_backend_version: Option<String>,
    /// 是否适用于这个 weflow。
    pub suits: bool,
    /// 不适用的原因；适用时为 `null`。
    pub why: Option<String>,
    /// 本机是否已经有这个版本。
    pub downloaded: bool,
}

/// `web use` 选定的网页版本。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WebChosen {
    /// 选定的版本；本机没有时先下载。
    pub version: String,
    /// 这个版本的文件夹。
    pub path: PathBuf,
}

/// `web update` 换到的网页版本。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WebUpdated {
    /// 适用于这个 weflow 的最新正式版；没有正式版时为适用的最新预发布版。
    pub version: String,
    /// 更新前 `serve` 要显示的版本；没有时为 `null`。
    pub before: Option<String>,
    /// 这个版本的文件夹。
    pub path: PathBuf,
}

/// `web prune` 做了什么。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WebPruned {
    /// 删除的版本：选定的版本和这个 weflow 的默认版本以外的全部版本。
    pub removed: Vec<String>,
    /// 删除之后本机还有的版本。
    pub kept: Vec<String>,
    /// 运行中的 `serve` 正在显示、因此保留到它停止的版本。
    pub shown: Vec<String>,
}
