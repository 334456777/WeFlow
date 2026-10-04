//! Read-only proof of concept for kvcomm candidate collection without a helper DLL.
//! Usage: image_code_probe <PRIVATE CLI config> <kvcomm directory> [...]
//! Prints counts and verification booleans only, never codes, identities or keys.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use weflow_core::config::{resolve_account_dir, ConfigStore};
use weflow_core::keys;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let config = ConfigStore::load(Path::new(&args.next().expect("private config required")))?;
    let profile = config.profile(None).expect("profile required");
    let mut codes = BTreeSet::new();
    let mut files = 0;
    for dir in args {
        for entry in std::fs::read_dir(PathBuf::from(dir))?.flatten() {
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("key_") {
                continue;
            }
            files += 1;
            for token in name.split('_') {
                if let Ok(code) = token.parse::<u32>() {
                    if code != 0 {
                        codes.insert(code as u64);
                    }
                }
            }
        }
    }
    let account = resolve_account_dir(
        profile.db_path.as_deref().unwrap_or(""),
        profile.wxid.as_deref().unwrap_or(""),
    );
    let candidates = keys::collect_wxid_candidates(account.to_str(), profile.wxid.as_deref());
    let (template, _) = keys::find_template_data(&account, 32);
    let mut verified = 0;
    let mut matches_saved = false;
    if let Some(template) = template {
        for identity in candidates {
            for code in &codes {
                let (xor, aes) = keys::derive_image_keys(*code, &identity);
                if keys::verify_derived_aes_key(&aes, &template) {
                    verified += 1;
                    matches_saved |= profile.image_xor_key == Some(xor as i64)
                        && profile.image_aes_key.as_deref() == Some(aes.as_str());
                }
            }
        }
    }
    println!(
        "{}",
        serde_json::json!({"key_files":files,"candidate_codes":codes.len(),"template_found":template.is_some(),"verified_pairs":verified,"matches_saved_image_keys":matches_saved})
    );
    Ok(())
}
