use std::process::Command;

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn main() {
    let commit = run("git", &["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = run("git", &["status", "--porcelain", "--untracked-files=no"]).map_or(false, |s| !s.is_empty());
    let stamp = run("date", &["-u", "+%Y-%m-%d %H:%MZ"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=WEFLOW_BUILD_INFO={commit}{} {stamp}", if dirty { "-dirty" } else { "" });
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-changed=src");
}
