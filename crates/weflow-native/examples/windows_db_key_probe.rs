//! Read-only candidate inspection; does not attach a debugger or print keys/account data.
#[cfg(all(windows, target_arch = "x86_64"))]
fn main() -> anyhow::Result<()> {
    let pid: u32 = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("pass the WeChat PID"))?
        .parse()?;
    let start = std::time::Instant::now();
    let candidate = weflow_native::windows_db_key::Hook::inspect(pid)?;
    println!(
        "{}",
        serde_json::json!({ "version": candidate.version,
        "candidate_found": true, "module_offset": candidate.module_offset,
        "elapsed_ms": start.elapsed().as_millis(), "debugger_attached": false })
    );
    Ok(())
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn main() {
    eprintln!("Candidate inspection requires Windows x64");
}
