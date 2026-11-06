pub mod api;
pub mod backup;
pub mod biz;
pub mod chat_msg;
pub mod collate;
pub mod config;
pub mod contact_extra;
pub mod contract;
pub mod dates;
pub mod decrypt;
pub mod error;
pub mod export;
pub mod export_msg;
pub mod hevc;
pub mod http_client;
pub mod image;
pub mod image_download;
pub mod image_keys;
pub mod insight;
pub mod isaac64;
pub mod keys;
pub mod locale;
pub mod media;
pub mod message;
pub mod messages_zh;
pub mod num;
pub mod output;
pub mod process;
pub mod push;
pub mod reads;
pub mod render;
pub mod services;
pub mod sns;
pub mod sync;
pub mod video;
pub mod webui;
pub mod weibo;
pub mod xml;

pub use config::{AppContext, ConfigStore, ProfileConfig};
pub use error::{AppError, AppResult};

/// The database keys `weflow serve` hands to the commands it runs, so they do not derive them again.
pub mod page_keys {
    pub use weflow_native::sqlcipher::{export_keys, import_keys};
}

/// The configuration of a property test (#502): `cases` cases, or `PROPTEST_CASES` of them when it is set (to run
/// more than CI does). A failure prints its smallest input; it is not saved into the source tree.
#[cfg(test)]
pub(crate) fn property_cases(cases: u32) -> proptest::test_runner::Config {
    let mut config = proptest::test_runner::Config::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = cases;
    }
    config.failure_persistence = None;
    config
}
