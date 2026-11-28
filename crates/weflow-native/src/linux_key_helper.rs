//! 用 Rust 重写的桌面版 Linux 取钥辅助程序（#796）。
//!
//! 辅助程序 `xkey_helper_linux` 来自 WeFlow 桌面版（`hicccc77/WeFlow`，CC BY-NC-SA 4.0，许可证见本目录中的
//! `LICENSE`）。
//!
//! `db_scan` 和 `db_hook` 为 `key db` 定位并捕获数据库密钥；`image_mem` 用于 `key image-mem`。
//! 与桌面版辅助程序兼容的命令行在 `cli` 中。

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod cli;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod image_mem;
mod locator;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod platform;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod process;
mod ptrace_access;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use cli::run_helper;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use image_mem::image_mem;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use platform::{interrupt, Hook};
pub use ptrace_access::{ptrace_access, PtraceAccess};
