//! 正在运行的微信进程，查找方式与桌面版的 `ListWeChatProcesses`、`GetDbKey` 相同：主程序排在最前，
//! 不含 AppEx、crashpad 和 helper 进程。

/// 一个正在运行的微信进程。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeChatProcess {
    /// 进程 ID。
    pub pid: u32,
    /// 进程名。
    pub name: String,
    /// 可执行文件的路径。
    pub path: String,
    /// 是否为主程序（`/Applications/WeChat.app/Contents/MacOS/WeChat`），而不是 helper。
    pub main: bool,
}

fn path_of(pid: libc::pid_t) -> Option<String> {
    let mut buffer = vec![0u8; usize::try_from(libc::PROC_PIDPATHINFO_MAXSIZE).unwrap_or(4096)];
    // SAFETY: `buffer` 在传入的长度内可写；`proc_pidpath` 复制一个以 NUL 结尾的路径并返回它的长度。
    let length = unsafe {
        libc::proc_pidpath(
            pid,
            buffer.as_mut_ptr().cast(),
            libc::c_uint::try_from(buffer.len()).unwrap_or(u32::MAX),
        )
    };
    // 返回长度；取不到这个进程的路径时为 0 或负数
    let length = usize::try_from(length).ok().filter(|&length| length > 0)?;
    let end = buffer.iter().position(|byte| *byte == 0).unwrap_or(length);
    Some(String::from_utf8_lossy(&buffer[..end]).into_owned())
}

fn name_of(pid: libc::pid_t) -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: `buffer` 在它的长度内可写；`proc_name` 最多复制这么多字节并返回长度。
    let length = unsafe {
        libc::proc_name(
            pid,
            buffer.as_mut_ptr().cast(),
            libc::c_uint::try_from(buffer.len()).unwrap_or(u32::MAX),
        )
    };
    // 返回长度；取不到这个进程的名字时为 0 或负数
    let length = usize::try_from(length).ok().filter(|&length| length > 0)?;
    let end = buffer.iter().position(|byte| *byte == 0).unwrap_or(length);
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

/// `path` 是否为应用包的主程序而不是 helper。
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

/// `proc_listallpids` 数出 `count` 个进程时要预留多少个 pid 的空间：这个数再加上余量，
/// 容纳第二次调用之前新启动的进程。`proc_listallpids` 数的是 pid 的个数，不是字节数。
fn pid_capacity(count: libc::c_int) -> usize {
    usize::try_from(count).map_or(0, |count| count + count / 4 + 64)
}

/// 返回 `returned` 的一次 `proc_listallpids` 调用往能容纳 `capacity` 个 pid 的缓冲区里写了多少个 pid：
/// 返回值本身就是 pid 的个数。
fn pids_written(returned: libc::c_int, capacity: usize) -> usize {
    usize::try_from(returned).map_or(0, |count| count.min(capacity))
}

/// 所有进程的 pid，最新的在前。
fn all_pids() -> Vec<libc::pid_t> {
    // SAFETY: 缓冲区为空指针时只查询进程数。
    let counted = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    let capacity = pid_capacity(counted);
    if counted < 0 {
        return Vec::new();
    }
    let Ok(buffer_bytes) = libc::c_int::try_from(capacity * size_of::<libc::pid_t>()) else {
        return Vec::new();
    };
    let mut pids: Vec<libc::pid_t> = vec![0; capacity];
    // SAFETY: `pids` 是存活的可写缓冲区，大小为传入的 `buffer_bytes` 字节；`proc_listallpids` 最多写入这么多。
    let returned = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), buffer_bytes) };
    pids.truncate(pids_written(returned, capacity));
    pids
}

/// 所有微信进程，主程序排在最前。
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

/// 应用包中可执行文件的路径，用来找 `Info.plist`。
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
        // #868：数出 494 个进程时必须为它们全部留出空间；返回值按 pid 的个数解读，写入 475 个 pid 的调用得到全部
        // 475 个（按字节数解读只会剩下 475 / 4 个）。
        let capacity = pid_capacity(494);
        assert!(capacity >= 494, "room for {capacity} pids");
        assert_eq!(pids_written(475, capacity), 475);
        // 不会超过缓冲区能容纳的个数。
        assert_eq!(pids_written(700, 600), 600);
        assert_eq!(pids_written(-1, capacity), 0);
        assert_eq!(pid_capacity(-1), 0);
        assert_eq!(pid_capacity(0), 64);
    }
}
