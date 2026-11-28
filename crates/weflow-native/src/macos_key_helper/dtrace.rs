//! x86_64：用 DTrace 从微信对 CommonCrypto 的 `CCKeyDerivationPBKDF` 的调用中读出数据库密钥。
//!
//! DTrace 在内核中运行探针，从微信的内存中复制密钥；它既不让微信停下，也不修改微信，所以不会像 `thread_set_state`
//! 或改写代码那样触发强化运行时的拦截。调用方用 `sudo` 运行 `key db`，这是 DTrace 所需要的；
//! 用户态探针还要求关闭系统完整性保护（SIP）。
//!
//! 探针只取口令为 32 字节、`rounds` 参数等于 SQLCipher 的 KDF 轮数的那一次调用，即对原始数据库密钥的派生。
//! 同一个函数还会用 2 轮派生 HMAC 密钥，那一次不能当作原始密钥。

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const DTRACE: &str = "/usr/sbin/dtrace";
const KEY_PREFIX: &str = "WEFLOWKEY:";
const KEY_LEN: usize = 32;

/// 监视微信、等待数据库密钥调用的 DTrace 进程。
#[derive(Debug)]
pub struct DTraceHook {
    child: Child,
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
}

impl DTraceHook {
    /// 在 `pid` 上启动 DTrace。DTrace 不存在或拒绝探针时，在调用方提示用户登录之前就失败。
    ///
    /// # Errors
    ///
    /// DTrace 启动失败或无法查询它的状态时返回 `dtrace_start_failed:…`、`dtrace_wait_failed:…`。
    /// DTrace 很快退出时按原因返回：权限不足为 `dtrace_privileges:…`，探针与这个微信不匹配为 `dtrace_probe:…`，
    /// 其他为 `dtrace_exited:…`。
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
        // 给 DTrace 编译探针的时间；出错时（权限不足、找不到匹配的符号）进程会立即结束。
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

    /// 等待密钥，通过 `on_tick` 报告剩余的秒数。
    ///
    /// # Errors
    ///
    /// 被中断时返回 `interrupted by user`，超时返回 `HOOK_TIMEOUT:no_breakpoint_hit`；
    /// DTrace 退出却没有输出密钥时，按 `classify_failure` 的规则返回原因。
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

/// 从 `WEFLOWKEY:<64 hex digits>` 这一行取出密钥；没有这一行时为 `None`。
fn parse_key(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let hex = line.strip_prefix(KEY_PREFIX)?.trim();
        (hex.len() == KEY_LEN * 2 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| hex.to_ascii_lowercase())
    })
}

/// 把 DTrace 的失败归为调用方需要区分的原因：权限不足、探针与这个微信不匹配，或者其他。
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

/// D 程序：在对原始密钥的那次调用上复制 `arg1` 处的 32 字节并打印出来。
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
