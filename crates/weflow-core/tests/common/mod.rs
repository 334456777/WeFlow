//! Test harness: builds the mock libwcdb_api.so and a ServiceHub wired to it (Linux only).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use weflow_core::config::{AppContext, ConfigStore};
use weflow_core::services::ServiceHub;

pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("weflow-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn build_mock(runtime: &Path) {
    let lib_dir = runtime.join("wcdb/linux/x64");
    std::fs::create_dir_all(&lib_dir).unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../weflow-native/tests/fixtures/mock_wcdb.c");
    let status = Command::new("cc")
        .args(["-shared", "-fPIC", "-o"])
        .arg(lib_dir.join("libwcdb_api.so"))
        .arg(&src)
        .status()
        .expect("cc is required for the mock WCDB tests");
    assert!(status.success(), "failed to compile the mock WCDB library");
}

/// Returns a hub connected to the mock library, plus the temp root for outputs.
pub fn mock_hub(tag: &str) -> (ServiceHub, PathBuf) {
    mock_hub_with(tag, |_| {})
}

/// Like [`mock_hub`], with a hook to adjust the default profile (keys, cache path, …).
pub fn mock_hub_with(tag: &str, tweak: impl FnOnce(&mut weflow_core::config::ProfileConfig)) -> (ServiceHub, PathBuf) {
    let root = temp_dir(tag);
    let runtime = root.join("runtime");
    build_mock(&runtime);
    let account = root.join("data/wxid_me_ab12");
    std::fs::create_dir_all(account.join("db_storage/session")).unwrap();
    std::fs::write(account.join("db_storage/session/session.db"), b"").unwrap();
    let ctx = AppContext {
        home_dir: root.join("home"),
        config_path: root.join("home/config.json"),
        runtime_dir: runtime,
        version: "test".into(),
    };
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let mut config = ConfigStore::default();
    tweak(config.profiles.get_mut("default").unwrap());
    let hub = ServiceHub::new(
        ctx,
        config,
        None,
        Some(account.to_string_lossy().to_string()),
        Some("00".repeat(32)),
        Some("wxid_me_ab12".into()),
    );
    (hub, root)
}
