//! The Rust rewrite of WeFlow's Linux key helper, in PR #796.
//!
//! `db_scan` and `db_hook` locate and capture the database key for `key db`;
//! `image_local` and `image_mem` back the Linux image-key commands. The
//! command-line compatibility layer is in `cli`.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod cli;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod image_local;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod image_mem;
mod locator;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod platform;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod process;
mod ptrace_access;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use cli::{run_helper, run_helper_at};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use image_local::{image_local, image_local_at};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use image_mem::image_mem;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use platform::{interrupt, Hook};
pub use ptrace_access::{ptrace_access, PtraceAccess};
