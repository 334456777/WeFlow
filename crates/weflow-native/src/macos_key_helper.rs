//! 用 Rust 重写的 macOS `xkey_helper`。
//!
//! `xkey_helper`、`libwx_key.dylib` 和 `image_scan_helper` 来自 WeFlow 桌面版（`hicccc77/WeFlow`，
//! CC BY-NC-SA 4.0，许可证见本目录中的 `LICENSE`）。
//!
//! ARM64 上，`Hook` 扫描微信的代码找到接收数据库密钥的调用，用 `ptrace` 附加到微信并让它停在那里；
//! x86_64 上用 DTrace 从微信对 CommonCrypto 的 `CCKeyDerivationPBKDF` 的调用中读出密钥，不让微信停下。
//! `scan_image_key` 用于图片密钥的内存扫描，`wechat_processes` 列出正在运行的微信。
//! `cli` 保留辅助程序自己的命令行、JSON 输出和退出码，供测试使用。
//!
//! 桌面版的可执行文件用 Mach 异常端口捕获断点，这里的 ARM64 实现则由 `ptrace` 报告断点：进程只在读取密钥时停下，
//! 判断命中的是否为接收数据库密钥的调用，用的是与桌面版相同的寄存器规则。
//! x86_64 上用 DTrace，是因为强化运行时会拒绝修改线程状态和改写代码。

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
