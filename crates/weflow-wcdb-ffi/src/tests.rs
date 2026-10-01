//! The C ABI exercised the way `wcdbCore.ts` calls it, against a synthetic encrypted account.

use std::ffi::{c_void, CStr, CString};
use std::ptr::null_mut;

use serde_json::{json, Value};
use weflow_native::fixture::Fixture;

use super::*;

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// Takes ownership of a string returned through an out pointer.
fn take(p: *mut c_void) -> String {
    assert!(!p.is_null(), "no string returned");
    let s = unsafe { CStr::from_ptr(p.cast()) }
        .to_string_lossy()
        .into_owned();
    unsafe { wcdb_free_string(p) };
    s
}

fn take_json(p: *mut c_void) -> Value {
    serde_json::from_str(&take(p)).unwrap()
}

fn open(tag: &str) -> (i64, Fixture) {
    let root = std::env::temp_dir().join(format!("weflow-ffi-{}-{tag}", std::process::id()));
    let f = Fixture::standard(&root);
    let session_db = f.db_storage().join("session/session.db");
    let mut handle = 0i64;
    assert_eq!(wcdb_init(), STATUS_OK);
    assert_eq!(InitProtection(c("/anywhere").as_ptr()), STATUS_OK);
    let rc = unsafe {
        wcdb_open_account(
            c(&session_db.to_string_lossy()).as_ptr(),
            c(&f.key_hex()).as_ptr(),
            &mut handle,
        )
    };
    assert_eq!(rc, STATUS_OK);
    assert!(handle > 0);
    assert_eq!(
        unsafe { wcdb_set_my_wxid(handle, c("wxid_me").as_ptr()) },
        STATUS_OK
    );
    (handle, f)
}

#[test]
fn sessions_messages_and_counts_come_back_as_json() {
    let (h, _f) = open("basic");
    let mut out: *mut c_void = null_mut();
    assert_eq!(unsafe { wcdb_get_sessions(h, &mut out) }, STATUS_OK);
    let sessions = take_json(out);
    assert_eq!(sessions.as_array().unwrap().len(), 3);
    assert_eq!(sessions[0]["username"], "wxid_bob");

    let mut out: *mut c_void = null_mut();
    assert_eq!(
        unsafe { wcdb_get_messages(h, c("wxid_bob").as_ptr(), 2, 0, &mut out) },
        STATUS_OK
    );
    let msgs = take_json(out);
    assert_eq!(msgs.as_array().unwrap().len(), 2);
    assert_eq!(msgs[0]["message_content"], "see you");
    assert_eq!(
        msgs[1]["is_send"], 1,
        "set_my_wxid decides which side sent a message"
    );

    let mut count = 0i32;
    assert_eq!(
        unsafe { wcdb_get_message_count(h, c("wxid_bob").as_ptr(), &mut count) },
        STATUS_OK
    );
    assert_eq!(count, 5);
    let mut members = 0i32;
    assert_eq!(
        unsafe { wcdb_get_group_member_count(h, c("room1@chatroom").as_ptr(), &mut members) },
        STATUS_OK
    );
    assert_eq!(members, 3);

    let mut out: *mut c_void = null_mut();
    assert_eq!(
        unsafe { wcdb_get_display_names(h, c(r#"["wxid_bob","nobody"]"#).as_ptr(), &mut out) },
        STATUS_OK
    );
    assert_eq!(
        take_json(out),
        json!({ "wxid_bob": "Bobby", "nobody": "nobody" })
    );
    assert_eq!(wcdb_close_account(h), STATUS_OK);
}

#[test]
fn cursors_page_through_a_conversation() {
    let (h, _f) = open("cursor");
    let mut cursor = 0i64;
    assert_eq!(
        unsafe {
            wcdb_open_message_cursor_lite(h, c("wxid_bob").as_ptr(), 2, 1, 0, 0, &mut cursor)
        },
        STATUS_OK
    );
    let mut seen = 0;
    loop {
        let (mut out, mut more): (*mut c_void, i32) = (null_mut(), 0);
        assert_eq!(
            unsafe { wcdb_fetch_message_batch(h, cursor, &mut out, &mut more) },
            STATUS_OK
        );
        seen += take_json(out).as_array().unwrap().len();
        if more == 0 {
            break;
        }
    }
    assert_eq!(seen, 5);
    assert_eq!(wcdb_close_message_cursor(h, cursor), STATUS_OK);
    let (mut out, mut more): (*mut c_void, i32) = (null_mut(), 0);
    assert_eq!(
        unsafe { wcdb_fetch_message_batch(h, cursor, &mut out, &mut more) },
        STATUS_FAILED,
        "a closed cursor is an error"
    );
    assert!(
        take(out).contains("cursor"),
        "the reason is returned in the out argument"
    );
}

#[test]
fn writes_are_refused_with_a_reason_and_triggers_report_not_installed() {
    let (h, _f) = open("ro");
    let mut err: *mut c_void = null_mut();
    let rc =
        unsafe { wcdb_update_message(h, c("wxid_bob").as_ptr(), 1, 0, c("x").as_ptr(), &mut err) };
    assert_eq!(rc, STATUS_READ_ONLY);
    assert!(take(err).contains("read-only"));
    let mut installed = 7i32;
    assert_eq!(
        unsafe {
            wcdb_check_message_anti_revoke_trigger(h, c("wxid_bob").as_ptr(), &mut installed)
        },
        STATUS_READ_ONLY
    );
    assert_eq!(installed, 0);
    let mut out: *mut c_void = null_mut();
    assert_eq!(
        unsafe {
            wcdb_import_table_snapshot(
                h,
                c("session").as_ptr(),
                c("").as_ptr(),
                c("SessionTable").as_ptr(),
                c("/x").as_ptr(),
                &mut out,
            )
        },
        STATUS_READ_ONLY
    );
    assert_eq!(take_json(out)["success"], false);
}

#[test]
fn errors_and_bad_handles_are_reported_not_crashed() {
    let mut out: *mut c_void = null_mut();
    assert_eq!(
        unsafe { wcdb_get_sessions(987_654, &mut out) },
        STATUS_BAD_HANDLE
    );
    assert_eq!(take(out), "invalid handle");
    let mut handle = 0i64;
    assert_eq!(
        unsafe {
            wcdb_open_account(
                c("/does/not/exist/db_storage/session/session.db").as_ptr(),
                c("00").as_ptr(),
                &mut handle,
            )
        },
        STATUS_FAILED
    );
    assert_eq!(
        unsafe { wcdb_open_account(std::ptr::null(), std::ptr::null(), &mut handle) },
        STATUS_BAD_ARGUMENT
    );
    let mut logs: *mut c_void = null_mut();
    assert_eq!(unsafe { wcdb_get_logs(&mut logs) }, STATUS_OK);
    assert!(take_json(logs)
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l.as_str().unwrap().contains("wcdb_open_account")));
    unsafe { wcdb_free_string(null_mut()) };
}

#[test]
fn strings_numbers_and_batches_cover_the_rest_of_the_surface() {
    let (h, f) = open("rest");
    let call = |rc: i32, out: *mut c_void| -> Value {
        assert_eq!(
            rc,
            STATUS_OK,
            "{}",
            if out.is_null() {
                String::new()
            } else {
                take(out)
            }
        );
        take_json(out)
    };
    let mut out: *mut c_void = null_mut();
    let stats = call(
        unsafe { wcdb_get_aggregate_stats(h, c(r#"["wxid_bob"]"#).as_ptr(), 0, 0, &mut out) },
        out,
    );
    assert_eq!(stats["total"], 5);
    let mut out: *mut c_void = null_mut();
    let annual = call(
        unsafe { wcdb_get_annual_report_stats(h, c(r#"["wxid_bob"]"#).as_ptr(), 0, 0, &mut out) },
        out,
    );
    assert!(annual["sessions"]["wxid_bob"]["monthly"].is_object());
    let mut out: *mut c_void = null_mut();
    let tl = call(
        unsafe { wcdb_get_sns_timeline(h, 10, 0, c("").as_ptr(), c("").as_ptr(), 0, 0, &mut out) },
        out,
    );
    assert_eq!(tl.as_array().unwrap().len(), 5);
    let mut out: *mut c_void = null_mut();
    let found = call(
        unsafe {
            wcdb_search_messages(
                h,
                c("wxid_bob").as_ptr(),
                c("see").as_ptr(),
                10,
                0,
                0,
                0,
                &mut out,
            )
        },
        out,
    );
    assert_eq!(found.as_array().unwrap().len(), 1);
    let mut out: *mut c_void = null_mut();
    let compact = call(
        unsafe { wcdb_get_contacts_compact(h, c(r#"["wxid_bob"]"#).as_ptr(), &mut out) },
        out,
    );
    assert_eq!(compact[0]["remark"], "Bobby");
    let mut out: *mut c_void = null_mut();
    let tables = call(
        unsafe { wcdb_list_tables(h, c("session").as_ptr(), c("").as_ptr(), &mut out) },
        out,
    );
    assert!(tables
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "SessionTable"));
    let mut out: *mut c_void = null_mut();
    let fp = call(
        unsafe { wcdb_get_my_footprint_stats(h, c("{}").as_ptr(), &mut out) },
        out,
    );
    assert!(fp["private_sessions"].is_array());

    let mut out: *mut c_void = null_mut();
    assert_eq!(
        unsafe {
            wcdb_get_emoticon_cdn_url(
                h,
                c("").as_ptr(),
                c("aabbccddeeff00112233445566778899").as_ptr(),
                &mut out,
            )
        },
        STATUS_OK
    );
    assert_eq!(
        take(out),
        "http://cdn/e1",
        "plain-text results are not JSON-quoted"
    );
    let mut out: *mut c_void = null_mut();
    assert_eq!(
        unsafe {
            wcdb_get_voice_data(
                h,
                c("wxid_bob").as_ptr(),
                weflow_native::fixture::T0 as i32 + 500,
                6,
                9_000_000_000_001,
                c("[]").as_ptr(),
                &mut out,
            )
        },
        STATUS_OK
    );
    assert_eq!(take(out), "01020304", "voice data is hex");

    let (mut out, mut more): (*mut c_void, i32) = (null_mut(), 9);
    assert_eq!(
        unsafe {
            wcdb_scan_media_stream(
                h,
                c(r#"["wxid_bob"]"#).as_ptr(),
                1,
                0,
                0,
                10,
                0,
                &mut out,
                &mut more,
            )
        },
        STATUS_OK
    );
    assert_eq!((take_json(out).as_array().unwrap().len(), more), (1, 0));
    let _ = f;
}

#[test]
fn the_account_directory_is_found_from_the_session_db_path() {
    assert_eq!(
        account_dir_of(Path::new("/a/wxid_x_1a2b/db_storage/session/session.db")),
        PathBuf::from("/a/wxid_x_1a2b")
    );
    assert_eq!(
        account_dir_of(Path::new("/a/b/c.db")),
        PathBuf::from("/a/b/c.db"),
        "no db_storage: left as is"
    );
}
