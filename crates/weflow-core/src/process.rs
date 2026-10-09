//! Running processes by image name. On Windows they come from a ToolHelp snapshot of the process list instead of
//! starting `tasklist` and parsing its CSV output.

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

/// Process ids of the running processes whose executable file name is one of `images` (see [`pick_pids`] for the
/// order). Empty when the process list cannot be read.
#[cfg(windows)]
pub fn pids_by_image(images: &[&str]) -> Vec<u32> {
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
    let mut entries = Vec::new();
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
        entries.push((
            String::from_utf16_lossy(&entry.szExeFile[..length]),
            entry.th32ProcessID,
        ));
        // SAFETY: as for Process32FirstW
        available = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: the handle is ours and closed once
    unsafe { CloseHandle(snapshot) };
    pick_pids(entries, images)
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
    }
}
