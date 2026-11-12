//! `weflow analytics`: statistics of the private chats.

use clap::{Args, Subcommand};

use serde_json::{json, Value};
use weflow_core::error::AppResult;
use weflow_core::services::ServiceHub;

#[derive(Args, Debug)]
pub(crate) struct AnalyticsCommand {
    #[command(subcommand)]
    pub(crate) command: AnalyticsSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum AnalyticsSubcommand {
    /// Overall chat statistics (private chats, honouring the exclusion list)
    Overall,
    /// Contacts ranked by message count
    Rankings {
        /// Number of contacts
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// First day (YYYY-MM-DD, local time)
        #[arg(long)]
        start: Option<String>,
        /// Last day, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// Hour / weekday / month distribution
    Time,
    /// Show the exclusion list, or replace it with --set
    Excluded {
        /// Comma-separated usernames that replace the list (pass "" to clear it)
        #[arg(long = "set", value_delimiter = ',', num_args = 0..)]
        set: Option<Vec<String>>,
    },
    /// Private chats that can be excluded from analytics
    ExcludeCandidates,
}

pub(crate) fn handle_analytics(command: &AnalyticsCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        AnalyticsSubcommand::Excluded { set: Some(list) } => Ok(json!(hub
            .analytics_set_excluded_usernames(
                &list
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
            )?)),
        _ => unreachable!("a read"),
    }
}

pub(super) fn day_span(start: &Option<String>, end: &Option<String>) -> AppResult<(i64, i64)> {
    weflow_core::reads::day_span(start.as_deref(), end.as_deref())
}
