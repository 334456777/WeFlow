//! WeFlow 的只读数据库层：用纯 Rust 和内置的 SQLite 读取微信的 SQLCipher 数据库。
//!
//! [`cipher_vfs`] 和 [`sqlcipher`] 在 SQLite 读取页时解密，[`db::Account`] 是一个账号的句柄，[`msg`]、[`contact`]、
//! [`sns`]、[`media`]、[`stats`] 和 [`report`] 各查询一个领域。[`wcdb::Wcdb`] 是业务层调用的门面，[`wxkey`]
//! 从正在运行的微信取得数据库密钥。

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
mod sha512x4;
mod shared_file;
pub mod sns;
pub mod sqlcipher;
pub mod stats;
pub mod wcdb;
#[cfg(any(test, all(windows, target_arch = "x86_64")))]
pub mod windows_db_key;
pub mod wxkey;

/// 性质测试（#502）的配置：跑 `cases` 个用例，设了 `PROPTEST_CASES` 时按它的数目跑（用来跑得比 CI 多）。
/// 失败时打印最小的输入，不保存到源码目录里。
#[cfg(test)]
pub(crate) fn property_cases(cases: u32) -> proptest::test_runner::Config {
    let mut config = proptest::test_runner::Config::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = cases;
    }
    config.failure_persistence = None;
    config
}
