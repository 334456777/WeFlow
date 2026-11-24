//! `cache clear --web` (#1139): every entry of the store except what a running `weflow serve` or an install under way
//! still uses.
//!
//! - A version goes through [`WebStore::remove`], under its exclusive lock: the version a server shows (see
//!   [`WebStore::hold`]) stays, with its lock file and, when it is the one picked, `current`.
//! - A download folder or a half-written pick stays while the process that writes it may be running.
//! - A lock file without its version goes when nobody holds it: an install lock is held for as long as the install of
//!   that version runs.
//! - Anything else in the store is left over from an earlier WeFlow and goes.

use std::fs::{self, File, TryLockError};
use std::path::{Path, PathBuf};

use super::{download_pid, valid_version, WebStore};

/// An entry of the store, by its name.
#[derive(Debug, PartialEq, Eq)]
enum Entry {
    /// `<version>/`: a version kept.
    Version(String),
    /// `.lock-<version>` or `.install-<version>`: the lock of a version, or of its install.
    Lock(String),
    /// `.download-<version>-<pid>-<nonce>/` or `.current-<pid>`: written by process `pid`.
    Scratch(u32),
    /// `current`: the version picked.
    Current,
    /// Anything else.
    Other,
}

impl Entry {
    fn of(name: &str, is_dir: bool) -> Self {
        if name == "current" {
            return Self::Current;
        }
        if is_dir && valid_version(name) {
            return Self::Version(name.to_string());
        }
        let lock = name
            .strip_prefix(".lock-")
            .or_else(|| name.strip_prefix(".install-"));
        if let Some(version) = lock.filter(|v| valid_version(v)) {
            return Self::Lock(version.to_string());
        }
        let pid = name
            .strip_prefix(".download-")
            .and_then(download_owner)
            .or_else(|| name.strip_prefix(".current-")?.parse().ok());
        pid.map_or(Self::Other, Self::Scratch)
    }
}

/// The process in `<version>-<pid>-<nonce>`, the rest of a download's name; the version may hold dashes itself.
fn download_owner(rest: &str) -> Option<u32> {
    let (head, nonce) = rest.rsplit_once('-')?;
    let (version, pid) = head.rsplit_once('-')?;
    if !valid_version(version) {
        return None;
    }
    download_pid(&format!("{pid}-{nonce}"))
}

/// Whether someone holds a lock on the file at `path`; a lock file that is not there is held by nobody.
fn is_locked(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    matches!(file.try_lock(), Err(TryLockError::WouldBlock))
}

/// Removes a file or folder; what went, or why it could not.
fn remove_entry(path: &Path, removed: &mut Vec<PathBuf>, warnings: &mut Vec<String>) {
    let gone = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match gone {
        Ok(()) => removed.push(path.to_path_buf()),
        Err(err) => warnings.push(format!("{}: {err}", path.display())),
    }
}

/// Removes the lock file at `path` while holding it, so no process can be taking it; one held is left.
fn remove_unheld_lock(path: &Path, removed: &mut Vec<PathBuf>, warnings: &mut Vec<String>) {
    let file = match File::options().write(true).open(path) {
        Ok(file) => file,
        Err(err) => {
            warnings.push(format!("{}: {err}", path.display()));
            return;
        }
    };
    match file.try_lock() {
        Ok(()) => remove_entry(path, removed, warnings),
        Err(TryLockError::WouldBlock) => {}
        Err(TryLockError::Error(err)) => warnings.push(format!("{}: {err}", path.display())),
    }
}

impl WebStore {
    /// The entries of the store, sorted, with what each one is.
    fn entries(&self) -> Vec<(PathBuf, Entry)> {
        let Ok(read) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut entries: Vec<(PathBuf, Entry)> = read
            .flatten()
            .map(|e| {
                let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
                let entry = Entry::of(&e.file_name().to_string_lossy(), is_dir);
                (e.path(), entry)
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    /// Whether a running `weflow serve` shows `version` now ([`WebStore::hold`]).
    pub fn is_held(&self, version: &str) -> bool {
        valid_version(version) && is_locked(&self.lock_path(version))
    }

    /// What [`WebStore::clear`] would remove now: the versions no running `weflow serve` shows, with the pick when it
    /// names none that stays, and what downloads and installs that ended left. A version's lock files go with it and
    /// are not listed.
    pub fn clearable(&self) -> Vec<PathBuf> {
        let stays = |version: &str| self.dir(version).is_dir() && self.is_held(version);
        self.entries()
            .into_iter()
            .filter(|(path, entry)| match entry {
                Entry::Version(version) => !self.is_held(version),
                Entry::Lock(version) => !self.dir(version).exists() && !is_locked(path),
                Entry::Scratch(pid) => !crate::process::may_be_running(*pid),
                Entry::Current => !self.chosen().is_some_and(|v| stays(&v)),
                Entry::Other => true,
            })
            .map(|(path, _)| path)
            .collect()
    }

    /// Removes [`WebStore::clearable`]; returns what was removed, and what was left with why. A version a running
    /// `weflow serve` shows is left with a warning that says so.
    pub fn clear(&self) -> (Vec<PathBuf>, Vec<String>) {
        let (mut removed, mut warnings) = (Vec::new(), Vec::new());
        // the versions first, each while its exclusive lock is taken
        for (path, entry) in self.entries() {
            let Entry::Version(version) = entry else {
                continue;
            };
            match self.remove(&version) {
                Ok(true) => removed.push(path),
                Ok(false) => {}
                Err(e) if e.code == "in_use" => warnings.push(format!(
                    "WebUI {version} is shown by a running `weflow serve`; it was kept"
                )),
                Err(e) => warnings.push(e.message.clone()),
            }
        }
        // then what is left beside them: a version still here is one that stays
        for (path, entry) in self.entries() {
            match entry {
                Entry::Version(_) => {}
                Entry::Lock(version) => {
                    if !self.dir(&version).exists() {
                        remove_unheld_lock(&path, &mut removed, &mut warnings);
                    }
                }
                Entry::Scratch(pid) => {
                    if !crate::process::may_be_running(pid) {
                        remove_entry(&path, &mut removed, &mut warnings);
                    }
                }
                Entry::Current => {
                    if !self.chosen().is_some_and(|v| self.dir(&v).is_dir()) {
                        remove_entry(&path, &mut removed, &mut warnings);
                    }
                }
                Entry::Other => remove_entry(&path, &mut removed, &mut warnings),
            }
        }
        (removed, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> (weflow_native::fixture::TestDir, WebStore) {
        let root = weflow_native::fixture::TestDir::new(&format!("weflow-web-clear-{tag}"));
        let store = WebStore::new(root.join("web"));
        (root, store)
    }

    fn keep(store: &WebStore, version: &str) {
        let dir = store.dir(version);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("index.html"), b"<!doctype html>").unwrap();
    }

    #[test]
    fn entries_are_told_apart_by_name() {
        assert_eq!(Entry::of("0.6.0", true), Entry::Version("0.6.0".into()));
        assert_eq!(Entry::of("0.6.0", false), Entry::Other);
        assert_eq!(Entry::of("current", false), Entry::Current);
        assert_eq!(Entry::of(".lock-0.6.0", false), Entry::Lock("0.6.0".into()));
        assert_eq!(
            Entry::of(".install-0.6.0-rc.1", false),
            Entry::Lock("0.6.0-rc.1".into())
        );
        assert_eq!(
            Entry::of(".download-0.6.0-rc.1-1234-9f0a", true),
            Entry::Scratch(1234)
        );
        assert_eq!(Entry::of(".current-42", false), Entry::Scratch(42));
        for other in [
            "notes.txt",
            ".lock-",
            ".download-0.6.0",
            ".current-x",
            "latest",
        ] {
            assert_eq!(Entry::of(other, true), Entry::Other, "{other}");
        }
    }

    #[test]
    fn the_version_a_server_shows_stays_with_its_pick() {
        let (_root, store) = store("held");
        keep(&store, "0.5.0");
        keep(&store, "0.6.0");
        store.choose("0.6.0").unwrap();
        let hold = store.hold("0.6.0").unwrap();
        // a download by a process that is gone, the lock of an install that ended, and an earlier WeFlow's file
        let gone_pid = u32::MAX - 1;
        let stale = store
            .root()
            .join(format!(".download-0.7.0-{gone_pid}-9f0a"));
        fs::create_dir_all(&stale).unwrap();
        fs::write(store.root().join(".install-0.7.0"), b"").unwrap();
        fs::write(store.root().join("notes.txt"), b"x").unwrap();

        // in the order of their names
        let want = [
            stale,
            store.root().join(".install-0.7.0"),
            store.dir("0.5.0"),
            store.root().join("notes.txt"),
        ];
        assert_eq!(store.clearable(), want);

        let (mut removed, warnings) = store.clear();
        removed.sort();
        let mut want = want.to_vec();
        want.sort();
        assert_eq!(removed, want);
        assert_eq!(
            warnings,
            ["WebUI 0.6.0 is shown by a running `weflow serve`; it was kept"]
        );
        assert!(store.dir("0.6.0").join("index.html").is_file());
        assert_eq!(store.chosen().as_deref(), Some("0.6.0"));
        assert!(store.clearable().is_empty());

        // once the server stops, everything goes
        drop(hold);
        assert!(!store.is_held("0.6.0"));
        let (removed, warnings) = store.clear();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(removed.contains(&store.dir("0.6.0")), "{removed:?}");
        assert_eq!(fs::read_dir(store.root()).unwrap().count(), 0);
    }

    #[test]
    fn a_pick_of_a_version_not_kept_goes() {
        let (_root, store) = store("pick");
        store.choose("0.4.0").unwrap();
        fs::write(store.root().join(".lock-0.4.0"), b"").unwrap();
        assert_eq!(
            store.clearable(),
            [
                store.root().join(".lock-0.4.0"),
                store.root().join("current")
            ]
        );
        let (removed, warnings) = store.clear();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(removed.len(), 2, "{removed:?}");
        assert_eq!(fs::read_dir(store.root()).unwrap().count(), 0);
    }

    #[test]
    fn what_a_running_process_writes_stays() {
        let (_root, store) = store("running");
        let pid = std::process::id();
        let download = store.root().join(format!(".download-0.7.0-{pid}-9f0a"));
        fs::create_dir_all(&download).unwrap();
        fs::write(store.root().join(format!(".current-{pid}")), b"0.7.0\n").unwrap();
        let installing = store.root().join(".install-0.7.0");
        let lock = File::create(&installing).unwrap();
        lock.lock().unwrap();

        assert!(store.clearable().is_empty(), "{:?}", store.clearable());
        let (removed, warnings) = store.clear();
        assert!(
            removed.is_empty() && warnings.is_empty(),
            "{removed:?} {warnings:?}"
        );
        assert!(download.is_dir() && installing.is_file());
    }
}
