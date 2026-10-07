use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

pub struct EmbeddedAsset {
    pub logical_path: &'static str,
    pub bytes: &'static [u8],
    /// Lowercase hex SHA-256 of `bytes`, computed at build time.
    pub sha256: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/assets_generated.rs"));

#[derive(Debug, Clone, Serialize)]
pub struct AssetManifestEntry {
    pub path: String,
    pub size: usize,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeManifest {
    pub version: String,
    pub target: String,
    pub commit: String,
    pub entries: Vec<AssetManifestEntry>,
}

pub fn target_triple() -> &'static str {
    EMBEDDED_TARGET
}

pub fn ensure_runtime(home: &Path, version: &str) -> Result<PathBuf> {
    let runtime_dir = home.join("runtime").join(version).join(target_triple());
    fs::create_dir_all(&runtime_dir).with_context(|| {
        format!(
            "failed to create runtime directory {}",
            runtime_dir.display()
        )
    })?;

    let mut entries = Vec::with_capacity(EMBEDDED_ASSETS.len());
    for asset in EMBEDDED_ASSETS {
        let relative = asset_relative_path(asset.logical_path);
        let target_path = runtime_dir.join(relative);
        // An extracted file is reused only when it is byte for byte the embedded one (cheaper than hashing it,
        // and stricter); a missing, truncated or modified file is written again.
        let up_to_date = fs::metadata(&target_path)
            .is_ok_and(|m| m.len() == asset.bytes.len() as u64)
            && fs::read(&target_path).is_ok_and(|existing| existing == asset.bytes);
        if !up_to_date {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            let tmp = target_path.with_extension("tmp-weflow");
            {
                let mut file = fs::File::create(&tmp)
                    .with_context(|| format!("failed to create {}", tmp.display()))?;
                file.write_all(asset.bytes)
                    .with_context(|| format!("failed to write {}", tmp.display()))?;
            }
            fs::rename(&tmp, &target_path).with_context(|| {
                format!(
                    "failed to move {} to {}",
                    tmp.display(),
                    target_path.display()
                )
            })?;
        }

        #[cfg(unix)]
        maybe_make_executable(&target_path)?;

        entries.push(AssetManifestEntry {
            path: relative.to_string(),
            size: asset.bytes.len(),
            sha256: asset.sha256.to_string(),
        });
    }

    let manifest = RuntimeManifest {
        version: version.to_string(),
        target: target_triple().to_string(),
        commit: BUILD_COMMIT.to_string(),
        entries,
    };
    let manifest_path = runtime_dir.join("manifest.json");
    let json = serde_json::to_vec_pretty(&manifest).context("serialize runtime manifest")?;
    if fs::read(&manifest_path).ok().as_deref() != Some(&json[..]) {
        fs::write(&manifest_path, &json)
            .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    }

    Ok(runtime_dir)
}

/// `version` is the program version, which the CLI takes from the git tag (see weflow-cli/build.rs).
pub fn manifest(version: &str) -> RuntimeManifest {
    RuntimeManifest {
        version: version.to_string(),
        target: target_triple().to_string(),
        commit: BUILD_COMMIT.to_string(),
        entries: EMBEDDED_ASSETS
            .iter()
            .map(|asset| AssetManifestEntry {
                path: asset_relative_path(asset.logical_path).to_string(),
                size: asset.bytes.len(),
                sha256: asset.sha256.to_string(),
            })
            .collect(),
    }
}

fn asset_relative_path(logical_path: &str) -> &str {
    logical_path
        .strip_prefix("resources/")
        .or_else(|| logical_path.strip_prefix("electron/assets/"))
        .unwrap_or(logical_path)
}

#[cfg(unix)]
fn maybe_make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let should_exec = file_name.starts_with("xkey_helper")
        || file_name == "image_scan_helper"
        || file_name.ends_with(".sh");
    if should_exec {
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(perms.mode() | 0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_manifest_is_not_empty() {
        let manifest = manifest("1.2.3");
        assert_eq!(manifest.version, "1.2.3");
        assert!(!manifest.entries.is_empty());
        assert!(!manifest.target.is_empty());
        assert!(manifest
            .entries
            .iter()
            .any(|entry| entry.path.ends_with("wasm/wasm_video_decode.wasm")));
    }

    #[test]
    fn build_time_hashes_match_the_embedded_bytes() {
        use sha2::{Digest, Sha256};
        for asset in EMBEDDED_ASSETS {
            let hex: String = Sha256::digest(asset.bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(asset.sha256, hex, "{}", asset.logical_path);
        }
    }

    #[test]
    fn extraction_reuses_identical_files_and_repairs_changed_ones() {
        let home = std::env::temp_dir().join(format!("weflow-assets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let dir = ensure_runtime(&home, "test").unwrap();
        let asset = &EMBEDDED_ASSETS[0];
        let path = dir.join(asset_relative_path(asset.logical_path));
        assert_eq!(fs::read(&path).unwrap(), asset.bytes);
        let manifest_time = fs::metadata(dir.join("manifest.json"))
            .unwrap()
            .modified()
            .unwrap();
        // a damaged file (same size) is written again; an unchanged manifest is left alone
        let mut damaged = asset.bytes.to_vec();
        if let Some(b) = damaged.first_mut() {
            *b ^= 0xff;
        }
        fs::write(&path, &damaged).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        ensure_runtime(&home, "test").unwrap();
        assert_eq!(fs::read(&path).unwrap(), asset.bytes);
        assert_eq!(
            fs::metadata(dir.join("manifest.json"))
                .unwrap()
                .modified()
                .unwrap(),
            manifest_time
        );
        let _ = fs::remove_dir_all(&home);
    }
}
