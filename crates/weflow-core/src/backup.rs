use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const MANIFEST_NAME: &str = "weflow_backup_manifest.json";

/// Where WeFlow's sticker cache is kept in a backup.
const EMOJI_FOLDER: &str = "emojis";

/// How many times a database is copied when WeChat writes to it, or starts its WAL over, while it is being copied.
/// The last copy is kept even then, and the backup names the database in `unsettled`.
const COPY_ATTEMPTS: usize = 3;

/// The length of a WAL's header (SQLite's `wal.c`): magic, format, page size, checkpoint sequence, the two salts and
/// its checksum. A checkpoint that starts the WAL over gives it a new sequence number and new salts.
const WAL_HEADER: usize = 32;

/// What a backup holds besides the databases (`db_storage/`, always there; the voice messages are in them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackupOptions {
    /// The images WeChat keeps: `msg/attach/`.
    pub images: bool,
    /// The videos WeChat keeps: `msg/video/`.
    pub videos: bool,
    /// The files sent and received: `msg/file/`.
    pub files: bool,
    /// WeFlow's cache of the stickers it downloaded, kept in the backup as `emojis/`.
    pub emojis: bool,
}

/// A file in a backup.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BackupEntry {
    /// Its path in the backup: `db_storage/…`, `msg/attach/…`, `msg/video/…`, `msg/file/…` or `emojis/…`; a backup
    /// of an older WeFlow can also hold `FileStorage/…`.
    pub path: String,
    pub sha256: String,
    /// Bytes.
    pub size: u64,
}

/// What a backup holds (`backup inspect`): the manifest stored in the archive.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BackupManifest {
    /// The manifest's own format.
    pub version: String,
    /// Unix seconds.
    pub created_at: u64,
    pub wxid: String,
    /// The WeFlow that wrote it.
    pub weflow_version: String,
    pub entries: Vec<BackupEntry>,
    /// Files of the media and stickers that could not be read and are not in the backup. Left out when there are
    /// none, and in a backup of an older WeFlow.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SkippedFile>,
    /// Databases (their path in the backup) WeChat kept writing to while they were copied: their main file and WAL
    /// in the backup may be of two moments. Left out when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsettled: Vec<String>,
}

/// A file left out of a backup because it could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SkippedFile {
    /// Its path in the backup, as it would have been.
    pub path: String,
    /// What the system said.
    pub reason: String,
}

/// A file to put in a backup.
struct Collected {
    path: PathBuf,
    /// Its path in the backup.
    name: String,
    method: CompressionMethod,
    /// A database's WAL (`<name>-wal` beside it) and its path in the backup: copied with the database.
    wal: Option<(PathBuf, String)>,
}

/// What has gone into a backup so far.
#[derive(Default)]
struct Written {
    entries: Vec<BackupEntry>,
    skipped: Vec<SkippedFile>,
    unsettled: Vec<String>,
}

impl Written {
    fn skip(&mut self, name: &str, err: &io::Error) {
        self.skipped.push(SkippedFile {
            path: name.to_string(),
            reason: err.to_string(),
        });
    }
}

/// Write a backup of the account in `account_dir` to `out_path`: its databases, and what `options` add.
/// `emoji_dir` is WeFlow's sticker cache.
///
/// Each file is read and written in pieces, so a large video takes no more memory than a small one. The media are
/// stored as they are, since they are compressed already. The archive is written beside `out_path` under another
/// name and takes its place only when complete: a backup already at `out_path` stays as it was when this fails.
/// Neither is backed up itself when it lies in a folder that is.
///
/// A database is copied with its WAL as one: when WeChat writes to the database (a checkpoint) or starts the WAL
/// over while they are copied, both are copied again, up to [`COPY_ATTEMPTS`] times (`unsettled` names those that
/// never held still). The WAL index (`-shm`) is left out, since SQLite rebuilds it from the WAL.
///
/// A file that is gone by the time it is read (WeChat removes some meanwhile) is left out. One that cannot be read is
/// listed in `skipped` when it is a medium or a sticker, and fails the backup when it is one of the databases.
pub fn create_backup(
    account_dir: &Path,
    options: &BackupOptions,
    emoji_dir: Option<&Path>,
    weflow_version: &str,
    out_path: &Path,
    progress_cb: &dyn Fn(usize, usize),
) -> Result<BackupManifest> {
    create_backup_watched(
        account_dir,
        options,
        emoji_dir,
        weflow_version,
        out_path,
        progress_cb,
        &|_| {},
    )
}

/// [`create_backup`], calling `copied` with each database just copied, before its WAL is read: the tests write to
/// it there as WeChat would.
fn create_backup_watched(
    account_dir: &Path,
    options: &BackupOptions,
    emoji_dir: Option<&Path>,
    weflow_version: &str,
    out_path: &Path,
    progress_cb: &dyn Fn(usize, usize),
    copied: &dyn Fn(&Path),
) -> Result<BackupManifest> {
    let partial = {
        let mut name = out_path.as_os_str().to_os_string();
        name.push(format!(".{}.weflow-partial", std::process::id()));
        PathBuf::from(name)
    };
    let file =
        fs::File::create(&partial).with_context(|| format!("create {}", partial.display()))?;
    let excluded: Vec<PathBuf> = [out_path, &partial]
        .into_iter()
        .filter_map(identity)
        .collect();
    let written = write_backup(
        ZipWriter::new(file),
        account_dir,
        options,
        emoji_dir,
        weflow_version,
        &excluded,
        progress_cb,
        copied,
    )
    .and_then(|manifest| {
        fs::rename(&partial, out_path)
            .with_context(|| format!("move the backup to {}", out_path.display()))?;
        Ok(manifest)
    });
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    written
}

#[expect(
    clippy::too_many_arguments,
    reason = "the inputs of create_backup, passed on as they are"
)]
fn write_backup(
    mut zip: ZipWriter<fs::File>,
    account_dir: &Path,
    options: &BackupOptions,
    emoji_dir: Option<&Path>,
    weflow_version: &str,
    excluded: &[PathBuf],
    progress_cb: &dyn Fn(usize, usize),
    copied: &dyn Fn(&Path),
) -> Result<BackupManifest> {
    let wxid = account_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    let mut databases = Vec::new();
    collect_dir(
        &account_dir.join("db_storage"),
        "db_storage",
        CompressionMethod::Deflated,
        excluded,
        &mut databases,
    );
    let databases = with_their_wals(databases);
    let mut media = Vec::new();
    for (wanted, folder) in [
        (options.images, "msg/attach"),
        (options.videos, "msg/video"),
        (options.files, "msg/file"),
    ] {
        if wanted {
            collect_dir(
                &account_dir.join(folder),
                folder,
                CompressionMethod::Stored,
                excluded,
                &mut media,
            );
        }
    }
    if let Some(dir) = emoji_dir.filter(|_| options.emojis) {
        collect_dir(
            dir,
            EMOJI_FOLDER,
            CompressionMethod::Stored,
            excluded,
            &mut media,
        );
    }

    let files = |items: &[Collected]| -> usize {
        items.iter().map(|i| 1 + usize::from(i.wal.is_some())).sum()
    };
    let total = files(&databases) + media.len();
    let mut done = 0;
    let mut written = Written::default();
    for item in &databases {
        progress_cb(done, total);
        done += files(std::slice::from_ref(item));
        add_database(&mut zip, item, &mut written, copied)?;
    }
    if !written.skipped.is_empty() {
        let list: Vec<String> = written
            .skipped
            .iter()
            .map(|s| format!("{}: {}", s.path, s.reason))
            .collect();
        bail!(
            "cannot read {} file(s) of the account's databases, so no backup was written: {}",
            list.len(),
            list.join("; ")
        );
    }
    for item in &media {
        progress_cb(done, total);
        done += 1;
        if let Some(entry) = copy_file(&mut zip, item, &mut written)? {
            written.entries.push(entry);
        }
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
        weflow_version: weflow_version.to_string(),
        entries: written.entries,
        skipped: written.skipped,
        unsettled: written.unsettled,
    };

    let manifest_bytes = serde_json::to_vec_pretty(&manifest).context("serialize manifest")?;
    zip.start_file(
        MANIFEST_NAME,
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
    )
    .context("zip manifest")?;
    zip.write_all(&manifest_bytes)
        .context("zip write manifest")?;
    zip.finish().context("zip finish")?;

    Ok(manifest)
}

/// Copy a database, and its WAL with it when it has one, into `zip`, again while WeChat changed either meanwhile.
fn add_database(
    zip: &mut ZipWriter<fs::File>,
    item: &Collected,
    written: &mut Written,
    copied: &dyn Fn(&Path),
) -> Result<()> {
    let Some((wal_path, wal_name)) = &item.wal else {
        if let Some(entry) = copy_file(zip, item, written)? {
            written.entries.push(entry);
        }
        return Ok(());
    };
    for attempt in 1..=COPY_ATTEMPTS {
        let before = (stamp(&item.path), wal_header(wal_path));
        let Some(database) = copy_file(zip, item, written)? else {
            return Ok(());
        };
        copied(&item.path);
        // read whole while the database's entry is still open, so that both can be copied again (a WAL is small:
        // SQLite moves it into the database every thousand pages or so)
        let wal = match fs::read(wal_path) {
            Ok(bytes) => Some(bytes),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => {
                // the backup fails for it (it is one of the databases): no point in copying again
                written.entries.push(database);
                written.skip(wal_name, &err);
                return Ok(());
            }
        };
        let after = (
            stamp(&item.path),
            wal.as_deref()
                .and_then(|bytes| bytes.get(..WAL_HEADER))
                .map(<[u8]>::to_vec),
        );
        let settled = before == after;
        if !settled && attempt < COPY_ATTEMPTS {
            zip.abort_file()
                .with_context(|| format!("zip abort {}", item.name))?;
            continue;
        }
        written.entries.push(database);
        if let Some(bytes) = wal {
            zip.start_file(
                wal_name.as_str(),
                SimpleFileOptions::default().compression_method(item.method),
            )
            .with_context(|| format!("zip start_file {wal_name}"))?;
            zip.write_all(&bytes)
                .with_context(|| format!("zip write {wal_name}"))?;
            written.entries.push(BackupEntry {
                path: wal_name.clone(),
                sha256: sha256_hex(&bytes),
                size: bytes.len() as u64,
            });
        }
        if !settled {
            written.unsettled.push(item.name.clone());
        }
        break;
    }
    Ok(())
}

/// Start `item` in `zip` and copy the file into it, leaving the entry open so that it can still be aborted. `None`
/// when the file is gone, or could not be read (then it is in `written.skipped`).
fn copy_file(
    zip: &mut ZipWriter<fs::File>,
    item: &Collected,
    written: &mut Written,
) -> Result<Option<BackupEntry>> {
    let mut source = match fs::File::open(&item.path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            written.skip(&item.name, &err);
            return Ok(None);
        }
    };
    let size = source.metadata().map_or(0, |m| m.len());
    let zip_options = SimpleFileOptions::default()
        .compression_method(item.method)
        .large_file(size >= u64::from(u32::MAX));
    zip.start_file(item.name.as_str(), zip_options)
        .with_context(|| format!("zip start_file {}", item.name))?;
    match copy_hashing(&mut source, zip).with_context(|| format!("zip write {}", item.name))? {
        Copied::Whole { sha256, size } => Ok(Some(BackupEntry {
            path: item.name.clone(),
            sha256,
            size,
        })),
        Copied::ReadFailed(err) => {
            zip.abort_file()
                .with_context(|| format!("zip abort {}", item.name))?;
            written.skip(&item.name, &err);
            Ok(None)
        }
    }
}

/// A file's size and last change, to tell whether it was written to.
#[derive(Debug, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    Some(Stamp {
        len: meta.len(),
        modified: meta.modified().ok(),
    })
}

/// The header of the WAL at `path`, when there is a whole one.
fn wal_header(path: &Path) -> Option<Vec<u8>> {
    let mut header = vec![0; WAL_HEADER];
    fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    Some(header)
}

/// The same file as `path` names, its links followed, whether it is there or only its folder.
fn identity(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok().or_else(|| {
        let folder = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        Some(fs::canonicalize(folder).ok()?.join(path.file_name()?))
    })
}

/// The databases with their WAL: each `<name>-wal` beside a `<name>` goes with it, the WAL index (`<name>-shm`) is
/// left out, and a WAL without its database stays a file of its own.
fn with_their_wals(files: Vec<Collected>) -> Vec<Collected> {
    let names: HashSet<String> = files.iter().map(|f| f.name.clone()).collect();
    let mut wals: HashMap<String, (PathBuf, String)> = HashMap::new();
    let mut out = Vec::new();
    for file in files {
        match file.name.strip_suffix("-wal") {
            Some(database) if names.contains(database) => {
                wals.insert(database.to_string(), (file.path, file.name));
            }
            _ if file.name.ends_with("-shm") => {}
            _ => out.push(file),
        }
    }
    for file in &mut out {
        file.wal = wals.remove(&file.name);
    }
    out
}

pub fn inspect_backup(archive_path: &Path) -> Result<BackupManifest> {
    let file =
        fs::File::open(archive_path).with_context(|| format!("open {}", archive_path.display()))?;
    let mut zip = ZipArchive::new(file).context("open zip archive")?;
    let mut entry = zip
        .by_name(MANIFEST_NAME)
        .context("manifest not found in archive")?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).context("read manifest")?;
    serde_json::from_slice(&buf).context("parse manifest")
}

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
    let mut zip = ZipArchive::new(file).context("open zip archive")?;

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
            let mut out =
                fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
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
                "refusing to restore through the link {} (it may point outside {})",
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

/// The files under `dir`, named in the backup `<prefix>/<their path under dir>`, but those `excluded` names (as
/// [`identity`] gives them); nothing when `dir` is missing.
fn collect_dir(
    dir: &Path,
    prefix: &str,
    method: CompressionMethod,
    excluded: &[PathBuf],
    out: &mut Vec<Collected>,
) {
    for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(dir) else {
            continue;
        };
        // the name first: resolving every file's path would cost a look-up of each folder on the way
        let maybe_excluded = excluded
            .iter()
            .any(|x| x.file_name() == Some(entry.file_name()));
        if maybe_excluded && identity(entry.path()).is_some_and(|id| excluded.contains(&id)) {
            continue;
        }
        let name = std::iter::once(prefix.to_string())
            .chain(
                relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned()),
            )
            .collect::<Vec<_>>()
            .join("/");
        out.push(Collected {
            path: entry.path().to_path_buf(),
            name,
            method,
            wal: None,
        });
    }
}

/// What [`copy_hashing`] copied.
enum Copied {
    /// All of it: its SHA-256 and size.
    Whole { sha256: String, size: u64 },
    /// Reading failed part way.
    ReadFailed(io::Error),
}

/// Copy `reader` into `out`. Failing to write is an error.
fn copy_hashing(reader: &mut impl Read, out: &mut impl Write) -> io::Result<Copied> {
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Ok(Copied::ReadFailed(err)),
        };
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        size += n as u64;
    }
    Ok(Copied::Whole {
        sha256: hex_digest(&hasher.finalize()),
        size,
    })
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex_digest(&h.finalize())
}

fn hex_digest(digest: &[u8]) -> String {
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {

    use super::*;

    fn temp_dir(prefix: &str) -> weflow_native::fixture::TestDir {
        weflow_native::fixture::TestDir::new(prefix)
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
        let manifest =
            create_backup(&account_dir, &options, None, "1.2.3", &archive, &|_, _| {}).unwrap();

        assert_eq!(manifest.entries.len(), 1);
        assert!(manifest.entries[0].path.ends_with("session.db"));
        assert_eq!(
            manifest.entries[0].size,
            b"fake-database-content".len() as u64
        );

        let inspected = inspect_backup(&archive).unwrap();
        assert_eq!(inspected.weflow_version, "1.2.3");
        assert_eq!(inspected.entries.len(), manifest.entries.len());
        assert_eq!(inspected.entries[0].sha256, manifest.entries[0].sha256);
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

    /// The media of WeChat 4.x (`msg/attach`, `msg/video`, `msg/file`) and WeFlow's sticker cache come in only when
    /// asked for (#643), stored as they are; the stickers go back into the sticker cache.
    #[test]
    fn media_and_stickers_come_in_when_asked() {
        let account_dir = temp_dir("weflow-backup-media");
        let emoji_dir = temp_dir("weflow-backup-emojis");
        for (path, data) in [
            ("db_storage/session/session.db", &b"db"[..]),
            ("msg/attach/0a1b/2024-01/Img/abc.dat", b"image"),
            ("msg/video/2024-01/abc.mp4", b"video"),
            ("msg/file/2024-01/report.pdf", b"file"),
            ("msg/migrate/left-out.bin", b"other"),
        ] {
            let file = account_dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, data).unwrap();
        }
        fs::write(emoji_dir.join("0123.gif"), b"sticker").unwrap();
        let out_dir = temp_dir("weflow-backup-media-out");
        let backup = |options: BackupOptions, name: &str| {
            let archive = out_dir.join(name);
            let manifest = create_backup(
                &account_dir,
                &options,
                Some(&emoji_dir),
                "1.2.3",
                &archive,
                &|_, _| {},
            )
            .unwrap();
            let mut paths: Vec<String> = manifest.entries.into_iter().map(|e| e.path).collect();
            paths.sort();
            (archive, paths)
        };

        let (_, paths) = backup(BackupOptions::default(), "databases.zip");
        assert_eq!(paths, ["db_storage/session/session.db"]);

        let all = BackupOptions {
            images: true,
            videos: true,
            files: true,
            emojis: true,
        };
        let (archive, paths) = backup(all, "all.zip");
        assert_eq!(
            paths,
            [
                "db_storage/session/session.db",
                "emojis/0123.gif",
                "msg/attach/0a1b/2024-01/Img/abc.dat",
                "msg/file/2024-01/report.pdf",
                "msg/video/2024-01/abc.mp4",
            ]
        );
        let mut zip = ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
        assert_eq!(
            zip.by_name("msg/video/2024-01/abc.mp4")
                .unwrap()
                .compression(),
            CompressionMethod::Stored
        );
        assert_eq!(
            zip.by_name("db_storage/session/session.db")
                .unwrap()
                .compression(),
            CompressionMethod::Deflated
        );

        let target = temp_dir("weflow-backup-media-target");
        let cache = temp_dir("weflow-backup-media-cache");
        assert!(restore_backup(&archive, &target, Some(&cache), &|_, _| {}).unwrap());
        assert_eq!(fs::read(cache.join("0123.gif")).unwrap(), b"sticker");
        assert!(!target.join("emojis").exists());
        assert_eq!(
            fs::read(target.join("msg/video/2024-01/abc.mp4")).unwrap(),
            b"video"
        );
    }

    /// A WAL as SQLite starts it: the header (with `salt` in its checkpoint sequence and salts), then a frame.
    fn wal(salt: u8, frame: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x37, 0x7f, 0x06, 0x82, 0, 0x2d, 0xe2, 0x18, 0, 0, 0x10, 0];
        bytes.extend([salt; 12]);
        bytes.extend([0; 8]);
        bytes.extend(frame);
        bytes
    }

    fn account_with_wal(prefix: &str) -> weflow_native::fixture::TestDir {
        let account = temp_dir(prefix);
        let dir = account.join("db_storage/message");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("message_0.db"), b"main file").unwrap();
        fs::write(dir.join("message_0.db-wal"), wal(1, b"frames")).unwrap();
        fs::write(dir.join("message_0.db-shm"), b"index").unwrap();
        account
    }

    fn archived(archive: &Path, name: &str) -> Vec<u8> {
        let mut zip = ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
        let mut bytes = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
        bytes
    }

    /// A database and its WAL go in together; the WAL index stays out, since SQLite rebuilds it (#817).
    #[test]
    fn a_database_goes_with_its_wal_and_without_its_index() {
        let account = account_with_wal("weflow-backup-wal");
        let out = temp_dir("weflow-backup-wal-out");
        let archive = out.join("b.zip");
        let manifest = create_backup(
            &account,
            &BackupOptions::default(),
            None,
            "1",
            &archive,
            &|_, _| {},
        )
        .unwrap();
        let paths: Vec<&str> = manifest.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "db_storage/message/message_0.db",
                "db_storage/message/message_0.db-wal"
            ]
        );
        assert!(manifest.unsettled.is_empty() && manifest.skipped.is_empty());
        assert_eq!(
            archived(&archive, "db_storage/message/message_0.db-wal"),
            wal(1, b"frames")
        );
        // the archive is checked against its manifest on restore, the WAL's entry included
        let target = temp_dir("weflow-backup-wal-target");
        restore_backup(&archive, &target, None, &|_, _| {}).unwrap();
        assert_eq!(
            fs::read_dir(&out).unwrap().count(),
            1,
            "only the backup is left beside it"
        );
    }

    /// WeChat checkpoints while the database is copied and starts its WAL over: both are copied again, and the
    /// backup holds the database and WAL of one moment (#817).
    #[test]
    fn a_database_written_to_meanwhile_is_copied_again() {
        let account = account_with_wal("weflow-backup-again");
        let out = temp_dir("weflow-backup-again-out");
        let archive = out.join("b.zip");
        let copies = std::cell::Cell::new(0);
        let manifest = create_backup_watched(
            &account,
            &BackupOptions::default(),
            None,
            "1",
            &archive,
            &|_, _| {},
            &|database| {
                copies.set(copies.get() + 1);
                if copies.get() == 1 {
                    fs::write(database, b"main file, checkpointed").unwrap();
                    fs::write(database.with_extension("db-wal"), wal(2, b"new frames")).unwrap();
                }
            },
        )
        .unwrap();
        assert_eq!(copies.get(), 2);
        assert!(manifest.unsettled.is_empty());
        assert_eq!(
            archived(&archive, "db_storage/message/message_0.db"),
            b"main file, checkpointed"
        );
        assert_eq!(
            archived(&archive, "db_storage/message/message_0.db-wal"),
            wal(2, b"new frames")
        );
        assert_eq!(
            manifest.entries.len(),
            2,
            "no copy of the first attempt left"
        );
    }

    /// A database WeChat keeps writing to is copied [`COPY_ATTEMPTS`] times, then kept as it is and named (#817).
    #[test]
    fn a_database_that_never_holds_still_is_named() {
        let account = account_with_wal("weflow-backup-busy");
        let out = temp_dir("weflow-backup-busy-out");
        let archive = out.join("b.zip");
        let copies = std::cell::Cell::new(0u8);
        let manifest = create_backup_watched(
            &account,
            &BackupOptions::default(),
            None,
            "1",
            &archive,
            &|_, _| {},
            &|database| {
                copies.set(copies.get() + 1);
                fs::write(
                    database.with_extension("db-wal"),
                    wal(copies.get() + 1, b"x"),
                )
                .unwrap();
            },
        )
        .unwrap();
        assert_eq!(usize::from(copies.get()), COPY_ATTEMPTS);
        assert_eq!(manifest.unsettled, ["db_storage/message/message_0.db"]);
        assert_eq!(manifest.entries.len(), 2);
        assert_eq!(
            inspect_backup(&archive).unwrap().unsettled,
            manifest.unsettled
        );
    }

    /// A backup written into a folder it backs up does not take itself in, nor its file being written; it replaces
    /// the backup already there only once complete (#817).
    #[test]
    fn the_backup_leaves_itself_out() {
        let account = temp_dir("weflow-backup-self");
        for (path, data) in [
            ("db_storage/session/session.db", &b"db"[..]),
            ("msg/file/2024-01/report.pdf", b"file"),
            ("msg/file/backup.zip", b"the backup before"),
        ] {
            let file = account.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, data).unwrap();
        }
        let archive = account.join("msg/file/backup.zip");
        let options = BackupOptions {
            files: true,
            ..BackupOptions::default()
        };
        let manifest = create_backup(&account, &options, None, "1", &archive, &|_, _| {}).unwrap();
        let paths: Vec<&str> = manifest.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "db_storage/session/session.db",
                "msg/file/2024-01/report.pdf"
            ]
        );
        assert_eq!(inspect_backup(&archive).unwrap().entries.len(), 2);
        assert_eq!(
            fs::read_dir(account.join("msg/file")).unwrap().count(),
            2,
            "no file being written is left"
        );
    }

    /// Keeps the file at `path` from being opened by anyone else until dropped, as WeChat may.
    struct Unreadable {
        #[cfg(windows)]
        _held: fs::File,
        #[cfg(unix)]
        path: PathBuf,
    }

    impl Unreadable {
        fn new(path: &Path) -> Self {
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                let held = fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(path)
                    .unwrap();
                Self { _held: held }
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
                Self {
                    path: path.to_path_buf(),
                }
            }
        }
    }

    #[cfg(unix)]
    impl Drop for Unreadable {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o644));
        }
    }

    /// A medium that cannot be read is left out and listed; a database that cannot be read fails the backup, and
    /// the one already at the output stays as it was (#817).
    #[test]
    fn files_that_cannot_be_read_are_reported() {
        let account = temp_dir("weflow-backup-unreadable");
        for (path, data) in [
            ("db_storage/session/session.db", &b"db"[..]),
            ("msg/attach/a.dat", b"readable"),
            ("msg/attach/b.dat", b"held"),
        ] {
            let file = account.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, data).unwrap();
        }
        let out = temp_dir("weflow-backup-unreadable-out");
        let archive = out.join("b.zip");
        let options = BackupOptions {
            images: true,
            ..BackupOptions::default()
        };
        let held = Unreadable::new(&account.join("msg/attach/b.dat"));
        let manifest = create_backup(&account, &options, None, "1", &archive, &|_, _| {}).unwrap();
        drop(held);
        assert_eq!(manifest.entries.len(), 2);
        assert_eq!(manifest.skipped.len(), 1);
        assert_eq!(manifest.skipped[0].path, "msg/attach/b.dat");
        assert!(!manifest.skipped[0].reason.is_empty());
        assert_eq!(inspect_backup(&archive).unwrap().skipped, manifest.skipped);

        let before = fs::read(&archive).unwrap();
        let held = Unreadable::new(&account.join("db_storage/session/session.db"));
        let err = create_backup(&account, &options, None, "1", &archive, &|_, _| {}).unwrap_err();
        drop(held);
        let message = format!("{err:#}");
        assert!(
            message.contains("db_storage/session/session.db"),
            "{message}"
        );
        assert_eq!(
            fs::read(&archive).unwrap(),
            before,
            "the backup before stays"
        );
        assert_eq!(
            fs::read_dir(&out).unwrap().count(),
            1,
            "nothing half written left"
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
}
