pub mod cipher_vfs;
pub mod contact;
pub mod db;
#[cfg(any(test, feature = "test-fixtures"))]
pub mod fixture;
mod leaf_scan;
pub mod media;
pub mod msg;
pub mod msg_row;
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
