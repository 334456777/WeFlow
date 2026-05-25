use std::ffi::{CStr, CString};
use std::fs;
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use libloading::Library;
use serde_json::Value;

type InitProtectionFn = unsafe extern "C" fn(*const c_char) -> c_int;
type InitFn = unsafe extern "C" fn() -> c_int;
type ShutdownFn = unsafe extern "C" fn() -> c_int;
type OpenAccountFn = unsafe extern "C" fn(*const c_char, *const c_char, *mut i64) -> c_int;
type CloseAccountFn = unsafe extern "C" fn(i64) -> c_int;
type FreeStringFn = unsafe extern "C" fn(*mut c_void);
type SetMyWxidFn = unsafe extern "C" fn(i64, *const c_char) -> c_int;
type OutJson0Fn = unsafe extern "C" fn(i64, *mut *mut c_void) -> c_int;
type GetMessagesFn =
    unsafe extern "C" fn(i64, *const c_char, c_int, c_int, *mut *mut c_void) -> c_int;
type GetContactFn = unsafe extern "C" fn(i64, *const c_char, *mut *mut c_void) -> c_int;
type GetContactsCompactFn = unsafe extern "C" fn(i64, *const c_char, *mut *mut c_void) -> c_int;
type OutJson1StringFn = unsafe extern "C" fn(i64, *const c_char, *mut *mut c_void) -> c_int;
type OutJsonStringRangeFn =
    unsafe extern "C" fn(i64, *const c_char, c_int, c_int, *mut *mut c_void) -> c_int;
type OutJsonRangeFn = unsafe extern "C" fn(i64, c_int, c_int, *mut *mut c_void) -> c_int;
type OutJsonNoArgTriggerFn = unsafe extern "C" fn(i64, *mut *mut c_void) -> c_int;
type CheckNoArgTriggerFn = unsafe extern "C" fn(i64, *mut c_int) -> c_int;
type GroupMemberCountFn = unsafe extern "C" fn(i64, *const c_char, *mut c_int) -> c_int;
type SearchMessagesFn = unsafe extern "C" fn(
    i64,
    *const c_char,
    *const c_char,
    c_int,
    c_int,
    c_int,
    c_int,
    *mut *mut c_void,
) -> c_int;
type ExecQueryFn = unsafe extern "C" fn(
    i64,
    *const c_char,
    *const c_char,
    *const c_char,
    *mut *mut c_void,
) -> c_int;
type UpdateMessageFn =
    unsafe extern "C" fn(i64, *const c_char, i64, c_int, *const c_char, *mut *mut c_void) -> c_int;
type DeleteMessageFn =
    unsafe extern "C" fn(i64, *const c_char, i64, c_int, *const c_char, *mut *mut c_void) -> c_int;
type TriggerFn = unsafe extern "C" fn(i64, *const c_char, *mut *mut c_void) -> c_int;
type CheckTriggerFn = unsafe extern "C" fn(i64, *const c_char, *mut c_int) -> c_int;
type SnsTimelineFn = unsafe extern "C" fn(
    i64,
    c_int,
    c_int,
    *const c_char,
    *const c_char,
    c_int,
    c_int,
    *mut *mut c_void,
) -> c_int;

pub struct Wcdb {
    runtime_dir: PathBuf,
    _deps: Vec<Library>,
    _lib: Library,
    init: InitFn,
    shutdown: ShutdownFn,
    open_account: OpenAccountFn,
    close_account: CloseAccountFn,
    free_string: FreeStringFn,
    set_my_wxid: Option<SetMyWxidFn>,
    get_sessions: OutJson0Fn,
    get_messages: GetMessagesFn,
    get_contact: Option<GetContactFn>,
    get_contacts_compact: Option<GetContactsCompactFn>,
    get_contact_type_counts: Option<OutJson0Fn>,
    get_group_member_count: Option<GroupMemberCountFn>,
    get_group_members: Option<OutJson1StringFn>,
    get_group_nicknames: Option<OutJson1StringFn>,
    get_group_stats: Option<OutJsonStringRangeFn>,
    get_aggregate_stats: Option<OutJsonStringRangeFn>,
    get_available_years: Option<OutJson1StringFn>,
    get_annual_report_stats: Option<OutJsonStringRangeFn>,
    get_dual_report_stats: Option<OutJsonStringRangeFn>,
    get_my_footprint_stats: Option<OutJson1StringFn>,
    get_message_dates: Option<OutJson1StringFn>,
    get_session_message_counts: Option<OutJson1StringFn>,
    get_session_message_type_stats: Option<OutJsonStringRangeFn>,
    get_session_message_date_counts: Option<OutJson1StringFn>,
    get_sns_timeline: Option<SnsTimelineFn>,
    get_sns_annual_stats: Option<OutJsonRangeFn>,
    get_sns_usernames: Option<OutJson0Fn>,
    get_sns_export_stats: Option<OutJson1StringFn>,
    search_messages: Option<SearchMessagesFn>,
    exec_query: Option<ExecQueryFn>,
    update_message: Option<UpdateMessageFn>,
    delete_message: Option<DeleteMessageFn>,
    install_anti_revoke: Option<TriggerFn>,
    uninstall_anti_revoke: Option<TriggerFn>,
    check_anti_revoke: Option<CheckTriggerFn>,
    install_sns_block_delete: Option<OutJsonNoArgTriggerFn>,
    uninstall_sns_block_delete: Option<OutJsonNoArgTriggerFn>,
    check_sns_block_delete: Option<CheckNoArgTriggerFn>,
    delete_sns_post: Option<OutJson1StringFn>,
    handle: Option<i64>,
    initialized: bool,
}

impl Wcdb {
    /// # Safety
    ///
    /// `runtime_dir` must point to the versioned runtime directory prepared by
    /// `weflow-assets`. The dynamic libraries in that directory are loaded and
    /// their C ABI is trusted to match the symbols declared in this module.
    pub unsafe fn load(runtime_dir: impl AsRef<Path>) -> Result<Self> {
        let runtime_dir = runtime_dir.as_ref().to_path_buf();
        let lib_path = wcdb_api_path(&runtime_dir)?;
        let mut deps = Vec::new();
        for dep in wcdb_dependency_paths(&runtime_dir) {
            if dep.exists() {
                deps.push(Library::new(&dep).with_context(|| {
                    format!("failed to preload WCDB dependency {}", dep.display())
                })?);
            }
        }

        let lib = Library::new(&lib_path)
            .with_context(|| format!("failed to load {}", lib_path.display()))?;

        if let Ok(init_protection) = symbol::<InitProtectionFn>(&lib, b"InitProtection\0") {
            let path = cstring(runtime_dir.to_string_lossy())?;
            let rc = init_protection(path.as_ptr());
            if rc != 0 {
                return Err(anyhow!("InitProtection failed with code {rc}"));
            }
        }

        let init = symbol::<InitFn>(&lib, b"wcdb_init\0")?;
        let shutdown = symbol::<ShutdownFn>(&lib, b"wcdb_shutdown\0")?;
        let open_account = symbol::<OpenAccountFn>(&lib, b"wcdb_open_account\0")?;
        let close_account = symbol::<CloseAccountFn>(&lib, b"wcdb_close_account\0")?;
        let free_string = symbol::<FreeStringFn>(&lib, b"wcdb_free_string\0")?;
        let get_sessions = symbol::<OutJson0Fn>(&lib, b"wcdb_get_sessions\0")?;
        let get_messages = symbol::<GetMessagesFn>(&lib, b"wcdb_get_messages\0")?;

        let set_my_wxid = optional_symbol::<SetMyWxidFn>(&lib, b"wcdb_set_my_wxid\0");
        let get_contact = optional_symbol::<GetContactFn>(&lib, b"wcdb_get_contact\0");
        let get_contacts_compact =
            optional_symbol::<GetContactsCompactFn>(&lib, b"wcdb_get_contacts_compact\0");
        let get_contact_type_counts =
            optional_symbol::<OutJson0Fn>(&lib, b"wcdb_get_contact_type_counts\0");
        let get_group_member_count =
            optional_symbol::<GroupMemberCountFn>(&lib, b"wcdb_get_group_member_count\0");
        let get_group_members =
            optional_symbol::<OutJson1StringFn>(&lib, b"wcdb_get_group_members\0");
        let get_group_nicknames =
            optional_symbol::<OutJson1StringFn>(&lib, b"wcdb_get_group_nicknames\0");
        let get_group_stats =
            optional_symbol::<OutJsonStringRangeFn>(&lib, b"wcdb_get_group_stats\0");
        let get_aggregate_stats =
            optional_symbol::<OutJsonStringRangeFn>(&lib, b"wcdb_get_aggregate_stats\0");
        let get_available_years =
            optional_symbol::<OutJson1StringFn>(&lib, b"wcdb_get_available_years\0");
        let get_annual_report_stats =
            optional_symbol::<OutJsonStringRangeFn>(&lib, b"wcdb_get_annual_report_stats\0");
        let get_dual_report_stats =
            optional_symbol::<OutJsonStringRangeFn>(&lib, b"wcdb_get_dual_report_stats\0");
        let get_my_footprint_stats =
            optional_symbol::<OutJson1StringFn>(&lib, b"wcdb_get_my_footprint_stats\0");
        let get_message_dates =
            optional_symbol::<OutJson1StringFn>(&lib, b"wcdb_get_message_dates\0");
        let get_session_message_counts =
            optional_symbol::<OutJson1StringFn>(&lib, b"wcdb_get_session_message_counts\0");
        let get_session_message_type_stats =
            optional_symbol::<OutJsonStringRangeFn>(&lib, b"wcdb_get_session_message_type_stats\0");
        let get_session_message_date_counts =
            optional_symbol::<OutJson1StringFn>(&lib, b"wcdb_get_session_message_date_counts\0");
        let get_sns_timeline = optional_symbol::<SnsTimelineFn>(&lib, b"wcdb_get_sns_timeline\0");
        let get_sns_annual_stats =
            optional_symbol::<OutJsonRangeFn>(&lib, b"wcdb_get_sns_annual_stats\0");
        let get_sns_usernames = optional_symbol::<OutJson0Fn>(&lib, b"wcdb_get_sns_usernames\0");
        let get_sns_export_stats =
            optional_symbol::<OutJson1StringFn>(&lib, b"wcdb_get_sns_export_stats\0");
        let search_messages = optional_symbol::<SearchMessagesFn>(&lib, b"wcdb_search_messages\0");
        let exec_query = optional_symbol::<ExecQueryFn>(&lib, b"wcdb_exec_query\0");
        let update_message = optional_symbol::<UpdateMessageFn>(&lib, b"wcdb_update_message\0");
        let delete_message = optional_symbol::<DeleteMessageFn>(&lib, b"wcdb_delete_message\0");
        let install_anti_revoke =
            optional_symbol::<TriggerFn>(&lib, b"wcdb_install_message_anti_revoke_trigger\0");
        let uninstall_anti_revoke =
            optional_symbol::<TriggerFn>(&lib, b"wcdb_uninstall_message_anti_revoke_trigger\0");
        let check_anti_revoke =
            optional_symbol::<CheckTriggerFn>(&lib, b"wcdb_check_message_anti_revoke_trigger\0");
        let install_sns_block_delete = optional_symbol::<OutJsonNoArgTriggerFn>(
            &lib,
            b"wcdb_install_sns_block_delete_trigger\0",
        );
        let uninstall_sns_block_delete = optional_symbol::<OutJsonNoArgTriggerFn>(
            &lib,
            b"wcdb_uninstall_sns_block_delete_trigger\0",
        );
        let check_sns_block_delete =
            optional_symbol::<CheckNoArgTriggerFn>(&lib, b"wcdb_check_sns_block_delete_trigger\0");
        let delete_sns_post = optional_symbol::<OutJson1StringFn>(&lib, b"wcdb_delete_sns_post\0");

        Ok(Self {
            runtime_dir,
            _deps: deps,
            _lib: lib,
            init,
            shutdown,
            open_account,
            close_account,
            free_string,
            set_my_wxid,
            get_sessions,
            get_messages,
            get_contact,
            get_contacts_compact,
            get_contact_type_counts,
            get_group_member_count,
            get_group_members,
            get_group_nicknames,
            get_group_stats,
            get_aggregate_stats,
            get_available_years,
            get_annual_report_stats,
            get_dual_report_stats,
            get_my_footprint_stats,
            get_message_dates,
            get_session_message_counts,
            get_session_message_type_stats,
            get_session_message_date_counts,
            get_sns_timeline,
            get_sns_annual_stats,
            get_sns_usernames,
            get_sns_export_stats,
            search_messages,
            exec_query,
            update_message,
            delete_message,
            install_anti_revoke,
            uninstall_anti_revoke,
            check_anti_revoke,
            install_sns_block_delete,
            uninstall_sns_block_delete,
            check_sns_block_delete,
            delete_sns_post,
            handle: None,
            initialized: false,
        })
    }

    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    pub fn init(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }
        let rc = unsafe { (self.init)() };
        if rc != 0 {
            return Err(anyhow!("wcdb_init failed with code {rc}"));
        }
        self.initialized = true;
        Ok(())
    }

    pub fn open(&mut self, account_dir: &Path, hex_key: &str, wxid: Option<&str>) -> Result<()> {
        self.init()?;
        let session_db = find_session_db(&account_dir.join("db_storage"))
            .ok_or_else(|| anyhow!("session.db not found under {}", account_dir.display()))?;
        let session_db = cstring(session_db.to_string_lossy())?;
        let key = cstring(hex_key)?;
        let mut handle = 0_i64;
        let rc = unsafe { (self.open_account)(session_db.as_ptr(), key.as_ptr(), &mut handle) };
        if rc != 0 || handle <= 0 {
            return Err(anyhow!("wcdb_open_account failed with code {rc}"));
        }
        self.handle = Some(handle);
        if let (Some(set_my_wxid), Some(wxid)) = (self.set_my_wxid, wxid) {
            let wxid = cstring(wxid)?;
            let _ = unsafe { set_my_wxid(handle, wxid.as_ptr()) };
        }
        Ok(())
    }

    pub fn test_connection(&mut self, account_dir: &Path, hex_key: &str) -> Result<()> {
        self.open(account_dir, hex_key, None)?;
        self.close();
        Ok(())
    }

    pub fn close(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = unsafe { (self.close_account)(handle) };
        }
        if self.initialized {
            let _ = unsafe { (self.shutdown)() };
            self.initialized = false;
        }
    }

    pub fn sessions(&self) -> Result<Value> {
        let handle = self.require_handle()?;
        self.call_json(|out| unsafe { (self.get_sessions)(handle, out) })
    }

    pub fn messages(&self, session_id: &str, limit: i32, offset: i32) -> Result<Value> {
        let handle = self.require_handle()?;
        let session_id = cstring(session_id)?;
        self.call_json(|out| unsafe {
            (self.get_messages)(handle, session_id.as_ptr(), limit, offset, out)
        })
    }

    pub fn search(
        &self,
        keyword: &str,
        session_id: Option<&str>,
        limit: i32,
        offset: i32,
        begin: i32,
        end: i32,
    ) -> Result<Value> {
        let handle = self.require_handle()?;
        let search = self
            .search_messages
            .ok_or_else(|| anyhow!("wcdb_search_messages is not available"))?;
        let keyword = cstring(keyword)?;
        let session_id = cstring(session_id.unwrap_or_default())?;
        self.call_json(|out| unsafe {
            search(
                handle,
                session_id.as_ptr(),
                keyword.as_ptr(),
                limit,
                offset,
                begin,
                end,
                out,
            )
        })
    }

    pub fn contact(&self, username: &str) -> Result<Value> {
        let handle = self.require_handle()?;
        let username_c = cstring(username)?;
        if let Some(get_contact) = self.get_contact {
            return self.call_json(|out| unsafe { get_contact(handle, username_c.as_ptr(), out) });
        }
        self.exec_query(
            "contact",
            "",
            &format!(
                "SELECT * FROM contact WHERE username='{}' LIMIT 1",
                username.replace('\'', "''")
            ),
        )
    }

    pub fn contacts(&self) -> Result<Value> {
        let handle = self.require_handle()?;
        if let Some(get_contacts_compact) = self.get_contacts_compact {
            return self
                .call_json(|out| unsafe { get_contacts_compact(handle, std::ptr::null(), out) });
        }
        self.exec_query("contact", "", "SELECT * FROM contact")
    }

    pub fn contact_type_counts(&self) -> Result<Value> {
        let handle = self.require_handle()?;
        let func = self
            .get_contact_type_counts
            .ok_or_else(|| anyhow!("wcdb_get_contact_type_counts is not available"))?;
        self.call_json(|out| unsafe { func(handle, out) })
    }

    pub fn group_member_count(&self, chatroom_id: &str) -> Result<Value> {
        let handle = self.require_handle()?;
        let func = self
            .get_group_member_count
            .ok_or_else(|| anyhow!("wcdb_get_group_member_count is not available"))?;
        let chatroom_id = cstring(chatroom_id)?;
        let mut count = 0;
        let rc = unsafe { func(handle, chatroom_id.as_ptr(), &mut count) };
        if rc != 0 {
            return Err(anyhow!("group member count failed with code {rc}"));
        }
        Ok(serde_json::json!({ "count": count }))
    }

    pub fn group_members(&self, chatroom_id: &str) -> Result<Value> {
        self.call_string_json(
            self.get_group_members,
            "wcdb_get_group_members",
            chatroom_id,
        )
    }

    pub fn group_nicknames(&self, chatroom_id: &str) -> Result<Value> {
        self.call_string_json(
            self.get_group_nicknames,
            "wcdb_get_group_nicknames",
            chatroom_id,
        )
    }

    pub fn group_stats(&self, chatroom_id: &str, begin: i32, end: i32) -> Result<Value> {
        self.call_string_range_json(
            self.get_group_stats,
            "wcdb_get_group_stats",
            chatroom_id,
            begin,
            end,
        )
    }

    pub fn aggregate_stats(&self, session_ids: &[String], begin: i32, end: i32) -> Result<Value> {
        let session_ids = serde_json::to_string(session_ids)?;
        self.call_string_range_json(
            self.get_aggregate_stats,
            "wcdb_get_aggregate_stats",
            &session_ids,
            begin,
            end,
        )
    }

    pub fn available_years(&self, session_ids: &[String]) -> Result<Value> {
        let session_ids = serde_json::to_string(session_ids)?;
        self.call_string_json(
            self.get_available_years,
            "wcdb_get_available_years",
            &session_ids,
        )
    }

    pub fn annual_report_stats(
        &self,
        session_ids: &[String],
        begin: i32,
        end: i32,
    ) -> Result<Value> {
        let session_ids = serde_json::to_string(session_ids)?;
        self.call_string_range_json(
            self.get_annual_report_stats,
            "wcdb_get_annual_report_stats",
            &session_ids,
            begin,
            end,
        )
    }

    pub fn dual_report_stats(&self, session_id: &str, begin: i32, end: i32) -> Result<Value> {
        self.call_string_range_json(
            self.get_dual_report_stats,
            "wcdb_get_dual_report_stats",
            session_id,
            begin,
            end,
        )
    }

    pub fn footprint_stats(&self, options: &Value) -> Result<Value> {
        let options = serde_json::to_string(options)?;
        self.call_string_json(
            self.get_my_footprint_stats,
            "wcdb_get_my_footprint_stats",
            &options,
        )
    }

    pub fn message_dates(&self, session_id: &str) -> Result<Value> {
        self.call_string_json(self.get_message_dates, "wcdb_get_message_dates", session_id)
    }

    pub fn session_message_counts(&self, session_ids: &[String]) -> Result<Value> {
        let session_ids = serde_json::to_string(session_ids)?;
        self.call_string_json(
            self.get_session_message_counts,
            "wcdb_get_session_message_counts",
            &session_ids,
        )
    }

    pub fn session_message_type_stats(
        &self,
        session_id: &str,
        begin: i32,
        end: i32,
    ) -> Result<Value> {
        self.call_string_range_json(
            self.get_session_message_type_stats,
            "wcdb_get_session_message_type_stats",
            session_id,
            begin,
            end,
        )
    }

    pub fn session_message_date_counts(&self, session_id: &str) -> Result<Value> {
        self.call_string_json(
            self.get_session_message_date_counts,
            "wcdb_get_session_message_date_counts",
            session_id,
        )
    }

    pub fn sns_timeline(
        &self,
        limit: i32,
        offset: i32,
        username: Option<&str>,
        keyword: Option<&str>,
        start: i32,
        end: i32,
    ) -> Result<Value> {
        let handle = self.require_handle()?;
        let func = self
            .get_sns_timeline
            .ok_or_else(|| anyhow!("wcdb_get_sns_timeline is not available"))?;
        let username = cstring(username.unwrap_or_default())?;
        let keyword = cstring(keyword.unwrap_or_default())?;
        self.call_json(|out| unsafe {
            func(
                handle,
                limit,
                offset,
                username.as_ptr(),
                keyword.as_ptr(),
                start,
                end,
                out,
            )
        })
    }

    pub fn sns_annual_stats(&self, begin: i32, end: i32) -> Result<Value> {
        let handle = self.require_handle()?;
        let func = self
            .get_sns_annual_stats
            .ok_or_else(|| anyhow!("wcdb_get_sns_annual_stats is not available"))?;
        self.call_json(|out| unsafe { func(handle, begin, end, out) })
    }

    pub fn sns_usernames(&self) -> Result<Value> {
        let handle = self.require_handle()?;
        let func = self
            .get_sns_usernames
            .ok_or_else(|| anyhow!("wcdb_get_sns_usernames is not available"))?;
        self.call_json(|out| unsafe { func(handle, out) })
    }

    pub fn sns_export_stats(&self, my_wxid: Option<&str>) -> Result<Value> {
        self.call_string_json(
            self.get_sns_export_stats,
            "wcdb_get_sns_export_stats",
            my_wxid.unwrap_or_default(),
        )
    }

    pub fn sns_block_delete_check(&self) -> Result<Value> {
        let handle = self.require_handle()?;
        let check = self
            .check_sns_block_delete
            .ok_or_else(|| anyhow!("wcdb_check_sns_block_delete_trigger is not available"))?;
        let mut installed = 0;
        let rc = unsafe { check(handle, &mut installed) };
        if rc != 0 {
            return Err(anyhow!("sns block-delete check failed with code {rc}"));
        }
        Ok(serde_json::json!({ "installed": installed != 0 }))
    }

    pub fn sns_block_delete_install(&self) -> Result<Value> {
        let func = self
            .install_sns_block_delete
            .ok_or_else(|| anyhow!("wcdb_install_sns_block_delete_trigger is not available"))?;
        self.trigger_no_arg(func)
    }

    pub fn sns_block_delete_uninstall(&self) -> Result<Value> {
        let func = self
            .uninstall_sns_block_delete
            .ok_or_else(|| anyhow!("wcdb_uninstall_sns_block_delete_trigger is not available"))?;
        self.trigger_no_arg(func)
    }

    pub fn sns_delete_post(&self, post_id: &str) -> Result<Value> {
        self.call_string_json_or_string(self.delete_sns_post, "wcdb_delete_sns_post", post_id)
    }

    pub fn exec_query(&self, kind: &str, path: &str, sql: &str) -> Result<Value> {
        let handle = self.require_handle()?;
        let exec_query = self
            .exec_query
            .ok_or_else(|| anyhow!("wcdb_exec_query is not available"))?;
        let kind = cstring(kind)?;
        let path = cstring(path)?;
        let sql = cstring(sql)?;
        self.call_json(|out| unsafe {
            exec_query(handle, kind.as_ptr(), path.as_ptr(), sql.as_ptr(), out)
        })
    }

    pub fn update_message(
        &self,
        session_id: &str,
        local_id: i64,
        create_time: i32,
        content: &str,
    ) -> Result<Value> {
        let handle = self.require_handle()?;
        let update = self
            .update_message
            .ok_or_else(|| anyhow!("wcdb_update_message is not available"))?;
        let session_id = cstring(session_id)?;
        let content = cstring(content)?;
        self.call_json_or_string(|out| unsafe {
            update(
                handle,
                session_id.as_ptr(),
                local_id,
                create_time,
                content.as_ptr(),
                out,
            )
        })
    }

    pub fn delete_message(
        &self,
        session_id: &str,
        local_id: i64,
        create_time: i32,
        db_path_hint: Option<&str>,
    ) -> Result<Value> {
        let handle = self.require_handle()?;
        let delete = self
            .delete_message
            .ok_or_else(|| anyhow!("wcdb_delete_message is not available"))?;
        let session_id = cstring(session_id)?;
        let hint = cstring(db_path_hint.unwrap_or_default())?;
        self.call_json_or_string(|out| unsafe {
            delete(
                handle,
                session_id.as_ptr(),
                local_id,
                create_time,
                hint.as_ptr(),
                out,
            )
        })
    }

    pub fn anti_revoke_check(&self, session_id: &str) -> Result<Value> {
        let handle = self.require_handle()?;
        let check = self
            .check_anti_revoke
            .ok_or_else(|| anyhow!("wcdb_check_message_anti_revoke_trigger is not available"))?;
        let session_id = cstring(session_id)?;
        let mut installed = 0;
        let rc = unsafe { check(handle, session_id.as_ptr(), &mut installed) };
        if rc != 0 {
            return Err(anyhow!("anti revoke check failed with code {rc}"));
        }
        Ok(serde_json::json!({ "installed": installed != 0 }))
    }

    pub fn anti_revoke_install(&self, session_id: &str) -> Result<Value> {
        let func = self
            .install_anti_revoke
            .ok_or_else(|| anyhow!("wcdb_install_message_anti_revoke_trigger is not available"))?;
        self.trigger(session_id, func)
    }

    pub fn anti_revoke_uninstall(&self, session_id: &str) -> Result<Value> {
        let func = self.uninstall_anti_revoke.ok_or_else(|| {
            anyhow!("wcdb_uninstall_message_anti_revoke_trigger is not available")
        })?;
        self.trigger(session_id, func)
    }

    fn trigger(&self, session_id: &str, func: TriggerFn) -> Result<Value> {
        let handle = self.require_handle()?;
        let session_id = cstring(session_id)?;
        self.call_json_or_string(|out| unsafe { func(handle, session_id.as_ptr(), out) })
    }

    fn trigger_no_arg(&self, func: OutJsonNoArgTriggerFn) -> Result<Value> {
        let handle = self.require_handle()?;
        self.call_json_or_string(|out| unsafe { func(handle, out) })
    }

    fn require_handle(&self) -> Result<i64> {
        self.handle.ok_or_else(|| anyhow!("WCDB is not connected"))
    }

    fn call_string_json(
        &self,
        func: Option<OutJson1StringFn>,
        name: &str,
        value: &str,
    ) -> Result<Value> {
        let handle = self.require_handle()?;
        let func = func.ok_or_else(|| anyhow!("{name} is not available"))?;
        let value = cstring(value)?;
        self.call_json(|out| unsafe { func(handle, value.as_ptr(), out) })
    }

    fn call_string_json_or_string(
        &self,
        func: Option<OutJson1StringFn>,
        name: &str,
        value: &str,
    ) -> Result<Value> {
        let handle = self.require_handle()?;
        let func = func.ok_or_else(|| anyhow!("{name} is not available"))?;
        let value = cstring(value)?;
        self.call_json_or_string(|out| unsafe { func(handle, value.as_ptr(), out) })
    }

    fn call_string_range_json(
        &self,
        func: Option<OutJsonStringRangeFn>,
        name: &str,
        value: &str,
        begin: i32,
        end: i32,
    ) -> Result<Value> {
        let handle = self.require_handle()?;
        let func = func.ok_or_else(|| anyhow!("{name} is not available"))?;
        let value = cstring(value)?;
        self.call_json(|out| unsafe { func(handle, value.as_ptr(), begin, end, out) })
    }

    fn call_json(&self, call: impl FnOnce(*mut *mut c_void) -> c_int) -> Result<Value> {
        let mut out: *mut c_void = std::ptr::null_mut();
        let rc = call(&mut out);
        if rc != 0 || out.is_null() {
            return Err(anyhow!("WCDB call failed with code {rc}"));
        }
        let raw = unsafe { self.take_string(out)? };
        let value: Value =
            serde_json::from_str(&normalize_int64_json(&raw)).with_context(|| {
                format!(
                    "failed to parse WCDB JSON payload: {}",
                    raw.chars().take(200).collect::<String>()
                )
            })?;
        Ok(value)
    }

    fn call_json_or_string(&self, call: impl FnOnce(*mut *mut c_void) -> c_int) -> Result<Value> {
        let mut out: *mut c_void = std::ptr::null_mut();
        let rc = call(&mut out);
        let message = if out.is_null() {
            String::new()
        } else {
            unsafe { self.take_string(out)? }
        };
        if rc != 0 {
            return Err(anyhow!(if message.is_empty() {
                format!("WCDB call failed with code {rc}")
            } else {
                message
            }));
        }
        if message.is_empty() {
            Ok(serde_json::json!({ "ok": true }))
        } else {
            serde_json::from_str(&message)
                .or_else(|_| Ok(serde_json::json!({ "message": message })))
        }
    }

    unsafe fn take_string(&self, out: *mut c_void) -> Result<String> {
        let text = CStr::from_ptr(out as *const c_char)
            .to_string_lossy()
            .to_string();
        (self.free_string)(out);
        Ok(text)
    }
}

impl Drop for Wcdb {
    fn drop(&mut self) {
        self.close();
    }
}

unsafe fn symbol<T: Copy>(lib: &Library, name: &[u8]) -> Result<T> {
    Ok(*lib
        .get::<T>(name)
        .with_context(|| format!("missing symbol {}", String::from_utf8_lossy(name)))?)
}

unsafe fn optional_symbol<T: Copy>(lib: &Library, name: &[u8]) -> Option<T> {
    lib.get::<T>(name).ok().map(|sym| *sym)
}

fn wcdb_api_path(runtime_dir: &Path) -> Result<PathBuf> {
    let candidates = if cfg!(target_os = "macos") {
        vec![
            runtime_dir.join("wcdb/macos/universal/libwcdb_api.dylib"),
            runtime_dir.join("wcdb/macos/libwcdb_api.dylib"),
            runtime_dir.join("libwcdb_api.dylib"),
        ]
    } else if cfg!(target_os = "linux") {
        vec![
            runtime_dir.join("wcdb/linux/x64/libwcdb_api.so"),
            runtime_dir.join("wcdb/linux/arm64/libwcdb_api.so"),
            runtime_dir.join("wcdb/linux/libwcdb_api.so"),
            runtime_dir.join("libwcdb_api.so"),
        ]
    } else {
        vec![
            runtime_dir.join("wcdb/win32/x64/wcdb_api.dll"),
            runtime_dir.join("wcdb/win32/arm64/wcdb_api.dll"),
            runtime_dir.join("wcdb_api.dll"),
        ]
    };
    candidates
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| anyhow!("wcdb_api library not found in {}", runtime_dir.display()))
}

fn wcdb_dependency_paths(runtime_dir: &Path) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![runtime_dir.join("wcdb/macos/universal/libWCDB.dylib")]
    } else if cfg!(target_os = "windows") {
        vec![
            runtime_dir.join("runtime/win32/msvcp140.dll"),
            runtime_dir.join("runtime/win32/msvcp140_1.dll"),
            runtime_dir.join("runtime/win32/vcruntime140.dll"),
            runtime_dir.join("runtime/win32/vcruntime140_1.dll"),
            runtime_dir.join("wcdb/win32/x64/SDL2.dll"),
            runtime_dir.join("wcdb/win32/x64/WCDB.dll"),
            runtime_dir.join("wcdb/win32/arm64/WCDB.dll"),
        ]
    } else {
        Vec::new()
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

fn cstring(value: impl AsRef<str>) -> Result<CString> {
    CString::new(value.as_ref()).map_err(|_| anyhow!("string contains interior NUL byte"))
}

fn normalize_int64_json(raw: &str) -> String {
    let marker = "\"server_id\":";
    if !raw.contains(marker) {
        return raw.to_string();
    }
    let mut result = String::with_capacity(raw.len() + 16);
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if raw[i..].starts_with(marker) {
            result.push_str(marker);
            i += marker.len();
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                result.push(bytes[i] as char);
                i += 1;
            }
            let start = i;
            if i < bytes.len() && bytes[i] == b'-' {
                i += 1;
            }
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let digits = &raw[start..i];
            if digits.trim_start_matches('-').len() >= 16 {
                result.push('"');
                result.push_str(digits);
                result.push('"');
            } else {
                result.push_str(digits);
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}
