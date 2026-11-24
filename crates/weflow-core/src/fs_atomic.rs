//! Writing files through a temporary file renamed into place, so no reader ever sees one half written, and locking
//! them while they are read, changed and written back.

use std::io::Write;
use std::path::{Path, PathBuf};

/// A temporary name next to `path`, unique in this process.
pub(crate) fn temporary_sibling(path: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{}-{n}.tmp", std::process::id()));
    PathBuf::from(name)
}

/// Writes `bytes` to `path` through a temporary file renamed into place. Another thread never sees the file half
/// written (an export's media threads check for an image another one is still writing, issue #52), and two that
/// write it at once both leave a complete file.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = temporary_sibling(path);
    let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// [`write_atomically`] for a copy of `from`.
pub(crate) fn copy_atomically(from: &Path, path: &Path) -> std::io::Result<()> {
    let tmp = temporary_sibling(path);
    let copied = std::fs::copy(from, &tmp).and_then(|_| std::fs::rename(&tmp, path));
    if copied.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    copied
}

/// [`write_atomically`] for a file only its owner may read (#812): the new file is made as
/// [`crate::private_files::open_options`] makes one, whatever the mode of the file it replaces, and reaches the disk
/// before it replaces it.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = temporary_sibling(path);
    let written = (|| {
        let mut file = crate::private_files::open_options()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        rename_replacing(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// `fs::rename` over an existing file. On Windows the replace can fail for a moment while another process (a reader,
/// an antivirus scan) has the old file open, so it is retried briefly there.
pub(crate) fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempts = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(err)
                if cfg!(windows)
                    && err.kind() == std::io::ErrorKind::PermissionDenied
                    && attempts < 20 =>
            {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(err) => return Err(err),
        }
    }
}

/// An exclusive lock on `<path>.lock`, waited for, so that processes and threads reading `path`, changing it and
/// writing it back take turns; released when the file returned is dropped. The folder of `path` must be there.
pub(crate) fn lock_beside(path: &Path) -> std::io::Result<std::fs::File> {
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    let file = crate::private_files::open_options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(PathBuf::from(name))?;
    file.lock()?;
    Ok(file)
}

#[cfg(test)]
mod tests {

    #[test]
    fn atomic_writes_replace_the_file_and_leave_no_temporary_file() {
        let dir = weflow_native::fixture::TestDir::new("weflow-atomic");
        let path = dir.join("a.jpg");
        super::write_atomically(&path, b"first").unwrap();
        super::write_atomically(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let src = dir.join("src.bin");
        std::fs::write(&src, b"copied").unwrap();
        let copy = dir.join("b.jpg");
        super::copy_atomically(&src, &copy).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), b"copied");
        // a write that fails (no such folder) leaves nothing behind either
        assert!(super::write_atomically(&dir.join("missing").join("c.jpg"), b"x").is_err());
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["a.jpg", "b.jpg", "src.bin"]);
    }

    #[test]
    fn a_write_that_cannot_be_renamed_into_place_leaves_nothing_behind() {
        let dir = weflow_native::fixture::TestDir::new("weflow-atomic-rename");
        // a folder where the file goes: the temporary file is written, then renaming it over the folder fails
        let path = dir.join("a.wav");
        std::fs::create_dir(&path).unwrap();
        assert!(super::write_atomically(&path, b"RIFF....WAVE").is_err());
        let src = dir.join("src.mp4");
        std::fs::write(&src, b"video").unwrap();
        assert!(super::copy_atomically(&src, &path).is_err());
        assert!(path.is_dir(), "the folder is left as it was");
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["a.wav", "src.mp4"], "no temporary file is left");
    }

    #[test]
    fn a_private_write_replaces_the_file_or_leaves_it_as_it_was() {
        let dir = weflow_native::fixture::TestDir::new("weflow-atomic-private");
        let path = dir.join("records.json");
        std::fs::write(&path, b"old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        super::write_private(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the owner's only, whatever the file replaced");
        }
        // a folder where the file goes: renaming over it fails, and the folder and the file stay as they were
        let folder = dir.join("a.json");
        std::fs::create_dir(&folder).unwrap();
        assert!(super::write_private(&folder, b"x").is_err());
        assert!(folder.is_dir());
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["a.json", "records.json"],
            "no temporary file is left"
        );
    }
}
