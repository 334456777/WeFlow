//! The insight records: what the engine wrote about each conversation, kept on disk.
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::now_millis;
use crate::locale::tr;
use crate::model_api::ModelApiFormat;

const MAX_RECORDS_PER_SCOPE: usize = 1000;
const RECORDS_FILE: &str = "weflow-insight-records.json";

/// What was asked of the AI model for a record, and what it answered.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightRecordLog {
    /// The format of the model API asked; records written before there was a choice asked an OpenAI one.
    #[serde(default)]
    pub format: ModelApiFormat,
    /// The address asked.
    pub endpoint: String,
    /// The model that answered, as the service names it, else the one asked for.
    pub model: String,
    /// The most tokens the answer could have.
    pub max_tokens: u64,
    /// Why it was asked: `activity`, `silence`, `test`, …
    pub trigger_reason: String,
    /// Whether the conversation's latest messages went with the question.
    pub allow_context: bool,
    /// How many of them, at most.
    pub context_count: u64,
    /// The system prompt sent.
    pub system_prompt: String,
    /// The user prompt sent.
    pub user_prompt: String,
    /// The model's answer as it came.
    pub raw_output: String,
    /// The insight kept from it.
    pub final_insight: String,
    /// From the question to the answer.
    pub duration_ms: i64,
    /// When the answer came, in Unix milliseconds.
    pub created_at: i64,
}

/// An insight the engine wrote about a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InsightRecord {
    /// A UUID.
    pub id: String,
    /// The account it belongs to: `wxid:<wxid>`, else `db:` and a hash of the data directory.
    pub account_scope: String,
    /// Unix milliseconds.
    pub created_at: i64,
    /// The conversation it is about.
    pub session_id: String,
    /// The conversation's name.
    pub display_name: String,
    /// The address of the conversation's avatar; left out when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// Why it was written: `activity`, `silence`, `test`, …
    pub trigger_reason: String,
    /// What the model wrote.
    pub insight: String,
    /// Whether it was marked read.
    pub read: bool,
    /// What was asked of the model, and what it answered.
    pub log: InsightRecordLog,
}

impl InsightRecord {
    /// The record as lists and events name it, without its log.
    pub fn summary(&self) -> crate::contract::InsightSummary {
        crate::contract::InsightSummary {
            id: self.id.clone(),
            created_at: self.created_at,
            session_id: self.session_id.clone(),
            display_name: self.display_name.clone(),
            avatar_url: self.avatar_url.clone(),
            trigger_reason: self.trigger_reason.clone(),
            insight: self.insight.clone(),
            read: self.read,
        }
    }
}

/// Which records `insight records` lists and `insight clear` removes; an empty or zero filter matches all.
#[derive(Debug, Clone, Default)]
pub struct RecordFilters {
    /// Text the conversation's name, its username or the insight holds, in any case; `insight clear` does not read
    /// it.
    pub keyword: String,
    /// The conversation.
    pub session_id: String,
    /// The earliest time, in Unix milliseconds.
    pub start_time: i64,
    /// The latest time, in Unix milliseconds.
    pub end_time: i64,
    /// The most records of the page, 1 to 200; 100 when none or 0.
    pub limit: Option<i64>,
    /// The records skipped before the page.
    pub offset: Option<i64>,
}

/// The insight records of every account, kept in one JSON file of WeFlow's home directory.
pub struct RecordStore {
    path: PathBuf,
    records: Vec<InsightRecord>,
}

fn start_of_today_ms() -> i64 {
    let n = Local::now();
    Local
        .with_ymd_and_hms(n.year(), n.month(), n.day(), 0, 0, 0)
        .earliest()
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}

/// What a new insight record says and about whom; [`RecordStore::add`] fills in the id, time and read flag.
pub struct NewRecord<'a> {
    /// The account's [`InsightRecord::account_scope`].
    pub scope: &'a str,
    /// The conversation.
    pub session_id: &'a str,
    /// The conversation's name.
    pub display_name: &'a str,
    /// The address of the conversation's avatar.
    pub avatar_url: Option<String>,
    /// Why it was written.
    pub trigger_reason: &'a str,
    /// What the model wrote.
    pub insight: &'a str,
}

impl RecordStore {
    /// The records kept in `dir`; none when the file is missing or cannot be read.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(RECORDS_FILE);
        let records = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|v| {
                let list = if v.is_array() {
                    v
                } else {
                    v.get("records").cloned()?
                };
                Some(
                    list.as_array()?
                        .iter()
                        .filter_map(|r| serde_json::from_value::<InsightRecord>(r.clone()).ok())
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or_default();
        Self { path, records }
    }

    fn persist(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(
            &self.path,
            serde_json::to_vec_pretty(&json!({ "version": 1, "records": self.records }))
                .unwrap_or_default(),
        );
    }

    fn scoped<'a>(&'a self, scope: &'a str) -> impl Iterator<Item = &'a InsightRecord> {
        self.records
            .iter()
            .filter(move |r| r.account_scope == scope)
    }

    /// Adds the record and saves the file, keeping the newest 1000 of its account.
    pub fn add(&mut self, new: NewRecord<'_>, log: InsightRecordLog) -> InsightRecord {
        let NewRecord {
            scope,
            session_id,
            display_name,
            avatar_url,
            trigger_reason,
            insight,
        } = new;
        let record = InsightRecord {
            id: uuid_v4(),
            account_scope: scope.into(),
            created_at: now_millis(),
            session_id: session_id.into(),
            display_name: display_name.into(),
            avatar_url,
            trigger_reason: trigger_reason.into(),
            insight: insight.into(),
            read: false,
            log,
        };
        self.records.push(record.clone());
        let mut mine: Vec<(i64, String)> = self
            .scoped(scope)
            .map(|r| (r.created_at, r.id.clone()))
            .collect();
        mine.sort_by_key(|a| std::cmp::Reverse(a.0));
        let keep: std::collections::HashSet<String> = mine
            .into_iter()
            .take(MAX_RECORDS_PER_SCOPE)
            .map(|(_, id)| id)
            .collect();
        self.records
            .retain(|r| r.account_scope != scope || keep.contains(&r.id));
        self.persist();
        record
    }

    /// A page of the account `scope`'s records that `f` matches, the newest first, with the counts over all of them.
    pub fn list(&self, scope: &str, f: &RecordFilters) -> crate::contract::InsightRecords {
        let all: Vec<&InsightRecord> = self.scoped(scope).collect();
        let today = start_of_today_ms();
        let mut contacts: Vec<(String, String, Option<String>, usize)> = Vec::new();
        for r in &all {
            match contacts.iter_mut().find(|c| c.0 == r.session_id) {
                Some(c) => c.3 += 1,
                None => contacts.push((
                    r.session_id.clone(),
                    r.display_name.clone(),
                    r.avatar_url.clone(),
                    1,
                )),
            }
        }
        contacts.sort_by_key(|a| std::cmp::Reverse(a.3));
        let keyword = f.keyword.trim().to_lowercase();
        let offset = usize::try_from(f.offset.unwrap_or(0)).unwrap_or(0);
        let limit = f.limit.filter(|l| *l != 0).unwrap_or(100).clamp(1, 200) as usize;
        let mut filtered: Vec<&InsightRecord> = all
            .iter()
            .copied()
            .filter(|r| {
                (f.session_id.is_empty() || r.session_id == f.session_id)
                    && !(f.start_time > 0 && r.created_at < f.start_time)
                    && !(f.end_time > 0 && r.created_at > f.end_time)
                    && (keyword.is_empty()
                        || format!("{}\n{}\n{}", r.display_name, r.session_id, r.insight)
                            .to_lowercase()
                            .contains(&keyword))
            })
            .collect();
        filtered.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        crate::contract::InsightRecords {
            success: true,
            records: filtered
                .iter()
                .skip(offset)
                .take(limit)
                .map(|r| r.summary())
                .collect(),
            total: filtered.len(),
            today_count: all.iter().filter(|r| r.created_at >= today).count(),
            unread_count: all.iter().filter(|r| !r.read).count(),
            contacts: contacts
                .into_iter()
                .map(|(session_id, display_name, avatar_url, count)| {
                    crate::contract::InsightContact {
                        session_id,
                        display_name,
                        count,
                        avatar_url,
                    }
                })
                .collect(),
        }
    }

    /// The account `scope`'s record `id`, or `success: false` and why when there is none.
    pub fn get(&self, scope: &str, id: &str) -> crate::contract::InsightLookup {
        let failed = |error: &str| crate::contract::InsightLookup {
            success: false,
            record: None,
            error: Some(error.to_string()),
        };
        let id = id.trim();
        if id.is_empty() {
            return failed(tr("record id is empty", "记录 ID 为空"));
        }
        match self.scoped(scope).find(|r| r.id == id) {
            Some(r) => crate::contract::InsightLookup {
                success: true,
                record: Some(r.clone()),
                error: None,
            },
            None => failed(tr("insight record not found", "未找到该见解记录")),
        }
    }

    /// Marks the account `scope`'s record `id` read and saves the file, or answers `success: false` when there is
    /// none.
    pub fn mark_read(&mut self, scope: &str, id: &str) -> crate::contract::InsightMarked {
        let id = id.trim();
        match self
            .records
            .iter_mut()
            .find(|r| r.id == id && r.account_scope == scope)
        {
            None => crate::contract::InsightMarked {
                success: false,
                error: Some(tr("insight record not found", "未找到该见解记录").to_string()),
            },
            Some(r) => {
                if !r.read {
                    r.read = true;
                    self.persist();
                }
                crate::contract::InsightMarked {
                    success: true,
                    error: None,
                }
            }
        }
    }

    /// Removes the account `scope`'s records that the conversation and the times of `f` match, and saves the file.
    pub fn clear(&mut self, scope: &str, f: &RecordFilters) -> crate::contract::InsightCleared {
        let mut removed = 0;
        self.records.retain(|r| {
            if r.account_scope != scope
                || (!f.session_id.is_empty() && r.session_id != f.session_id)
                || (f.start_time > 0 && r.created_at < f.start_time)
                || (f.end_time > 0 && r.created_at > f.end_time)
            {
                return true;
            }
            removed += 1;
            false
        });
        self.persist();
        crate::contract::InsightCleared {
            success: true,
            removed,
        }
    }
}

fn uuid_v4() -> String {
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_store_scopes_filters_and_trims() {
        let dir = weflow_native::fixture::TestDir::new("weflow-insight");
        let mut s = RecordStore::load(&dir);
        let a = s.add(
            NewRecord {
                scope: "wxid:me",
                session_id: "wxid_a",
                display_name: "Alice",
                avatar_url: None,
                trigger_reason: "activity",
                insight: "be nice",
            },
            InsightRecordLog::default(),
        );
        s.add(
            NewRecord {
                scope: "wxid:me",
                session_id: "wxid_b",
                display_name: "Bob",
                avatar_url: Some("http://a".into()),
                trigger_reason: "silence",
                insight: "call him",
            },
            InsightRecordLog::default(),
        );
        s.add(
            NewRecord {
                scope: "wxid:other",
                session_id: "wxid_a",
                display_name: "Alice",
                avatar_url: None,
                trigger_reason: "test",
                insight: "hidden",
            },
            InsightRecordLog::default(),
        );
        let list = s.list("wxid:me", &RecordFilters::default());
        assert_eq!(list.total, 2);
        assert_eq!(list.unread_count, 2);
        assert_eq!(list.contacts.len(), 2);
        let f = RecordFilters {
            keyword: "CALL".into(),
            ..Default::default()
        };
        assert_eq!(s.list("wxid:me", &f).total, 1);
        assert!(
            !s.get("wxid:other", &a.id).success,
            "records of other accounts are invisible"
        );
        assert!(s.mark_read("wxid:me", &a.id).success);
        assert_eq!(s.list("wxid:me", &RecordFilters::default()).unread_count, 1);
        // persisted and reloadable
        let again = RecordStore::load(&dir);
        assert_eq!(again.list("wxid:me", &RecordFilters::default()).total, 2);
        let mut again = again;
        assert_eq!(
            again
                .clear(
                    "wxid:me",
                    &RecordFilters {
                        session_id: "wxid_a".into(),
                        ..Default::default()
                    }
                )
                .removed,
            1
        );
        assert_eq!(again.list("wxid:other", &RecordFilters::default()).total, 1);
    }
}
