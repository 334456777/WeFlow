//! `weflow biz`: official accounts and WeChat Pay notices (reads of `weflow_core::reads`, run by `read_of`).

use clap::{Args, Subcommand};

use crate::numbers::non_negative_i32;

#[derive(Args, Debug)]
pub(crate) struct BizCommand {
    #[command(subcommand)]
    pub(crate) command: BizSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum BizSubcommand {
    /// List official accounts
    Accounts,
    /// Messages of one official account
    Messages {
        /// Official account username
        username: String,
        /// Maximum number of results; 0 means all
        #[arg(long, default_value_t = 50, value_parser = non_negative_i32(), allow_negative_numbers = true)]
        limit: i32,
        /// Number of results to skip
        #[arg(long, default_value_t = 0, value_parser = non_negative_i32(), allow_negative_numbers = true)]
        offset: i32,
    },
    /// WeChat Pay records
    PayRecords {
        /// Number of the WeChat Pay account's messages to read; 0 means all
        #[arg(long, default_value_t = 50, value_parser = non_negative_i32(), allow_negative_numbers = true)]
        limit: i32,
        /// Number of the account's messages to skip; `hasMore` says whether more follow
        #[arg(long, default_value_t = 0, value_parser = non_negative_i32(), allow_negative_numbers = true)]
        offset: i32,
    },
}
