//! What `weflow serve` tells a command it runs in a child process: the environment variables it sets, each saying
//! what the command's stdin carries. The server sets them (`serve_api::child`); the command reads them.

/// Set for a command whose stdin carries the database keys the server derived (see `serve_api::child::spawn`).
pub(crate) const KEYS_ON_STDIN: &str = "WEFLOW_KEYS_ON_STDIN";

/// Set for a command that ends by itself once its stdin ends (see `serve_api::child::ends_by_itself`).
pub(crate) const CANCEL_ON_STDIN: &str = "WEFLOW_CANCEL_ON_STDIN";
