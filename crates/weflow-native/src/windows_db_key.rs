//! Windows x64 上的数据库密钥捕获。
//!
//! 各版本的特征码、偏移和 RDX 参数结构已与原版 `wx_key` 的二进制及 `ycccccccy/wx_key`（MIT，许可证见本目录中的
//! `LICENSE`）核对。捕获使用 Windows 调试 API：不改写微信的指令，也不加载厂商的辅助库。

mod locator;
#[cfg(all(windows, target_arch = "x86_64"))]
mod platform;
#[cfg(all(windows, target_arch = "x86_64"))]
pub use platform::{interrupt, CaptureDiagnostics, Hook, KeyLocation};
