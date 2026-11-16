//! The Rust rewrite of macOS `image_scan_helper`.
//!
//! The original program loaded `libwx_key.dylib` from its own directory,
//! called `ScanMemoryForImageKey`, and printed one JSON line. WeFlow scans in
//! the same process now; this module keeps the helper's argument rules, JSON
//! and exit codes, and leaves the scan itself behind [`Scanner`] so every
//! answer can be tested without a WeChat process.

/// The scan could not be started. The original reported these from `dlopen`
/// and `dlsym`; a build that scans in-process never returns them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanError {
    CannotGetExecutablePath,
    Dlopen(String),
    SymbolNotFound,
}

/// The function the helper looked up in `libwx_key.dylib`.
pub trait Scanner {
    fn scan(&self, pid: u32, ciphertext: &str) -> Result<Option<String>, ScanError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    InvalidArguments,
    InvalidPid,
    ScanFailed(ScanError),
    NoKeyFound,
    /// `ScanMemoryForImageKey` returned a string that starts with `ERROR`.
    ScanError(String),
    Key(String),
}

impl Outcome {
    fn json(&self) -> String {
        match self {
            Self::InvalidArguments => failure("invalid arguments"),
            Self::InvalidPid => failure("invalid pid"),
            Self::ScanFailed(ScanError::CannotGetExecutablePath) => {
                failure("cannot get executable path")
            }
            Self::ScanFailed(ScanError::Dlopen(reason)) => {
                failure(&format!("dlopen failed: {reason}"))
            }
            Self::ScanFailed(ScanError::SymbolNotFound) => {
                failure("symbol not found: ScanMemoryForImageKey")
            }
            Self::NoKeyFound => failure("no key found"),
            Self::ScanError(error) => failure(error),
            Self::Key(key) => format!("{{\"success\":true,\"aesKey\":{}}}\n", quoted(key)),
        }
    }

    fn exit_code(&self) -> i32 {
        match self {
            Self::InvalidArguments | Self::InvalidPid | Self::ScanFailed(_) => 1,
            Self::NoKeyFound | Self::ScanError(_) | Self::Key(_) => 0,
        }
    }
}

fn failure(message: &str) -> String {
    format!("{{\"success\":false,\"error\":{}}}\n", quoted(message))
}

/// Escapes a value for the JSON strings the helper prints.
fn quoted(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

/// `atoi`'s prefix: white space, an optional sign and decimal digits.
fn atoi(value: &str) -> i32 {
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
    let number = if negative {
        number.saturating_neg()
    } else {
        number
    };
    i32::try_from(number).unwrap_or(if negative { i32::MIN } else { i32::MAX })
}

fn parse(args: &[String]) -> Result<(u32, &str), Outcome> {
    if args.len() != 3 {
        return Err(Outcome::InvalidArguments);
    }
    let pid = atoi(&args[1]);
    if pid <= 0 {
        return Err(Outcome::InvalidPid);
    }
    Ok((u32::try_from(pid).unwrap_or(u32::MAX), &args[2]))
}

/// Runs the helper's command line against `scanner` and answers
/// `(stdout, stderr, exit code)`; `args` includes the program name.
pub fn run_with<S: Scanner>(args: &[String], scanner: &S) -> (String, String, i32) {
    let (pid, ciphertext) = match parse(args) {
        Ok(parsed) => parsed,
        Err(outcome) => {
            let stderr = if outcome == Outcome::InvalidArguments {
                format!(
                    "Usage: {} <pid> <ciphertext_hex>\n",
                    args.first().map_or("image_scan_helper", String::as_str)
                )
            } else {
                String::new()
            };
            return (outcome.json(), stderr, outcome.exit_code());
        }
    };
    let stderr = format!(
        "[image_scan_helper] calling ScanMemoryForImageKey(pid={pid}, ciphertext={ciphertext})\n"
    );
    let outcome = match scanner.scan(pid, ciphertext) {
        Err(error) => Outcome::ScanFailed(error),
        Ok(None) => Outcome::NoKeyFound,
        Ok(Some(text)) if text.is_empty() => Outcome::NoKeyFound,
        Ok(Some(text)) if text.starts_with("ERROR") => Outcome::ScanError(text),
        Ok(Some(key)) => Outcome::Key(key),
    };
    (outcome.json(), stderr, outcome.exit_code())
}

#[cfg(target_os = "macos")]
struct RustScanner;

#[cfg(target_os = "macos")]
impl Scanner for RustScanner {
    fn scan(&self, pid: u32, ciphertext: &str) -> Result<Option<String>, ScanError> {
        Ok(scan(pid, ciphertext))
    }
}

/// The image AES key as 32 hex characters, or `None` when no writable page
/// holds it.
#[cfg(target_os = "macos")]
pub fn scan(pid: u32, ciphertext: &str) -> Option<String> {
    crate::macos_key_helper::scan_memory_for_image_key(pid, ciphertext)
}

/// Runs the helper's command line and writes its stdout and stderr; the
/// exit code is the original helper's.
#[cfg(target_os = "macos")]
pub fn run_helper(args: &[String]) -> i32 {
    let (stdout, stderr, code) = run_with(args, &RustScanner);
    use std::io::Write as _;
    let _ = std::io::stdout().write_all(stdout.as_bytes());
    let _ = std::io::stderr().write_all(stderr.as_bytes());
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        answer: Result<Option<String>, ScanError>,
    }

    impl Scanner for Fake {
        fn scan(&self, _pid: u32, _ciphertext: &str) -> Result<Option<String>, ScanError> {
            self.answer.clone()
        }
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn run(values: &[&str], answer: Result<Option<String>, ScanError>) -> (String, String, i32) {
        run_with(&args(values), &Fake { answer })
    }

    #[test]
    fn takes_exactly_a_pid_and_a_ciphertext() {
        let missing = run(&["helper"], Ok(None));
        assert_eq!(
            missing.0,
            "{\"success\":false,\"error\":\"invalid arguments\"}\n"
        );
        assert_eq!(missing.1, "Usage: helper <pid> <ciphertext_hex>\n");
        assert_eq!(missing.2, 1);

        let extra = run(&["helper", "7", "00", "00"], Ok(None));
        assert_eq!(
            extra.0,
            "{\"success\":false,\"error\":\"invalid arguments\"}\n"
        );
        assert_eq!(extra.2, 1);
    }

    #[test]
    fn rejects_a_pid_that_is_not_positive() {
        for pid in ["0", "-1", "abc"] {
            let answer = run(&["helper", pid, "00"], Ok(None));
            assert_eq!(
                answer.0, "{\"success\":false,\"error\":\"invalid pid\"}\n",
                "{pid}"
            );
            assert_eq!(answer.2, 1, "{pid}");
        }
        assert_eq!(
            run(&["helper", "12abc", "00"], Ok(None)).2,
            0,
            "atoi reads the numeric prefix"
        );
    }

    #[test]
    fn reports_a_missing_symbol_like_the_wrapper() {
        let answer = run(
            &["helper", "42", &"00".repeat(16)],
            Err(ScanError::SymbolNotFound),
        );
        assert_eq!(
            answer.0,
            "{\"success\":false,\"error\":\"symbol not found: ScanMemoryForImageKey\"}\n"
        );
        assert_eq!(answer.2, 1);
    }

    #[test]
    fn reports_a_scan_error_and_still_exits_zero() {
        let answer = run(
            &["helper", "42", &"00".repeat(16)],
            Ok(Some("ERROR:SCAN_FAILED:no task".into())),
        );
        assert_eq!(
            answer.0,
            "{\"success\":false,\"error\":\"ERROR:SCAN_FAILED:no task\"}\n"
        );
        assert_eq!(answer.2, 0);
    }

    #[test]
    fn prints_a_found_key_and_exits_zero() {
        let key = "00112233445566778899aabbccddeeff";
        let answer = run(
            &["helper", "42", &"00".repeat(16)],
            Ok(Some(key.to_string())),
        );
        assert_eq!(
            answer.0,
            format!("{{\"success\":true,\"aesKey\":\"{key}\"}}\n")
        );
        assert_eq!(
            answer.1,
            "[image_scan_helper] calling ScanMemoryForImageKey(pid=42, ciphertext=00000000000000000000000000000000)\n"
        );
        assert_eq!(answer.2, 0);
    }

    #[test]
    fn reports_no_key_when_the_scan_is_empty() {
        for answer in [Ok(None), Ok(Some(String::new()))] {
            let answer = run(&["helper", "42", &"00".repeat(16)], answer);
            assert_eq!(answer.0, "{\"success\":false,\"error\":\"no key found\"}\n");
            assert_eq!(answer.2, 0);
        }
    }

    #[test]
    fn escapes_json_strings() {
        assert_eq!(quoted("a\"b"), "\"a\\\"b\"");
        let answer = run(
            &["helper", "42", &"00".repeat(16)],
            Ok(Some("ERROR:bad \"value\"".into())),
        );
        assert_eq!(
            answer.0,
            "{\"success\":false,\"error\":\"ERROR:bad \\\"value\\\"\"}\n"
        );
    }

    #[test]
    fn reports_how_the_library_could_not_be_loaded() {
        let answer = run(
            &["helper", "42", &"00".repeat(16)],
            Err(ScanError::Dlopen("no such file".into())),
        );
        assert_eq!(
            answer.0,
            "{\"success\":false,\"error\":\"dlopen failed: no such file\"}\n"
        );
        assert_eq!(answer.2, 1);

        let answer = run(
            &["helper", "42", &"00".repeat(16)],
            Err(ScanError::CannotGetExecutablePath),
        );
        assert_eq!(
            answer.0,
            "{\"success\":false,\"error\":\"cannot get executable path\"}\n"
        );
        assert_eq!(answer.2, 1);
    }
}
