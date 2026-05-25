use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use walkdir::WalkDir;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf();
    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let output = out_dir.join("assets_generated.rs");

    println!(
        "cargo:rerun-if-changed={}",
        repo_root.join("resources").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        repo_root.join("electron/assets/wasm").display()
    );

    let mut assets = Vec::new();
    collect_assets(&repo_root, &target, &mut assets);
    assets.sort_by(|a, b| a.0.cmp(&b.0));

    let mut generated = String::new();
    generated.push_str("pub const EMBEDDED_TARGET: &str = ");
    generated.push_str(&format!("{:?}", target));
    generated.push_str(";\n");
    generated.push_str("pub const EMBEDDED_ASSETS: &[EmbeddedAsset] = &[\n");
    for (logical, absolute) in assets {
        generated.push_str("    EmbeddedAsset { logical_path: ");
        generated.push_str(&format!("{:?}", logical));
        generated.push_str(", bytes: include_bytes!(");
        generated.push_str(&format!("{:?}", absolute.display().to_string()));
        generated.push_str(") },\n");
    }
    generated.push_str("];\n");

    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|out| if out.status.success() { Some(out.stdout) } else { None })
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    generated.push_str("pub const BUILD_COMMIT: &str = ");
    generated.push_str(&format!("{:?}", commit));
    generated.push_str(";\n");

    fs::write(output, generated).expect("write assets_generated.rs");
}

fn collect_assets(repo_root: &Path, target: &str, out: &mut Vec<(String, PathBuf)>) {
    let resources = repo_root.join("resources");
    if resources.exists() {
        for entry in WalkDir::new(&resources).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(repo_root)
                .expect("relative resource path");
            let logical = normalize(relative);
            if include_resource(&logical, target) {
                out.push((logical, path.to_path_buf()));
            }
        }
    }

    let wasm_dir = repo_root.join("electron/assets/wasm");
    if wasm_dir.exists() {
        for entry in WalkDir::new(&wasm_dir).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let relative = path.strip_prefix(repo_root).expect("relative wasm path");
            out.push((normalize(relative), path.to_path_buf()));
        }
    }
}

fn include_resource(logical: &str, target: &str) -> bool {
    if logical.starts_with("resources/fonts/") {
        return true;
    }
    if logical == "resources/image/README.md" {
        return true;
    }

    let is_windows = target.contains("windows");
    let is_macos = target.contains("apple-darwin");
    let is_linux = target.contains("linux");
    let is_arm64 = target.contains("aarch64") || target.contains("arm64");

    if logical.starts_with("resources/runtime/win32/") {
        return is_windows;
    }
    if logical.starts_with("resources/image/win32/") {
        return is_windows;
    }
    if logical.starts_with("resources/installer/") || logical.starts_with("resources/icons/") {
        return false;
    }

    if logical.starts_with("resources/wcdb/win32/") {
        return is_windows && arch_match(logical, is_arm64);
    }
    if logical.starts_with("resources/wcdb/macos/") {
        return is_macos;
    }
    if logical.starts_with("resources/wcdb/linux/") {
        return is_linux && arch_match(logical, is_arm64);
    }

    if logical.starts_with("resources/key/win32/") {
        return is_windows && arch_match(logical, is_arm64);
    }
    if logical.starts_with("resources/key/macos/") {
        return is_macos && !logical.contains("/source/");
    }
    if logical.starts_with("resources/key/linux/") {
        return is_linux && arch_match(logical, is_arm64);
    }

    if logical.starts_with("resources/wedecrypt/win32/") {
        return is_windows && arch_match(logical, is_arm64);
    }
    if logical.starts_with("resources/wedecrypt/macos/") {
        return is_macos && (logical.contains("/universal/") || arch_match(logical, is_arm64));
    }
    if logical.starts_with("resources/wedecrypt/linux/") {
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
