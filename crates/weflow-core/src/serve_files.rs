//! The files `weflow serve` writes for `?return=file` requests that name no `--out`, under `<cache>/serve-files/`
//! (#815): a folder of each running server's own, `<id>/`, held by an exclusive lock on `<id>.lock` beside it for as
//! long as the server runs. The lock ends with the process however it ends, so a server starting, and
//! `cache clear --api`, remove only what no running server holds: two servers on one `WEFLOW_HOME` keep each other's
//! files, and the folders of servers that stopped go.

use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

const LOCK_SUFFIX: &str = ".lock";

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
    /// When the folder or its lock file cannot be made, or the lock taken.
    pub fn hold(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        let id = format!("{:016x}", rand::random::<u64>());
        let lock_path = root.join(format!("{id}{LOCK_SUFFIX}"));
        let lock = File::create(&lock_path)?;
        lock.lock()?;
        let dir = root.join(id);
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            lock_path,
            lock: Some(lock),
        })
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

/// What under `root` no running server holds: the folders of servers that stopped with their lock files, and
/// anything else there (a folder of an older WeFlow, which kept no lock). Each folder is removed while its lock is
/// taken here, so no server can be starting with it. Returns what was removed, and what could not be with why.
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
        drop(file);
        match fs::remove_file(lock) {
            Ok(()) => removed.push(lock.to_path_buf()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => warnings.push(format!("{}: {err}", lock.display())),
        }
    }
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
}
