//! The command API's contract (#399): what the `data` of its key reads holds, as Rust types that serialize to the
//! answer and describe it as JSON Schema. `/openapi.json` carries the schemas, and the WebUI's types are generated
//! from them, so a change here that its clients would not read the same shows in their types.
//!
//! Throughout: times are Unix seconds unless a field says otherwise; an id of WeChat's that may pass 2^53 (a
//! message's server id) is a decimal string; an optional field is left out when there is nothing to say, a nullable
//! one is `null`. The rows of WeChat's own tables (`chat messages` without `parsed`, `chat contacts` without
//! `parsed`…) have the columns of those tables and are not described further.

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

/// The version of the API's rules (routes, methods, parameters, answers, jobs), for clients such as the WebUI to
/// check. Raised only by changes that break existing clients; new commands and options do not raise it.
///
/// `web/package.json` and `web/contract/openapi.json` say it too:
/// `the_webui_and_the_contract_have_the_server_s_api_version` fails until they agree with this.
pub const API_VERSION: &str = "3";
