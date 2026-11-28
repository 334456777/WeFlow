//! 由多个线程同时读取的文件，每次读取都通过没有其他线程在用的句柄。
//!
//! 在 Windows 上，以同步 I/O 打开的句柄（`File::open` 就是这样打开的）对通过它进行的每次读取和查询，
//! 都要逐个取得文件对象的锁：读同一个句柄的线程互相等待，每次等待都是一次上下文切换（#1105）。
//! 因此 Windows 上每个读取者用自己的句柄：它从第一个句柄重新打开，无论路径后来怎样变化都指向同一个文件，
//! 用完后留给下一个读取者。其他系统上通过同一个句柄的按位置读取和查询不互相等待，所有读取者都用第一个句柄。

use std::fs::File;
#[cfg(windows)]
use std::sync::Mutex;

/// 一个文件，以及（Windows 上）读取者留给后来者的句柄。
pub(crate) struct SharedFile {
    first: File,
    /// 同一个文件的、当前没有读取者在用的句柄。
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

    /// 用文件的句柄运行 `f`：Windows 上是没有其他线程在用的句柄，另开不了（文件正在被删除）时用第一个句柄；
    /// 其他系统上一律用第一个句柄。
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

/// 为 `file` 所打开的文件另开一个用于读取的新句柄，它有自己的文件对象；打不开时为 `None`。
#[cfg(windows)]
fn reopen(file: &File) -> Option<File> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Win32::Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        ReOpenFile, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    // SAFETY: 借用 `file` 期间句柄一直打开。共享模式与 `File::open` 打开时的相同，
    // 所以微信仍能写入、重命名和删除这个文件。
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
    // SAFETY: `ReOpenFile` 返回的句柄是打开的，没有其他地方拥有或关闭它
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
