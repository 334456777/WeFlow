//! CLI 和命令 API 调用的服务：每个领域一个模块，各是 [`ServiceHub`] 的一个 `impl` 块。

use std::path::{Path, PathBuf};

use serde_json::Value;
use weflow_native::msg_row::MsgRow;

use crate::config::{clean_account_dir_name, AppContext, ConfigStore, ServeSettings};
use crate::error::{AppError, AppResult};

mod account;
mod analytics;
mod backup;
mod chat;
mod cleanup;
mod export;
mod export_media;
mod export_stream;
pub use cleanup::disk_usage;
mod footprint;
mod group;
mod identity;
mod image;
mod insight;
pub use image::{ImageBatch, ImagePayload, ImageResult, ImageVariant};
pub use insight::Trigger as InsightTrigger;
mod key;
mod media;
mod reports;
mod session_kind;
mod sessions;
mod sns;
mod voice;
pub use chat::ResourceQuery;
pub use export::{MessageExportRequest, MESSAGE_EXPORT_FORMATS};
pub use group::MEMBER_MESSAGES_LIMIT;
pub use key::interrupt_key_db;
pub use sns::{SnsExportOptions, SnsMediaFetch, SnsProxyResult, SnsTimelineQuery};

/// 读取能否用 WeFlow 保存的上一次的回答，还是必须重新查询数据库。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Freshness {
    /// 保存的回答没有过期时就用它。
    Cached,
    /// 重新查询数据库。
    #[default]
    Fresh,
}

impl Freshness {
    /// `cached` 为真时为 [`Cached`](Self::Cached)，对应 `--prefer-cache` 这类开关。
    pub fn cached_if(cached: bool) -> Self {
        if cached {
            Self::Cached
        } else {
            Self::Fresh
        }
    }
}

/// 一个档案的账号、配置，以及各服务在调用之间保留的状态。
///
/// 克隆出的副本共用这些状态，配置则各有一份。
#[derive(Clone)]
pub struct ServiceHub {
    ctx: AppContext,
    config: ConfigStore,
    profile_name: String,
    sns_state: std::sync::Arc<std::sync::Mutex<sns::SnsState>>,
    /// 正在进行的统计（见 [`analytics::Counting`]）。
    analytics_counting: std::sync::Arc<analytics::Counting>,
    image_state: std::sync::Arc<std::sync::Mutex<image::ImageState>>,
    insight_state: std::sync::Arc<std::sync::Mutex<insight::InsightState>>,
}

/// 只显示档案名：配置里有密钥。
impl std::fmt::Debug for ServiceHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceHub")
            .field("profile_name", &self.profile_name)
            .finish_non_exhaustive()
    }
}

impl ServiceHub {
    /// 以配置中的当前档案创建，各服务保留的状态从空开始。
    pub fn new(ctx: AppContext, config: ConfigStore) -> Self {
        let profile_name = config.current_profile.clone();
        Self {
            ctx,
            config,
            profile_name,
            sns_state: Default::default(),
            analytics_counting: Default::default(),
            image_state: Default::default(),
            insight_state: Default::default(),
        }
    }

    /// 读取的配置文件。
    pub fn config_path(&self) -> &Path {
        &self.ctx.config_path
    }

    /// 配置文件中保存的 `weflow serve` 的选项（#406）。
    pub fn serve_settings(&self) -> &ServeSettings {
        &self.config.serve
    }

    /// 存放文件的位置和这个 `weflow` 的版本。
    pub fn context(&self) -> &AppContext {
        &self.ctx
    }

    /// 本机保存的各个 WebUI 版本（#148）：与 WeFlow 的其他缓存放在一起，位于 `<home>/cache/web/<version>/`，
    /// 用 `weflow web use` 选中的版本记在 `<home>/cache/web/current` 里。
    pub fn web_store(&self) -> crate::webui::WebStore {
        crate::webui::WebStore::new(self.ctx.cache_dir().join("web"))
    }

    /// 报告耗时较长的命令的进度（见 [`crate::output::progress`]）。
    pub fn emit_progress(&self, stage: &str, message: &str, current: usize, total: usize) {
        crate::output::progress(stage, message, current, total);
    }
}

/// 换算成秒的时间：大于 10^12 的值按毫秒换算，结果限制在 `i32` 的范围内，不是正数时为 0。
pub(crate) fn normalize_timestamp(input: i64) -> i32 {
    if input <= 0 {
        return 0;
    }
    let seconds = if input > 1_000_000_000_000 {
        input / 1000
    } else {
        input
    };
    crate::num::saturating_i32(seconds.max(0))
}

/// 以秒为单位的时间范围：没有结束时间时到现在为止，结束时间不早于开始时间。
pub(crate) fn normalize_range(begin: i64, end: i64) -> (i32, i32) {
    let b = normalize_timestamp(begin);
    let mut e = normalize_timestamp(end);
    if e <= 0 {
        e = normalize_timestamp(chrono::Utc::now().timestamp_millis());
    }
    if b > 0 && e < b {
        e = b;
    }
    (b, e)
}

impl ServiceHub {
    /// 当前档案中一个配置键（如 `message_push_filter_mode`）的原始值。
    pub fn config_value(&self, key: &str) -> Value {
        self.config.get_key(Some(&self.profile_name), key)
    }

    /// 打开配置的账号的数据库；同样的密钥和数据库以前没有校验通过时，先校验密钥。
    ///
    /// # Errors
    /// 同 [`Self::db_test`]。
    pub fn open_wcdb_pub(&self) -> AppResult<weflow_native::wcdb::Wcdb> {
        self.open_wcdb()
    }

    /// 账号的裸 wxid（去掉账号目录名的后缀）；没有设置时为空。
    pub fn my_wxid_pub(&self) -> String {
        self.my_wxid_cleaned()
    }

    /// 消息推送提供的头像存放的目录：缓存目录下的 `push-avatar-files`。
    pub fn push_avatar_dir(&self) -> PathBuf {
        self.cache_base().join("push-avatar-files")
    }
}
