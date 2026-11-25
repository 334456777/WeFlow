//! `weflow insight`: the AI insights and the footprint.

use clap::{Args, Subcommand};

use crate::numbers::non_negative_i64;
use crate::term::{ask_yes, can_ask};
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
    /// Delete insight records, all of them or those the filters pick; shows how many go and asks for y first
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
        /// Do not ask before deleting
        #[arg(short = 'y', long)]
        yes: bool,
        /// Only count the records that would be deleted; delete nothing
        #[arg(long, conflicts_with = "yes")]
        dry_run: bool,
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

/// Asks on the terminal before `insight clear` deletes `count` records, of the conversation `session` or of all when
/// it is empty; only `y` or `yes` goes ahead. Without a terminal to ask on, `--yes` is required (#1243).
fn confirm_clear(count: usize, session: &str) -> AppResult<bool> {
    use weflow_core::colour::{Paint, Role, Stream};
    use weflow_core::locale::tr;

    if !can_ask() {
        return Err(AppError::usage(
            "this deletes insight records; add --yes to confirm",
        ));
    }
    let heading = if session.is_empty() {
        tr(
            "About to delete {n} insight records.",
            "将要删除 {n} 条见解记录。",
        )
    } else {
        tr(
            "About to delete {n} insight records of {session}.",
            "将要删除会话 {session} 的 {n} 条见解记录。",
        )
    }
    .replace("{n}", &count.to_string())
    .replace("{session}", session);
    let paint = Paint::of(Stream::Stderr);
    ask_yes(&format!(
        "{}\n{}",
        paint.paint(Role::Strong, &heading),
        tr(
            "Delete them? Type y to confirm, anything else cancels: ",
            "确认删除？输入 y 确认，其他输入取消：",
        )
    ))
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
            yes,
            dry_run,
        } => {
            use weflow_core::contract::{Cancelled, InsightCleared};
            let filters = insight_filters(session, None, start, end, None, None);
            let plan = hub.insight_clear_plan(&filters);
            if dry_run {
                return Ok(json!(plan));
            }
            if plan.records == 0 {
                return Ok(json!(InsightCleared { removed: 0 }));
            }
            if !yes && !confirm_clear(plan.records, &filters.session_id)? {
                return Ok(json!(Cancelled { cancelled: true }));
            }
            Ok(json!(hub.insight_clear_records(&filters)?))
        }
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
