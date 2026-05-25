use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub struct EmbeddedAsset {
    pub logical_path: &'static str,
    pub bytes: &'static [u8],
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
        let sha = sha256_hex(asset.bytes);
        let mut needs_write = true;
        if let Ok(existing) = fs::read(&target_path) {
            needs_write = sha256_hex(&existing) != sha;
        }
        if needs_write {
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
            sha256: sha,
        });
    }

    let manifest = RuntimeManifest {
        version: version.to_string(),
        target: target_triple().to_string(),
        commit: BUILD_COMMIT.to_string(),
        entries,
    };
    let manifest_path = runtime_dir.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).context("serialize runtime manifest")?,
    )
    .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    Ok(runtime_dir)
}

pub fn manifest() -> RuntimeManifest {
    RuntimeManifest {
        version: env!("CARGO_PKG_VERSION").to_string(),
        target: target_triple().to_string(),
        commit: BUILD_COMMIT.to_string(),
        entries: EMBEDDED_ASSETS
            .iter()
            .map(|asset| AssetManifestEntry {
                path: asset_relative_path(asset.logical_path).to_string(),
                size: asset.bytes.len(),
                sha256: sha256_hex(asset.bytes),
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

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
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
        let manifest = manifest();
        assert!(!manifest.entries.is_empty());
        assert!(!manifest.target.is_empty());
        assert!(manifest
            .entries
            .iter()
            .any(|entry| entry.path.ends_with("wasm/wasm_video_decode.wasm")));
    }
}
