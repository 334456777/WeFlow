//! The human-readable views of the answers people read most: a table for a list, a card for one thing, a chat log
//! for messages and bars for statistics.
//!
//! A view shows what a reader needs: names rather than wxids, times and kinds in words, each line fitted to the
//! terminal, and under it a grey note on stderr saying what was left out and how to see more. `--verbose` shows the
//! answer as [`weflow_core::render`] does, every field of it, and `--json` the document itself; an answer without
//! the shape a view expects is shown that way too.

mod chat;
mod layout;
mod lists;
mod messages;
mod stats;
mod when;

use std::collections::HashMap;

use serde_json::Value;
use weflow_core::reads::Read;
use weflow_core::services::ServiceHub;

use crate::cli::Commands;
use crate::commands::analytics::{AnalyticsCommand, AnalyticsSubcommand};
use crate::commands::biz::{BizCommand, BizSubcommand};
use crate::commands::chat::{ChatCommand, ChatSubcommand};
use crate::commands::group::{GroupCommand, GroupSubcommand};
use crate::commands::sns::{SnsCommand, SnsSubcommand};

pub(crate) use layout::{Glyphs, Style};

/// What the chat window calls a username: a contact's remark or nickname, a member's group nickname, a group's
/// name. A username missing from it has no name to show.
pub(crate) type Names = HashMap<String, String>;

/// What a view prints: its lines for stdout, and the note for stderr.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Shown {
    /// The lines, ending with a newline.
    pub(crate) body: String,
    /// What was left out and how to see more, ending with a newline.
    pub(crate) note: Option<String>,
}

impl Shown {
    /// `lines`, each ended with a newline, and `note`.
    fn of(lines: Vec<String>, note: Option<String>) -> Self {
        let mut body = lines.join("\n");
        body.push('\n');
        Self { body, note }
    }
}

/// The days of a range (`--start`, `--end`), as the command was given them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Span {
    start: Option<String>,
    end: Option<String>,
}

impl Span {
    fn of(start: &Option<String>, end: &Option<String>) -> Self {
        Self {
            start: start.clone(),
            end: end.clone(),
        }
    }

    /// `全部时间`, `2026-01-01 ~ 2026-03-31`, `2026-01-01 起` or `至 2026-03-31`.
    fn words(&self, style: &Style) -> String {
        match (&self.start, &self.end) {
            (None, None) => style.tr("all time", "全部时间").to_string(),
            (Some(s), Some(e)) => format!("{s} ~ {e}"),
            (Some(s), None) => format!("{}{s}{}", style.tr("from ", ""), style.tr("", " 起")),
            (None, Some(e)) => format!("{}{e}", style.tr("until ", "至 ")),
        }
    }
}

/// The view a command's answer is shown in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum View {
    /// `chat sessions`, at most `limit` of them (0: all).
    Sessions { limit: usize },
    /// `chat contacts`.
    Contacts,
    /// `chat contact`.
    Contact,
    /// `chat tab-counts`.
    TabCounts,
    /// `chat detail`.
    Detail,
    /// `chat messages`: a page of at most `limit` messages of `session_id`.
    Messages { session_id: String, limit: i32 },
    /// `chat message`.
    Message { session_id: String },
    /// `chat search`: at most `limit` hits of `keyword`, after skipping `offset`.
    Search {
        keyword: String,
        limit: i32,
        offset: i32,
    },
    /// `group list`.
    GroupList,
    /// `group members`.
    GroupMembers { chatroom_id: String, counts: bool },
    /// `group ranking`, the first `limit` (0: all).
    GroupRanking { limit: usize, span: Span },
    /// `group hours`.
    GroupHours { span: Span },
    /// `group media`.
    GroupMedia { span: Span },
    /// `group member`.
    GroupMember { span: Span },
    /// `group member-messages`.
    MemberMessages {
        chatroom_id: String,
        username: String,
        limit: usize,
    },
    /// `analytics overall`.
    AnalyticsOverall,
    /// `analytics rankings`, the first `limit` (0: all).
    AnalyticsRankings { limit: usize, span: Span },
    /// `analytics time`.
    AnalyticsTime,
    /// `biz accounts`.
    BizAccounts,
    /// `sns users`.
    SnsUsers,
    /// `sns stats`.
    SnsStats,
}

impl View {
    /// The view of `command`'s answer; none for a command shown as [`weflow_core::render`] shows any answer.
    pub(crate) fn of(command: &Commands) -> Option<Self> {
        Some(match command {
            Commands::Chat(ChatCommand { command }) => match command {
                ChatSubcommand::Sessions { limit } => Self::Sessions { limit: *limit },
                ChatSubcommand::Contacts { .. } => Self::Contacts,
                ChatSubcommand::Contact { .. } => Self::Contact,
                ChatSubcommand::TabCounts => Self::TabCounts,
                ChatSubcommand::Detail { .. } => Self::Detail,
                ChatSubcommand::Messages(page) => Self::Messages {
                    session_id: page.session_id.clone(),
                    limit: page.limit,
                },
                ChatSubcommand::Message { session_id, .. } => Self::Message {
                    session_id: session_id.clone(),
                },
                ChatSubcommand::Search {
                    keyword,
                    limit,
                    offset,
                    ..
                } => Self::Search {
                    keyword: keyword.clone(),
                    limit: *limit,
                    offset: *offset,
                },
                _ => return None,
            },
            Commands::Group(GroupCommand { command }) => match command {
                GroupSubcommand::List => Self::GroupList,
                GroupSubcommand::Members {
                    chatroom_id,
                    counts,
                } => Self::GroupMembers {
                    chatroom_id: chatroom_id.clone(),
                    counts: *counts,
                },
                GroupSubcommand::Ranking {
                    limit, start, end, ..
                } => Self::GroupRanking {
                    limit: *limit,
                    span: Span::of(start, end),
                },
                GroupSubcommand::Hours { start, end, .. } => Self::GroupHours {
                    span: Span::of(start, end),
                },
                GroupSubcommand::Media { start, end, .. } => Self::GroupMedia {
                    span: Span::of(start, end),
                },
                GroupSubcommand::Member { start, end, .. } => Self::GroupMember {
                    span: Span::of(start, end),
                },
                GroupSubcommand::MemberMessages {
                    chatroom_id,
                    username,
                    limit,
                    ..
                } => Self::MemberMessages {
                    chatroom_id: chatroom_id.clone(),
                    username: username.clone(),
                    limit: *limit,
                },
                _ => return None,
            },
            Commands::Analytics(AnalyticsCommand { command }) => match command {
                AnalyticsSubcommand::Overall => Self::AnalyticsOverall,
                AnalyticsSubcommand::Rankings { limit, start, end } => Self::AnalyticsRankings {
                    limit: *limit,
                    span: Span::of(start, end),
                },
                AnalyticsSubcommand::Time => Self::AnalyticsTime,
                _ => return None,
            },
            Commands::Biz(BizCommand {
                command: BizSubcommand::Accounts,
            }) => Self::BizAccounts,
            Commands::Sns(SnsCommand { command }) => match command {
                SnsSubcommand::Users => Self::SnsUsers,
                SnsSubcommand::Stats { .. } => Self::SnsStats,
                _ => return None,
            },
            _ => return None,
        })
    }

    /// The read this view is drawn from, for `read` of its command: messages and contacts parsed as the chat window
    /// shows them, and a list the view cuts to the limit itself read whole, so it can say how many there are in all.
    pub(crate) fn read(&self, read: Read) -> Read {
        match (self, read) {
            (Self::Sessions { .. }, Read::ChatSessions { .. }) => Read::ChatSessions { limit: 0 },
            (Self::Contacts, Read::ChatContacts { .. }) => Read::ChatContacts { parsed: true },
            (Self::Messages { .. }, Read::ChatMessages(mut page)) => {
                page.parsed = true;
                Read::ChatMessages(page)
            }
            (
                Self::Message { .. },
                Read::ChatMessage {
                    session_id,
                    local_id,
                    server_id,
                    create_time,
                    ..
                },
            ) => Read::ChatMessage {
                session_id,
                local_id,
                server_id,
                create_time,
                parsed: true,
            },
            (
                Self::Search { .. },
                Read::ChatSearch {
                    keyword,
                    session,
                    limit,
                    offset,
                    start,
                    end,
                    ..
                },
            ) => Read::ChatSearch {
                keyword,
                session,
                limit,
                offset,
                start,
                end,
                parsed: true,
            },
            (Self::AnalyticsRankings { .. }, Read::AnalyticsRankings { start, end, .. }) => {
                Read::AnalyticsRankings {
                    limit: 0,
                    start,
                    end,
                }
            }
            (
                Self::GroupRanking { .. },
                Read::GroupRanking {
                    chatroom_id,
                    start,
                    end,
                    ..
                },
            ) => Read::GroupRanking {
                chatroom_id,
                limit: 0,
                start,
                end,
            },
            (_, read) => read,
        }
    }

    /// The names the view shows for the usernames in `data`, looked up on `hub`; none for a view without usernames
    /// to name.
    pub(crate) fn names(&self, hub: &ServiceHub, data: &Value) -> Names {
        let usernames = |items: &mut dyn Iterator<Item = &Value>, key: &str| -> Vec<String> {
            let mut seen: Vec<String> = Vec::new();
            for name in items.filter_map(|item| item[key].as_str()) {
                if !name.is_empty() && !seen.iter().any(|s| s == name) {
                    seen.push(name.to_string());
                }
            }
            seen
        };
        match self {
            Self::Messages { session_id, .. } | Self::Message { session_id } => {
                let items: Vec<&Value> = match data {
                    Value::Array(items) => items.iter().collect(),
                    other => vec![other],
                };
                let mut wanted = usernames(&mut items.iter().copied(), "senderUsername");
                wanted.push(session_id.clone());
                hub.names_in_chat(Some(session_id), &wanted)
            }
            Self::Search { .. } => {
                let items = data.as_array().map(Vec::as_slice).unwrap_or_default();
                let mut wanted = usernames(&mut items.iter(), "senderUsername");
                wanted.extend(usernames(&mut items.iter(), "_session_id"));
                hub.names_in_chat(None, &wanted)
            }
            Self::MemberMessages {
                chatroom_id,
                username,
                ..
            } => hub.names_in_chat(Some(chatroom_id), &[username.clone(), chatroom_id.clone()]),
            Self::GroupMembers { chatroom_id, .. } => {
                hub.names_in_chat(None, std::slice::from_ref(chatroom_id))
            }
            Self::SnsUsers => {
                let wanted: Vec<String> = data["usernames"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                hub.names_in_chat(None, &wanted)
            }
            _ => Names::new(),
        }
    }

    /// `data` in this view; none when `data` does not have the shape the view expects.
    pub(crate) fn render(&self, data: &Value, names: &Names, style: &Style) -> Option<Shown> {
        match self {
            Self::Sessions { limit } => chat::sessions(data, *limit, style),
            Self::Contacts => chat::contacts(data, style),
            Self::Contact => chat::contact(data, style),
            Self::TabCounts => chat::tab_counts(data, style),
            Self::Detail => chat::detail(data, style),
            Self::Messages { session_id, limit } => {
                messages::page(data, session_id, *limit, names, style)
            }
            Self::Message { session_id } => messages::one(data, session_id, names, style),
            Self::Search {
                keyword,
                limit,
                offset,
            } => messages::search(data, keyword, *limit, *offset, names, style),
            Self::MemberMessages {
                chatroom_id,
                username,
                limit,
            } => messages::member(data, chatroom_id, username, *limit, names, style),
            Self::GroupList => lists::groups(data, style),
            Self::GroupMembers {
                chatroom_id,
                counts,
            } => lists::members(data, chatroom_id, *counts, names, style),
            Self::GroupRanking { limit, span } => stats::group_ranking(data, *limit, span, style),
            Self::GroupHours { span } => stats::group_hours(data, span, style),
            Self::GroupMedia { span } => stats::group_media(data, span, style),
            Self::GroupMember { span } => stats::group_member(data, span, style),
            Self::AnalyticsOverall => stats::overall(data, style),
            Self::AnalyticsRankings { limit, span } => stats::rankings(data, *limit, span, style),
            Self::AnalyticsTime => stats::time(data, style),
            Self::BizAccounts => lists::biz_accounts(data, style),
            Self::SnsUsers => lists::sns_users(data, names, style),
            Self::SnsStats => lists::sns_stats(data, style),
        }
    }
}

/// The text of `value[key]`; empty when it is none or not text.
fn text<'v>(value: &'v Value, key: &str) -> &'v str {
    value[key].as_str().unwrap_or("").trim()
}

/// The number `value[key]`; 0 when it is none or not a number.
fn number(value: &Value, key: &str) -> i64 {
    value[key].as_i64().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn view(args: &[&str]) -> Option<View> {
        View::of(&crate::cli::Cli::try_parse_from(args).unwrap().command)
    }

    #[test]
    fn the_commands_people_read_have_a_view_and_the_others_do_not() {
        assert_eq!(
            view(&["weflow", "chat", "sessions", "--limit", "30"]),
            Some(View::Sessions { limit: 30 })
        );
        assert_eq!(
            view(&["weflow", "chat", "messages", "a@chatroom"]),
            Some(View::Messages {
                session_id: "a@chatroom".into(),
                limit: 50
            })
        );
        assert_eq!(
            view(&["weflow", "analytics", "time"]),
            Some(View::AnalyticsTime)
        );
        assert_eq!(view(&["weflow", "config", "path"]), None);
        assert_eq!(view(&["weflow", "chat", "dates", "x"]), None);
    }

    #[test]
    fn a_view_reads_messages_parsed_and_lists_whole() {
        let page = weflow_core::reads::MessagePage {
            session_id: "x".into(),
            limit: 10,
            offset: 0,
            before: None,
            after: None,
            since: None,
            parsed: false,
        };
        let messages = View::Messages {
            session_id: "x".into(),
            limit: 10,
        };
        let Read::ChatMessages(read) = messages.read(Read::ChatMessages(page)) else {
            panic!("a page of messages");
        };
        assert!(read.parsed);
        assert_eq!(
            View::Sessions { limit: 30 }.read(Read::ChatSessions { limit: 30 }),
            Read::ChatSessions { limit: 0 }
        );
        // a view leaves a read of another kind as it is
        assert_eq!(
            View::Contacts.read(Read::ChatTabCounts),
            Read::ChatTabCounts
        );
    }
}
