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
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-changed=../../.git/refs/tags");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
    println!("cargo:rerun-if-changed=src");
}
