use std::process::Command;

fn main() {
    let commit = Command::new("git").args(["rev-parse", "--short", "HEAD"]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_else(|| "unknown".into());
    let date = Command::new("date").args(["-u", "+%Y-%m-%d"]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=WEFLOW_BUILD_INFO={commit} {date}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}
