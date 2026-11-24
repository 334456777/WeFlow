//! `weflow insight`: the AI insights and the footprint.

use clap::{Args, Subcommand};

use crate::numbers::non_negative_i64;
use serde_json::{json, Value};
use weflow_core::error::{AppError, AppResult};
use weflow_core::services::ServiceHub;

#[derive(Args, Debug)]
pub(crate) struct InsightCommand {
    #[command(subcommand)]
    pub(crate) command: InsightSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum InsightSubcommand {
    /// Test the AI model connection
    Test,
    /// List the models the AI model API offers
    Models,
    /// Generate a test insight for the first eligible private chat, or for the given conversation
    Trigger {
        /// Generate for this conversation instead of the first eligible one
        session_id: Option<String>,
    },
    /// List insight records, newest first
    Records {
        /// Only records containing this text
        #[arg(long)]
        keyword: Option<String>,
        /// Only this conversation
        #[arg(long)]
        session: Option<String>,
        /// Only records from this time on, in Unix seconds
        #[arg(long, value_parser = non_negative_i64(), allow_negative_numbers = true)]
        start: Option<i64>,
        /// Only records up to this time, in Unix seconds
        #[arg(long, value_parser = non_negative_i64(), allow_negative_numbers = true)]
        end: Option<i64>,
        /// Maximum number of records, at most 200; without it, or 0, 100
        #[arg(long, value_parser = non_negative_i64(), allow_negative_numbers = true)]
        limit: Option<i64>,
        /// Number of records to skip
        #[arg(long, value_parser = non_negative_i64(), allow_negative_numbers = true)]
        offset: Option<i64>,
    },
    /// Show one insight record
    Get {
        /// Record id
        id: String,
    },
    /// Mark an insight record as read
    MarkRead {
        /// Record id
        id: String,
    },
    /// Delete insight records, all of them or those the filters pick
    Clear {
        /// Only this conversation
        #[arg(long)]
        session: Option<String>,
        /// Only records from this time on, in Unix seconds
        #[arg(long, value_parser = non_negative_i64(), allow_negative_numbers = true)]
        start: Option<i64>,
        /// Only records up to this time, in Unix seconds
        #[arg(long, value_parser = non_negative_i64(), allow_negative_numbers = true)]
        end: Option<i64>,
    },
    /// Today's trigger counts; only meaningful inside a running `serve --insight`
    TodayStats,
    /// Run the silence scan once
    Scan,
    /// Footprint statistics from the database
    Footprint {
        /// Start date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        start: Option<String>,
        /// End date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        end: Option<String>,
    },
    /// AI footprint summary
    FootprintSummary {
        /// JSON payload {range_label, summary, private_segments, mention_groups}, in the keys of `insight footprint`
        payload_json: String,
    },
}

fn insight_filters(
    session: Option<String>,
    keyword: Option<String>,
    start: Option<i64>,
    end: Option<i64>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> weflow_core::insight::RecordFilters {
    // `--start` and `--end` are Unix seconds and the records' times milliseconds (#1157): `--end` takes the whole of
    // its second; 0 or none is no limit
    let millis = |seconds: Option<i64>, within: i64| {
        seconds
            .filter(|s| *s > 0)
            .map_or(0, |s| s.saturating_mul(1000).saturating_add(within))
    };
    weflow_core::insight::RecordFilters {
        keyword: keyword.unwrap_or_default(),
        session_id: session.unwrap_or_default(),
        start_time: millis(start, 0),
        end_time: millis(end, 999),
        limit,
        offset,
    }
}

pub(crate) fn print_insight(r: &weflow_core::insight::InsightRecord) {
    weflow_core::output::event(json!({ "type": "insight", "record": r.summary() }));
}

pub(crate) async fn handle_insight(command: InsightCommand, hub: &ServiceHub) -> AppResult<Value> {
    use weflow_core::contract::InsightScan;
    match command.command {
        InsightSubcommand::Test => Ok(json!(hub.insight_test_connection().await?)),
        InsightSubcommand::Trigger { session_id } => match session_id {
            None => Ok(json!(hub.insight_trigger_test().await?)),
            Some(id) => Ok(json!(hub.insight_trigger_session(&id).await?)),
        },
        InsightSubcommand::Models => Ok(json!(hub.insight_models().await?)),
        InsightSubcommand::Records {
            keyword,
            session,
            start,
            end,
            limit,
            offset,
        } => Ok(json!(hub.insight_list_records(&insight_filters(
            session, keyword, start, end, limit, offset,
        )))),
        InsightSubcommand::MarkRead { id } => Ok(json!(hub.insight_mark_record_read(&id)?)),
        InsightSubcommand::Clear {
            session,
            start,
            end,
        } => Ok(json!(hub.insight_clear_records(&insight_filters(
            session, None, start, end, None, None
        ))?)),
        InsightSubcommand::TodayStats => Ok(json!(hub.insight_today_stats())),
        InsightSubcommand::Scan => {
            let mut records = Vec::new();
            let generated = hub
                .insight_silence_scan(&mut |r| {
                    print_insight(r);
                    records.push(r.summary());
                })
                .await;
            Ok(json!(InsightScan { generated, records }))
        }
        InsightSubcommand::Get { .. } | InsightSubcommand::Footprint { .. } => {
            unreachable!("a read")
        }
        InsightSubcommand::FootprintSummary { payload_json } => {
            let payload: Value = serde_json::from_str(&payload_json)
                .map_err(|e| AppError::usage(format!("payload_json must be JSON: {e}")))?;
            Ok(json!(hub.insight_footprint_summary(&payload).await?))
        }
    }
}
