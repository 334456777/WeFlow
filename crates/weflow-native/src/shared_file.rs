//! A file read from several threads at once, each read through a handle no other thread is using.
//!
//! On Windows a handle opened for synchronous I/O, as `File::open` opens one, serializes every read and query made
//! through it behind a lock of its file object: threads reading the same handle wait for one another, and each wait
//! is a context switch (#1105). A reader there takes a handle of its own, reopened from the first one so that it is
//! the same file whatever happened to its path, and leaves it for the next reader. Elsewhere positional reads and
//! queries through one handle do not wait for one another, and every reader uses the first.

use std::fs::File;
#[cfg(windows)]
use std::sync::Mutex;

/// One file and, on Windows, the handles readers left for the next ones.
pub(crate) struct SharedFile {
    first: File,
    /// Handles of the same file that no reader is using now.
    #[cfg(windows)]
    spare: Mutex<Vec<File>>,
}

impl SharedFile {
    pub(crate) fn new(first: File) -> Self {
        Self {
            first,
            #[cfg(windows)]
            spare: Mutex::default(),
        }
    }

    /// `f` with a handle of the file that no other thread is reading through, on Windows; the first handle when
    /// another cannot be opened (the file is being deleted), and on other systems.
    pub(crate) fn with<T>(&self, f: impl FnOnce(&File) -> T) -> T {
        #[cfg(windows)]
        {
            let spare = self.spare.lock().ok().and_then(|mut s| s.pop());
            if let Some(file) = spare.or_else(|| reopen(&self.first)) {
                let out = f(&file);
                if let Ok(mut s) = self.spare.lock() {
                    s.push(file);
                }
                return out;
            }
        }
        f(&self.first)
    }
}

/// A new handle, with a file object of its own, to the file `file` is open on, for reading; `None` when it cannot be
/// opened.
#[cfg(windows)]
fn reopen(file: &File) -> Option<File> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Win32::Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        ReOpenFile, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    // SAFETY: the handle is open for as long as `file` is borrowed. The share mode is the one `File::open` opens
    // with, so WeChat can go on writing, renaming and deleting the file.
    let handle = unsafe {
        ReOpenFile(
            file.as_raw_handle(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    // SAFETY: `ReOpenFile` returned a handle that is open and that nothing else owns or closes
    Some(unsafe { File::from_raw_handle(handle) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::TestDir;
    use std::io::Write;

    fn read(file: &File, at: u64, n: usize) -> Vec<u8> {
        let mut buf = vec![0u8; n];
        #[cfg(unix)]
        std::os::unix::fs::FileExt::read_exact_at(file, &mut buf, at).unwrap();
        #[cfg(windows)]
        {
            let got = std::os::windows::fs::FileExt::seek_read(file, &mut buf, at).unwrap();
            assert_eq!(got, n, "one read of {n} bytes at {at}");
        }
        buf
    }

    #[test]
    fn readers_on_several_threads_read_the_same_file() {
        let dir = TestDir::new("shared-file");
        let path = dir.path().join("data.bin");
        let data: Vec<u8> = (0..64 * 1024u32)
            .map(|i| u8::try_from(i % 251).unwrap())
            .collect();
        std::fs::write(&path, &data).unwrap();
        let shared = SharedFile::new(File::open(&path).unwrap());
        std::thread::scope(|s| {
            for t in 0..8u64 {
                let (shared, data) = (&shared, &data);
                s.spawn(move || {
                    for k in 0..200u64 {
                        let at = (t * 4099 + k * 517) % (64 * 1024 - 300);
                        let got = shared.with(|f| read(f, at, 300));
                        let at = usize::try_from(at).unwrap();
                        assert_eq!(got, data[at..at + 300], "bytes at {at}");
                    }
                });
            }
        });
    }

    #[test]
    fn a_reopened_handle_follows_the_file_past_a_rename() {
        let dir = TestDir::new("shared-file-rename");
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"first file").unwrap();
        let shared = SharedFile::new(File::open(&path).unwrap());
        let moved = dir.path().join("b.bin");
        std::fs::rename(&path, &moved).unwrap();
        let mut other = File::create(&path).unwrap();
        other.write_all(b"other file").unwrap();
        drop(other);
        assert_eq!(shared.with(|f| read(f, 0, 10)), b"first file");
        assert_eq!(shared.with(|f| read(f, 0, 10)), b"first file");
    }
}
