//! Linux x64 database-key capture. The hook point is located in WeChat's ELF
//! image from the `com.Tencent.WCDB.Config.Cipher` reference chain; capture
//! uses ptrace and debug registers, with no client instructions patched and no
//! vendor helper started.

mod locator;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod platform;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use platform::{interrupt, CaptureDiagnostics, Hook};
