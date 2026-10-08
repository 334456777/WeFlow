//! Platform helper implementation. Native binaries remain available for comparison.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub mod linux;
pub mod scan;
