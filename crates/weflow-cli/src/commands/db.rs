//! `weflow db`: WeChat's data directories, their accounts and the connection test.

use clap::{Args, Subcommand};

use serde_json::{json, Value};
use weflow_core::error::AppResult;
use weflow_core::services::ServiceHub;

#[derive(Args, Debug)]
pub(crate) struct DbCommand {
    #[command(subcommand)]
    pub(crate) command: DbSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum DbSubcommand {
    /// Find WeChat data directories on this computer
    Detect,
    /// Scan a folder for WeChat account directories
    Scan {
        /// Folder to scan
        root: String,
    },
    /// Open the configured database with the key and show a short summary
    Test,
    /// Show the wxid of each account in the WeChat data directory, the value `config set wxid` takes
    Wxid {
        /// WeChat data directory; default: `db_path` from the config, else the usual locations
        root: Option<String>,
    },
}

pub(crate) fn handle_db(command: &DbCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        DbSubcommand::Scan { root } => Ok(json!(hub.db_scan(root))),
        DbSubcommand::Detect | DbSubcommand::Test | DbSubcommand::Wxid { .. } => {
            unreachable!("a read")
        }
    }
}
