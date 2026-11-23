//! WeFlow's read-only database layer: WeChat's SQLCipher databases read in pure Rust with a bundled SQLite.
//!
//! [`cipher_vfs`] and [`sqlcipher`] decrypt the pages as SQLite reads them, [`db::Account`] is the handle of an
//! account, and [`msg`], [`contact`], [`sns`], [`media`], [`stats`] and [`report`] query one domain each.
//! [`wcdb::Wcdb`] is the facade the service layer calls, and [`wxkey`] gets the database key from a running WeChat.

pub mod cipher_vfs;
pub mod contact;
pub mod db;
pub mod entities;
#[cfg(any(test, feature = "test-fixtures"))]
pub mod fixture;
pub mod image_key_scan;
mod kdf;
mod leaf_scan;
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
pub mod linux_key_helper;
#[cfg(any(test, target_os = "macos"))]
pub mod macos_key_helper;
pub mod media;
pub mod msg;
pub mod msg_row;
pub mod quote;
pub mod report;
pub mod sns;
pub mod sqlcipher;
pub mod stats;
pub mod wcdb;
#[cfg(any(test, all(windows, target_arch = "x86_64")))]
pub mod windows_db_key;
pub mod wxkey;

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
