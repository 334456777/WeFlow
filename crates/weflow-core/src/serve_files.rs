//! The files `weflow serve` writes for `?return=file` requests that name no `--out` (#815).
//!
//! They go under `<cache>/serve-files/`, into a folder of each running server's own, `<id>/`, held by an exclusive
//! lock on `<id>.lock` beside it for as long as the server runs. The lock ends with the process however it ends, so a
//! server starting, and
//! `cache clear --api`, remove only what no running server holds: two servers on one `WEFLOW_HOME` keep each other's
//! files, and the folders of servers that stopped go.

use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

const LOCK_SUFFIX: &str = ".lock";

/// How many ids [`ServeFiles::hold`] tries before giving up. An id is passed over only when its lock file already
/// exists or a cleanup removed it between its making and its locking, both rare, so a few attempts are plenty.
const HOLD_ATTEMPTS: usize = 8;

/// A running server's own folder under `serve-files/`, held while this lives and removed when it is dropped.
#[derive(Debug)]
pub struct ServeFiles {
    dir: PathBuf,
    lock_path: PathBuf,
    lock: Option<File>,
}

impl ServeFiles {
    /// A new folder of this server's own under `root`, made and held.
    ///
    /// # Errors
    /// When the folder or its lock file cannot be made, or the lock taken, or none of the few new lock files tried
    /// stayed in place.
    pub fn hold(root: &Path) -> io::Result<Self> {
        Self::hold_with(root, rand::random::<u64>)
    }

    /// [`ServeFiles::hold`] with the ids drawn from `next_id`.
    fn hold_with(root: &Path, mut next_id: impl FnMut() -> u64) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        for _ in 0..HOLD_ATTEMPTS {
            let id = format!("{:016x}", next_id());
            let lock_path = root.join(format!("{id}{LOCK_SUFFIX}"));
            let lock = match File::create_new(&lock_path) {
                Ok(lock) => lock,
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            };
            lock.lock()?;
            // Until the lock is taken the file has no folder and nobody holding it, which a cleanup removes as a
            // lock file left alone. A cleanup removes it while holding its lock, so once the lock is taken here
            // any such removal is over, and the file at `lock_path` is still this one exactly when it is held.
            if !is_held(&lock_path) {
                continue;
            }
            let dir = root.join(id);
            fs::create_dir_all(&dir)?;
            return Ok(Self {
                dir,
                lock_path,
                lock: Some(lock),
            });
        }
        Err(io::Error::other(format!(
            "{}: none of {HOLD_ATTEMPTS} new lock files stayed in place",
            root.display()
        )))
    }

    /// Where this server writes the files of its requests, one folder each.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for ServeFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
        drop(self.lock.take());
        let _ = fs::remove_file(&self.lock_path);
    }
}

/// Removes what under `root` no running server holds; returns what was removed, and what could not be with why.
///
/// That is the folders of servers that stopped with their lock files, and anything else there (a folder of an older
/// WeFlow, which kept no lock). Each folder is removed while its lock is taken here, so no server can be starting
/// with it.
pub fn remove_unheld(root: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let (mut removed, mut warnings) = (Vec::new(), Vec::new());
    let Ok(entries) = fs::read_dir(root) else {
        return (removed, warnings);
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let Some(name) = name else { continue };
        if let Some(id) = name.strip_suffix(LOCK_SUFFIX) {
            // a lock file goes with its folder, below; one left alone goes when nobody holds it
            if !root.join(id).exists() {
                remove_if_unheld(&path, None, &mut removed, &mut warnings);
            }
            continue;
        }
        let lock = root.join(format!("{name}{LOCK_SUFFIX}"));
        remove_if_unheld(&lock, Some(&path), &mut removed, &mut warnings);
    }
    (removed, warnings)
}

/// Removes `folder` (when given) and the lock file `lock` unless a running server holds the lock; a folder without
/// a lock file is nobody's.
fn remove_if_unheld(
    lock: &Path,
    folder: Option<&Path>,
    removed: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
) {
    let held = match File::options().write(true).open(lock) {
        Ok(file) => match file.try_lock() {
            Ok(()) => Some(file),
            Err(TryLockError::WouldBlock) => return,
            Err(TryLockError::Error(err)) => {
                warnings.push(format!("{}: {err}", lock.display()));
                return;
            }
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => {
            warnings.push(format!("{}: {err}", lock.display()));
            return;
        }
    };
    if let Some(folder) = folder {
        let gone = if folder.is_dir() {
            fs::remove_dir_all(folder)
        } else {
            fs::remove_file(folder)
        };
        match gone {
            Ok(()) => removed.push(folder.to_path_buf()),
            Err(err) => {
                warnings.push(format!("{}: {err}", folder.display()));
                return;
            }
        }
    }
    if let Some(file) = held {
        // removed while still locked, so a server that was waiting for this lock finds the file gone once it has it
        match fs::remove_file(lock) {
            Ok(()) => removed.push(lock.to_path_buf()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => warnings.push(format!("{}: {err}", lock.display())),
        }
        drop(file);
    }
}

/// Whether a lock is held on the file now at `lock`. A server asks this of its own lock file to learn whether the
/// file there is still the one it locked: a second handle of the same process is refused the lock as well (`flock`
/// locks belong to the open file description, `LockFileEx` locks to the handle).
fn is_held(lock: &Path) -> bool {
    let Ok(file) = File::open(lock) else {
        return false;
    };
    matches!(file.try_lock(), Err(TryLockError::WouldBlock))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> weflow_native::fixture::TestDir {
        weflow_native::fixture::TestDir::new(&format!("weflow-serve-files-{tag}"))
    }

    #[test]
    fn a_server_keeps_its_folder_from_the_others() {
        let root = root("held");
        let first = ServeFiles::hold(&root).unwrap();
        fs::write(first.dir().join("kept.json"), b"{}").unwrap();
        // what a server that was killed left (its lock ended with it), and a folder of an older WeFlow
        let killed = root.join("aaaaaaaaaaaaaaaa");
        fs::create_dir_all(&killed).unwrap();
        fs::write(killed.join("export.zip"), b"zip").unwrap();
        fs::write(root.join("aaaaaaaaaaaaaaaa.lock"), b"").unwrap();
        let older = root.join("0123456789abcdef");
        fs::create_dir_all(&older).unwrap();

        let second = ServeFiles::hold(&root).unwrap();
        let (removed, warnings) = remove_unheld(&root);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(first.dir().join("kept.json").exists());
        assert!(second.dir().exists());
        for gone in [&older, &killed, &root.join("aaaaaaaaaaaaaaaa.lock")] {
            assert!(!gone.exists() && removed.contains(gone), "{gone:?}");
        }

        let first_dir = first.dir().to_path_buf();
        drop(first);
        assert!(
            !first_dir.exists(),
            "a server removes its folder when it stops"
        );
        // the second server's folder and lock file are all that is left
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    }

    #[test]
    fn a_lock_file_alone_goes_when_nobody_holds_it() {
        let root = root("stray");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("feedfacecafebeef.lock"), b"").unwrap();
        let (removed, _) = remove_unheld(&root);
        assert_eq!(removed, [root.join("feedfacecafebeef.lock")]);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn a_lock_file_is_held_only_while_it_is_the_one_locked() {
        let root = root("is-held");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("0000000000000001.lock");
        let lock = File::create_new(&path).unwrap();
        assert!(!is_held(&path), "nobody has locked it yet");
        lock.lock().unwrap();
        assert!(
            is_held(&path),
            "a second handle of the same process is refused the lock"
        );

        fs::remove_file(&path).unwrap();
        assert!(!is_held(&path), "the locked file was removed");
        let _other = File::create_new(&path).unwrap();
        assert!(!is_held(&path), "another file took its place");
    }

    #[test]
    fn a_server_passes_over_an_id_already_taken() {
        let root = root("taken");
        fs::create_dir_all(&root).unwrap();
        let taken = root.join("0000000000000001.lock");
        fs::write(&taken, b"").unwrap();
        let mut ids = [1, 2].into_iter();
        let held = ServeFiles::hold_with(&root, || ids.next().unwrap()).unwrap();
        assert_eq!(held.dir(), root.join("0000000000000002"));
        assert!(taken.exists());

        let err = ServeFiles::hold_with(&root, || 1).unwrap_err();
        assert!(err.to_string().contains("stayed in place"), "{err}");
    }
}
