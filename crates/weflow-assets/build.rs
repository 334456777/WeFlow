use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let assets_dir = manifest_dir.join("assets");
    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let output = out_dir.join("assets_generated.rs");

    println!("cargo:rerun-if-changed={}", assets_dir.display());

    let mut assets = Vec::new();
    collect_assets(&assets_dir, &target, &mut assets)?;
    assets.sort_by(|a, b| a.0.cmp(&b.0));

    let mut generated = String::new();
    generated.push_str("pub const EMBEDDED_TARGET: &str = ");
    generated.push_str(&format!("{:?}", target));
    generated.push_str(";\n");
    generated.push_str("pub const EMBEDDED_ASSETS: &[EmbeddedAsset] = &[\n");
    for (logical, absolute) in assets {
        // hashed here, so starting the program never has to hash the embedded files
        let bytes = fs::read(&absolute).map_err(|e| format!("read {}: {e}", absolute.display()))?;
        let sha256: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        generated.push_str("    EmbeddedAsset { logical_path: ");
        generated.push_str(&format!("{:?}", logical));
        generated.push_str(", bytes: include_bytes!(");
        generated.push_str(&format!("{:?}", absolute.display().to_string()));
        generated.push_str(&format!("), sha256: {sha256:?} }},\n"));
    }
    generated.push_str("];\n");

    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                Some(out.stdout)
            } else {
                None
            }
        })
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    generated.push_str("pub const BUILD_COMMIT: &str = ");
    generated.push_str(&format!("{:?}", commit));
    generated.push_str(";\n");

    fs::write(output, generated).map_err(|e| format!("write assets_generated.rs: {e}"))?;
    Ok(())
}

/// The files under `assets/` the program for `target` embeds, by their path there.
fn collect_assets(
    assets_dir: &Path,
    target: &str,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), Box<dyn Error>> {
    for entry in WalkDir::new(assets_dir).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let logical = normalize(path.strip_prefix(assets_dir)?);
        if include_asset(&logical, target) {
            out.push((logical, path.to_path_buf()));
        }
    }
    Ok(())
}

fn include_asset(logical: &str, target: &str) -> bool {
    let is_windows = target.contains("windows");
    let is_macos = target.contains("apple-darwin");
    let is_linux = target.contains("linux");
    let is_arm64 = target.contains("aarch64") || target.contains("arm64");

    if logical.starts_with("image/win32/") {
        // image_download.rs loads img_helper.dll on x64 only
        return is_windows && arch_match(logical, is_arm64);
    }
    if logical.starts_with("key/macos/") {
        // the helpers' source is kept next to them, not embedded
        return is_macos && !logical.contains("/source/");
    }
    if logical.starts_with("key/linux/") {
        return is_linux && arch_match(logical, is_arm64);
    }

    false
}

fn arch_match(logical: &str, is_arm64: bool) -> bool {
    if logical.contains("/universal/") {
        return true;
    }
    if is_arm64 {
        logical.contains("/arm64/") || logical.contains("/aarch64/")
    } else {
        logical.contains("/x64/") || logical.contains("/amd64/")
    }
}

fn normalize(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
