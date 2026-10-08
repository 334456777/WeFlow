//! Read-only kvcomm probe using the production Rust acquisition path.
//! Usage: image_code_probe <PRIVATE CLI config> <kvcomm directory> [...]
//! Prints diagnostics and verification booleans only, never codes, identities or keys.
use std::path::{Path, PathBuf};
use weflow_core::config::{resolve_account_dir, ConfigStore};
use weflow_core::image_keys::{acquire_image_keys, DEFAULT_SCAN_BUDGET};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let config_path = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("private config required"))?;
    let config = ConfigStore::load(Path::new(&config_path))?;
    let profile = config
        .profile(None)
        .ok_or_else(|| anyhow::anyhow!("profile required"))?;
    let directories: Vec<PathBuf> = args.map(PathBuf::from).collect();
    let account = resolve_account_dir(
        profile.db_path.as_deref().unwrap_or(""),
        profile.wxid.as_deref().unwrap_or(""),
    );
    let result = acquire_image_keys(
        &directories,
        &account,
        profile.wxid.as_deref(),
        DEFAULT_SCAN_BUDGET,
    )?;
    println!(
        "{}",
        serde_json::json!({
            "candidate_codes": result["collection"]["candidate_codes"],
            "scan": result["scan"],
            "aes_verified": result["aes_verified"],
            "xor_verified": result["xor_verified"],
            "verified": result["verified"],
        })
    );
    Ok(())
}
