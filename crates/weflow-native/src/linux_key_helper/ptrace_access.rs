//! 本进程能否用 `ptrace` 附加到微信，在为挂钩重启微信之前就能知道：
//! 由 Yama 的 `ptrace_scope` 和本进程的有效能力集决定。

/// Yama 设置所在的文件；内核没有启用 Yama 时这个文件不存在。
const PTRACE_SCOPE: &str = "/proc/sys/kernel/yama/ptrace_scope";

/// `CAP_SYS_PTRACE` 在 `/proc/<pid>/status` 的能力掩码中的位。
const CAP_SYS_PTRACE: u32 = 19;

/// `ptrace` 能否附加到同一用户的其他进程。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtraceAccess {
    /// 允许附加，或者无法排除附加的可能。
    Allowed,
    /// `ptrace_scope`（1 或 2）只允许有 `CAP_SYS_PTRACE` 的进程附加，而本进程没有：
    /// 以 root 运行，或者把它设为 0，就能附加。
    NeedsCapability {
        /// `ptrace_scope` 的值。
        scope: u8,
    },
    /// `ptrace_scope` 为 3：重启系统之前任何进程（包括 root）都不能附加。
    Disabled,
}

/// 根据 `/proc/sys/kernel/yama/ptrace_scope` 和 `/proc/self/status` 判断本进程现在的权限。
#[must_use]
pub fn ptrace_access() -> PtraceAccess {
    let scope = std::fs::read_to_string(PTRACE_SCOPE).ok();
    let status = std::fs::read_to_string("/proc/self/status").ok();
    decide(
        scope.as_deref().and_then(|text| text.trim().parse().ok()),
        status.as_deref().and_then(has_sys_ptrace),
    )
}

/// `ptrace_scope` 为 `scope`（没有 Yama 时为 `None`）、有无 `sys_ptrace` 能力（未知时为 `None`）时对应的权限。
fn decide(scope: Option<u8>, sys_ptrace: Option<bool>) -> PtraceAccess {
    match (scope, sys_ptrace) {
        (Some(3), _) => PtraceAccess::Disabled,
        (Some(scope @ (1 | 2)), Some(false)) => PtraceAccess::NeedsCapability { scope },
        _ => PtraceAccess::Allowed,
    }
}

/// `/proc/<pid>/status` 的文本中，有效能力集（`CapEff`）是否包含 `CAP_SYS_PTRACE`。
fn has_sys_ptrace(status: &str) -> Option<bool> {
    let mask = status
        .lines()
        .find_map(|line| line.strip_prefix("CapEff:"))?
        .trim();
    let mask = u64::from_str_radix(mask, 16).ok()?;
    Some((mask >> CAP_SYS_PTRACE) & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scope_and_the_capability_decide_the_access() {
        assert_eq!(decide(None, Some(false)), PtraceAccess::Allowed);
        assert_eq!(decide(Some(0), Some(false)), PtraceAccess::Allowed);
        for scope in [1, 2] {
            assert_eq!(
                decide(Some(scope), Some(false)),
                PtraceAccess::NeedsCapability { scope }
            );
            assert_eq!(decide(Some(scope), Some(true)), PtraceAccess::Allowed);
            assert_eq!(
                decide(Some(scope), None),
                PtraceAccess::Allowed,
                "unknown capabilities are not refused"
            );
        }
        assert_eq!(decide(Some(3), Some(true)), PtraceAccess::Disabled);
    }

    #[test]
    fn reads_cap_sys_ptrace_from_the_effective_mask() {
        let status = |mask: &str| {
            format!("Name:\tweflow\nCapInh:\t0000000000000000\nCapEff:\t{mask}\nCapBnd:\t000001ffffffffff\n")
        };
        assert_eq!(has_sys_ptrace(&status("0000000000000000")), Some(false));
        assert_eq!(
            has_sys_ptrace(&status("000001ffffffffff")),
            Some(true),
            "root"
        );
        assert_eq!(
            has_sys_ptrace(&status("0000000000080000")),
            Some(true),
            "only CAP_SYS_PTRACE"
        );
        assert_eq!(has_sys_ptrace(&status("0000000000040000")), Some(false));
        assert_eq!(has_sys_ptrace("Name:\tweflow\n"), None);
    }
}
