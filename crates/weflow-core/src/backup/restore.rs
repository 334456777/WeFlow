//! `backup restore`: an archive checked whole against its manifest, then written into a folder.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use super::{hex_digest, BackupEntry, BackupManifest, EMOJI_FOLDER, MANIFEST_NAME};

/// Restore every manifest entry under `target_dir`, except WeFlow's stickers (`emojis/…`), which go back into
/// `emoji_dir`, the sticker cache, when one is given. Returns whether any sticker went there.
///
/// The whole archive is checked before anything is written: each entry must be a plain relative path, every file in
/// the archive must be listed in the manifest (and the other way round), and sizes and SHA-256 must match. Files are
/// then written through a temporary file, never through a symbolic link or junction inside the folder they go to.
pub fn restore_backup(
    archive_path: &Path,
    target_dir: &Path,
    emoji_dir: Option<&Path>,
    progress_cb: &dyn Fn(usize, usize),
) -> Result<bool> {
    let file =
        fs::File::open(archive_path).with_context(|| format!("open {}", archive_path.display()))?;
    let mut zip = ZipArchive::new(file)
        .with_context(|| format!("{} is not a zip archive", archive_path.display()))?;

    let manifest: BackupManifest = {
        let mut entry = zip.by_name(MANIFEST_NAME).context("manifest not found")?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).context("read manifest")?;
        serde_json::from_slice(&buf).context("parse manifest")?
    };
    let mut listed: HashMap<&str, (&BackupEntry, PathBuf)> = HashMap::new();
    for entry in &manifest.entries {
        let relative = safe_relative_path(&entry.path)?;
        if listed
            .insert(entry.path.as_str(), (entry, relative))
            .is_some()
        {
            bail!("{} is listed twice in the backup manifest", entry.path);
        }
    }

    let total = manifest.entries.len() * 2;
    let mut done = 0;

    // Pass 1: check the archive against the manifest without touching the target folder.
    let mut seen = HashSet::new();
    for idx in 0..zip.len() {
        let mut entry = zip.by_index(idx).context("zip by_index")?;
        let name = entry.name().to_string();
        if name == MANIFEST_NAME || entry.is_dir() {
            continue;
        }
        let Some((expected, _)) = listed.get(name.as_str()) else {
            bail!("{name} is in the archive but not in the backup manifest");
        };
        if !seen.insert(name.clone()) {
            bail!("{name} appears twice in the archive");
        }
        progress_cb(done, total);
        done += 1;
        verify_copy(&mut entry, &mut io::sink(), expected)?;
    }
    if let Some(missing) = manifest.entries.iter().find(|e| !seen.contains(&e.path)) {
        bail!(
            "{} is listed in the backup manifest but missing from the archive",
            missing.path
        );
    }

    // Pass 2: write each file to a temporary name next to its target, then move it into place.
    let mut stickers = false;
    for entry in &manifest.entries {
        progress_cb(done, total);
        done += 1;
        let (_, relative) = &listed[entry.path.as_str()];
        let (base, relative) = match (emoji_dir, relative.strip_prefix(EMOJI_FOLDER)) {
            (Some(dir), Ok(sticker)) if !sticker.as_os_str().is_empty() => {
                stickers = true;
                (dir, sticker)
            }
            _ => (target_dir, relative.as_path()),
        };
        let out_path = base.join(relative);
        create_parent_dirs(base, relative)?;

        let tmp = out_path.with_file_name(format!(
            "{}.weflow-restore",
            out_path.file_name().unwrap_or_default().to_string_lossy()
        ));
        let written = (|| -> Result<()> {
            let mut zipped = zip
                .by_name(&entry.path)
                .with_context(|| format!("read {}", entry.path))?;
            let mut out = create_fresh(&tmp)?;
            // checked again: the archive could have changed since pass 1
            verify_copy(&mut zipped, &mut out, entry)?;
            out.sync_all()
                .with_context(|| format!("write {}", tmp.display()))?;
            drop(out);
            fs::rename(&tmp, &out_path).with_context(|| format!("write {}", out_path.display()))
        })();
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written?;
    }
    progress_cb(total, total);
    Ok(stickers)
}

/// Turn a manifest path (`/`-separated, as `create_backup` writes it) into a relative path that cannot leave the
/// restore folder: no `..`, `.`, empty parts, root, drive letter or `\` separators.
fn safe_relative_path(name: &str) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for part in name.split('/') {
        let mut components = Path::new(part).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(c)), None) if c == part => out.push(part),
            _ => bail!("unsafe path in backup: {name}"),
        }
    }
    Ok(out)
}

/// Create the folders between `target_dir` and the restored file one level at a time, failing on a symbolic link
/// (or, on Windows, a junction), which would make the write land outside `target_dir`.
fn create_parent_dirs(target_dir: &Path, relative: &Path) -> Result<()> {
    fs::create_dir_all(target_dir).with_context(|| format!("create {}", target_dir.display()))?;
    let mut path = target_dir.to_path_buf();
    for part in relative.parent().into_iter().flat_map(Path::components) {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => bail!(
                "refusing to restore through the link {}: it may point outside {}",
                path.display(),
                target_dir.display()
            ),
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => fs::create_dir(&path)
                .or_else(|err| match err.kind() {
                    io::ErrorKind::AlreadyExists => Ok(()),
                    _ => Err(err),
                })
                .with_context(|| format!("create {}", path.display()))?,
            Err(err) => return Err(err).with_context(|| format!("inspect {}", path.display())),
        }
    }
    Ok(())
}

/// Create `path` as a new file. Whatever already has that name, including a symbolic link, is removed first, and the
/// file is opened with `create_new`, which fails on anything still in the way instead of following it, so the write
/// cannot land where a link points.
fn create_fresh(path: &Path) -> Result<fs::File> {
    match fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => {
            return Err(err).with_context(|| format!("remove {}", path.display()));
        }
        _ => {}
    }
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))
}

/// Copy `reader` into `out`, failing unless its size and SHA-256 match `expected`.
fn verify_copy(reader: &mut impl Read, out: &mut impl Write, expected: &BackupEntry) -> Result<()> {
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("read {}", expected.path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])
            .with_context(|| format!("write {}", expected.path))?;
        size += n as u64;
    }
    let actual = hex_digest(&hasher.finalize());
    if actual != expected.sha256 {
        bail!(
            "sha256 mismatch for {}: expected {}, got {actual}",
            expected.path,
            expected.sha256
        );
    }
    if size != expected.size {
        bail!(
            "size mismatch for {}: expected {}, got {size}",
            expected.path,
            expected.size
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use super::*;
    use crate::backup::{create_backup, sha256_hex, BackupOptions};

    fn temp_dir(prefix: &str) -> weflow_native::fixture::TestDir {
        weflow_native::fixture::TestDir::new(prefix)
    }

    #[test]
    fn restore_roundtrip() {
        let account_dir = temp_dir("weflow-restore-account");
        let db_dir = account_dir.join("db_storage");
        fs::create_dir_all(&db_dir).unwrap();
        fs::write(db_dir.join("data.db"), b"restore-me").unwrap();

        let out_dir = temp_dir("weflow-restore-out");
        let archive = out_dir.join("backup.zip");

        create_backup(
            &account_dir,
            &BackupOptions::default(),
            None,
            "1.2.3",
            &archive,
            &|_, _| {},
        )
        .unwrap();

        let restore_dir = temp_dir("weflow-restore-target");
        assert!(!restore_backup(&archive, &restore_dir, None, &|_, _| {}).unwrap());

        let restored = fs::read(restore_dir.join("db_storage/data.db")).unwrap();
        assert_eq!(restored, b"restore-me");
        assert_eq!(
            fs::read_dir(restore_dir.join("db_storage"))
                .unwrap()
                .count(),
            1,
            "no temporary file left behind"
        );
    }

    /// Write an archive holding `files` and a manifest listing `listed` as (path, content the hash is taken from).
    fn hand_made_archive(dir: &Path, files: &[(&str, &[u8])], listed: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join("hand-made.zip");
        let mut zip = ZipWriter::new(fs::File::create(&path).unwrap());
        let options = SimpleFileOptions::default();
        let manifest = BackupManifest {
            version: "1".to_string(),
            created_at: 0,
            wxid: "test".to_string(),
            weflow_version: "test".to_string(),
            entries: listed
                .iter()
                .map(|(p, data)| BackupEntry {
                    path: p.to_string(),
                    sha256: sha256_hex(data),
                    size: data.len() as u64,
                })
                .collect(),
            skipped: Vec::new(),
            unsettled: Vec::new(),
        };
        zip.start_file(MANIFEST_NAME, options).unwrap();
        zip.write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        for (name, data) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn restore_err(archive: &Path, target: &Path) -> String {
        restore_backup(archive, target, None, &|_, _| {})
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn restore_refuses_paths_that_leave_the_target() {
        let dir = temp_dir("weflow-restore-escape");
        let target = dir.join("target");
        fs::create_dir_all(&target).unwrap();
        let outside = dir.join("outside.txt");
        let absolute = outside.to_string_lossy().replace('\\', "/");
        for name in [
            "../outside.txt",
            "db_storage/../../outside.txt",
            absolute.as_str(),
            "./a.txt",
            "a//b.txt",
            "a\\..\\..\\outside.txt",
        ] {
            let archive = hand_made_archive(&dir, &[(name, b"x")], &[(name, b"x")]);
            // a backslash is an ordinary file-name character on Unix: there the name is one file inside the target
            if cfg!(windows) || !name.contains('\\') {
                let err = restore_err(&archive, &target);
                assert!(err.contains("unsafe path"), "{name}: {err}");
            } else {
                restore_backup(&archive, &target, None, &|_, _| {}).unwrap();
                assert!(target.join(name).is_file());
            }
            assert!(!outside.exists(), "{name} escaped the target folder");
        }
    }

    #[test]
    fn restore_requires_archive_and_manifest_to_match() {
        let dir = temp_dir("weflow-restore-unlisted");
        let target = dir.join("target");
        fs::create_dir_all(&target).unwrap();

        let unlisted = hand_made_archive(&dir, &[("unlisted.txt", b"x")], &[]);
        assert!(restore_err(&unlisted, &target).contains("not in the backup manifest"));
        let missing = hand_made_archive(&dir, &[], &[("gone.txt", b"x")]);
        assert!(restore_err(&missing, &target).contains("missing from the archive"));
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    }

    #[test]
    fn failed_restore_leaves_existing_files_alone() {
        let dir = temp_dir("weflow-restore-partial");
        let target = dir.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("first.txt"), b"old first").unwrap();

        let archive = hand_made_archive(
            &dir,
            &[("first.txt", b"new first"), ("second.txt", b"tampered")],
            &[("first.txt", b"new first"), ("second.txt", b"original")],
        );
        assert!(restore_err(&archive, &target).contains("sha256 mismatch for second.txt"));
        assert_eq!(fs::read(target.join("first.txt")).unwrap(), b"old first");
        assert!(!target.join("second.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn restore_refuses_to_write_through_a_symlink() {
        let dir = temp_dir("weflow-restore-symlink");
        let target = dir.join("target");
        let elsewhere = dir.join("elsewhere");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, target.join("db_storage")).unwrap();

        let archive = hand_made_archive(
            &dir,
            &[("db_storage/session/a.db", b"x")],
            &[("db_storage/session/a.db", b"x")],
        );
        assert!(restore_err(&archive, &target).contains("refusing to restore through the link"));
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn restore_does_not_follow_a_link_named_like_the_temporary_file() {
        let dir = temp_dir("weflow-restore-tmp-link");
        let target = dir.join("target");
        fs::create_dir_all(&target).unwrap();
        let outside = dir.join("outside.txt");
        fs::write(&outside, b"keep me").unwrap();
        std::os::unix::fs::symlink(&outside, target.join("a.txt.weflow-restore")).unwrap();

        let archive = hand_made_archive(&dir, &[("a.txt", b"restored")], &[("a.txt", b"restored")]);
        restore_backup(&archive, &target, None, &|_, _| {}).unwrap();
        assert_eq!(fs::read(&outside).unwrap(), b"keep me");
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"restored");
        assert!(fs::symlink_metadata(target.join("a.txt.weflow-restore")).is_err());
    }
}
