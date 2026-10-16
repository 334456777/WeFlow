use std::process::Command;

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Version from the git tag (`v1.2.3` -> `1.2.3`, `v1.2.3-4-gabc` between tags), falling back to Cargo.toml.
fn tag_version() -> String {
    run("git", &["describe", "--tags", "--match", "v[0-9]*"])
        .and_then(|t| t.strip_prefix('v').map(str::to_string))
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap_or_default())
}

/// `"version"` of web/package.json (its first line holding one), or empty when it is not there.
fn web_version() -> String {
    let text = std::fs::read_to_string("../../web/package.json").unwrap_or_default();
    text.lines()
        .find_map(|line| {
            let rest = line.trim().strip_prefix("\"version\":")?;
            Some(
                rest.trim()
                    .trim_end_matches(',')
                    .trim_matches('"')
                    .to_string(),
            )
        })
        .unwrap_or_default()
}

fn main() {
    println!("cargo:rustc-env=WEFLOW_VERSION={}", tag_version());
    let commit = run("git", &["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = run("git", &["status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|s| !s.is_empty());
    let stamp = run("date", &["-u", "+%Y-%m-%d %H:%MZ"]).unwrap_or_else(|| "unknown".into());
    println!(
        "cargo:rustc-env=WEFLOW_BUILD_INFO={commit}{} {stamp}",
        if dirty { "-dirty" } else { "" }
    );
    // the main thread's stack, 8 MB as on Linux and macOS: Windows gives it 1 MB, which the command tree clap
    // derives (built again by `serve`) and the commands' futures outgrow in a debug build
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
            Ok("msvc") => println!("cargo:rustc-link-arg-bin=weflow=/STACK:8388608"),
            Ok("gnu") => println!("cargo:rustc-link-arg-bin=weflow=-Wl,--stack,8388608"),
            _ => {}
        }
    }
    // the WebUI version this build runs by default (issue #148): the one in web/ next to it, checked together
    println!("cargo:rustc-env=WEFLOW_WEB_VERSION={}", web_version());
    println!("cargo:rerun-if-changed=../../web/package.json");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-changed=../../.git/refs/tags");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
    println!("cargo:rerun-if-changed=src");
}
