//! WeFlow 的缓存：各部分占用多少、清理时删除什么，以及没有得到确认的删除。

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Serialize;

/// WeFlow 缓存中单独列出、单独清理的一部分。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CachePart {
    /// 解密后的图片（`Images`）。
    Images,
    /// 解码后的语音（`Voices`）。
    Voices,
    /// 下载的表情（`Emojis`）。
    Emojis,
    /// 取回的朋友圈媒体（`sns_cache`）。
    Sns,
    /// 保存的统计结果（`analytics_cache.json`）。
    Analytics,
    /// `weflow serve` 和消息推送为客户端写的文件（`api-media`、`push-avatar-files` 和 `serve-files`），
    /// 运行中的 `weflow serve` 正在用的文件夹除外。
    Api,
    /// 校验通过的密钥的指纹（`verified-keys`）。
    Keys,
    /// 早期版本在 WeFlow 的文件夹中留下的运行时目录（`runtime/`）和 ffmpeg（`ffmpeg/<版本>/`）。
    Runtime,
    /// 下载的网页版本，运行中的 `weflow serve` 正在显示的除外。
    Web,
}

impl CachePart {
    /// 全部部分，按列出的顺序排列；`cache clear --all` 清理全部。
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

    /// JSON 输出中的名字，也是 `weflow cache clear` 的选项名（`--images` 等）。
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

/// WeFlow 缓存的一部分（`cache list`），以及它占用的磁盘空间。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct CacheUsage {
    /// 这一部分的名字，即 `cache clear` 接受的写法。
    pub part: CachePart,
    /// `cache clear` 删除的文件和文件夹，不存在的也列出；`runtime` 和 `web` 只列出存在的。
    pub paths: Vec<PathBuf>,
    /// `paths` 中文件的总字节数。
    pub bytes: u64,
    /// `paths` 中的文件数。
    pub files: u64,
}

/// `cache clear` 删除了什么。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CacheCleared {
    /// 删除的文件和文件夹。
    pub removed_paths: Vec<String>,
    /// 删除的文件原来的总字节数。
    pub freed_bytes: u64,
    /// 没有删掉的路径及原因；全部删掉时省略。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// `cache clear-account` 删除了什么。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountCleared {
    /// 删除的文件和文件夹：WeFlow 为这个账号保存的缓存和见解记录，以及以账号命名的导出。
    pub removed_paths: Vec<String>,
    /// 账号是否已移出档案（`db_path`、`wxid`、数据库密钥和图片密钥），总是 `true`。
    pub profile_reset: bool,
    /// 没有删掉的路径及原因；全部删掉时省略。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// `cache clear` 或 `cache clear-account` 将要删除的内容（`--dry-run`），供页面在删除前询问。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemovalPlan {
    /// 总是 `true`。
    pub dry_run: bool,
    /// 将要删除的文件和文件夹，只列出存在的。
    pub paths: Vec<PlannedRemoval>,
    /// 它们的总字节数。
    pub bytes: u64,
    /// 将要清除的档案键（`clear-account`）；`cache clear` 时为空。
    pub profile_keys: Vec<String>,
}

/// 将要删除的一个文件或文件夹。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PlannedRemoval {
    /// 文件或文件夹。
    pub path: PathBuf,
    /// 占用的字节数；文件夹为其中所有文件的总和。
    pub bytes: u64,
}

/// 在终端上要求确认、但没有得到确认的删除。
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
pub struct Cancelled {
    /// 总是 `true`。
    pub cancelled: bool,
}
