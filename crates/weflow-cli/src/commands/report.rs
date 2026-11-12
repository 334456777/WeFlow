//! `weflow report`: the annual and the dual report.

use clap::{Args, Subcommand};

use serde_json::{json, Value};
use weflow_core::error::AppResult;
use weflow_core::services::ServiceHub;

#[derive(Args, Debug)]
pub(crate) struct ReportCommand {
    #[command(subcommand)]
    pub(crate) command: ReportSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ReportSubcommand {
    /// Annual report
    Annual {
        #[command(subcommand)]
        command: AnnualSubcommand,
    },
    /// Dual-person report
    Dual {
        #[command(subcommand)]
        command: DualSubcommand,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum AnnualSubcommand {
    /// Years that have data for the annual report
    Years,
    /// Generate the annual report
    Generate {
        /// Report year; 0 (default) covers all years.
        #[arg(long, default_value_t = 0)]
        year: i32,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum DualSubcommand {
    /// Generate the report for you and one friend
    Generate {
        /// Friend's username (wxid)
        #[arg(long)]
        friend: String,
        /// Report year; 0 (default) covers all years.
        #[arg(long, default_value_t = 0)]
        year: i32,
        /// Words to exclude from the phrase rankings (repeatable).
        #[arg(long = "exclude-word")]
        exclude_words: Vec<String>,
    },
}

pub(crate) fn handle_report(command: &ReportCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        ReportSubcommand::Annual { command } => match command {
            AnnualSubcommand::Years => unreachable!("a read"),
            AnnualSubcommand::Generate { year } => Ok(json!(hub.report_annual(*year)?)),
        },
        ReportSubcommand::Dual { command } => match command {
            DualSubcommand::Generate {
                friend,
                year,
                exclude_words,
            } => Ok(json!(hub.report_dual(friend, *year, exclude_words)?)),
        },
    }
}
