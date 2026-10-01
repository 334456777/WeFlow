//! C ABI of the native database layer: the same `wcdb_*` functions, argument lists and JSON results the desktop app
//! used to get from the closed-source `wcdb_api` library, served by [`weflow_native::wcdb::Wcdb`].
//!
//! Conventions (as the desktop app's `wcdbCore.ts` expects them):
//! - every function returns `0` on success and a negative status otherwise;
//! - text results are written to `_Out_ void**` arguments as NUL-terminated UTF-8 that the caller releases with
//!   [`wcdb_free_string`]; on failure the error text goes to the same out argument when the function has one;
//! - an account is opened with [`wcdb_open_account`] and addressed by the returned handle.
//!
//! The databases are opened read-only: functions that would modify them (edits, deletes, triggers, read marks,
//! table imports) return [`STATUS_READ_ONLY`] with an explanation. There is no expiry check, no network access and no
//! change monitor (the desktop app falls back to polling when the monitor symbols are absent).

use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, Result};
use serde_json::Value;
use weflow_native::wcdb::{Arg, Wcdb};

pub const STATUS_OK: i32 = 0;
/// Generic failure; the reason is in the out argument (when there is one) and in `wcdb_get_logs`.
pub const STATUS_FAILED: i32 = -1;
/// The handle is unknown or was closed.
pub const STATUS_BAD_HANDLE: i32 = -2;
/// A required argument was missing or not valid.
pub const STATUS_BAD_ARGUMENT: i32 = -3;
/// Refused: the operation would modify WeChat's databases.
pub const STATUS_READ_ONLY: i32 = -4;
/// A Rust panic was caught at the boundary.
pub const STATUS_PANIC: i32 = -99;

struct Account {
    dir: PathBuf,
    key: String,
    db: Wcdb,
}

fn accounts() -> &'static Mutex<HashMap<i64, Arc<Account>>> {
    static ACCOUNTS: OnceLock<Mutex<HashMap<i64, Arc<Account>>>> = OnceLock::new();
    ACCOUNTS.get_or_init(Default::default)
}

fn logs() -> &'static Mutex<VecDeque<String>> {
    static LOGS: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    LOGS.get_or_init(Default::default)
}

fn log(line: String) {
    if let Ok(mut l) = logs().lock() {
        if l.len() >= 500 {
            l.pop_front();
        }
        l.push_back(line);
    }
}

static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);

/// # Safety
/// `p` is null or a NUL-terminated string.
unsafe fn text(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// # Safety
/// `out` is null or points to writable storage for one pointer.
unsafe fn put(out: *mut *mut c_void, value: &str) {
    if out.is_null() {
        return;
    }
    let c = CString::new(value.replace('\0', "")).unwrap_or_default();
    *out = c.into_raw().cast();
}

/// # Safety
/// `out` is null or points to writable storage of type `T`.
unsafe fn put_value<T>(out: *mut T, value: T) {
    if !out.is_null() {
        *out = value;
    }
}

fn account(handle: i64) -> Option<Arc<Account>> {
    accounts().lock().ok()?.get(&handle).cloned()
}

fn status_of(e: &anyhow::Error) -> i32 {
    let m = e.to_string();
    if m.contains("read-only") {
        STATUS_READ_ONLY
    } else {
        STATUS_FAILED
    }
}

/// Runs `f` with the account, catching panics; on error writes the message to `err_out` and the log.
fn run(name: &str, handle: i64, err_out: *mut *mut c_void, f: impl FnOnce(&Wcdb) -> Result<()>) -> i32 {
    let Some(acct) = account(handle) else {
        log(format!("{name}: unknown handle {handle}"));
        // SAFETY: caller contract of every exported function.
        unsafe { put(err_out, "invalid handle") };
        return STATUS_BAD_HANDLE;
    };
    match catch_unwind(AssertUnwindSafe(|| f(&acct.db))) {
        Ok(Ok(())) => STATUS_OK,
        Ok(Err(e)) => {
            log(format!("{name}: {e:#}"));
            // SAFETY: see above.
            unsafe { put(err_out, &format!("{e:#}")) };
            status_of(&e)
        }
        Err(_) => {
            log(format!("{name}: internal error (panic)"));
            STATUS_PANIC
        }
    }
}

/// `run` for functions whose result is JSON in `out`.
fn json(name: &str, handle: i64, out: *mut *mut c_void, f: impl FnOnce(&Wcdb) -> Result<Value>) -> i32 {
    run(name, handle, out, |db| {
        let v = f(db)?;
        // SAFETY: caller contract.
        unsafe { put(out, &v.to_string()) };
        Ok(())
    })
}

/// `run` for functions whose result is plain text in `out`.
fn string(name: &str, handle: i64, out: *mut *mut c_void, f: impl FnOnce(&Wcdb) -> Result<String>) -> i32 {
    run(name, handle, out, |db| {
        let v = f(db)?;
        // SAFETY: caller contract.
        unsafe { put(out, &v) };
        Ok(())
    })
}

fn read_only(name: &str, out: *mut *mut c_void) -> i32 {
    let msg = format!("{name} is not supported: the native database backend opens WeChat's databases read-only");
    log(msg.clone());
    // SAFETY: caller contract.
    unsafe { put(out, &msg) };
    STATUS_READ_ONLY
}

fn opt(s: String) -> Option<String> {
    Some(s).filter(|s| !s.trim().is_empty())
}

// ───────────────────────── lifecycle ─────────────────────────

/// Kept for the desktop app's start-up sequence; there is nothing to verify.
#[no_mangle]
pub extern "C" fn InitProtection(_resource_path: *const c_char) -> i32 {
    STATUS_OK
}

#[no_mangle]
pub extern "C" fn wcdb_init() -> i32 {
    STATUS_OK
}

/// Closes every open account.
#[no_mangle]
pub extern "C" fn wcdb_shutdown() -> i32 {
    if let Ok(mut a) = accounts().lock() {
        a.clear();
    }
    STATUS_OK
}

/// The account directory of a `session.db` path (`<account>/db_storage/session/session.db`), or the path itself
/// when it already is a directory.
fn account_dir_of(path: &Path) -> PathBuf {
    if path.is_dir() {
        return if path.file_name().is_some_and(|n| n == "db_storage") { path.parent().unwrap_or(path).to_path_buf() } else { path.to_path_buf() };
    }
    path.ancestors().find(|p| p.file_name().is_some_and(|n| n == "db_storage")).and_then(Path::parent).unwrap_or(path).to_path_buf()
}

fn open(dir: &Path, key: &str, wxid: Option<&str>) -> Result<Wcdb> {
    let mut db = Wcdb::new();
    db.open(dir, key, wxid)?;
    Ok(db)
}

/// # Safety
/// `path` and `key` are NUL-terminated strings; `handle` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_open_account(path: *const c_char, key: *const c_char, handle: *mut i64) -> i32 {
    let (path, key) = (text(path), text(key));
    if path.trim().is_empty() || key.trim().is_empty() {
        return STATUS_BAD_ARGUMENT;
    }
    let dir = account_dir_of(Path::new(path.trim()));
    match catch_unwind(AssertUnwindSafe(|| open(&dir, key.trim(), None))) {
        Ok(Ok(db)) => {
            let id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut a) = accounts().lock() {
                a.insert(id, Arc::new(Account { dir, key: key.trim().to_string(), db }));
            }
            put_value(handle, id);
            STATUS_OK
        }
        Ok(Err(e)) => {
            log(format!("wcdb_open_account: {e:#}"));
            STATUS_FAILED
        }
        Err(_) => STATUS_PANIC,
    }
}

#[no_mangle]
pub extern "C" fn wcdb_close_account(handle: i64) -> i32 {
    match accounts().lock().ok().and_then(|mut a| a.remove(&handle)) {
        Some(_) => STATUS_OK,
        None => STATUS_BAD_HANDLE,
    }
}

/// Tells the layer which wxid owns the account (decides `is_send`).
///
/// # Safety
/// `wxid` is a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn wcdb_set_my_wxid(handle: i64, wxid: *const c_char) -> i32 {
    let wxid = text(wxid);
    let Some(acct) = account(handle) else { return STATUS_BAD_HANDLE };
    match open(&acct.dir, &acct.key, opt(wxid).as_deref()) {
        Ok(db) => {
            if let Ok(mut a) = accounts().lock() {
                a.insert(handle, Arc::new(Account { dir: acct.dir.clone(), key: acct.key.clone(), db }));
            }
            STATUS_OK
        }
        Err(e) => {
            log(format!("wcdb_set_my_wxid: {e:#}"));
            STATUS_FAILED
        }
    }
}

/// # Safety
/// `ptr` is null or a string returned by this library.
#[no_mangle]
pub unsafe extern "C" fn wcdb_free_string(ptr: *mut c_void) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr.cast()));
    }
}

/// Recent errors as a JSON array of strings.
///
/// # Safety
/// `out` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_get_logs(out: *mut *mut c_void) -> i32 {
    let lines: Vec<String> = logs().lock().map(|l| l.iter().cloned().collect()).unwrap_or_default();
    put(out, &Value::from(lines).to_string());
    STATUS_OK
}

/// The cloud reporting of the original library is not reproduced; these are accepted and ignored.
#[no_mangle]
pub extern "C" fn wcdb_cloud_init(_interval_seconds: i32) -> i32 {
    STATUS_OK
}

#[no_mangle]
pub extern "C" fn wcdb_cloud_report(_stats_json: *const c_char) -> i32 {
    STATUS_OK
}

#[no_mangle]
pub extern "C" fn wcdb_cloud_stop() {}

// ───────────────────────── generated JSON functions ─────────────────────────

macro_rules! json_fn {
    ($name:ident($($arg:ident: $ty:ty),*) |$db:ident| $body:expr) => {
        /// # Safety
        /// String arguments are null or NUL-terminated; `out` is null or writable.
        #[no_mangle]
        pub unsafe extern "C" fn $name(handle: i64, $($arg: $ty,)* out: *mut *mut c_void) -> i32 {
            $(let $arg = conv::Conv::conv($arg);)*
            json(stringify!($name), handle, out, |$db| $body)
        }
    };
}

macro_rules! string_fn {
    ($name:ident($($arg:ident: $ty:ty),*) |$db:ident| $body:expr) => {
        /// # Safety
        /// String arguments are null or NUL-terminated; `out` is null or writable.
        #[no_mangle]
        pub unsafe extern "C" fn $name(handle: i64, $($arg: $ty,)* out: *mut *mut c_void) -> i32 {
            $(let $arg = conv::Conv::conv($arg);)*
            string(stringify!($name), handle, out, |$db| $body)
        }
    };
}

/// Argument conversion inside the macros: C strings become `String`, numbers stay as they are.
mod conv {
    use std::ffi::c_char;
    pub trait Conv {
        type Out;
        unsafe fn conv(self) -> Self::Out;
    }
    impl Conv for *const c_char {
        type Out = String;
        unsafe fn conv(self) -> String {
            super::text(self)
        }
    }
    impl Conv for i32 {
        type Out = i32;
        unsafe fn conv(self) -> i32 {
            self
        }
    }
    impl Conv for i64 {
        type Out = i64;
        unsafe fn conv(self) -> i64 {
            self
        }
    }
}

type Str = *const c_char;

fn ids(json: &str) -> Vec<String> {
    weflow_native::native_contact::usernames_from_json(json)
}

json_fn!(wcdb_get_sessions() |db| db.sessions());
json_fn!(wcdb_get_messages(username: Str, limit: i32, offset: i32) |db| db.messages(&username, limit, offset));
json_fn!(wcdb_get_message_dates(session_id: Str) |db| db.message_dates(&session_id));
json_fn!(wcdb_get_session_message_counts(session_ids_json: Str) |db| db.session_message_counts(&ids(&session_ids_json)));
json_fn!(wcdb_get_session_message_date_counts(session_id: Str) |db| db.session_message_date_counts(&session_id));
json_fn!(wcdb_get_session_message_date_counts_batch(session_ids_json: Str) |db| db.session_message_date_counts_batch(&session_ids_json));
json_fn!(wcdb_get_session_message_type_stats(session_id: Str, begin: i32, end: i32) |db| db.session_message_type_stats(&session_id, begin, end));
json_fn!(wcdb_get_session_message_type_stats_batch(session_ids_json: Str, options_json: Str) |db| db.session_message_type_stats_batch(&session_ids_json, &options_json));
json_fn!(wcdb_get_messages_by_type(session_id: Str, local_type: i64, ascending: i32, limit: i32, offset: i32) |db| db.messages_by_type(&session_id, local_type, ascending != 0, limit, offset));
json_fn!(wcdb_get_message_by_id(session_id: Str, local_id: i32) |db| db.message_by_id(&session_id, local_id));
json_fn!(wcdb_get_message_by_svrid(session_id: Str, svrid: Str) |db| db.message_by_server_id(&session_id, &svrid));
json_fn!(wcdb_search_messages(session_id: Str, keyword: Str, limit: i32, offset: i32, begin: i32, end: i32) |db| db.search(&keyword, opt(session_id).as_deref(), limit, offset, begin, end));
json_fn!(wcdb_get_contact(username: Str) |db| db.contact(&username));
json_fn!(wcdb_get_contacts_compact(usernames_json: Str) |db| db.invoke_json("wcdb_get_contacts_compact", &[Arg::S(&usernames_json)]));
json_fn!(wcdb_get_contact_type_counts() |db| db.contact_type_counts());
json_fn!(wcdb_get_contact_status(usernames_json: Str) |db| db.contact_status(&usernames_json));
json_fn!(wcdb_get_contact_alias_map(usernames_json: Str) |db| db.contact_alias_map(&usernames_json));
json_fn!(wcdb_get_contact_friend_flags(usernames_json: Str) |db| db.contact_friend_flags(&usernames_json));
json_fn!(wcdb_get_display_names(usernames_json: Str) |db| db.display_names(&usernames_json));
json_fn!(wcdb_get_avatar_urls(usernames_json: Str) |db| db.avatar_urls(&usernames_json));
json_fn!(wcdb_get_head_image_buffers(usernames_json: Str) |db| db.head_image_buffers(&usernames_json));
json_fn!(wcdb_get_group_members(chatroom_id: Str) |db| db.group_members(&chatroom_id));
json_fn!(wcdb_get_group_nicknames(chatroom_id: Str) |db| db.group_nicknames(&chatroom_id));
json_fn!(wcdb_get_group_member_counts(chatroom_ids_json: Str) |db| db.group_member_counts(&chatroom_ids_json));
json_fn!(wcdb_get_group_stats(chatroom_id: Str, begin: i32, end: i32) |db| db.group_stats(&chatroom_id, begin, end));
json_fn!(wcdb_get_chat_room_ext_buffer(chatroom_id: Str) |db| db.chat_room_ext_buffer(&chatroom_id));
json_fn!(wcdb_get_aggregate_stats(session_ids_json: Str, begin: i32, end: i32) |db| db.aggregate_stats(&ids(&session_ids_json), begin, end));
json_fn!(wcdb_get_available_years(session_ids_json: Str) |db| db.available_years(&ids(&session_ids_json)));
json_fn!(wcdb_get_annual_report_stats(session_ids_json: Str, begin: i32, end: i32) |db| db.annual_report_stats(&ids(&session_ids_json), begin, end));
json_fn!(wcdb_get_annual_report_extras(session_ids_json: Str, begin: i32, end: i32, peak_begin: i32, peak_end: i32) |db| db.annual_report_extras(&session_ids_json, begin, end, peak_begin, peak_end));
json_fn!(wcdb_get_dual_report_stats(session_id: Str, begin: i32, end: i32) |db| db.dual_report_stats(&session_id, begin, end));
json_fn!(wcdb_get_my_footprint_stats(options_json: Str) |db| db.footprint_stats(&serde_json::from_str(&options_json).unwrap_or(Value::Null)));
json_fn!(wcdb_get_sns_timeline(limit: i32, offset: i32, username: Str, keyword: Str, start: i32, end: i32) |db| db.sns_timeline(limit, offset, opt(username).as_deref(), opt(keyword).as_deref(), start, end));
json_fn!(wcdb_get_sns_annual_stats(begin: i32, end: i32) |db| db.sns_annual_stats(begin, end));
json_fn!(wcdb_get_sns_usernames() |db| db.sns_usernames());
json_fn!(wcdb_get_sns_export_stats(my_wxid: Str) |db| db.sns_export_stats(opt(my_wxid).as_deref()));
json_fn!(wcdb_exec_query(kind: Str, path: Str, sql: Str) |db| db.exec_query(&kind, &path, &sql));
json_fn!(wcdb_get_message_tables(session_id: Str) |db| db.message_tables(&session_id));
json_fn!(wcdb_get_message_meta(db_path: Str, table: Str, limit: i32, offset: i32) |db| db.message_meta(&db_path, &table, limit, offset));
json_fn!(wcdb_get_message_table_stats(session_id: Str) |db| db.message_table_stats(&session_id));
json_fn!(wcdb_get_message_table_columns(db_path: Str, table: Str) |db| db.message_table_columns(&db_path, &table));
json_fn!(wcdb_get_message_table_time_range(db_path: Str, table: Str) |db| db.message_table_time_range(&db_path, &table));
json_fn!(wcdb_list_message_dbs() |db| db.list_message_dbs());
json_fn!(wcdb_list_media_dbs() |db| db.list_media_dbs());
json_fn!(wcdb_get_db_status() |db| db.db_status());
json_fn!(wcdb_get_media_schema_summary(db_path: Str) |db| db.media_schema_summary(&db_path));
json_fn!(wcdb_list_tables(kind: Str, db_path: Str) |db| db.list_tables(&kind, &db_path));
json_fn!(wcdb_get_table_schema(kind: Str, db_path: Str, table: Str) |db| db.table_schema(&kind, &db_path, &table));
json_fn!(wcdb_export_table_snapshot(kind: Str, db_path: Str, table: Str, output_path: Str) |db| db.export_table_snapshot(&kind, &db_path, &table, &output_path));
json_fn!(wcdb_get_voice_data_batch(requests_json: Str) |db| db.voice_data_batch(&requests_json));
json_fn!(wcdb_resolve_image_hardlink(md5: Str, account_dir: Str) |db| db.resolve_image_hardlink(&md5, &account_dir));
json_fn!(wcdb_resolve_image_hardlink_batch(requests_json: Str) |db| db.resolve_image_hardlink_batch(&requests_json));
json_fn!(wcdb_resolve_video_hardlink_md5(md5: Str, db_path: Str) |db| db.resolve_video_hardlink_md5(&md5, &db_path));
json_fn!(wcdb_resolve_video_hardlink_md5_batch(requests_json: Str) |db| db.resolve_video_hardlink_md5_batch(&requests_json));

string_fn!(wcdb_get_voice_data(session_id: Str, create_time: i32, local_id: i32, svr_id: i64, candidates_json: Str) |db| db.voice_data(&session_id, create_time, local_id, svr_id, &candidates_json));
string_fn!(wcdb_get_emoticon_cdn_url(db_path: Str, md5: Str) |db| db.emoticon_cdn_url(&db_path, &md5));
string_fn!(wcdb_get_emoticon_caption(db_path: Str, md5: Str) |db| db.emoticon_caption(&db_path, &md5));
string_fn!(wcdb_get_emoticon_caption_strict(md5: Str) |db| db.emoticon_caption_strict(&md5));

// ───────────────────────── numbers and cursors ─────────────────────────

/// # Safety
/// `username` is NUL-terminated; `out_count` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_get_message_count(handle: i64, username: *const c_char, out_count: *mut i32) -> i32 {
    let username = text(username);
    run("wcdb_get_message_count", handle, std::ptr::null_mut(), |db| {
        put_value(out_count, db.message_count(&username)?);
        Ok(())
    })
}

/// # Safety
/// `chatroom_id` is NUL-terminated; `out_count` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_get_group_member_count(handle: i64, chatroom_id: *const c_char, out_count: *mut i32) -> i32 {
    let id = text(chatroom_id);
    run("wcdb_get_group_member_count", handle, std::ptr::null_mut(), |db| {
        let v = db.group_member_count(&id)?;
        let n = v.as_i64().or_else(|| v.get("count").and_then(Value::as_i64)).ok_or_else(|| anyhow!("unexpected member count {v}"))?;
        put_value(out_count, n as i32);
        Ok(())
    })
}

/// # Safety
/// See [`wcdb_open_message_cursor`].
unsafe fn open_cursor(name: &str, handle: i64, session_id: *const c_char, batch: i32, ascending: i32, begin: i32, end: i32, lite: bool, out: *mut i64) -> i32 {
    let sid = text(session_id);
    run(name, handle, std::ptr::null_mut(), |db| {
        put_value(out, db.open_message_cursor(&sid, batch, ascending != 0, begin, end, lite)?);
        Ok(())
    })
}

/// # Safety
/// `session_id` is NUL-terminated; `out_cursor` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_open_message_cursor(handle: i64, session_id: *const c_char, batch_size: i32, ascending: i32, begin: i32, end: i32, out_cursor: *mut i64) -> i32 {
    open_cursor("wcdb_open_message_cursor", handle, session_id, batch_size, ascending, begin, end, false, out_cursor)
}

/// # Safety
/// As [`wcdb_open_message_cursor`]; rows leave out the heavy binary columns.
#[no_mangle]
pub unsafe extern "C" fn wcdb_open_message_cursor_lite(handle: i64, session_id: *const c_char, batch_size: i32, ascending: i32, begin: i32, end: i32, out_cursor: *mut i64) -> i32 {
    open_cursor("wcdb_open_message_cursor_lite", handle, session_id, batch_size, ascending, begin, end, true, out_cursor)
}

/// # Safety
/// `out_json` and `out_has_more` are null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_fetch_message_batch(handle: i64, cursor: i64, out_json: *mut *mut c_void, out_has_more: *mut i32) -> i32 {
    run("wcdb_fetch_message_batch", handle, out_json, |db| {
        let (rows, more) = db.fetch_message_batch(cursor)?;
        put(out_json, &rows.to_string());
        put_value(out_has_more, more as i32);
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn wcdb_close_message_cursor(handle: i64, cursor: i64) -> i32 {
    run("wcdb_close_message_cursor", handle, std::ptr::null_mut(), |db| db.close_message_cursor(cursor))
}

/// # Safety
/// `session_ids_json` is NUL-terminated; `out_json` and `out_has_more` are null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_scan_media_stream(handle: i64, session_ids_json: *const c_char, media_type: i32, begin: i32, end: i32, limit: i32, offset: i32, out_json: *mut *mut c_void, out_has_more: *mut i32) -> i32 {
    let sessions = text(session_ids_json);
    run("wcdb_scan_media_stream", handle, out_json, |db| {
        let (rows, more) = db.scan_media_stream(&sessions, media_type, begin, end, limit, offset)?;
        put(out_json, &rows.to_string());
        put_value(out_has_more, more as i32);
        Ok(())
    })
}

// ───────────────────────── refused: would modify WeChat's databases ─────────────────────────

/// # Safety
/// `out_error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_update_message(_handle: i64, _session_id: *const c_char, _local_id: i64, _create_time: i32, _content: *const c_char, out_error: *mut *mut c_void) -> i32 {
    read_only("update_message", out_error)
}

/// # Safety
/// `out_error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_delete_message(_handle: i64, _session_id: *const c_char, _local_id: i64, _create_time: i32, _db_path_hint: *const c_char, out_error: *mut *mut c_void) -> i32 {
    read_only("delete_message", out_error)
}

/// # Safety
/// `out_error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_delete_sns_post(_handle: i64, _post_id: *const c_char, out_error: *mut *mut c_void) -> i32 {
    read_only("sns_delete_post", out_error)
}

/// # Safety
/// `out_error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_mark_all_sessions_read(_handle: i64, out_error: *mut *mut c_void) -> i32 {
    read_only("mark_all_sessions_read", out_error)
}

/// # Safety
/// `out_error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_install_message_anti_revoke_trigger(_handle: i64, _session_id: *const c_char, out_error: *mut *mut c_void) -> i32 {
    read_only("anti_revoke_install", out_error)
}

/// # Safety
/// `out_error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_uninstall_message_anti_revoke_trigger(_handle: i64, _session_id: *const c_char, out_error: *mut *mut c_void) -> i32 {
    read_only("anti_revoke_uninstall", out_error)
}

/// No trigger can exist in a read-only layer: reports "not installed" and refuses.
///
/// # Safety
/// `out_installed` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_check_message_anti_revoke_trigger(_handle: i64, _session_id: *const c_char, out_installed: *mut i32) -> i32 {
    put_value(out_installed, 0);
    read_only("anti_revoke_check", std::ptr::null_mut())
}

/// # Safety
/// `out_error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_install_sns_block_delete_trigger(_handle: i64, out_error: *mut *mut c_void) -> i32 {
    read_only("sns_block_delete_install", out_error)
}

/// # Safety
/// `out_error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_uninstall_sns_block_delete_trigger(_handle: i64, out_error: *mut *mut c_void) -> i32 {
    read_only("sns_block_delete_uninstall", out_error)
}

/// # Safety
/// `out_installed` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_check_sns_block_delete_trigger(_handle: i64, out_installed: *mut i32) -> i32 {
    put_value(out_installed, 0);
    read_only("sns_block_delete_check", std::ptr::null_mut())
}

/// # Safety
/// `out_json` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_import_table_snapshot(_handle: i64, _kind: *const c_char, _db_path: *const c_char, _table: *const c_char, _input: *const c_char, out_json: *mut *mut c_void) -> i32 {
    put(out_json, &serde_json::json!({ "success": false, "error": "import_table_snapshot is not supported: the native database backend opens WeChat's databases read-only" }).to_string());
    read_only("import_table_snapshot", std::ptr::null_mut())
}

/// # Safety
/// `out_json` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_import_table_snapshot_with_schema(_handle: i64, _kind: *const c_char, _db_path: *const c_char, _table: *const c_char, _input: *const c_char, _sql: *const c_char, out_json: *mut *mut c_void) -> i32 {
    put(out_json, &serde_json::json!({ "success": false, "error": "import_table_snapshot_with_schema is not supported: the native database backend opens WeChat's databases read-only" }).to_string());
    read_only("import_table_snapshot_with_schema", std::ptr::null_mut())
}

// ───────────────────────── change monitor ─────────────────────────

mod monitor;

/// Starts the change monitor (see the `monitor` module) for every open account.
#[no_mangle]
pub extern "C" fn wcdb_start_monitor_pipe() -> i32 {
    let watch = || -> Vec<PathBuf> {
        let dirs: Vec<PathBuf> = accounts().lock().map(|a| a.values().map(|acct| acct.dir.clone()).collect()).unwrap_or_default();
        dirs.iter().flat_map(|d| monitor::databases(&d.join("db_storage"))).collect()
    };
    match catch_unwind(|| monitor::start(watch)) {
        Ok(Ok(_)) => STATUS_OK,
        Ok(Err(e)) => {
            log(format!("wcdb_start_monitor_pipe: {e}"));
            STATUS_FAILED
        }
        Err(_) => STATUS_PANIC,
    }
}

/// The pipe (Windows) or socket path to connect to.
///
/// # Safety
/// `out` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn wcdb_get_monitor_pipe_name(out: *mut *mut c_void) -> i32 {
    match monitor::name() {
        Some(n) => {
            put(out, &n);
            STATUS_OK
        }
        None => STATUS_FAILED,
    }
}

#[no_mangle]
pub extern "C" fn wcdb_stop_monitor_pipe() {
    monitor::stop();
}

#[cfg(test)]
mod tests;
