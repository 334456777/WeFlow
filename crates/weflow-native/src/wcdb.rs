//! Database facade used by the service layer.
//!
//! Everything is served by the pure-Rust backend in [`crate::native_db`]/[`crate::native_msg`]; there is
//! no dependency on the closed-source `wcdb_api` library. Calls that have not been ported yet return a
//! clear "not implemented" error instead of touching anything native.
// Pending stubs keep their historical signatures; drop this once they are all ported.
#![allow(unused_variables)]

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::native_db::NativeAccount;

/// One argument of a generic by-name call (after the implicit account).
#[derive(Clone, Copy, Debug)]
pub enum Arg<'a> {
    S(&'a str),
    I32(i32),
    I64(i64),
}

#[derive(Default)]
pub struct Wcdb {
    native: Option<NativeAccount>,
}

impl Wcdb {
    pub fn new() -> Self {
        Self::default()
    }

    fn pending<T>(&self, name: &str) -> Result<T> {
        if self.native.is_none() {
            return Err(anyhow!("WCDB is not connected"));
        }
        // by-name calls (install/uninstall triggers, deletes, ...) that would write to WeChat's databases
        if ["install", "uninstall", "delete", "update_message", "mark_all", "trigger"].iter().any(|w| name.contains(w)) {
            return self.read_only(name);
        }
        Err(anyhow!("{name} is not implemented in the native database backend yet"))
    }

    /// Operations that would modify WeChat's own database files (triggers, edits, deletes, read marks).
    fn read_only<T>(&self, name: &str) -> Result<T> {
        Err(anyhow!("{name} is not supported: the native database backend opens WeChat's databases read-only"))
    }

    fn account(&self) -> Result<&NativeAccount> {
        self.native.as_ref().ok_or_else(|| anyhow!("WCDB is not connected"))
    }

    /// Open the account: decrypt `session.db` with the key to prove the key and path are right.
    pub fn open(&mut self, account_dir: &Path, hex_key: &str, wxid: Option<&str>) -> Result<()> {
        let session_db = find_session_db(&account_dir.join("db_storage"))
            .ok_or_else(|| anyhow!("session.db not found under {}", account_dir.display()))?;
        // session.db lives in <db_storage>/session/, so its grandparent is db_storage.
        let db_storage = session_db.parent().and_then(Path::parent).unwrap_or(account_dir);
        // Without an explicit wxid, the account directory name (`<wxid>_<4 chars>`) identifies the owner.
        let me = wxid
            .map(str::to_string)
            .or_else(|| account_dir.file_name().map(|n| n.to_string_lossy().to_string()));
        let account = NativeAccount::new(db_storage, hex_key)?.with_my_wxid(me);
        account.test_connection()?;
        self.native = Some(account);
        Ok(())
    }

    pub fn test_connection(&mut self, account_dir: &Path, hex_key: &str) -> Result<()> {
        self.open(account_dir, hex_key, None)?;
        self.close();
        Ok(())
    }

    pub fn close(&mut self) {
        self.native = None;
    }

    // ── sessions and messages ──

    pub fn sessions(&self) -> Result<Value> {
        self.account()?.sessions()
    }

    pub fn messages(&self, session_id: &str, limit: i32, offset: i32) -> Result<Value> {
        self.account()?.messages(session_id, limit, offset)
    }

    pub fn message_count(&self, session_id: &str) -> Result<i32> {
        self.account()?.message_count(session_id)
    }

    pub fn message_dates(&self, session_id: &str) -> Result<Value> {
        self.account()?.message_dates(session_id)
    }

    pub fn session_message_counts(&self, session_ids: &[String]) -> Result<Value> {
        self.account()?.session_message_counts(session_ids)
    }

    pub fn session_message_date_counts(&self, session_id: &str) -> Result<Value> {
        self.account()?.session_message_date_counts(session_id)
    }

    pub fn messages_by_type(&self, session_id: &str, local_type: i64, ascending: bool, limit: i32, offset: i32) -> Result<Value> {
        self.account()?.messages_by_type(session_id, local_type, ascending, limit, offset)
    }

    pub fn message_by_id(&self, session_id: &str, local_id: i32) -> Result<Value> {
        self.account()?.message_by_id(session_id, local_id as i64)
    }

    pub fn message_by_server_id(&self, session_id: &str, svrid: &str) -> Result<Value> {
        self.account()?.message_by_server_id(session_id, svrid)
    }

    pub fn open_message_cursor(&self, session_id: &str, batch_size: i32, ascending: bool, begin: i32, end: i32, lite: bool) -> Result<i64> {
        self.account()?.open_message_cursor(session_id, batch_size, ascending, begin, end, lite)
    }

    /// Returns the next batch of rows and whether more remain.
    pub fn fetch_message_batch(&self, cursor: i64) -> Result<(Value, bool)> {
        self.account()?.fetch_message_batch(cursor)
    }

    pub fn close_message_cursor(&self, cursor: i64) -> Result<()> {
        self.account()?.close_message_cursor(cursor)
    }

    // ── generic by-name calls (kept for callers that build their own argument lists) ──

    /// Call a database function by its historical `wcdb_*` name and get JSON back.
    pub fn invoke_json(&self, name: &str, args: &[Arg<'_>]) -> Result<Value> {
        match (name, args) {
            ("wcdb_get_contacts_compact", [Arg::S(payload)]) => self.account()?.contacts_compact(&crate::native_contact::usernames_from_json(payload)),
            ("wcdb_get_aggregate_stats", [Arg::S(ids), Arg::I32(b), Arg::I32(e)]) => {
                self.account()?.aggregate_stats(&crate::native_contact::usernames_from_json(ids), *b as i64, *e as i64)
            }
            ("wcdb_get_message_table_time_range", [Arg::S(db), Arg::S(table)]) => self.account()?.message_table_time_range(db, table),
            _ => self.pending(name),
        }
    }

    /// Like [`invoke_json`](Self::invoke_json) for functions whose output is a plain string.
    pub fn invoke_string(&self, name: &str, _args: &[Arg<'_>]) -> Result<String> {
        self.pending(name)
    }

    /// Like [`invoke_json`](Self::invoke_json) for functions that report success by status.
    pub fn invoke_status(&self, name: &str, _args: &[Arg<'_>]) -> Result<Value> {
        self.pending(name)
    }

    /// Raw status code and message (some triggers report "already installed" as a positive code).
    pub fn invoke_status_code(&self, name: &str, _args: &[Arg<'_>]) -> Result<(i32, String)> {
        self.pending(name)
    }

    // ── not ported yet ──

    pub fn search(&self, keyword: &str, session_id: Option<&str>, limit: i32, offset: i32, begin: i32, end: i32) -> Result<Value> {
        self.account()?.search_messages(keyword, session_id, limit, offset, begin as i64, end as i64)
    }

    pub fn contact(&self, username: &str) -> Result<Value> {
        self.account()?.contact(username)
    }

    pub fn contacts(&self) -> Result<Value> {
        self.account()?.contacts()
    }

    pub fn contact_type_counts(&self) -> Result<Value> {
        self.account()?.contact_type_counts()
    }

    pub fn group_member_count(&self, chatroom_id: &str) -> Result<Value> {
        self.account()?.group_member_count(chatroom_id)
    }

    pub fn group_members(&self, chatroom_id: &str) -> Result<Value> {
        self.account()?.group_members(chatroom_id)
    }

    pub fn group_nicknames(&self, chatroom_id: &str) -> Result<Value> {
        self.account()?.group_nicknames(chatroom_id)
    }

    pub fn group_stats(&self, chatroom_id: &str, begin: i32, end: i32) -> Result<Value> {
        self.account()?.group_stats(chatroom_id, begin as i64, end as i64)
    }

    pub fn aggregate_stats(&self, session_ids: &[String], begin: i32, end: i32) -> Result<Value> {
        self.account()?.aggregate_stats(session_ids, begin as i64, end as i64)
    }

    pub fn available_years(&self, session_ids: &[String]) -> Result<Value> {
        self.account()?.available_years(session_ids)
    }

    pub fn annual_report_stats(&self, session_ids: &[String], begin: i32, end: i32) -> Result<Value> {
        self.pending("annual_report_stats")
    }

    pub fn dual_report_stats(&self, session_id: &str, begin: i32, end: i32) -> Result<Value> {
        self.account()?.dual_report_stats(session_id, begin as i64, end as i64)
    }

    pub fn footprint_stats(&self, options: &Value) -> Result<Value> {
        self.account()?.footprint_stats(options)
    }

    pub fn session_message_type_stats(&self, session_id: &str, begin: i32, end: i32) -> Result<Value> {
        self.account()?.session_message_type_stats_batch(&[session_id.to_string()], &serde_json::json!({ "begin": begin, "end": end })).map(|v| v[session_id].clone())
    }

    pub fn sns_timeline(&self, limit: i32, offset: i32, username: Option<&str>, keyword: Option<&str>, start: i32, end: i32) -> Result<Value> {
        self.account()?.sns_timeline(limit, offset, &username.map(crate::native_contact::usernames_from_json).unwrap_or_default(), keyword, start as i64, end as i64)
    }

    pub fn sns_annual_stats(&self, begin: i32, end: i32) -> Result<Value> {
        self.account()?.sns_annual_stats(begin as i64, end as i64)
    }

    pub fn sns_usernames(&self) -> Result<Value> {
        self.account()?.sns_usernames()
    }

    pub fn sns_export_stats(&self, my_wxid: Option<&str>) -> Result<Value> {
        self.account()?.sns_export_stats(my_wxid)
    }

    pub fn sns_block_delete_check(&self) -> Result<Value> {
        self.read_only("sns_block_delete_check")
    }

    pub fn sns_block_delete_install(&self) -> Result<Value> {
        self.read_only("sns_block_delete_install")
    }

    pub fn sns_block_delete_uninstall(&self) -> Result<Value> {
        self.read_only("sns_block_delete_uninstall")
    }

    pub fn sns_delete_post(&self, post_id: &str) -> Result<Value> {
        self.read_only("sns_delete_post")
    }

    pub fn exec_query(&self, kind: &str, path: &str, sql: &str) -> Result<Value> {
        self.account()?.exec_query(kind, Some(path), sql)
    }

    pub fn update_message(&self, session_id: &str, local_id: i64, create_time: i32, content: &str) -> Result<Value> {
        self.read_only("update_message")
    }

    pub fn delete_message(&self, session_id: &str, local_id: i64, create_time: i32, db_path_hint: Option<&str>) -> Result<Value> {
        self.read_only("delete_message")
    }

    pub fn anti_revoke_check(&self, session_id: &str) -> Result<Value> {
        self.read_only("anti_revoke_check")
    }

    pub fn anti_revoke_install(&self, session_id: &str) -> Result<Value> {
        self.read_only("anti_revoke_install")
    }

    pub fn anti_revoke_uninstall(&self, session_id: &str) -> Result<Value> {
        self.read_only("anti_revoke_uninstall")
    }

    pub fn scan_media_stream(&self, session_ids_json: &str, media_type: i32, begin: i32, end: i32, limit: i32, offset: i32) -> Result<(Value, bool)> {
        self.account()?.scan_media_stream(&crate::native_contact::usernames_from_json(session_ids_json), media_type, begin as i64, end as i64, limit, offset)
    }

    pub fn mark_all_sessions_read(&self) -> Result<Value> {
        self.read_only("mark_all_sessions_read")
    }

    pub fn display_names(&self, usernames_json: &str) -> Result<Value> {
        self.account()?.display_names(&crate::native_contact::usernames_from_json(usernames_json))
    }

    pub fn avatar_urls(&self, usernames_json: &str) -> Result<Value> {
        self.account()?.avatar_urls(&crate::native_contact::usernames_from_json(usernames_json))
    }

    pub fn group_member_counts(&self, chatroom_ids_json: &str) -> Result<Value> {
        self.account()?.group_member_counts(&crate::native_contact::usernames_from_json(chatroom_ids_json))
    }

    pub fn message_tables(&self, session_id: &str) -> Result<Value> {
        self.account()?.message_tables_json(session_id)
    }

    pub fn message_meta(&self, db_path: &str, table: &str, limit: i32, offset: i32) -> Result<Value> {
        self.pending("message_meta")
    }

    pub fn contact_status(&self, usernames_json: &str) -> Result<Value> {
        self.account()?.contact_status(&crate::native_contact::usernames_from_json(usernames_json))
    }

    pub fn contact_alias_map(&self, usernames_json: &str) -> Result<Value> {
        self.account()?.contact_alias_map(&crate::native_contact::usernames_from_json(usernames_json))
    }

    pub fn contact_friend_flags(&self, usernames_json: &str) -> Result<Value> {
        self.account()?.contact_friend_flags(&crate::native_contact::usernames_from_json(usernames_json))
    }

    pub fn chat_room_ext_buffer(&self, chatroom_id: &str) -> Result<Value> {
        self.account()?.chat_room_ext_buffer(chatroom_id)
    }

    pub fn message_table_stats(&self, session_id: &str) -> Result<Value> {
        self.account()?.message_table_stats(session_id)
    }

    pub fn annual_report_extras(&self, session_ids_json: &str, begin: i32, end: i32, peak_begin: i32, peak_end: i32) -> Result<Value> {
        self.pending("annual_report_extras")
    }

    pub fn emoticon_cdn_url(&self, db_path: &str, md5: &str) -> Result<String> {
        self.account()?.emoticon_cdn_url(md5)
    }

    pub fn emoticon_caption(&self, db_path: &str, md5: &str) -> Result<String> {
        self.account()?.emoticon_caption(md5)
    }

    pub fn emoticon_caption_strict(&self, md5: &str) -> Result<String> {
        self.account()?.emoticon_caption(md5)
    }

    pub fn list_message_dbs(&self) -> Result<Value> {
        Ok(self.account()?.list_message_dbs())
    }

    pub fn list_media_dbs(&self) -> Result<Value> {
        Ok(self.account()?.list_media_dbs())
    }

    pub fn db_status(&self) -> Result<Value> {
        self.pending("db_status")
    }

    pub fn voice_data(&self, session_id: &str, create_time: i32, local_id: i32, svr_id: i64, candidates_json: &str) -> Result<String> {
        self.account()?.voice_data(create_time as i64, local_id as i64, svr_id, &crate::native_contact::usernames_from_json(candidates_json))
    }

    pub fn voice_data_batch(&self, requests_json: &str) -> Result<Value> {
        self.account()?.voice_data_batch(&serde_json::from_str(requests_json).unwrap_or_default())
    }

    pub fn media_schema_summary(&self, db_path: &str) -> Result<Value> {
        self.pending("media_schema_summary")
    }

    pub fn session_message_type_stats_batch(&self, session_ids_json: &str, options_json: &str) -> Result<Value> {
        self.account()?.session_message_type_stats_batch(&crate::native_contact::usernames_from_json(session_ids_json), &serde_json::from_str(options_json).unwrap_or_default())
    }

    pub fn session_message_date_counts_batch(&self, session_ids_json: &str) -> Result<Value> {
        self.account()?.session_message_date_counts_batch(&crate::native_contact::usernames_from_json(session_ids_json))
    }

    pub fn head_image_buffers(&self, usernames_json: &str) -> Result<Value> {
        self.pending("head_image_buffers")
    }

    pub fn message_table_columns(&self, db_path: &str, table: &str) -> Result<Value> {
        self.pending("message_table_columns")
    }

    pub fn list_tables(&self, kind: &str, db_path: &str) -> Result<Value> {
        self.pending("list_tables")
    }

    pub fn table_schema(&self, kind: &str, db_path: &str, table: &str) -> Result<Value> {
        self.pending("table_schema")
    }

    pub fn export_table_snapshot(&self, kind: &str, db_path: &str, table: &str, output_path: &str) -> Result<Value> {
        self.pending("export_table_snapshot")
    }

    pub fn import_table_snapshot(&self, kind: &str, db_path: &str, table: &str, input_path: &str) -> Result<Value> {
        self.pending("import_table_snapshot")
    }

    pub fn import_table_snapshot_with_schema(&self, kind: &str, db_path: &str, table: &str, input_path: &str, create_table_sql: &str) -> Result<Value> {
        self.pending("import_table_snapshot_with_schema")
    }

    pub fn message_table_time_range(&self, db_path: &str, table: &str) -> Result<Value> {
        self.account()?.message_table_time_range(db_path, table)
    }

    pub fn resolve_image_hardlink(&self, md5: &str, account_dir: &str) -> Result<Value> {
        self.pending("resolve_image_hardlink")
    }

    pub fn resolve_image_hardlink_batch(&self, requests_json: &str) -> Result<Value> {
        self.pending("resolve_image_hardlink_batch")
    }

    pub fn resolve_video_hardlink_md5(&self, md5: &str, db_path: &str) -> Result<Value> {
        self.pending("resolve_video_hardlink_md5")
    }

    pub fn resolve_video_hardlink_md5_batch(&self, requests_json: &str) -> Result<Value> {
        self.pending("resolve_video_hardlink_md5_batch")
    }
}

fn find_session_db(root: &Path) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }
    let direct = root.join("session/session.db");
    if direct.exists() {
        return Some(direct);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case("session.db"))
                .unwrap_or(false)
            {
                return Some(path);
            }
        }
    }
    None
}
