//! The Rust rewrite of macOS `xkey_helper`.
//!
//! On ARM64, `Hook` locates the database-key call by scanning WeChat's code,
//! attaches to WeChat with `ptrace` and stops it there; on x86_64 the key is
//! read from WeChat's call to CommonCrypto's `CCKeyDerivationPBKDF` with
//! DTrace, without stopping the client. `scan_image_key` backs the image-key scan, and `wechat_processes` lists the
//! running client.
//! `cli` keeps the helper's own command line, JSON and exit codes for tests.
//!
//! The original executable used Mach exception ports. The ARM64 rewrite lets
//! `ptrace` report the breakpoint instead: the process stops only while the
//! key is read, and the same register rules decide whether a hit is the
//! database-key call. The x86_64 path uses DTrace because the hardened
//! runtime refuses thread state changes and code writes.

mod cli;
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
mod dtrace;
#[cfg(target_os = "macos")]
mod image_mem;
mod locator;
mod module;
#[cfg(target_os = "macos")]
mod platform;
#[cfg(target_os = "macos")]
mod process;

pub use cli::{run_helper, run_helper_at};
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub use dtrace::DTraceHook;
#[cfg(target_os = "macos")]
pub use image_mem::scan_image_key;
#[cfg(target_os = "macos")]
pub use platform::{get_db_key_target, interrupt, Hook};
#[cfg(target_os = "macos")]
pub use process::{wechat_processes, WeChatProcess};
