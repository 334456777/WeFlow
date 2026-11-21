//! The insight records: what the engine wrote about each conversation, a file per account in WeFlow's home folder
//! (#1014).
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::now_millis;
use crate::locale::tr;
use crate::model_api::ModelApiFormat;

const MAX_RECORDS_PER_SCOPE: usize = 1000;

/// The folder of the records in WeFlow's home folder, a file per account ([`records_file`]).
const RECORDS_DIR: &str = "insights";

/// The file in WeFlow's home folder that holds the records of every account in WeFlow 7.0.0 and earlier;
/// [`split_legacy_records`] moves them into the accounts' files.
const LEGACY_RECORDS_FILE: &str = "weflow-insight-records.json";

/// The file of the account `scope`'s records in the home folder `home`: `insights/<wxid>.json` for `wxid:<wxid>`,
/// `insights/db-<hash>.json` for `db:<hash>`, else `insights/<scope>.json`. Characters other than ASCII letters,
/// digits, `_` and `-` become `_`; the records carry their account, so two accounts sharing a file still see only
/// their own.
pub(crate) fn records_file(home: &Path, scope: &str) -> PathBuf {
    let name = match scope.strip_prefix("wxid:") {
        Some(wxid) => wxid.to_string(),
        None => scope.replacen(':', "-", 1),
    };
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    home.join(RECORDS_DIR).join(format!("{name}.json"))
}

/// The records in the file at `path`: a list of them, or an object with the list under `records`; `None` when the
/// file cannot be read or holds neither. A record that does not read as one is left out.
fn read_records(path: &Path) -> Option<Vec<InsightRecord>> {
    let list = match serde_json::from_str(&fs::read_to_string(path).ok()?).ok()? {
        Value::Array(list) => list,
        Value::Object(mut file) => match file.remove("records")? {
            Value::Array(list) => list,
            _ => return None,
        },
        _ => return None,
    };
    Some(
        list.into_iter()
            .filter_map(|r| serde_json::from_value(r).ok())
            .collect(),
    )
}

/// A file of records as it is written.
#[derive(Serialize)]
struct RecordsFile<'a> {
    version: u32,
    records: &'a [InsightRecord],
}

/// Writes `records` to the file at `path`, making its folder; both are their owner's only (#812), since the records
/// hold what was sent to the model.
fn write_records(path: &Path, records: &[InsightRecord]) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        crate::private_files::create_dir_all(folder)?;
    }
    let bytes = serde_json::to_vec_pretty(&RecordsFile {
        version: 1,
        records,
    })?;
    crate::private_files::open_options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?
        .write_all(&bytes)
}

/// Moves the records of [`LEGACY_RECORDS_FILE`] in the home folder `home` into the files of their accounts, beside the
/// records there (one with the same id there already stays as it is), then removes it.
///
/// The old file stays when it does not read as records, or when one of the accounts' files cannot be written; the
/// next call tries again.
pub(crate) fn split_legacy_records(home: &Path) {
    let legacy = home.join(LEGACY_RECORDS_FILE);
    let Some(records) = read_records(&legacy) else {
        return;
    };
    let mut by_file: BTreeMap<PathBuf, Vec<InsightRecord>> = BTreeMap::new();
    for record in records {
        by_file
            .entry(records_file(home, &record.account_scope))
            .or_default()
            .push(record);
    }
    for (path, moved) in by_file {
        let mut kept = read_records(&path).unwrap_or_default();
        let ids: HashSet<String> = kept.iter().map(|r| r.id.clone()).collect();
        kept.extend(moved.into_iter().filter(|r| !ids.contains(&r.id)));
        if write_records(&path, &kept).is_err() {
            return;
        }
    }
    let _ = fs::remove_file(legacy);
}

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

/// The insight records of one account, kept in its file in `insights/` of WeFlow's home folder.
pub struct RecordStore {
    path: PathBuf,
    /// The account, as [`InsightRecord::account_scope`] names it.
    scope: String,
    /// What the file holds: the account's records, and those of any other account whose file name is the same.
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

/// What a new insight record says and about whom; [`RecordStore::add`] fills in the id, account, time and read flag.
pub struct NewRecord<'a> {
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
    /// The records of the account `scope` in the home folder `home`, once the file holding every account's records
    /// that WeFlow 7.0.0 and earlier wrote is split into the accounts' files; none when the account's file is missing
    /// or cannot be read.
    pub fn load(home: &Path, scope: &str) -> Self {
        split_legacy_records(home);
        let path = records_file(home, scope);
        let records = read_records(&path).unwrap_or_default();
        Self {
            path,
            scope: scope.to_string(),
            records,
        }
    }

    fn persist(&self) {
        let _ = write_records(&self.path, &self.records);
    }

    fn scoped(&self) -> impl Iterator<Item = &InsightRecord> {
        self.records
            .iter()
            .filter(|r| r.account_scope == self.scope)
    }

    /// Adds the record and saves the file, keeping the newest 1000 of its account.
    pub fn add(&mut self, new: NewRecord<'_>, log: InsightRecordLog) -> InsightRecord {
        let NewRecord {
            session_id,
            display_name,
            avatar_url,
            trigger_reason,
            insight,
        } = new;
        let record = InsightRecord {
            id: uuid_v4(),
            account_scope: self.scope.clone(),
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
            .scoped()
            .map(|r| (r.created_at, r.id.clone()))
            .collect();
        mine.sort_by_key(|a| std::cmp::Reverse(a.0));
        let keep: HashSet<String> = mine
            .into_iter()
            .take(MAX_RECORDS_PER_SCOPE)
            .map(|(_, id)| id)
            .collect();
        self.records
            .retain(|r| r.account_scope != self.scope || keep.contains(&r.id));
        self.persist();
        record
    }

    /// A page of the account's records that `f` matches, the newest first, with the counts over all of them.
    pub fn list(&self, f: &RecordFilters) -> crate::contract::InsightRecords {
        let all: Vec<&InsightRecord> = self.scoped().collect();
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

    /// The account's record `id`, or `success: false` and why when there is none.
    pub fn get(&self, id: &str) -> crate::contract::InsightLookup {
        let failed = |error: &str| crate::contract::InsightLookup {
            success: false,
            record: None,
            error: Some(error.to_string()),
        };
        let id = id.trim();
        if id.is_empty() {
            return failed(tr("record id is empty", "记录 ID 为空"));
        }
        match self.scoped().find(|r| r.id == id) {
            Some(r) => crate::contract::InsightLookup {
                success: true,
                record: Some(r.clone()),
                error: None,
            },
            None => failed(tr("insight record not found", "未找到该见解记录")),
        }
    }

    /// Marks the account's record `id` read and saves the file, or answers `success: false` when there is none.
    pub fn mark_read(&mut self, id: &str) -> crate::contract::InsightMarked {
        let id = id.trim();
        match self
            .records
            .iter_mut()
            .find(|r| r.id == id && r.account_scope == self.scope)
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

    /// Removes the account's records that the conversation and the times of `f` match, and saves the file.
    pub fn clear(&mut self, f: &RecordFilters) -> crate::contract::InsightCleared {
        let mut removed = 0;
        self.records.retain(|r| {
            if r.account_scope != self.scope
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
    use serde_json::json;

    use super::*;

    /// A new record about `session_id` named `name`, saying `insight`.
    fn new_record<'a>(session_id: &'a str, name: &'a str, insight: &'a str) -> NewRecord<'a> {
        NewRecord {
            session_id,
            display_name: name,
            avatar_url: None,
            trigger_reason: "activity",
            insight,
        }
    }

    /// A record `id` of the account `scope`, as an older WeFlow wrote it.
    fn stored(id: &str, scope: &str, read: bool) -> InsightRecord {
        InsightRecord {
            id: id.into(),
            account_scope: scope.into(),
            session_id: "wxid_a".into(),
            read,
            ..Default::default()
        }
    }

    #[test]
    fn each_account_keeps_its_records_in_its_own_file() {
        let home = weflow_native::fixture::TestDir::new("weflow-insight");
        let mut mine = RecordStore::load(&home, "wxid:me");
        let a = mine.add(
            new_record("wxid_a", "Alice", "be nice"),
            InsightRecordLog::default(),
        );
        mine.add(
            new_record("wxid_b", "Bob", "call him"),
            InsightRecordLog::default(),
        );
        let mut other = RecordStore::load(&home, "wxid:other");
        other.add(
            new_record("wxid_a", "Alice", "hidden"),
            InsightRecordLog::default(),
        );
        assert!(home.join("insights/me.json").is_file());
        assert!(home.join("insights/other.json").is_file());

        let list = mine.list(&RecordFilters::default());
        assert_eq!(
            (list.total, list.unread_count, list.contacts.len()),
            (2, 2, 2)
        );
        let f = RecordFilters {
            keyword: "CALL".into(),
            ..Default::default()
        };
        assert_eq!(mine.list(&f).total, 1);
        assert!(
            !other.get(&a.id).success,
            "records of other accounts are invisible"
        );
        assert!(mine.mark_read(&a.id).success);
        assert_eq!(mine.list(&RecordFilters::default()).unread_count, 1);

        let mut again = RecordStore::load(&home, "wxid:me");
        assert_eq!(again.list(&RecordFilters::default()).total, 2, "saved");
        let only_alice = RecordFilters {
            session_id: "wxid_a".into(),
            ..Default::default()
        };
        assert_eq!(again.clear(&only_alice).removed, 1);
        assert_eq!(
            RecordStore::load(&home, "wxid:other")
                .list(&RecordFilters::default())
                .total,
            1
        );
    }

    #[test]
    fn the_file_of_every_account_is_split_into_theirs() {
        let home = weflow_native::fixture::TestDir::new("weflow-insight-split");
        let legacy = home.join(LEGACY_RECORDS_FILE);
        let records = [
            stored("a", "wxid:me", false),
            stored("b", "wxid:me", false),
            stored("c", "wxid:other", false),
        ];
        fs::write(
            &legacy,
            serde_json::to_vec(&json!({ "version": 1, "records": records })).unwrap(),
        )
        .unwrap();
        // a record in the account's file already stays as it is there
        write_records(
            &records_file(&home, "wxid:me"),
            &[stored("a", "wxid:me", true)],
        )
        .unwrap();

        let mine = RecordStore::load(&home, "wxid:me").list(&RecordFilters::default());
        assert_eq!((mine.total, mine.unread_count), (2, 1), "{mine:?}");
        let other = read_records(&records_file(&home, "wxid:other")).unwrap();
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].id, "c");
        assert!(!legacy.exists(), "split, then removed");

        fs::write(&legacy, b"not records").unwrap();
        RecordStore::load(&home, "wxid:me");
        assert!(
            legacy.exists(),
            "a file that does not read as records stays"
        );
    }

    #[test]
    fn a_file_is_named_after_its_account() {
        let home = Path::new("home");
        let name = |scope: &str| records_file(home, scope);
        assert_eq!(name("wxid:wxid_abc"), home.join("insights/wxid_abc.json"));
        assert_eq!(name("db:0123abcd"), home.join("insights/db-0123abcd.json"));
        assert_eq!(name("default"), home.join("insights/default.json"));
        assert_eq!(name("wxid:a/b:c.d"), home.join("insights/a_b_c_d.json"));
    }

    #[test]
    fn accounts_sharing_a_file_see_only_their_own_records() {
        let home = weflow_native::fixture::TestDir::new("weflow-insight-shared");
        assert_eq!(
            records_file(&home, "wxid:db-x"),
            records_file(&home, "db:x")
        );
        let mut a = RecordStore::load(&home, "wxid:db-x");
        a.add(
            new_record("wxid_a", "Alice", "one"),
            InsightRecordLog::default(),
        );
        let mut b = RecordStore::load(&home, "db:x");
        b.add(
            new_record("wxid_b", "Bob", "two"),
            InsightRecordLog::default(),
        );
        b.clear(&RecordFilters::default());
        let a = RecordStore::load(&home, "wxid:db-x");
        assert_eq!(
            a.list(&RecordFilters::default()).total,
            1,
            "b's clear left a's record"
        );
        assert!(read_records(&records_file(&home, "db:x"))
            .unwrap()
            .iter()
            .all(|r| r.account_scope == "wxid:db-x"));
    }
}
