use std::path::{Path, PathBuf};

use weflow_native::msg_row::MsgRow;

use crate::config::{clean_account_dir_name, AppContext, ConfigStore, ServeSettings};
use crate::error::{AppError, AppResult};

mod account;
mod analytics;
mod api;
mod backup;
mod chat;
mod cleanup;
mod export;
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
mod sns;
mod voice;
pub use chat::ResourceQuery;
pub use export::{MessageExportRequest, MESSAGE_EXPORT_FORMATS};
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

    pub fn runtime_dir(&self) -> &Path {
        &self.ctx.runtime_dir
    }

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

    pub fn emit_progress(&self, stage: &str, message: &str, current: usize, total: usize) {
        crate::output::progress(stage, message, current, total);
    }
}

/// A temporary name next to `path`, unique in this process.
fn temporary_sibling(path: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{}-{n}.tmp", std::process::id()));
    PathBuf::from(name)
}

/// Writes `bytes` to `path` through a temporary file renamed into place. Another thread never sees the file half
/// written (an export's media threads check for an image another one is still writing, issue #52), and two that
/// write it at once both leave a complete file.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = temporary_sibling(path);
    let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// [`write_atomically`] for a copy of `from`.
fn copy_atomically(from: &Path, path: &Path) -> std::io::Result<()> {
    let tmp = temporary_sibling(path);
    let copied = std::fs::copy(from, &tmp).and_then(|_| std::fs::rename(&tmp, path));
    if copied.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    copied
}

/// `normalizeTimestamp` of the WCDB wrapper: ms → s, clamped to i32.
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

/// `normalizeRange`: open end → now, end never before begin.
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

#[cfg(test)]
mod tests {

    #[test]
    fn atomic_writes_replace_the_file_and_leave_no_temporary_file() {
        let dir = weflow_native::fixture::TestDir::new("weflow-atomic");
        let path = dir.join("a.jpg");
        super::write_atomically(&path, b"first").unwrap();
        super::write_atomically(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let src = dir.join("src.bin");
        std::fs::write(&src, b"copied").unwrap();
        let copy = dir.join("b.jpg");
        super::copy_atomically(&src, &copy).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), b"copied");
        // a write that fails (no such folder) leaves nothing behind either
        assert!(super::write_atomically(&dir.join("missing").join("c.jpg"), b"x").is_err());
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["a.jpg", "b.jpg", "src.bin"]);
    }
}
