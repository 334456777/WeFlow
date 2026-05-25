use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

const MANIFEST_NAME: &str = "weflow_backup_manifest.json";

#[derive(Debug, Clone, Default)]
pub struct BackupOptions {
    pub include_images: bool,
    pub include_voice: bool,
    pub include_emojis: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub version: String,
    pub created_at: u64,
    pub wxid: String,
    pub weflow_version: String,
    pub entries: Vec<BackupEntry>,
}

pub fn create_backup(
    account_dir: &Path,
    options: &BackupOptions,
    weflow_home: Option<&Path>,
    out_path: &Path,
    progress_cb: &dyn Fn(usize, usize),
) -> Result<BackupManifest> {
    let file = fs::File::create(out_path)
        .with_context(|| format!("create {}", out_path.display()))?;
    let mut zip = ZipWriter::new(file);
    let zip_options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut entries = Vec::new();
    let wxid = account_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    let mut files_to_add: Vec<(PathBuf, String)> = Vec::new();

    // Always include db_storage
    collect_dir(account_dir, "db_storage", &mut files_to_add);

    if options.include_images {
        collect_dir(account_dir, "FileStorage/Image", &mut files_to_add);
        collect_dir(account_dir, "FileStorage/Image2", &mut files_to_add);
    }

    if options.include_voice {
        collect_dir(account_dir, "FileStorage/Audio", &mut files_to_add);
    }

    if options.include_emojis {
        if let Some(home) = weflow_home {
            collect_dir(home, "emojis", &mut files_to_add);
        }
    }

    let total = files_to_add.len();
    for (idx, (abs_path, zip_path)) in files_to_add.iter().enumerate() {
        progress_cb(idx, total);
        let data = match fs::read(abs_path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let sha = sha256_hex(&data);
        zip.start_file(zip_path, zip_options)
            .with_context(|| format!("zip start_file {zip_path}"))?;
        zip.write_all(&data)
            .with_context(|| format!("zip write {zip_path}"))?;
        entries.push(BackupEntry {
            path: zip_path.clone(),
            sha256: sha,
            size: data.len() as u64,
        });
    }
    progress_cb(total, total);

    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let manifest = BackupManifest {
        version: "1".to_string(),
        created_at,
        wxid,
        weflow_version: env!("CARGO_PKG_VERSION").to_string(),
        entries,
    };

    let manifest_bytes = serde_json::to_vec_pretty(&manifest).context("serialize manifest")?;
    zip.start_file(MANIFEST_NAME, zip_options).context("zip manifest")?;
    zip.write_all(&manifest_bytes).context("zip write manifest")?;
    zip.finish().context("zip finish")?;

    Ok(manifest)
}

pub fn inspect_backup(archive_path: &Path) -> Result<BackupManifest> {
    let file = fs::File::open(archive_path)
        .with_context(|| format!("open {}", archive_path.display()))?;
    let mut zip = ZipArchive::new(file).context("open zip archive")?;
    let mut entry = zip
        .by_name(MANIFEST_NAME)
        .context("manifest not found in archive")?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).context("read manifest")?;
    serde_json::from_slice(&buf).context("parse manifest")
}

pub fn restore_backup(
    archive_path: &Path,
    target_dir: &Path,
    progress_cb: &dyn Fn(usize, usize),
) -> Result<()> {
    let file = fs::File::open(archive_path)
        .with_context(|| format!("open {}", archive_path.display()))?;
    let mut zip = ZipArchive::new(file).context("open zip archive")?;

    // Load manifest first for sha256 verification
    let manifest: BackupManifest = {
        let mut entry = zip.by_name(MANIFEST_NAME).context("manifest not found")?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).context("read manifest")?;
        serde_json::from_slice(&buf).context("parse manifest")?
    };
    let sha_map: std::collections::HashMap<&str, &str> = manifest
        .entries
        .iter()
        .map(|e| (e.path.as_str(), e.sha256.as_str()))
        .collect();

    let total = zip.len();
    for idx in 0..total {
        progress_cb(idx, total);
        let mut entry = zip.by_index(idx).context("zip by_index")?;
        let name = entry.name().to_string();
        if name == MANIFEST_NAME {
            continue;
        }
        let out_path = target_dir.join(&name);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data).with_context(|| format!("read {name}"))?;

        if let Some(expected_sha) = sha_map.get(name.as_str()) {
            let actual = sha256_hex(&data);
            if actual != *expected_sha {
                anyhow::bail!("sha256 mismatch for {name}: expected {expected_sha}, got {actual}");
            }
        }

        fs::write(&out_path, &data)
            .with_context(|| format!("write {}", out_path.display()))?;
    }
    progress_cb(total, total);
    Ok(())
}

fn collect_dir(base: &Path, subdir: &str, out: &mut Vec<(PathBuf, String)>) {
    let dir = base.join(subdir);
    if !dir.exists() {
        return;
    }
    for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path().to_path_buf();
        let rel = abs
            .strip_prefix(base)
            .map(|p: &Path| {
                p.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default();
        out.push((abs, rel));
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_and_inspect_roundtrip() {
        let account_dir = temp_dir("weflow-backup-account");
        let db_dir = account_dir.join("db_storage/session");
        fs::create_dir_all(&db_dir).unwrap();
        fs::write(db_dir.join("session.db"), b"fake-database-content").unwrap();

        let out_dir = temp_dir("weflow-backup-out");
        let archive = out_dir.join("backup.zip");

        let options = BackupOptions::default();
        let manifest = create_backup(&account_dir, &options, None, &archive, &|_, _| {}).unwrap();

        assert_eq!(manifest.entries.len(), 1);
        assert!(manifest.entries[0].path.ends_with("session.db"));
        assert_eq!(manifest.entries[0].size, b"fake-database-content".len() as u64);

        let inspected = inspect_backup(&archive).unwrap();
        assert_eq!(inspected.entries.len(), manifest.entries.len());
        assert_eq!(inspected.entries[0].sha256, manifest.entries[0].sha256);

        let _ = fs::remove_dir_all(&account_dir);
        let _ = fs::remove_dir_all(&out_dir);
    }

    #[test]
    fn restore_roundtrip() {
        let account_dir = temp_dir("weflow-restore-account");
        let db_dir = account_dir.join("db_storage");
        fs::create_dir_all(&db_dir).unwrap();
        fs::write(db_dir.join("data.db"), b"restore-me").unwrap();

        let out_dir = temp_dir("weflow-restore-out");
        let archive = out_dir.join("backup.zip");

        create_backup(&account_dir, &BackupOptions::default(), None, &archive, &|_, _| {}).unwrap();

        let restore_dir = temp_dir("weflow-restore-target");
        restore_backup(&archive, &restore_dir, &|_, _| {}).unwrap();

        let restored = fs::read(restore_dir.join("db_storage/data.db")).unwrap();
        assert_eq!(restored, b"restore-me");

        let _ = fs::remove_dir_all(&account_dir);
        let _ = fs::remove_dir_all(&out_dir);
        let _ = fs::remove_dir_all(&restore_dir);
    }
}
