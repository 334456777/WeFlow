//! 命令 API 的契约（#399）：各命令回答中 `data` 的 Rust 类型。
//!
//! 这些类型既序列化成回答，又描述成 JSON Schema。`/openapi.json` 带有这些 schema，网页的类型由它们的快照
//! `web/contract/openapi.json` 生成，所以这里让客户端读到不同内容的改动会反映在网页的类型中。
//!
//! 通用的约定：时间除非字段另有说明都是 Unix 秒；微信可能超过 2^53 的 ID（如消息的服务器 ID）是十进制字符串；
//! 没有内容的可选字段省略，可为空的字段为 `null`。微信自己的表的原始行（不带 `parsed` 的 `chat contacts`、
//! `biz accounts` 的 `accounts` 等）是 `Row`，只带表中的列，不再细述。

pub use crate::chat_msg::{ChatMessage, Type49Info};

mod account;
mod biz;
mod chat;
mod cleanup;
mod export;
mod group;
mod insight;
mod media;
mod report;
mod sns;
mod web;
pub use account::*;
pub use biz::*;
pub use chat::*;
pub use cleanup::*;
pub use export::*;
pub use group::*;
pub use insight::*;
pub use media::*;
pub use report::*;
pub use sns::*;
pub use web::*;

/// API 规则（路由、方法、参数、回答、作业）的版本，供客户端检查。
///
/// 网页等客户端把它与自己构建时对应的版本比较。只有会让现有客户端出错的改动才提升它，新增命令和选项不提升。
///
/// `web/package.json` 和 `web/contract/openapi.json` 中也写着它，与这里不一致时
/// `the_webui_and_the_contract_have_the_server_s_api_version` 失败。
pub const API_VERSION: &str = "4";
