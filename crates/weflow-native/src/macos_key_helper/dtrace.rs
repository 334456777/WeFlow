//! x86_64: reads the database key from WeChat's call to CommonCrypto's
//! `CCKeyDerivationPBKDF` with DTrace.
//!
//! DTrace runs the probe in the kernel and copies the key from WeChat's
//! memory; it neither stops nor changes the client, so it does not trip the
//! hardened runtime the way `thread_set_state` or code writes do. The caller
//! runs `key db` with `sudo`, which DTrace needs, and userland probes need
//! System Integrity Protection turned off.
//!
//! The probe filters the one call whose password is 32 bytes and whose
//! `rounds` argument is SQLCipher's KDF iteration count: the raw database key
//! call. The same function derives the HMAC key in 2 rounds, which must not be
//! taken for the raw key.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const DTRACE: &str = "/usr/sbin/dtrace";
const KEY_PREFIX: &str = "WEFLOWKEY:";
const KEY_LEN: usize = 32;

/// A DTrace process that watches WeChat for the database-key call.
pub struct DTraceHook {
    child: Child,
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
}

impl DTraceHook {
    /// Starts DTrace on `pid`. It fails when DTrace is missing or refuses the
    /// probe before the caller can ask the user to log in.
    pub fn start(pid: u32) -> Result<Self, String> {
        let mut child = Command::new(DTRACE)
            .arg("-q")
            .arg("-p")
            .arg(pid.to_string())
            .arg("-n")
            .arg(probe())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("dtrace_start_failed:{error}"))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let mut hook = Self {
            child,
            stdout,
            stderr,
        };
        // Give DTrace time to compile the probe; an error (missing
        // privileges, no matching symbol) ends the process right away.
        thread::sleep(Duration::from_millis(300));
        if let Some(status) = hook
            .child
            .try_wait()
            .map_err(|error| format!("dtrace_wait_failed:{error}"))?
        {
            return Err(hook.failure(status));
        }
        Ok(hook)
    }

    /// Waits for the key, reporting the seconds left through `on_tick`.
    pub fn wait(
        &mut self,
        timeout: Duration,
        on_tick: &mut dyn FnMut(u64),
    ) -> Result<String, String> {
        let started = Instant::now();
        let mut last_tick = u64::MAX;
        loop {
            if super::platform::interrupted() {
                self.kill();
                return Err("interrupted by user".into());
            }
            let left = timeout.saturating_sub(started.elapsed()).as_secs();
            if left != last_tick {
                last_tick = left;
                on_tick(left);
            }
            if started.elapsed() >= timeout {
                self.kill();
                return Err("HOOK_TIMEOUT:no_breakpoint_hit".into());
            }
            match self
                .child
                .try_wait()
                .map_err(|error| format!("dtrace_wait_failed:{error}"))?
            {
                Some(status) => {
                    let output = self.read_stdout();
                    let error = self.read_stderr();
                    return match parse_key(&output) {
                        Some(key) => Ok(key),
                        None => Err(classify_failure(status, &error)),
                    };
                }
                None => thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    fn read_stdout(&mut self) -> String {
        read_to_string(self.stdout.take())
    }

    fn read_stderr(&mut self) -> String {
        read_to_string(self.stderr.take())
    }

    fn failure(&mut self, status: std::process::ExitStatus) -> String {
        self.kill();
        let error = self.read_stderr();
        classify_failure(status, &error)
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for DTraceHook {
    fn drop(&mut self) {
        self.kill();
    }
}

fn read_to_string<R: Read>(reader: Option<R>) -> String {
    let Some(mut reader) = reader else {
        return String::new();
    };
    let mut text = String::new();
    let _ = reader.read_to_string(&mut text);
    text
}

/// The key from a `WEFLOWKEY:<64 hex digits>` line, if any.
fn parse_key(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let hex = line.strip_prefix(KEY_PREFIX)?.trim();
        (hex.len() == KEY_LEN * 2 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| hex.to_ascii_lowercase())
    })
}

/// A DTrace failure with the reason the caller needs: missing privileges,
/// a probe that does not match this WeChat, or anything else.
fn classify_failure(status: std::process::ExitStatus, error: &str) -> String {
    let message = error.trim();
    if message.contains("requires additional privileges")
        || message.contains("not permitted")
        || message.contains("could not enable tracing")
        || message.contains("operation not permitted")
    {
        format!("dtrace_privileges:{message}")
    } else if message.contains("invalid probe")
        || message.contains("no probes matched")
        || message.contains("matched 0 probes")
        || message.contains("failed to resolve")
    {
        format!("dtrace_probe:{message}")
    } else if message.is_empty() {
        format!("dtrace_exited:{status}")
    } else {
        format!("dtrace_exited:{status}:{message}")
    }
}

/// The D program: on the raw-key call copy 32 bytes of `arg1` and print them.
fn probe() -> String {
    let format = "%02x".repeat(KEY_LEN);
    let arguments = (0..KEY_LEN)
        .map(|index| format!("self->key[{index}]"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "pid$target::CCKeyDerivationPBKDF:entry\n\
         /arg2 == {key_len} && arg6 == {kdf_iter}/\n\
         {{\n\
             self->key = (uint8_t *)copyin(arg1, {key_len});\n\
             printf(\"{KEY_PREFIX}{format}\\n\", {arguments});\n\
             exit(0);\n\
         }}\n",
        key_len = KEY_LEN,
        kdf_iter = crate::sqlcipher::KDF_ITER,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_key_line() {
        let key = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        assert_eq!(
            parse_key(&format!("noise\n{KEY_PREFIX}{key}\n")),
            Some(key.to_string())
        );
        assert_eq!(parse_key(&format!("{KEY_PREFIX}{key}00")), None);
        assert_eq!(parse_key(&format!("{KEY_PREFIX}{}", "z".repeat(64))), None);
        assert_eq!(parse_key("dtrace: no probes matched"), None);
    }

    #[test]
    fn the_probe_takes_only_the_raw_key_call() {
        let probe = probe();
        assert!(probe.contains("arg2 == 32"));
        assert!(probe.contains("arg6 == 256000"));
        assert!(probe.contains("copyin(arg1, 32)"));
        assert!(probe.contains(KEY_PREFIX));
        assert!(probe.contains("exit(0)"));
    }

    #[test]
    fn tells_missing_privileges_and_unknown_probes_apart() {
        let status = |code| {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                std::process::ExitStatus::from_raw(code)
            }
            #[cfg(not(unix))]
            {
                let _ = code;
                unreachable!()
            }
        };
        assert!(classify_failure(
            status(1),
            "dtrace: failed to initialize dtrace: DTrace requires additional privileges"
        )
        .starts_with("dtrace_privileges:"));
        assert!(
            classify_failure(status(1), "dtrace: no probes matched").starts_with("dtrace_probe:")
        );
        assert!(classify_failure(status(1), "something else").starts_with("dtrace_exited:"));
    }
}
