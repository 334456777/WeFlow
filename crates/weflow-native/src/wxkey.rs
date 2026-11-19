//! Database-key extraction, one implementation of [`WxKey`] per platform.

use anyhow::Result;

/// Database-key extraction on a platform without an implementation; `is_available` is false and
/// `key db` reports that.
#[cfg(not(any(
    windows,
    target_os = "macos",
    all(target_os = "linux", target_arch = "x86_64")
)))]
pub struct WxKey;

#[cfg(not(any(
    windows,
    target_os = "macos",
    all(target_os = "linux", target_arch = "x86_64")
)))]
impl WxKey {
    /// The extractor of this platform.
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Always false.
    pub fn is_available(&self) -> bool {
        false
    }

    /// Always fails: there is no extraction on this platform.
    pub fn get_db_key_with_tick(
        &self,
        _pid: u32,
        _timeout: std::time::Duration,
        _on_status: &mut dyn FnMut(&str, i32),
        _on_tick: &mut dyn FnMut(u64),
    ) -> std::result::Result<String, DbKeyError> {
        Err(DbKeyError::Other(
            "database-key extraction is not available on this platform".into(),
        ))
    }
}

/// Database-key extraction on Windows: a hook in the WeChat process catches the key when the user logs in.
#[cfg(windows)]
pub struct WxKey;

#[cfg(windows)]
impl WxKey {
    /// The extractor of this platform.
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
    /// Whether this build can extract the key: on x64 only.
    pub fn is_available(&self) -> bool {
        cfg!(target_arch = "x86_64")
    }
    /// Hooks the WeChat process `pid` and waits up to `timeout` for the key, as 64 lowercase hex characters.
    ///
    /// `on_status` hears each step with a level (0 information, 1 progress, 2 a failure's diagnostics); `on_tick`
    /// hears the seconds left whenever they change.
    ///
    /// # Errors
    /// [`DbKeyError`]: the process cannot be opened or is not ready, no key came in time, or the run was
    /// interrupted.
    pub fn get_db_key_with_tick(
        &self,
        pid: u32,
        timeout: std::time::Duration,
        on_status: &mut dyn FnMut(&str, i32),
        on_tick: &mut dyn FnMut(u64),
    ) -> std::result::Result<String, DbKeyError> {
        #[cfg(target_arch = "x86_64")]
        {
            let start = std::time::Instant::now();
            let mut hook = crate::windows_db_key::Hook::start(pid).map_err(|error| {
                let error = error.to_string();
                if error == "interrupted by user" {
                    DbKeyError::Interrupted
                } else if error.contains("ACCESS_DENIED") {
                    DbKeyError::AccessDenied(error)
                } else if error.contains("is not loaded yet") || error.contains("os error 24") {
                    DbKeyError::NotReady(error)
                } else {
                    DbKeyError::Other(error)
                }
            })?;
            on_status(&format!("WeChat version {}", hook.version), 0);
            on_status("Hook installed; log in to WeChat now", 1);
            let mut last_tick = u64::MAX;
            while start.elapsed() < timeout {
                let left = timeout.saturating_sub(start.elapsed()).as_secs();
                if left != last_tick {
                    last_tick = left;
                    on_tick(left);
                }
                if let Some(key) = hook.poll().map_err(|e| {
                    if e.to_string() == "interrupted by user" {
                        DbKeyError::Interrupted
                    } else {
                        DbKeyError::Other(e.to_string())
                    }
                })? {
                    hook.cleanup()
                        .map_err(|e| DbKeyError::Other(e.to_string()))?;
                    on_status("key obtained", 1);
                    return Ok(key);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            hook.cleanup()
                .map_err(|e| DbKeyError::Other(e.to_string()))?;
            let diagnostics = hook.diagnostics();
            on_status(
                &format!(
                    "Capture diagnostics: {}",
                    serde_json::to_string(&diagnostics).unwrap_or_default()
                ),
                2,
            );
            Err(DbKeyError::Timeout)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = (pid, timeout, on_status, on_tick);
            Err(DbKeyError::Other(
                "Windows database-key extraction requires x64".into(),
            ))
        }
    }
}

/// Database-key extraction on Linux x64: `ptrace` stops WeChat where it takes the key.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub struct WxKey;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl WxKey {
    /// The extractor of this platform.
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Always true.
    pub fn is_available(&self) -> bool {
        true
    }

    /// Reads WeChat's ELF image, attaches with ptrace, and waits for the key.
    pub fn get_db_key_with_tick(
        &self,
        pid: u32,
        timeout: std::time::Duration,
        on_status: &mut dyn FnMut(&str, i32),
        on_tick: &mut dyn FnMut(u64),
    ) -> std::result::Result<String, DbKeyError> {
        let mut hook = crate::linux_key_helper::Hook::start(pid).map_err(db_key_error)?;
        on_status("Hook installed; log in to WeChat now", 1);
        match hook.wait(timeout, on_tick) {
            Ok(key) => {
                on_status("key obtained", 1);
                Ok(key)
            }
            Err(error) => Err(db_key_error(error)),
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn db_key_error(message: String) -> DbKeyError {
    if message == "interrupted by user" {
        DbKeyError::Interrupted
    } else if message == "Timeout waiting for 32-byte key" {
        DbKeyError::Timeout
    } else if message.contains("Failed to attach to any thread")
        || message.contains("ACCESS_DENIED")
    {
        DbKeyError::AccessDenied(message)
    } else if message == "CANNOT_GET_EXEC_PATH"
        || message == "CANNOT_GET_BASE_ADDR"
        || message.contains("Failed to open task directory")
    {
        DbKeyError::NotReady(message)
    } else {
        DbKeyError::Other(message)
    }
}

/// Database-key extraction on macOS: DTrace on x86_64, `ptrace` at the key's sink on ARM64.
#[cfg(target_os = "macos")]
pub struct WxKey;

#[cfg(target_os = "macos")]
impl WxKey {
    /// The extractor of this platform.
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Always true.
    pub fn is_available(&self) -> bool {
        true
    }

    /// On x86_64 the key comes from DTrace watching WeChat's call to
    /// CommonCrypto's `CCKeyDerivationPBKDF`; on ARM64 the Rust scanner finds
    /// the database-key sink and `ptrace` stops the client there.
    pub fn get_db_key_with_tick(
        &self,
        pid: u32,
        timeout: std::time::Duration,
        on_status: &mut dyn FnMut(&str, i32),
        on_tick: &mut dyn FnMut(u64),
    ) -> std::result::Result<String, DbKeyError> {
        #[cfg(target_arch = "x86_64")]
        {
            let mut hook =
                crate::macos_key_helper::DTraceHook::start(pid).map_err(macos_db_key_error)?;
            on_status("Hook installed; log in to WeChat now", 1);
            match hook.wait(timeout, on_tick) {
                Ok(key) => {
                    on_status("key obtained", 1);
                    Ok(key)
                }
                Err(error) => Err(macos_db_key_error(error)),
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            let mut hook =
                crate::macos_key_helper::Hook::start(pid, timeout).map_err(macos_db_key_error)?;
            on_status("Hook installed; log in to WeChat now", 1);
            match hook.wait(on_tick) {
                Ok(key) => {
                    on_status("key obtained", 1);
                    Ok(key)
                }
                Err(error) => Err(macos_db_key_error(error)),
            }
        }
    }
}

/// The kind of a failure of the macOS hook, from its message.
#[cfg(any(target_os = "macos", test))]
fn macos_db_key_error(message: String) -> DbKeyError {
    if message == "interrupted by user" {
        DbKeyError::Interrupted
    } else if message.starts_with("HOOK_TIMEOUT") {
        DbKeyError::Timeout
    } else if let Some(reason) = message.strip_prefix("scan:") {
        DbKeyError::Unsupported(reason.to_string())
    } else if message.starts_with("dtrace_privileges") {
        DbKeyError::AccessDenied(message)
    } else if message.starts_with("dtrace_probe") {
        DbKeyError::Unsupported(message)
    } else if message.starts_with("dtrace_start_failed") {
        DbKeyError::Other(message)
    } else if message.contains("task_for_pid")
        || message.contains("ptrace_attach_failed")
        || message.contains("ATTACH_FAILED")
    {
        DbKeyError::AccessDenied(message)
    } else if message.contains("CANNOT_GET_EXEC_PATH") || message.contains("attach_wait_timeout") {
        DbKeyError::NotReady(message)
    } else {
        DbKeyError::Other(message)
    }
}

/// Why the database key was not extracted.
#[derive(Debug, PartialEq, Eq)]
pub enum DbKeyError {
    /// The process could not be opened (needs administrator rights / a security product interferes).
    AccessDenied(String),
    /// No key came before the time ran out: the user did not log in, or WeChat did not take the key.
    Timeout,
    /// The run was interrupted (Ctrl+C).
    Interrupted,
    /// WeChat has just started and its modules are not ready for inspection.
    NotReady(String),
    /// The hook found nowhere to stop in this WeChat (macOS: no module to scan, or no call that takes the database
    /// key in it): another try finds the same (#806).
    Unsupported(String),
    /// Any other failure, as the hook said it.
    Other(String),
}

impl std::fmt::Display for DbKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AccessDenied(d) => write!(f, "access denied: cannot open the WeChat process: {d}"),
            Self::Timeout => write!(f, "timed out waiting for the key; log in to WeChat, or restart it, while the command is running"),
            Self::Interrupted => write!(f, "interrupted by user"),
            Self::NotReady(m) | Self::Unsupported(m) | Self::Other(m) => write!(f, "{m}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The messages of the macOS hook (`macos_key_helper::platform`) and the kind each is taken for (#806).
    #[test]
    fn the_macos_hooks_failures_are_told_apart() {
        let kind = |m: &str| macos_db_key_error(m.to_string());
        for scan in [
            "scan:No suitable module found: neither wechat.dylib nor a WeChat binary over 100MB",
            "scan:Both wechat.dylib and the main binary are over 100MB, so which to scan is ambiguous",
            "scan:Sink pattern not found",
        ] {
            assert_eq!(
                kind(scan),
                DbKeyError::Unsupported(scan["scan:".len()..].to_string())
            );
        }
        assert!(matches!(
            kind("attach:task_for_pid(5): (os/kern) failure"),
            DbKeyError::AccessDenied(_)
        ));
        assert!(matches!(
            kind("attach_wait_timeout"),
            DbKeyError::NotReady(_)
        ));
        assert_eq!(kind("HOOK_TIMEOUT"), DbKeyError::Timeout);
        assert_eq!(kind("interrupted by user"), DbKeyError::Interrupted);
        for other in [
            "patch_breakpoint_failed:ptrace_word:-1",
            "thread_get_state(5): (os/kern) failure",
            "process_died_while_waiting",
        ] {
            assert_eq!(kind(other), DbKeyError::Other(other.to_string()));
        }
    }
}
