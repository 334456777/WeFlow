//! Running processes by image name, and their command lines. On Windows the list comes from a ToolHelp snapshot of
//! the process list.

/// Process ids of the `(image name, pid)` entries whose image is one of `images`, ignoring ASCII case. The ids are
/// grouped in the order of `images`, each group in the order of `entries` (the order the system lists them).
pub fn pick_pids(entries: impl IntoIterator<Item = (String, u32)>, images: &[&str]) -> Vec<u32> {
    let mut found: Vec<(usize, u32)> = entries
        .into_iter()
        .filter_map(|(name, pid)| {
            let rank = images
                .iter()
                .position(|image| image.eq_ignore_ascii_case(&name))?;
            Some((rank, pid))
        })
        .collect();
    // stable: keeps the system's order within one image name
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, pid)| pid).collect()
}

/// A running process: its executable file name, id and parent's id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub image: String,
    pub pid: u32,
    pub parent: u32,
}

/// The running processes, in the order the system lists them. Empty when the process list cannot be read.
#[cfg(windows)]
pub fn processes() -> Vec<Process> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    // SAFETY: a snapshot of all processes (the pid argument is ignored for TH32CS_SNAPPROCESS)
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let mut list = Vec::new();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the struct is a few hundred bytes"
    )]
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: `snapshot` is a valid snapshot handle and `entry` has its size set, as both calls require
    let mut available = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while available {
        let length = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        list.push(Process {
            image: String::from_utf16_lossy(&entry.szExeFile[..length]),
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
        });
        // SAFETY: as for Process32FirstW
        available = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: the handle is ours and closed once
    unsafe { CloseHandle(snapshot) };
    list
}

/// Process ids of the running processes whose executable file name is one of `images` (see [`pick_pids`] for the
/// order). Empty when the process list cannot be read.
#[cfg(windows)]
pub fn pids_by_image(images: &[&str]) -> Vec<u32> {
    pick_pids(processes().into_iter().map(|p| (p.image, p.pid)), images)
}

/// The command line process `pid` was started with (#101). `None` when it cannot be opened (another user's, or one
/// running as administrator while this one does not) or its command line read.
///
/// It needs only `PROCESS_QUERY_LIMITED_INFORMATION`, which any process of the same user grants, and asks the system
/// for the command line (`ProcessCommandLineInformation`, Windows 8.1 and later) instead of reading the other
/// process's memory.
#[cfg(windows)]
pub fn command_line(pid: u32) -> Option<String> {
    use windows_sys::Wdk::System::Threading::{
        NtQueryInformationProcess, ProcessCommandLineInformation,
    };
    use windows_sys::Win32::Foundation::{
        CloseHandle, STATUS_INFO_LENGTH_MISMATCH, UNICODE_STRING,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: a handle to query `pid`, closed below; a process that is gone or not ours gives null
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    // a `UNICODE_STRING` with the characters right after it; u64 words keep it aligned
    let mut buf: Vec<u64> = vec![0; 128];
    let mut status = 0;
    for _ in 0..2 {
        let mut needed = 0u32;
        let len = u32::try_from(buf.len() * 8).unwrap_or(u32::MAX);
        // SAFETY: `handle` is open with the right to query, and `buf` holds the `len` bytes passed
        status = unsafe {
            NtQueryInformationProcess(
                handle,
                ProcessCommandLineInformation,
                buf.as_mut_ptr().cast(),
                len,
                &mut needed,
            )
        };
        if status != STATUS_INFO_LENGTH_MISMATCH {
            break;
        }
        buf = vec![0; (needed as usize).div_ceil(8)];
    }
    // SAFETY: the handle is ours and closed once
    unsafe { CloseHandle(handle) };
    if status < 0 {
        return None;
    }
    // SAFETY: on success the buffer starts with a UNICODE_STRING, aligned by the u64 words
    let text = unsafe { &*buf.as_ptr().cast::<UNICODE_STRING>() };
    let chars = usize::from(text.Length) / 2;
    if chars == 0 || text.Buffer.is_null() {
        return Some(String::new());
    }
    // SAFETY: the system points `Buffer` at `Length` bytes inside `buf`, which is still alive
    let units = unsafe { std::slice::from_raw_parts(text.Buffer, chars) };
    Some(String::from_utf16_lossy(units))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(list: &[(&str, u32)]) -> Vec<(String, u32)> {
        list.iter()
            .map(|(name, pid)| (name.to_string(), *pid))
            .collect()
    }

    #[test]
    fn groups_by_image_in_system_order() {
        let list = entries(&[
            ("WeChat.exe", 7),
            ("Weixin.exe", 30),
            ("explorer.exe", 4),
            ("Weixin.exe", 12),
        ]);
        assert_eq!(pick_pids(list, &["Weixin.exe", "WeChat.exe"]), [30, 12, 7]);
    }

    #[test]
    fn matches_image_names_ignoring_case_only() {
        let list = entries(&[
            ("WEIXIN.EXE", 1),
            ("weixin.exe", 2),
            ("Weixin.exe.bak", 3),
            ("Weixin", 4),
            ("WeixinUpdate.exe", 5),
        ]);
        assert_eq!(pick_pids(list, &["Weixin.exe"]), [1, 2]);
        assert!(pick_pids(Vec::new(), &["Weixin.exe"]).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn finds_this_process() {
        let exe = std::env::current_exe().unwrap();
        let name = exe
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_ascii_uppercase();
        assert!(pids_by_image(&[&name]).contains(&std::process::id()));
        assert!(pids_by_image(&["no-such-image-weflow.exe"]).is_empty());
        let me = processes()
            .into_iter()
            .find(|p| p.pid == std::process::id())
            .unwrap();
        assert!(me.image.eq_ignore_ascii_case(&name));
        assert_ne!(me.parent, 0);
    }

    /// Run by `reads_the_command_line_of_another_process` as the child whose command line it reads.
    #[test]
    #[ignore = "a child process of another test"]
    fn sleeps_while_its_command_line_is_read() {
        std::thread::sleep(std::time::Duration::from_secs(20));
    }

    #[cfg(windows)]
    #[test]
    fn reads_the_command_line_of_another_process() {
        let marker = "process::tests::sleeps_while_its_command_line_is_read";
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", marker])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let line = command_line(child.id());
        let _ = child.kill();
        let _ = child.wait();
        let line = line.expect("a child of this process can be queried");
        assert!(line.contains(marker), "{line}");
        assert!(line.contains("--exact"), "{line}");
        assert_eq!(command_line(u32::MAX), None, "no such process");
    }
}
