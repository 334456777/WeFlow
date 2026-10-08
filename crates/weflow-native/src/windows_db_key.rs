//! Windows x64 database-key capture. Version signatures, offsets and the RDX
//! argument layout were checked against the original wx_key
//! binary and ycccccccy/wx_key (MIT). Capture uses Windows debugging APIs; no
//! client instructions are patched and no vendor helper is loaded.

mod locator;
#[cfg(all(windows, target_arch = "x86_64"))]
mod platform;
#[cfg(all(windows, target_arch = "x86_64"))]
pub use platform::{CaptureDiagnostics, Hook, KeyLocation};
