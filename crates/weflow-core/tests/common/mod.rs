//! Test harness: a `ServiceHub` wired to a synthetic account made of real encrypted databases
//! (`weflow_native::fixture`).
#![allow(dead_code)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::path::Path;

use weflow_core::config::{AppContext, ConfigStore};
use weflow_core::services::ServiceHub;
use weflow_native::fixture::{Fixture, TestDir};

/// A directory of the test's own, removed when it is dropped.
pub fn temp_dir(tag: &str) -> TestDir {
    TestDir::new(&format!("weflow-e2e-{tag}"))
}

/// Returns the temp root for outputs, plus a hub connected to the standard fixture account. The root comes first so
/// that `let (root, hub) = …` drops the hub, and the files it has open, before the directory.
pub fn mock_hub(tag: &str) -> (TestDir, ServiceHub) {
    mock_hub_with(tag, |_| {})
}

/// Like [`mock_hub`], with a hook to adjust the default profile (keys, cache path, …).
pub fn mock_hub_with(
    tag: &str,
    tweak: impl FnOnce(&mut weflow_core::config::ProfileConfig),
) -> (TestDir, ServiceHub) {
    let root = temp_dir(tag);
    let fixture = Fixture::standard(&root.join("data"));
    let hub = hub_for(&root, &fixture, tweak);
    (root, hub)
}

/// A hub over an account built from scratch by `build` (the account is `wxid_me_ab12`, key from the fixture).
pub fn custom_hub(tag: &str, build: impl FnOnce(&Fixture)) -> (TestDir, ServiceHub, Fixture) {
    custom_hub_with(tag, build, |_| {})
}

pub fn custom_hub_with(
    tag: &str,
    build: impl FnOnce(&Fixture),
    tweak: impl FnOnce(&mut weflow_core::config::ProfileConfig),
) -> (TestDir, ServiceHub, Fixture) {
    let root = temp_dir(tag);
    let fixture = Fixture::new(&root.join("data"), "wxid_me_ab12");
    build(&fixture);
    let hub = hub_for(&root, &fixture, tweak);
    (root, hub, fixture)
}

fn hub_for(
    root: &Path,
    fixture: &Fixture,
    tweak: impl FnOnce(&mut weflow_core::config::ProfileConfig),
) -> ServiceHub {
    hub_for_account(
        root,
        &fixture.account_dir,
        &fixture.key_hex(),
        "wxid_me_ab12",
        tweak,
    )
}

/// A hub over the account in `account_dir` (its folder name `wxid`), with outputs under `root`.
pub fn hub_for_account(
    root: &Path,
    account_dir: &Path,
    key_hex: &str,
    wxid: &str,
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
    let profile = config.profiles.get_mut("default").unwrap();
    profile.db_path = Some(account_dir.to_string_lossy().to_string());
    profile.decrypt_key = Some(key_hex.into());
    profile.wxid = Some(wxid.into());
    tweak(profile);
    ServiceHub::new(ctx, config)
}
