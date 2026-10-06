//! Test harness: a `ServiceHub` wired to a synthetic account made of real encrypted databases
//! (`weflow_native::fixture`). There is no mock database library any more.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use weflow_core::config::{AppContext, ConfigStore};
use weflow_core::services::ServiceHub;
use weflow_native::fixture::Fixture;

pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("weflow-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Returns a hub connected to the standard fixture account, plus the temp root for outputs.
pub fn mock_hub(tag: &str) -> (ServiceHub, PathBuf) {
    mock_hub_with(tag, |_| {})
}

/// Like [`mock_hub`], with a hook to adjust the default profile (keys, cache path, …).
pub fn mock_hub_with(
    tag: &str,
    tweak: impl FnOnce(&mut weflow_core::config::ProfileConfig),
) -> (ServiceHub, PathBuf) {
    let root = temp_dir(tag);
    let fixture = Fixture::standard(&root.join("data"));
    (hub_for(&root, &fixture, tweak), root)
}

/// A hub over an account built from scratch by `build` (the account is `wxid_me_ab12`, key from the fixture).
pub fn custom_hub(tag: &str, build: impl FnOnce(&Fixture)) -> (ServiceHub, PathBuf, Fixture) {
    custom_hub_with(tag, build, |_| {})
}

pub fn custom_hub_with(
    tag: &str,
    build: impl FnOnce(&Fixture),
    tweak: impl FnOnce(&mut weflow_core::config::ProfileConfig),
) -> (ServiceHub, PathBuf, Fixture) {
    let root = temp_dir(tag);
    let fixture = Fixture::new(&root.join("data"), "wxid_me_ab12");
    build(&fixture);
    (hub_for(&root, &fixture, tweak), root, fixture)
}

fn hub_for(
    root: &Path,
    fixture: &Fixture,
    tweak: impl FnOnce(&mut weflow_core::config::ProfileConfig),
) -> ServiceHub {
    // the tests compare English texts: they must not depend on the language of the machine they run on
    weflow_core::locale::set(weflow_core::locale::Lang::En);
    let ctx = AppContext {
        home_dir: root.join("home"),
        config_path: root.join("home/config.json"),
        runtime_dir: root.join("runtime"),
        version: "test".into(),
    };
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let mut config = ConfigStore::default();
    tweak(config.profiles.get_mut("default").unwrap());
    ServiceHub::new(
        ctx,
        config,
        None,
        Some(fixture.account_dir.to_string_lossy().to_string()),
        Some(fixture.key_hex()),
        Some("wxid_me_ab12".into()),
    )
}
