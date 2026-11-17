//! The Rust rewrite of macOS `xkey_helper`.
//!
//! `Hook` locates the database-key call, attaches to WeChat with `ptrace` and
//! Mach breakpoints, and reads the 32-byte key; `scan_image_key` and
//! `scan_memory_for_image_key` back the image-key scan, and `wechat_processes` lists the running client.
//! `cli` keeps the helper's own command line, JSON and exit codes for tests.
//!
//! The original executable used Mach exception ports. This rewrite lets
//! `ptrace` report the breakpoint instead: the process stops only while the
//! key is read, and the same register rules decide whether a hit is the
//! database-key call.

mod cli;
#[cfg(target_os = "macos")]
mod image_mem;
mod locator;
mod module;
#[cfg(target_os = "macos")]
mod platform;
#[cfg(target_os = "macos")]
mod process;

pub use cli::{run_helper, run_helper_at};
#[cfg(target_os = "macos")]
pub use image_mem::{scan_image_key, scan_memory_for_image_key};
#[cfg(target_os = "macos")]
pub use platform::{get_db_key_target, interrupt, Hook};
#[cfg(target_os = "macos")]
pub use process::{wechat_processes, WeChatProcess};
