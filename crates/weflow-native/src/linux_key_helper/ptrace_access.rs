//! Whether this process may attach to WeChat with `ptrace`, known before WeChat is restarted for the hook: Yama's
//! `ptrace_scope` and the effective capabilities of this process decide it.

/// Where Yama's setting is; the file is missing when the kernel has no Yama.
const PTRACE_SCOPE: &str = "/proc/sys/kernel/yama/ptrace_scope";

/// The bit of `CAP_SYS_PTRACE` in the capability masks of `/proc/<pid>/status`.
const CAP_SYS_PTRACE: u32 = 19;

/// Whether `ptrace` may attach to another process of the same user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtraceAccess {
    /// Attaching is allowed, or could not be ruled out.
    Allowed,
    /// `ptrace_scope` (1 or 2) allows attaching only with `CAP_SYS_PTRACE`, which this process does not have:
    /// running as root, or setting the scope to 0, allows it.
    NeedsCapability { scope: u8 },
    /// `ptrace_scope` is 3: no process may attach, root included, until the system restarts.
    Disabled,
}

/// The access this process has now, from `/proc/sys/kernel/yama/ptrace_scope` and `/proc/self/status`.
#[must_use]
pub fn ptrace_access() -> PtraceAccess {
    let scope = std::fs::read_to_string(PTRACE_SCOPE).ok();
    let status = std::fs::read_to_string("/proc/self/status").ok();
    decide(
        scope.as_deref().and_then(|text| text.trim().parse().ok()),
        status.as_deref().and_then(has_sys_ptrace),
    )
}

/// The access that a `ptrace_scope` of `scope` (`None` without Yama) and the capability `sys_ptrace` (`None` when
/// unknown) give.
fn decide(scope: Option<u8>, sys_ptrace: Option<bool>) -> PtraceAccess {
    match (scope, sys_ptrace) {
        (Some(3), _) => PtraceAccess::Disabled,
        (Some(scope @ (1 | 2)), Some(false)) => PtraceAccess::NeedsCapability { scope },
        _ => PtraceAccess::Allowed,
    }
}

/// Whether the effective capabilities (`CapEff`) in the text of `/proc/<pid>/status` include `CAP_SYS_PTRACE`.
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
