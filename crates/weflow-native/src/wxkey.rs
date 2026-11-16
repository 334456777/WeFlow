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
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub fn is_available(&self) -> bool {
        false
    }

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

#[cfg(windows)]
pub struct WxKey;

#[cfg(windows)]
impl WxKey {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
    pub fn is_available(&self) -> bool {
        cfg!(target_arch = "x86_64")
    }
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub struct WxKey;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl WxKey {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

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

#[cfg(target_os = "macos")]
pub struct WxKey;

#[cfg(target_os = "macos")]
impl WxKey {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub fn is_available(&self) -> bool {
        true
    }

    /// Locates the database-key sink with the Rust scanner, attaches with
    /// `ptrace` and waits for the 32-byte call.
    pub fn get_db_key_with_tick(
        &self,
        pid: u32,
        timeout: std::time::Duration,
        on_status: &mut dyn FnMut(&str, i32),
        on_tick: &mut dyn FnMut(u64),
    ) -> std::result::Result<String, DbKeyError> {
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

#[cfg(target_os = "macos")]
fn macos_db_key_error(message: String) -> DbKeyError {
    if message == "interrupted by user" {
        DbKeyError::Interrupted
    } else if message.starts_with("HOOK_TIMEOUT") {
        DbKeyError::Timeout
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

#[derive(Debug, PartialEq, Eq)]
pub enum DbKeyError {
    /// The process could not be opened (needs administrator rights / a security product interferes).
    AccessDenied(String),
    /// WeChat is running but has not logged in, so it never opened its databases.
    LoginRequired,
    Timeout,
    Interrupted,
    /// WeChat has just started and its modules are not ready for inspection.
    NotReady(String),
    Other(String),
}

impl std::fmt::Display for DbKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AccessDenied(d) => write!(f, "access denied: cannot open the WeChat process ({d})"),
            Self::LoginRequired => write!(f, "WeChat is running but not logged in; log in while the command is waiting"),
            Self::Timeout => write!(f, "timed out waiting for the key; log in to WeChat (or restart it) while the command is running"),
            Self::Interrupted => write!(f, "interrupted by user"),
            Self::NotReady(m) => write!(f, "{m}"),
            Self::Other(m) => write!(f, "{m}"),
        }
    }
}
