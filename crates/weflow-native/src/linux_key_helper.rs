//! Rust port of `xkey_helper_linux`.
//!
//! The database modes (`db_scan`, `db_hook`) are used by `key db` in this
//! process; `image_local` and `image_mem` back the Linux image-key commands.
//! The command-line compatibility layer is in `cli`.

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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use cli::{run_helper, run_helper_at};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use image_local::{image_local, image_local_at};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use image_mem::image_mem;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use platform::{interrupt, Hook};
