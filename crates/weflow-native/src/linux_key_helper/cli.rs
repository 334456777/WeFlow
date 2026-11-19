//! The helper's command line and JSON, including exit codes, for comparison
//! with the original executable and for a future standalone binary.

use super::{image_mem, platform, Hook};
use crate::image_key_scan::{self, MemoryScanError};

const USAGE: &str = "{\"success\":false,\"result\":\"ERROR:USAGE: helper <pid> [timeout_ms]\"}\n";
const IMAGE_MEM_USAGE: &str =
    "{\"success\":false,\"result\":\"ERROR:USAGE: helper image_mem <pid> <ciphertext>\"}\n";
const MISSING_PID: &str = "{\"success\":false,\"result\":\"ERROR:MISSING_PID\"}\n";
const MISSING_ARGS: &str = "{\"success\":false,\"result\":\"ERROR:MISSING_ARGS\"}\n";
const UNKNOWN_MODE: &str = "{\"success\":false,\"result\":\"ERROR:UNKNOWN_MODE\"}\n";

fn failure(result: &str) -> String {
    format!("{{\"success\":false,\"result\":\"{result}\"}}\n")
}

fn success_key(key: &str) -> String {
    format!("{{\"success\":true,\"key\":\"{key}\"}}\n")
}

/// Runs the helper's command line (`args` includes the program name): its stdout and exit code.
pub fn run_helper(args: &[String]) -> (String, i32) {
    if args.len() < 2 {
        return (USAGE.to_string(), 1);
    }
    match args[1].as_str() {
        "db_scan" => {
            if args.len() == 2 {
                return (MISSING_PID.to_string(), 1);
            }
            let pid = args[2].parse().unwrap_or(0);
            match platform::scan(pid) {
                Ok(target) => (
                    format!("{{\"success\":true,\"target_addr\":\"{target}\"}}\n"),
                    0,
                ),
                Err(platform::ScanError::ExecutablePath) => {
                    (failure("ERROR:CANNOT_GET_EXEC_PATH"), 0)
                }
                Err(platform::ScanError::Scan(message)) => {
                    (failure(&format!("ERROR:SCAN_FAILED:{message}")), 0)
                }
                Err(platform::ScanError::BaseAddress) => (failure("ERROR:CANNOT_GET_BASE_ADDR"), 0),
            }
        }
        "db_hook" => {
            if args.len() < 4 {
                return (MISSING_ARGS.to_string(), 1);
            }
            let pid = args[2].parse().unwrap_or(0);
            let address = args[3].parse().unwrap_or(0);
            let timeout = args
                .get(4)
                .and_then(|value| value.parse::<i64>().ok())
                .filter(|value| *value > 0)
                .map_or(180_000, |value| u64::try_from(value).unwrap_or(180_000));
            match Hook::start_at(pid, address) {
                Ok(mut hook) => {
                    match hook.wait(std::time::Duration::from_millis(timeout), &mut |_| {}) {
                        Ok(key) => (success_key(&key), 0),
                        Err(message) => (failure(&format!("ERROR:HOOK_FAILED:{message}")), 0),
                    }
                }
                Err(message) => (failure(&format!("ERROR:HOOK_FAILED:{message}")), 0),
            }
        }
        "image_mem" => {
            if args.len() < 4 {
                return (IMAGE_MEM_USAGE.to_string(), 1);
            }
            let pid = args[2].parse().unwrap_or(0);
            let Some(ciphertext) = image_key_scan::parse_ciphertext(&args[3]) else {
                return (failure("ERROR:NOT_FOUND"), 0);
            };
            match image_mem(pid, &[ciphertext], &mut |_| true) {
                Ok(Some(key)) => (success_key(&image_key_scan::hex(&key)), 0),
                Ok(None) | Err(MemoryScanError::Failed(_)) => (failure("ERROR:NOT_FOUND"), 0),
                Err(MemoryScanError::PermissionDenied(_)) => {
                    (failure("ERROR:PERMISSION_DENIED"), 0)
                }
            }
        }
        _ => (UNKNOWN_MODE.to_string(), 1),
    }
}
