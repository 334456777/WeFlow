//! 数据库模式和图片模式共用的 `/proc` 访问。

use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Mapping {
    pub start: usize,
    pub end: usize,
    pub permissions: String,
    pub path: String,
}

pub(super) fn pid_t(pid: u32) -> io::Result<libc::pid_t> {
    libc::pid_t::try_from(pid).map_err(|_| io::Error::other("invalid process id"))
}

fn io_error(operation: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{operation}: {error}"))
}

pub(super) fn executable_path(pid: u32) -> io::Result<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .map_err(|error| io_error("cannot read WeChat's executable path", error))
}

pub(super) fn read_executable(path: &Path) -> io::Result<Vec<u8>> {
    std::fs::read(path).map_err(|error| io_error("cannot read WeChat's executable", error))
}

pub(super) fn thread_ids(pid: u32) -> io::Result<Vec<libc::pid_t>> {
    let directory = format!("/proc/{pid}/task");
    let entries = std::fs::read_dir(&directory)
        .map_err(|error| io_error(&format!("cannot list {directory}"), error))?;
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| io_error(&format!("cannot list {directory}"), error))?;
        if let Some(name) = entry.file_name().to_str() {
            if let Ok(tid) = name.parse() {
                ids.push(tid);
            }
        }
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

pub(super) fn mappings(pid: u32) -> io::Result<Vec<Mapping>> {
    let path = format!("/proc/{pid}/maps");
    let text = std::fs::read_to_string(&path)
        .map_err(|error| io_error(&format!("cannot read {path}"), error))?;
    let mut mappings = Vec::new();
    for line in text.lines() {
        // inode 一列有填充的空格，路径名里也可能有空格。
        let mut rest = line;
        let mut fields = [""; 5];
        for field in &mut fields {
            rest = rest.trim_start();
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            *field = &rest[..end];
            rest = &rest[end..];
        }
        if fields.iter().any(|field| field.is_empty()) {
            continue;
        }
        let Some((start, end)) = fields[0].split_once('-') else {
            continue;
        };
        let (Ok(start), Ok(end)) = (
            usize::from_str_radix(start, 16),
            usize::from_str_radix(end, 16),
        ) else {
            continue;
        };
        if end <= start {
            continue;
        }
        mappings.push(Mapping {
            start,
            end,
            permissions: fields[1].to_string(),
            path: rest.trim_start().to_string(),
        });
    }
    Ok(mappings)
}

/// 路径含有可执行文件路径的各个映射中最小的起始地址；桌面版的辅助程序取的也是这个最小值。
pub(super) fn base_address(maps: &[Mapping], executable: &Path) -> Option<usize> {
    let executable = executable.to_string_lossy();
    maps.iter()
        .filter(|mapping| mapping.path.contains(executable.as_ref()))
        .map(|mapping| mapping.start)
        .min()
}

pub(super) fn read_memory(pid: u32, address: usize, bytes: &mut [u8]) -> io::Result<()> {
    let read = read_memory_partial(pid, address, bytes)?;
    if read != bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "short process_vm_readv",
        ));
    }
    Ok(())
}

pub(super) fn read_memory_partial(pid: u32, address: usize, bytes: &mut [u8]) -> io::Result<usize> {
    let pid = pid_t(pid)?;
    let local = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let remote = libc::iovec {
        iov_base: address as *mut libc::c_void,
        iov_len: bytes.len(),
    };
    // SAFETY: 两个 iovec 都描述了长度与自身一致的有效缓冲区；内核要么填写 `bytes`，要么报告错误。
    let read = unsafe { libc::process_vm_readv(pid, &raw const local, 1, &raw const remote, 1, 0) };
    // 返回负数表示失败，原因在 errno 中
    usize::try_from(read).map_err(|_| io::Error::last_os_error())
}
