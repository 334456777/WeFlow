//! `xkey_helper` 的命令行 `<pid> [timeout_ms]`、它输出的 JSON 和退出码。解析沿用桌面版所用的 C `strtol` 规则，
//! 所以 `12abc` 这样的 pid 仍表示进程 12。

const USAGE: &str = "{\"success\":false,\"result\":\"ERROR:USAGE: helper <pid> [timeout_ms]\"}\n";
const INVALID_PID: &str = "{\"success\":false,\"result\":\"ERROR:INVALID_PID\"}\n";
const DEFAULT_TIMEOUT_MS: u64 = 180_000;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Usage,
    InvalidPid,
    AttachFailed(String),
    ScanFailed(String),
    HookFailed(String),
    Success(String),
}

impl Outcome {
    fn render(&self) -> String {
        match self {
            Self::Usage => USAGE.to_string(),
            Self::InvalidPid => INVALID_PID.to_string(),
            Self::AttachFailed(reason) => json_failure("ERROR:ATTACH_FAILED:", reason),
            Self::ScanFailed(reason) => json_failure(
                "ERROR:SCAN_FAILED:",
                if reason.is_empty() {
                    "Sink pattern not found"
                } else {
                    reason
                },
            ),
            Self::HookFailed(reason) => json_failure("ERROR:HOOK_FAILED:", reason),
            Self::Success(key) => {
                format!("{{\"success\":true,\"key\":\"{}\"}}\n", json_escape(key))
            }
        }
    }

    fn exit_code(&self) -> i32 {
        match self {
            Self::Usage | Self::InvalidPid => 1,
            Self::AttachFailed(_)
            | Self::ScanFailed(_)
            | Self::HookFailed(_)
            | Self::Success(_) => 0,
        }
    }
}

fn json_failure(prefix: &str, reason: &str) -> String {
    format!(
        "{{\"success\":false,\"result\":\"{prefix}{}\"}}\n",
        json_escape(reason)
    )
}

/// 转义桌面版辅助程序在 JSON 字符串中转义的那些字符。
fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// 按十进制取 `value` 中能被 `strtol` 解析的前缀：可选的空白和正负号，之后是数字；没有数字时为 `0`。
fn strtol(value: &str) -> i64 {
    let value = value.trim_start();
    let (negative, digits) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let mut number: i64 = 0;
    for byte in digits.bytes() {
        if !byte.is_ascii_digit() {
            break;
        }
        number = number
            .saturating_mul(10)
            .saturating_add(i64::from(byte - b'0'));
    }
    if negative {
        number.saturating_neg()
    } else {
        number
    }
}

/// 解析命令行，得到 pid 和超时的毫秒数；`args` 与 `argv` 一样含有程序名。
pub(super) fn parse(args: &[String]) -> Result<(u32, u64), Outcome> {
    let Some(pid) = args.get(1) else {
        return Err(Outcome::Usage);
    };
    let pid = strtol(pid);
    if pid <= 0 || pid > i64::from(u32::MAX) {
        return Err(Outcome::InvalidPid);
    }
    let timeout = args
        .get(2)
        .map(|value| strtol(value))
        .filter(|value| *value > 0)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(DEFAULT_TIMEOUT_MS);
    Ok((u32::try_from(pid).unwrap_or(0), timeout))
}

/// 按命令行 `args`（`<pid> [timeout_ms]`）运行辅助程序，返回它输出的 JSON 和退出码。
pub fn run_helper(args: &[String]) -> (String, i32) {
    run_helper_at(args)
}

/// 与 [`run_helper`] 的结果相同，供脚本化的测试调用；`platform` 只在 macOS 上可用。
pub fn run_helper_at(args: &[String]) -> (String, i32) {
    match parse(args) {
        Ok((pid, timeout_ms)) => {
            let _ = (pid, timeout_ms);
            #[cfg(target_os = "macos")]
            {
                run(pid, timeout_ms)
            }
            #[cfg(not(target_os = "macos"))]
            {
                (
                    json_failure(
                        "ERROR:ATTACH_FAILED:",
                        "the macOS key helper is not available on this platform",
                    ),
                    0,
                )
            }
        }
        Err(outcome) => (outcome.render(), outcome.exit_code()),
    }
}

#[cfg(target_os = "macos")]
fn run(pid: u32, timeout_ms: u64) -> (String, i32) {
    use std::time::Duration;

    eprintln!("[xkey_helper] pid={pid} timeoutMs={timeout_ms}");
    match super::Hook::start(pid, Duration::from_millis(timeout_ms))
        .and_then(|mut hook| hook.wait(&mut |_| {}))
    {
        Ok(key) => {
            eprintln!("[xkey_helper] SUCCESS: key={key}");
            let outcome = Outcome::Success(key);
            (outcome.render(), outcome.exit_code())
        }
        Err(reason) => {
            eprintln!("[xkey_helper] HOOK_FAILED: {reason}");
            let outcome = if reason.starts_with("scan:") || reason == "Sink pattern not found" {
                Outcome::ScanFailed(reason.trim_start_matches("scan:").to_string())
            } else if reason.starts_with("attach:") {
                Outcome::AttachFailed(reason.trim_start_matches("attach:").to_string())
            } else {
                Outcome::HookFailed(reason)
            };
            (outcome.render(), outcome.exit_code())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn keeps_the_helpers_usage_and_exit_codes() {
        assert_eq!(run_helper_at(&args(&["helper"])), (USAGE.to_string(), 1));
        assert_eq!(
            run_helper_at(&args(&["helper", "0"])),
            (INVALID_PID.to_string(), 1)
        );
        assert_eq!(
            run_helper_at(&args(&["helper", "-3"])),
            (INVALID_PID.to_string(), 1)
        );
    }

    #[test]
    fn parses_the_pid_timeout_and_strtol_prefixes() {
        assert_eq!(
            parse(&args(&["helper", "12abc"])),
            Ok((12, DEFAULT_TIMEOUT_MS))
        );
        assert_eq!(parse(&args(&["helper", " 9", "2500"])), Ok((9, 2500)));
        assert_eq!(
            parse(&args(&["helper", "9", "0"])),
            Ok((9, DEFAULT_TIMEOUT_MS))
        );
        assert_eq!(
            parse(&args(&["helper", "9", "-1"])),
            Ok((9, DEFAULT_TIMEOUT_MS))
        );
        assert_eq!(parse(&args(&["helper", "9", "abc"])), Ok((9, 180_000)));
    }

    #[test]
    fn renders_the_same_json_lines() {
        assert_eq!(
            Outcome::AttachFailed("task_for_pid:5".into()).render(),
            "{\"success\":false,\"result\":\"ERROR:ATTACH_FAILED:task_for_pid:5\"}\n"
        );
        assert_eq!(
            Outcome::ScanFailed(String::new()).render(),
            "{\"success\":false,\"result\":\"ERROR:SCAN_FAILED:Sink pattern not found\"}\n"
        );
        assert_eq!(
            Outcome::HookFailed("HOOK_TIMEOUT:no_breakpoint_hit".into()).render(),
            "{\"success\":false,\"result\":\"ERROR:HOOK_FAILED:HOOK_TIMEOUT:no_breakpoint_hit\"}\n"
        );
        assert_eq!(
            Outcome::Success("ab\"c".into()).render(),
            "{\"success\":true,\"key\":\"ab\\\"c\"}\n"
        );
    }
}
