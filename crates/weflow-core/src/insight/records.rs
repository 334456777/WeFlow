//! The insight records: what the engine wrote about each conversation, a file per account in WeFlow's home folder
//! (#1014).
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::now_millis;
use crate::error::{AppError, AppResult};
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

/// The copies of the records file at `path` that [`records_to_change`] set aside because they did not read as
/// records: `<file>.corrupt-<time>` beside it.
pub(crate) fn set_aside_records_files(path: &Path) -> Vec<PathBuf> {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}{SET_ASIDE_MARK}", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .map(|e| e.path())
        .collect();
    found.sort();
    found
}

/// What comes between the name of a records file and the time in the name of a copy set aside.
const SET_ASIDE_MARK: &str = ".corrupt-";

/// What a records file holds.
enum Stored {
    /// There is no file.
    Missing,
    /// The records it holds.
    Records(Vec<InsightRecord>),
    /// A file that does not read as records: cut short, or not WeFlow's.
    NotRecords,
}

/// What the file at `path` holds: a list of records, or an object with the list under `records`. A record that does
/// not read as one is left out.
///
/// # Errors
/// When the file is there but cannot be read.
fn read_records(path: &Path) -> io::Result<Stored> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Stored::Missing),
        Err(err) => return Err(err),
    };
    let list = match serde_json::from_slice(&bytes) {
        Ok(Value::Array(list)) => list,
        Ok(Value::Object(mut file)) => match file.remove("records") {
            Some(Value::Array(list)) => list,
            _ => return Ok(Stored::NotRecords),
        },
        _ => return Ok(Stored::NotRecords),
    };
    Ok(Stored::Records(
        list.into_iter()
            .filter_map(|r| serde_json::from_value(r).ok())
            .collect(),
    ))
}

/// Makes the folder of the records file at `path`, its owner's only (#812).
fn make_folder(path: &Path) -> AppResult<()> {
    let Some(folder) = path.parent() else {
        return Ok(());
    };
    crate::private_files::create_dir_all(folder)
        .map_err(|e| AppError::runtime(format!("failed to create {}: {e}", folder.display())))
}

/// Locks the records file at `path` against other processes and threads changing it (#1137), making its folder; the
/// lock holds until the file returned is dropped.
fn lock_records(path: &Path) -> AppResult<fs::File> {
    make_folder(path)?;
    crate::fs_atomic::lock_beside(path)
        .map_err(|e| AppError::runtime(format!("failed to lock {}.lock: {e}", path.display())))
}

/// The records of the file at `path`, read to be changed and written back with [`lock_records`] held; none when
/// there is no file.
///
/// A file that does not read as records (written by something else, or cut short by a crash) is not written over:
/// it is renamed to `<file>.corrupt-<time>` beside it, with a warning, and the change starts from no records.
///
/// # Errors
/// When the file cannot be read, or cannot be set aside.
fn records_to_change(path: &Path) -> AppResult<Vec<InsightRecord>> {
    let stored = read_records(path)
        .map_err(|e| AppError::runtime(format!("failed to read {}: {e}", path.display())))?;
    match stored {
        Stored::Missing => Ok(Vec::new()),
        Stored::Records(records) => Ok(records),
        Stored::NotRecords => {
            let mut name = path.as_os_str().to_os_string();
            name.push(format!(
                "{SET_ASIDE_MARK}{}",
                Local::now().format("%Y%m%d-%H%M%S-%3f")
            ));
            let aside = PathBuf::from(name);
            fs::rename(path, &aside).map_err(|e| {
                AppError::runtime(format!("failed to set aside {}: {e}", path.display()))
            })?;
            crate::output::warn(&super::fill(
                tr(
                    "the insight records in {path} could not be read; the file is kept as {aside}, and new records go to a new file",
                    "无法读取 {path} 中的见解记录；该文件已另存为 {aside}，新的记录写入新文件",
                ),
                &[
                    ("path", path.display().to_string()),
                    ("aside", aside.display().to_string()),
                ],
            ));
            Ok(Vec::new())
        }
    }
}

/// A file of records as it is written.
#[derive(Serialize)]
struct RecordsFile<'a> {
    version: u32,
    records: &'a [InsightRecord],
}

/// Writes `records` to the file at `path` through a temporary file renamed into place, so no reader sees it half
/// written (#1137), making its folder; both are their owner's only (#812), since the records hold what was sent to
/// the model.
fn write_records(path: &Path, records: &[InsightRecord]) -> AppResult<()> {
    make_folder(path)?;
    let failed = |e: &dyn std::fmt::Display| {
        AppError::runtime(format!("failed to write {}: {e}", path.display()))
    };
    let bytes = serde_json::to_vec_pretty(&RecordsFile {
        version: 1,
        records,
    })
    .map_err(|e| failed(&e))?;
    crate::fs_atomic::write_private(path, &bytes).map_err(|e| failed(&e))
}

/// Moves the records of [`LEGACY_RECORDS_FILE`] in the home folder `home` into the files of their accounts, beside the
/// records there (one with the same id there already stays as it is), then removes it.
///
/// The old file stays when it does not read as records, or when one of the accounts' files cannot be written; the
/// next call tries again.
pub(crate) fn split_legacy_records(home: &Path) {
    let legacy = home.join(LEGACY_RECORDS_FILE);
    let Ok(Stored::Records(records)) = read_records(&legacy) else {
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
        if merge_records(&path, moved).is_err() {
            return;
        }
    }
    let _ = fs::remove_file(legacy);
}

/// Adds `moved` to the records of the file at `path` with its lock held, leaving out those whose id is there already.
fn merge_records(path: &Path, moved: Vec<InsightRecord>) -> AppResult<()> {
    let _lock = lock_records(path)?;
    let mut kept = records_to_change(path)?;
    let ids: HashSet<String> = kept.iter().map(|r| r.id.clone()).collect();
    kept.extend(moved.into_iter().filter(|r| !ids.contains(&r.id)));
    write_records(path, &kept)
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

impl RecordFilters {
    /// Whether `insight clear` with these filters deletes `r`: its conversation and its time match; the keyword is
    /// not read.
    fn clears(&self, r: &InsightRecord) -> bool {
        (self.session_id.is_empty() || r.session_id == self.session_id)
            && (self.start_time <= 0 || r.created_at >= self.start_time)
            && (self.end_time <= 0 || r.created_at <= self.end_time)
    }
}

/// The insight records of one account, kept in its file in `insights/` of WeFlow's home folder.
pub struct RecordStore {
    path: PathBuf,
    /// The account, as [`InsightRecord::account_scope`] names it.
    scope: String,
    /// What the file holds: the account's records, and those of any other account whose file name is the same.
    records: Vec<InsightRecord>,
}

/// The file and how many records it holds, not the records.
impl std::fmt::Debug for RecordStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordStore")
            .field("path", &self.path)
            .field("scope", &self.scope)
            .field("records", &self.records.len())
            .finish()
    }
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
#[derive(Debug)]
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
    ///
    /// [`list`](Self::list) and [`get`](Self::get) answer from what is read here; [`add`](Self::add),
    /// [`mark_read`](Self::mark_read) and [`clear`](Self::clear) read the file again with its lock held.
    pub fn load(home: &Path, scope: &str) -> Self {
        split_legacy_records(home);
        let path = records_file(home, scope);
        let records = match read_records(&path) {
            Ok(Stored::Records(records)) => records,
            _ => Vec::new(),
        };
        Self {
            path,
            scope: scope.to_string(),
            records,
        }
    }

    /// Reads the file again with its lock held, applies `change` to what it holds, and writes the records back when
    /// `change` says it changed them, so a change another process or thread made meanwhile is not lost (#1137).
    fn update<R>(
        &mut self,
        change: impl FnOnce(&mut Self) -> AppResult<(R, bool)>,
    ) -> AppResult<R> {
        let _lock = lock_records(&self.path)?;
        self.records = records_to_change(&self.path)?;
        let (result, changed) = change(self)?;
        if changed {
            write_records(&self.path, &self.records)?;
        }
        Ok(result)
    }

    fn scoped(&self) -> impl Iterator<Item = &InsightRecord> {
        self.records
            .iter()
            .filter(|r| r.account_scope == self.scope)
    }

    /// Adds the record and saves the file, keeping the newest 1000 of its account.
    ///
    /// # Errors
    /// When the file cannot be locked, read or written.
    pub fn add(&mut self, new: NewRecord<'_>, log: InsightRecordLog) -> AppResult<InsightRecord> {
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
        self.update(|store| {
            store.records.push(record.clone());
            let mut mine: Vec<(i64, String)> = store
                .scoped()
                .map(|r| (r.created_at, r.id.clone()))
                .collect();
            mine.sort_by_key(|a| std::cmp::Reverse(a.0));
            let keep: HashSet<String> = mine
                .into_iter()
                .take(MAX_RECORDS_PER_SCOPE)
                .map(|(_, id)| id)
                .collect();
            store
                .records
                .retain(|r| r.account_scope != store.scope || keep.contains(&r.id));
            Ok(((), true))
        })?;
        Ok(record)
    }

    /// When the account's newest record about `session_id` written for `trigger_reason` was written, in Unix
    /// milliseconds; none when there is no such record.
    pub fn last_written(&self, session_id: &str, trigger_reason: &str) -> Option<i64> {
        self.scoped()
            .filter(|r| r.session_id == session_id && r.trigger_reason == trigger_reason)
            .map(|r| r.created_at)
            .max()
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
        let limit = usize::try_from(f.limit.filter(|l| *l != 0).unwrap_or(100).clamp(1, 200))
            .unwrap_or(100);
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

    /// The account's record `id`.
    ///
    /// # Errors
    /// A usage error when `id` is empty, and a not-found error (a 404 of the command API, #822) when the account has
    /// no such record.
    pub fn get(&self, id: &str) -> AppResult<crate::contract::InsightLookup> {
        let id = id.trim();
        if id.is_empty() {
            return Err(AppError::usage(tr("record id is empty", "记录 ID 为空")));
        }
        self.scoped()
            .find(|r| r.id == id)
            .map(|r| crate::contract::InsightLookup { record: r.clone() })
            .ok_or_else(not_found)
    }

    /// Marks the account's record `id` read and saves the file.
    ///
    /// # Errors
    /// A not-found error when the account has no such record, and an error when the file cannot be locked, read or
    /// written.
    pub fn mark_read(&mut self, id: &str) -> AppResult<crate::contract::InsightMarked> {
        let id = id.trim();
        self.update(|store| {
            let record = store
                .records
                .iter_mut()
                .find(|r| r.id == id && r.account_scope == store.scope)
                .ok_or_else(not_found)?;
            let changed = !record.read;
            record.read = true;
            Ok((
                crate::contract::InsightMarked { id: id.to_string() },
                changed,
            ))
        })
    }

    /// How many of the account's records [`clear`](Self::clear) with `f` would remove, as the file held them when
    /// the store was loaded.
    pub fn clearable(&self, f: &RecordFilters) -> usize {
        self.scoped().filter(|r| f.clears(r)).count()
    }

    /// Removes the account's records that the conversation and the times of `f` match, and saves the file.
    ///
    /// # Errors
    /// When the file cannot be locked, read or written.
    pub fn clear(&mut self, f: &RecordFilters) -> AppResult<crate::contract::InsightCleared> {
        self.update(|store| {
            let mut removed = 0;
            store.records.retain(|r| {
                let goes = r.account_scope == store.scope && f.clears(r);
                removed += usize::from(goes);
                !goes
            });
            Ok((crate::contract::InsightCleared { removed }, true))
        })
    }
}

/// The failure of a record id the account has no record of.
fn not_found() -> AppError {
    AppError::not_found(tr("insight record not found", "未找到该见解记录"))
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

    /// Adds to `store` a record about `session_id` named `name`, saying `insight`.
    fn add(store: &mut RecordStore, session_id: &str, name: &str, insight: &str) -> InsightRecord {
        let new = NewRecord {
            session_id,
            display_name: name,
            avatar_url: None,
            trigger_reason: "activity",
            insight,
        };
        store.add(new, InsightRecordLog::default()).unwrap()
    }

    /// The records the file at `path` holds.
    fn on_disk(path: &Path) -> Vec<InsightRecord> {
        match read_records(path).unwrap() {
            Stored::Records(records) => records,
            Stored::Missing => panic!("{} is missing", path.display()),
            Stored::NotRecords => panic!("{} does not read as records", path.display()),
        }
    }

    /// The names of the files in the folder `dir`, sorted.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
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
        let a = add(&mut mine, "wxid_a", "Alice", "be nice");
        add(&mut mine, "wxid_b", "Bob", "call him");
        let mut other = RecordStore::load(&home, "wxid:other");
        add(&mut other, "wxid_a", "Alice", "hidden");
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
            other.get(&a.id).is_err(),
            "records of other accounts are invisible"
        );
        assert_eq!(mine.mark_read(&a.id).unwrap().id, a.id);
        assert_eq!(mine.list(&RecordFilters::default()).unread_count, 1);

        let mut again = RecordStore::load(&home, "wxid:me");
        assert_eq!(again.list(&RecordFilters::default()).total, 2, "saved");
        let only_alice = RecordFilters {
            session_id: "wxid_a".into(),
            ..Default::default()
        };
        assert_eq!(again.clear(&only_alice).unwrap().removed, 1);
        assert_eq!(
            RecordStore::load(&home, "wxid:other")
                .list(&RecordFilters::default())
                .total,
            1
        );
    }

    /// What `insight clear --dry-run` counts is what `insight clear` removes (#1243): the conversation and the times
    /// pick, the keyword is not read, and another account's records never count.
    #[test]
    fn what_clear_would_remove_is_what_it_removes() {
        let filters = [
            RecordFilters::default(),
            RecordFilters {
                session_id: "wxid_a".into(),
                ..Default::default()
            },
            RecordFilters {
                session_id: "wxid_nobody".into(),
                ..Default::default()
            },
            RecordFilters {
                keyword: "CALL".into(),
                ..Default::default()
            },
            RecordFilters {
                start_time: i64::MAX / 2,
                ..Default::default()
            },
            RecordFilters {
                end_time: 1,
                ..Default::default()
            },
        ];
        let mut counts = Vec::new();
        for (i, f) in filters.iter().enumerate() {
            let home =
                weflow_native::fixture::TestDir::new(&format!("weflow-insight-clearable-{i}"));
            let mut mine = RecordStore::load(&home, "wxid:me");
            add(&mut mine, "wxid_a", "Alice", "be nice");
            add(&mut mine, "wxid_b", "Bob", "call him");
            add(&mut mine, "wxid_a", "Alice", "lunch");
            add(
                &mut RecordStore::load(&home, "wxid:other"),
                "wxid_a",
                "Alice",
                "hidden",
            );
            let would = RecordStore::load(&home, "wxid:me").clearable(f);
            let removed = RecordStore::load(&home, "wxid:me")
                .clear(f)
                .unwrap()
                .removed;
            assert_eq!(would, removed, "{f:?}");
            counts.push(would);
        }
        assert_eq!(counts, [3, 2, 0, 3, 0, 0]);
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
        let other = on_disk(&records_file(&home, "wxid:other"));
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
        add(&mut a, "wxid_a", "Alice", "one");
        let mut b = RecordStore::load(&home, "db:x");
        add(&mut b, "wxid_b", "Bob", "two");
        b.clear(&RecordFilters::default()).unwrap();
        let a = RecordStore::load(&home, "wxid:db-x");
        assert_eq!(
            a.list(&RecordFilters::default()).total,
            1,
            "b's clear left a's record"
        );
        assert!(on_disk(&records_file(&home, "db:x"))
            .iter()
            .all(|r| r.account_scope == "wxid:db-x"));
    }

    #[test]
    fn a_file_cut_short_is_set_aside_not_written_over() {
        let home = weflow_native::fixture::TestDir::new("weflow-insight-cut");
        let mut store = RecordStore::load(&home, "wxid:me");
        let first = add(&mut store, "wxid_a", "Alice", "one");
        add(&mut store, "wxid_b", "Bob", "two");
        let path = records_file(&home, "wxid:me");
        // as another process would leave it, killed while writing it in place
        let whole = fs::read(&path).unwrap();
        let half = &whole[..whole.len() / 2];
        fs::write(&path, half).unwrap();

        let mut store = RecordStore::load(&home, "wxid:me");
        assert_eq!(store.list(&RecordFilters::default()).total, 0);
        assert!(store.mark_read(&first.id).is_err());
        let set_aside = set_aside_records_files(&path);
        assert_eq!(set_aside.len(), 1, "{:?}", names_in(&home.join("insights")));
        assert_eq!(fs::read(&set_aside[0]).unwrap(), half, "kept as it was");
        let third = add(&mut store, "wxid_c", "Carol", "three");
        let now = on_disk(&path);
        assert_eq!(now.len(), 1);
        assert_eq!(now[0].id, third.id);
        assert_eq!(set_aside_records_files(&path), set_aside);
        assert_eq!(fs::read(&set_aside[0]).unwrap(), half, "not written over");
    }

    #[test]
    fn an_accounts_file_cut_short_is_set_aside_when_the_old_file_is_split() {
        let home = weflow_native::fixture::TestDir::new("weflow-insight-split-cut");
        let path = records_file(&home, "wxid:me");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{\"version\":1,\"records\":[{\"id\":").unwrap();
        let legacy = home.join(LEGACY_RECORDS_FILE);
        let records = [stored("a", "wxid:me", false)];
        fs::write(
            &legacy,
            serde_json::to_vec(&json!({ "version": 1, "records": records })).unwrap(),
        )
        .unwrap();

        RecordStore::load(&home, "wxid:me");
        assert_eq!(on_disk(&path).len(), 1);
        assert!(!legacy.exists());
        let set_aside = set_aside_records_files(&path);
        assert_eq!(set_aside.len(), 1);
        assert_eq!(
            fs::read(&set_aside[0]).unwrap(),
            b"{\"version\":1,\"records\":[{\"id\":"
        );
    }

    #[test]
    fn two_stores_taking_turns_lose_no_change() {
        let home = weflow_native::fixture::TestDir::new("weflow-insight-turns");
        // as the engine in `serve` and a `mark-read` child process each hold one, read before the other's changes
        let mut engine = RecordStore::load(&home, "wxid:me");
        let mut child = RecordStore::load(&home, "wxid:me");
        for i in 0..5 {
            let a = add(&mut engine, "wxid_a", "Alice", &format!("a{i}"));
            let b = add(&mut child, "wxid_b", "Bob", &format!("b{i}"));
            engine.mark_read(&b.id).unwrap();
            child.mark_read(&a.id).unwrap();
        }
        let list = RecordStore::load(&home, "wxid:me").list(&RecordFilters::default());
        assert_eq!((list.total, list.unread_count), (10, 0), "{list:?}");
    }

    #[test]
    fn stores_changing_the_file_at_once_lose_no_record() {
        let home = weflow_native::fixture::TestDir::new("weflow-insight-threads");
        std::thread::scope(|s| {
            for who in ["wxid_a", "wxid_b", "wxid_c"] {
                let home = &home;
                s.spawn(move || {
                    for i in 0..15 {
                        let mut store = RecordStore::load(home, "wxid:me");
                        let record = add(&mut store, who, who, &format!("{who} {i}"));
                        let mut store = RecordStore::load(home, "wxid:me");
                        store.mark_read(&record.id).unwrap();
                    }
                });
            }
        });
        let list = RecordStore::load(&home, "wxid:me").list(&RecordFilters::default());
        assert_eq!((list.total, list.unread_count), (45, 0), "{list:?}");
        assert_eq!(
            names_in(&home.join("insights")),
            ["me.json", "me.json.lock"],
            "no temporary file is left"
        );
    }
}
