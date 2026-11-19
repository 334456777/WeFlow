//! The running WeChat clients, as `ListWeChatProcesses` and `GetDbKey` find
//! them: the main application first, never the AppEx, crashpad or helper
//! processes.

/// A running WeChat client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeChatProcess {
    /// The process id.
    pub pid: u32,
    /// The process's name.
    pub name: String,
    /// The executable.
    pub path: String,
    /// `/Applications/WeChat.app/Contents/MacOS/WeChat`, not a helper.
    pub main: bool,
}

fn path_of(pid: libc::pid_t) -> Option<String> {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: `buffer` is writable for the length passed; `proc_pidpath`
    // copies a NUL-terminated path and returns its length.
    let length = unsafe {
        libc::proc_pidpath(
            pid,
            buffer.as_mut_ptr().cast(),
            libc::c_uint::try_from(buffer.len()).unwrap_or(u32::MAX),
        )
    };
    if length <= 0 {
        return None;
    }
    let end = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(length as usize);
    Some(String::from_utf8_lossy(&buffer[..end]).into_owned())
}

fn name_of(pid: libc::pid_t) -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: `buffer` is writable for its length; `proc_name` copies at most
    // that many bytes and returns the length.
    let length = unsafe {
        libc::proc_name(
            pid,
            buffer.as_mut_ptr().cast(),
            libc::c_uint::try_from(buffer.len()).unwrap_or(u32::MAX),
        )
    };
    if length <= 0 {
        return None;
    }
    let end = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(length as usize);
    Some(String::from_utf8_lossy(&buffer[..end]).into_owned())
}

fn is_helper(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("wechatappex")
        || lower.contains("crashpad_handler")
        || lower.contains("helper")
        || lower.contains("/helpers/")
        || lower.contains("/plugins/")
}

/// Whether `path` is the main bundle binary rather than a helper.
pub(super) fn is_main(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    !is_helper(&lower) && lower.ends_with("/contents/macos/wechat")
}

fn belongs_to_wechat(name: &str, path: &str) -> bool {
    name == "WeChat"
        || name == "微信"
        || path.to_ascii_lowercase().ends_with("/wechat")
        || is_main(path)
}

/// How many pids to make room for when `proc_listallpids` counted `count`
/// processes: that count plus headroom for processes started before the
/// second call. `proc_listallpids` counts pids, not bytes.
fn pid_capacity(count: libc::c_int) -> usize {
    usize::try_from(count).map_or(0, |count| count + count / 4 + 64)
}

/// How many pids a `proc_listallpids` call that returned `returned` wrote into
/// a buffer of `capacity` pids: the return value is already a count of pids.
fn pids_written(returned: libc::c_int, capacity: usize) -> usize {
    usize::try_from(returned).map_or(0, |count| count.min(capacity))
}

/// The pids of every process, newest first.
fn all_pids() -> Vec<libc::pid_t> {
    // SAFETY: a null buffer only asks for the number of processes.
    let counted = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    let capacity = pid_capacity(counted);
    if counted < 0 {
        return Vec::new();
    }
    let Ok(buffer_bytes) = libc::c_int::try_from(capacity * std::mem::size_of::<libc::pid_t>())
    else {
        return Vec::new();
    };
    let mut pids: Vec<libc::pid_t> = vec![0; capacity];
    // SAFETY: `pids` is a live, writable buffer of `buffer_bytes` bytes, the
    // length passed; `proc_listallpids` writes at most that many.
    let returned = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), buffer_bytes) };
    pids.truncate(pids_written(returned, capacity));
    pids
}

/// Every `WeChat` process, the main application first.
pub fn wechat_processes() -> Vec<WeChatProcess> {
    let mut processes = Vec::new();
    for pid in all_pids().into_iter().filter(|pid| *pid > 0) {
        let Some(path) = path_of(pid) else {
            continue;
        };
        let Some(name) = name_of(pid) else {
            continue;
        };
        if !belongs_to_wechat(&name, &path) || is_helper(&path) {
            continue;
        }
        processes.push(WeChatProcess {
            pid: u32::try_from(pid).unwrap_or(0),
            name,
            path: path.clone(),
            main: is_main(&path),
        });
    }
    processes.sort_by(|left, right| {
        right
            .main
            .cmp(&left.main)
            .then_with(|| right.pid.cmp(&left.pid))
    });
    processes
}

/// The bundle's executable path, used to find `Info.plist`.
pub(super) fn executable_path(pid: u32) -> Result<String, String> {
    path_of(pid as libc::pid_t).ok_or_else(|| format!("CANNOT_GET_EXEC_PATH: pid {pid}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_main_binary_but_not_helpers() {
        assert!(is_main("/Applications/WeChat.app/Contents/MacOS/WeChat"));
        assert!(!is_main(
            "/Applications/WeChat.app/Contents/MacOS/WeChatAppEx"
        ));
        assert!(!is_main(
            "/Applications/WeChat.app/Contents/Frameworks/WeChatHelper.app/Contents/MacOS/WeChatHelper"
        ));
        assert!(belongs_to_wechat("WeChat", "/tmp/WeChat"));
        assert!(belongs_to_wechat("微信", "/tmp/微信"));
        assert!(!belongs_to_wechat(
            "Safari",
            "/Applications/Safari.app/Contents/MacOS/Safari"
        ));
    }

    #[test]
    fn the_pid_list_is_read_as_a_count_of_pids() {
        // #868: 494 processes counted must leave room for all of them, and a
        // call that wrote 475 pids yields all 475 rather than 475 / 4.
        let capacity = pid_capacity(494);
        assert!(capacity >= 494, "room for {capacity} pids");
        assert_eq!(pids_written(475, capacity), 475);
        // Never more than the buffer holds.
        assert_eq!(pids_written(700, 600), 600);
        assert_eq!(pids_written(-1, capacity), 0);
        assert_eq!(pid_capacity(-1), 0);
        assert_eq!(pid_capacity(0), 64);
    }
}
