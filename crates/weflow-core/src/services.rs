//! The services the CLI and the command API call, one module per domain, each an `impl` block of [`ServiceHub`].

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
pub use cleanup::{disk_usage, CachePart};
mod footprint;
mod group;
mod image;
mod insight;
pub use image::{ImageBatch, ImagePayload, ImageResult, ImageVariant};
pub use insight::Trigger as InsightTrigger;
mod key;
mod media;
mod reports;
mod sessions;
mod sns;
mod voice;
pub use chat::ResourceQuery;
pub use export::{MessageExportRequest, MESSAGE_EXPORT_FORMATS};
pub use group::MEMBER_MESSAGES_LIMIT;
pub use key::interrupt_key_db;
pub use sns::{SnsExportOptions, SnsMediaFetch, SnsProxyResult, SnsTimelineQuery};

/// Whether a read may answer with what WeFlow kept of an earlier answer, or must ask the databases again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Freshness {
    /// What was kept will do while it has not expired.
    Cached,
    /// Ask the databases again.
    #[default]
    Fresh,
}

impl Freshness {
    /// [`Cached`](Self::Cached) when `cached`, as a switch such as `--prefer-cache` asks.
    pub fn cached_if(cached: bool) -> Self {
        if cached {
            Self::Cached
        } else {
            Self::Fresh
        }
    }
}

/// The account of a profile, its config and the state its services keep between calls.
///
/// Clones share that state; each has its own copy of the config.
#[derive(Clone)]
pub struct ServiceHub {
    ctx: AppContext,
    config: ConfigStore,
    profile_name: String,
    sns_state: std::sync::Arc<std::sync::Mutex<sns::SnsState>>,
    /// Statistics being counted right now (see [`analytics::Counting`]).
    analytics_counting: std::sync::Arc<analytics::Counting>,
    image_state: std::sync::Arc<std::sync::Mutex<image::ImageState>>,
    insight_state: std::sync::Arc<std::sync::Mutex<insight::InsightState>>,
}

/// The profile's name: the config holds the keys.
impl std::fmt::Debug for ServiceHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceHub")
            .field("profile_name", &self.profile_name)
            .finish_non_exhaustive()
    }
}

impl ServiceHub {
    /// A hub over the account of the config's current profile.
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

    /// The config file this hub was loaded from.
    pub fn config_path(&self) -> &Path {
        &self.ctx.config_path
    }

    /// The options of `weflow serve` saved in the config file (#406).
    pub fn serve_settings(&self) -> &ServeSettings {
        &self.config.serve
    }

    /// Where this hub keeps its files, and its version.
    pub fn context(&self) -> &AppContext {
        &self.ctx
    }

    /// The WebUI versions kept on this machine (issue #148): with WeFlow's other caches, in
    /// `<home>/cache/web/<version>/`, the one picked with `weflow web use` named in `<home>/cache/web/current`.
    pub fn web_store(&self) -> crate::webui::WebStore {
        crate::webui::WebStore::new(self.ctx.cache_dir().join("web"))
    }

    /// Reports the progress of a long command (see [`crate::output::progress`]).
    pub fn emit_progress(&self, stage: &str, message: &str, current: usize, total: usize) {
        crate::output::progress(stage, message, current, total);
    }
}

/// A time in seconds: ms → s, clamped to i32.
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

/// A time range in seconds: open end → now, end never before begin.
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
    /// Raw configuration value (`message_push_filter_mode`, …) of the active profile.
    pub fn config_value(&self, key: &str) -> Value {
        self.config.get_key(Some(&self.profile_name), key)
    }

    /// The configured account's databases, opened; the key is proved unless it opened the same databases before.
    ///
    /// # Errors
    /// As [`Self::db_test`].
    pub fn open_wcdb_pub(&self) -> AppResult<weflow_native::wcdb::Wcdb> {
        self.open_wcdb()
    }

    /// The account's wxid without the suffix of its folder's name; empty when it is not set.
    pub fn my_wxid_pub(&self) -> String {
        self.my_wxid_cleaned()
    }

    /// Where the message push keeps the avatars it serves, `push-avatar-files` in the cache folder.
    pub fn push_avatar_dir(&self) -> PathBuf {
        self.cache_base().join("push-avatar-files")
    }
}
