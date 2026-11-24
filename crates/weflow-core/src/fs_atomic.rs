//! Writing files through a temporary file renamed into place, so no reader ever sees one half written.

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
}
