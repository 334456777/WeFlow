//! `weflow group`: group chats, their members and statistics.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use super::analytics::day_span;
use crate::numbers::non_negative_usize;
use serde_json::{json, Value};
use weflow_core::error::AppResult;
use weflow_core::services::ServiceHub;

#[derive(Args, Debug)]
pub(crate) struct GroupCommand {
    #[command(subcommand)]
    pub(crate) command: GroupSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum GroupSubcommand {
    /// Group chats with member counts
    List,
    /// Members of a group chat with friend flag, owner and group nickname; --counts adds message counts
    Members {
        /// Group chat id, such as xxx@chatroom
        chatroom_id: String,
        /// Include per-member message counts
        #[arg(long)]
        counts: bool,
    },
    /// Most active members
    Ranking {
        /// Group chat id, such as xxx@chatroom
        chatroom_id: String,
        /// Number of members; 0 means all
        #[arg(long, default_value_t = 20, value_parser = non_negative_usize, allow_negative_numbers = true)]
        limit: usize,
        /// Start date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        start: Option<String>,
        /// End date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        end: Option<String>,
    },
    /// Messages per hour of day
    Hours {
        /// Group chat id, such as xxx@chatroom
        chatroom_id: String,
        /// Start date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        start: Option<String>,
        /// End date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        end: Option<String>,
    },
    /// Message type mix
    Media {
        /// Group chat id, such as xxx@chatroom
        chatroom_id: String,
        /// Start date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        start: Option<String>,
        /// End date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        end: Option<String>,
    },
    /// One member's statistics: types, hours, common phrases and emoji
    Member {
        /// Group chat id, such as xxx@chatroom
        chatroom_id: String,
        /// Member username, the wxid
        username: String,
        /// Start date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        start: Option<String>,
        /// End date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        end: Option<String>,
    },
    /// A page of one member's messages, newest first
    MemberMessages {
        /// Group chat id, such as xxx@chatroom
        chatroom_id: String,
        /// Member username, the wxid
        username: String,
        /// Number of messages, from 1 to 100; 0 counts as 1
        #[arg(long, default_value_t = 50, value_parser = non_negative_usize, allow_negative_numbers = true)]
        limit: usize,
        /// Cursor returned as `nextCursor` by the previous page
        #[arg(long, default_value_t = 0, value_parser = non_negative_usize, allow_negative_numbers = true)]
        cursor: usize,
        /// Start date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        start: Option<String>,
        /// End date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        end: Option<String>,
    },
    /// Export a member's messages to .csv or .xlsx
    ExportMemberMessages {
        /// Group chat id, such as xxx@chatroom
        chatroom_id: String,
        /// Member username, the wxid
        username: String,
        /// Output file
        out: PathBuf,
        /// Start date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        start: Option<String>,
        /// End date as YYYY-MM-DD, local time, inclusive
        #[arg(long)]
        end: Option<String>,
    },
    /// Export the member list to .csv or .xlsx
    ExportMembers {
        /// Group chat id, such as xxx@chatroom
        chatroom_id: String,
        /// Output file
        out: PathBuf,
    },
}

pub(crate) fn handle_group(command: &GroupCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        GroupSubcommand::List
        | GroupSubcommand::Members { .. }
        | GroupSubcommand::Ranking { .. }
        | GroupSubcommand::Hours { .. }
        | GroupSubcommand::Media { .. }
        | GroupSubcommand::Member { .. }
        | GroupSubcommand::MemberMessages { .. } => unreachable!("a read"),
        GroupSubcommand::ExportMemberMessages {
            chatroom_id,
            username,
            out,
            start,
            end,
        } => {
            let (b, e) = day_span(start, end)?;
            Ok(json!(hub.group_export_member_messages(
                chatroom_id,
                username,
                out,
                b,
                e
            )?))
        }
        GroupSubcommand::ExportMembers { chatroom_id, out } => {
            Ok(json!(hub.group_export_members(chatroom_id, out)?))
        }
    }
}
