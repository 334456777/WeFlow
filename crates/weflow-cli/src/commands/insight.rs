//! `weflow insight`: the AI insights and the footprint.

use clap::{Args, Subcommand};

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
    /// Generate a test insight for the first eligible private chat, or for the given conversation
    Trigger {
        /// Generate for this conversation instead of the first eligible one
        session_id: Option<String>,
    },
    /// List insight records (newest first)
    Records {
        /// Only records containing this text
        #[arg(long)]
        keyword: Option<String>,
        /// Only this conversation
        #[arg(long)]
        session: Option<String>,
        /// Only records from this time on (Unix seconds)
        #[arg(long)]
        start: Option<i64>,
        /// Only records up to this time (Unix seconds)
        #[arg(long)]
        end: Option<i64>,
        /// Maximum number of records
        #[arg(long)]
        limit: Option<i64>,
        /// Number of records to skip
        #[arg(long)]
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
    /// Delete insight records (all, or filtered)
    Clear {
        /// Only this conversation
        #[arg(long)]
        session: Option<String>,
        /// Only records from this time on (Unix seconds)
        #[arg(long)]
        start: Option<i64>,
        /// Only records up to this time (Unix seconds)
        #[arg(long)]
        end: Option<i64>,
    },
    /// Today's trigger counts (only meaningful inside a running `serve --insight`)
    TodayStats,
    /// Run the silence scan once
    Scan,
    /// Footprint statistics from the database
    Footprint {
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// AI footprint summary; takes the JSON payload {rangeLabel, summary, privateSegments, mentionGroups}
    FootprintSummary {
        /// JSON payload
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
    weflow_core::insight::RecordFilters {
        keyword: keyword.unwrap_or_default(),
        session_id: session.unwrap_or_default(),
        start_time: start.unwrap_or(0),
        end_time: end.unwrap_or(0),
        limit,
        offset,
    }
}

pub(crate) fn print_insight(r: &weflow_core::insight::InsightRecord) {
    weflow_core::output::event(json!({ "type": "insight", "record": r.summary() }));
}

pub(crate) async fn handle_insight(command: InsightCommand, hub: &ServiceHub) -> AppResult<Value> {
    use weflow_core::contract::{InsightReply, InsightScan};
    match command.command {
        InsightSubcommand::Test => Ok(json!(hub.insight_test_connection().await)),
        InsightSubcommand::Trigger { session_id } => match session_id {
            None => Ok(json!(hub.insight_trigger_test().await)),
            Some(id) => {
                let name = hub
                    .chat_contact_avatar(&id)
                    .map(|(_, n)| n)
                    .unwrap_or_else(|| id.clone());
                let reply = match hub
                    .insight_generate(&id, &name, weflow_core::services::InsightTrigger::Test, None)
                    .await
                {
                    Some(r) => InsightReply {
                        success: true,
                        message: None,
                        record: Some(r.summary()),
                        insight: None,
                    },
                    None => InsightReply::failed(
                        "no insight was generated (AI not configured, request failed, or the model answered SKIP)",
                    ),
                };
                Ok(json!(reply))
            }
        },
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
        InsightSubcommand::MarkRead { id } => Ok(json!(hub.insight_mark_record_read(&id))),
        InsightSubcommand::Clear {
            session,
            start,
            end,
        } => Ok(json!(hub.insight_clear_records(&insight_filters(
            session, None, start, end, None, None
        )))),
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
            Ok(json!(hub.insight_footprint_summary(&payload).await))
        }
    }
}
