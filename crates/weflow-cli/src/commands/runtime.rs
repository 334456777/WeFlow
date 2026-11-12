//! `weflow runtime`: where the runtime is unpacked.

use clap::{Args, Subcommand};

use serde_json::Value;
use weflow_core::config::AppContext;
use weflow_core::error::{AppError, AppResult};

#[derive(Args, Debug)]
pub(crate) struct RuntimeCommand {
    #[command(subcommand)]
    pub(crate) command: RuntimeSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum RuntimeSubcommand {
    /// Show where the runtime is unpacked and its version
    Info,
    /// List the files of the embedded runtime with sizes and hashes
    Manifest,
}

pub(crate) fn handle_runtime(command: &RuntimeCommand, ctx: &AppContext) -> AppResult<Value> {
    match command.command {
        RuntimeSubcommand::Info => unreachable!("a read"),
        RuntimeSubcommand::Manifest => serde_json::to_value(weflow_assets::manifest(&ctx.version))
            .map_err(|err| {
                AppError::runtime(format!("cannot serialize the runtime manifest: {err}"))
            }),
    }
}
