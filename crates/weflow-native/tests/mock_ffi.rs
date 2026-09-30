#![cfg(target_os = "linux")]
//! Verifies argument marshalling for every WCDB wrapper against the generated mock library.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};
use weflow_native::wcdb::{Arg, Wcdb};

fn setup(tag: &str) -> (Wcdb, PathBuf) {
    let root = std::env::temp_dir().join(format!("weflow-ffi-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let lib_dir = root.join("runtime/wcdb/linux/x64");
    std::fs::create_dir_all(&lib_dir).unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_wcdb.c");
    let status = Command::new("cc").args(["-shared", "-fPIC", "-o"]).arg(lib_dir.join("libwcdb_api.so")).arg(&src).status().expect("cc required");
    assert!(status.success());
    let account = root.join("acct");
    std::fs::create_dir_all(account.join("db_storage/session")).unwrap();
    std::fs::write(account.join("db_storage/session/session.db"), b"").unwrap();
    let mut wcdb = unsafe { Wcdb::load(root.join("runtime")) }.unwrap();
    wcdb.open(&account, &"00".repeat(32), Some("wxid_me")).unwrap();
    (wcdb, root)
}

/// Arguments of the most recent generated mock call (works for canned responses too).
fn last(root: &Path) -> Value {
    use std::ffi::CStr;
    let lib_path = root.join("runtime/wcdb/linux/x64/libwcdb_api.so");
    unsafe {
        let lib = libloading::Library::new(&lib_path).unwrap();
        let f: libloading::Symbol<unsafe extern "C" fn() -> *const std::os::raw::c_char> = lib.get(b"mock_last_call\0").unwrap();
        let text = CStr::from_ptr(f()).to_string_lossy().to_string();
        serde_json::from_str(&text).unwrap()
    }
}

fn echoed(v: &Value, name: &str) -> Vec<Value> {
    assert_eq!(v["fn"], name, "wrong function called: {v}");
    v["args"].as_array().unwrap().clone()
}

#[test]
fn string_shaped_wrappers_pass_arguments_through() {
    let (w, root) = setup("strings");
    assert_eq!(echoed(&{ let _ = w.message_by_server_id("s", "99"); last(&root) }, "wcdb_get_message_by_svrid"), vec![json!(1), json!("s"), json!("99")]);
    assert_eq!(echoed(&{ let _ = w.message_by_id("s", 7); last(&root) }, "wcdb_get_message_by_id"), vec![json!(1), json!("s"), json!(7)]);
    assert_eq!(echoed(&{ let _ = w.display_names("[\"a\"]"); last(&root) }, "wcdb_get_display_names")[1], json!("[\"a\"]"));
    assert_eq!(echoed(&{ let _ = w.avatar_urls("[]"); last(&root) }, "wcdb_get_avatar_urls")[1], json!("[]"));
    assert_eq!(echoed(&{ let _ = w.group_member_counts("[]"); last(&root) }, "wcdb_get_group_member_counts").len(), 2);
    assert_eq!(echoed(&{ let _ = w.message_tables("s"); last(&root) }, "wcdb_get_message_tables")[1], json!("s"));
    assert_eq!(echoed(&{ let _ = w.contact_status("[]"); last(&root) }, "wcdb_get_contact_status").len(), 2);
    assert_eq!(echoed(&{ let _ = w.contact_alias_map("[]"); last(&root) }, "wcdb_get_contact_alias_map").len(), 2);
    assert_eq!(echoed(&{ let _ = w.contact_friend_flags("[]"); last(&root) }, "wcdb_get_contact_friend_flags").len(), 2);
    assert_eq!(echoed(&{ let _ = w.chat_room_ext_buffer("r"); last(&root) }, "wcdb_get_chat_room_ext_buffer")[1], json!("r"));
    assert_eq!(echoed(&{ let _ = w.message_table_stats("s"); last(&root) }, "wcdb_get_message_table_stats")[1], json!("s"));
    assert_eq!(echoed(&{ let _ = w.media_schema_summary("/db"); last(&root) }, "wcdb_get_media_schema_summary")[1], json!("/db"));
    assert_eq!(echoed(&{ let _ = w.session_message_date_counts_batch("[]"); last(&root) }, "wcdb_get_session_message_date_counts_batch").len(), 2);
    assert_eq!(echoed(&{ let _ = w.head_image_buffers("[]"); last(&root) }, "wcdb_get_head_image_buffers").len(), 2);
    assert_eq!(echoed(&{ let _ = w.voice_data_batch("[]"); last(&root) }, "wcdb_get_voice_data_batch").len(), 2);
    assert_eq!(echoed(&{ let _ = w.resolve_image_hardlink_batch("[]"); last(&root) }, "wcdb_resolve_image_hardlink_batch").len(), 2);
    assert_eq!(echoed(&{ let _ = w.resolve_video_hardlink_md5_batch("[]"); last(&root) }, "wcdb_resolve_video_hardlink_md5_batch").len(), 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn multi_argument_wrappers_keep_order_and_types() {
    let (w, root) = setup("multi");
    assert_eq!(
        echoed(&{ let _ = w.message_meta("/db", "Msg_1", 10, 20); last(&root) }, "wcdb_get_message_meta"),
        vec![json!(1), json!("/db"), json!("Msg_1"), json!(10), json!(20)]
    );
    assert_eq!(
        echoed(&{ let _ = w.annual_report_extras("[\"a\"]", 1, 2, 3, 4); last(&root) }, "wcdb_get_annual_report_extras"),
        vec![json!(1), json!("[\"a\"]"), json!(1), json!(2), json!(3), json!(4)]
    );
    assert_eq!(
        echoed(&{ let _ = w.messages_by_type("s", 244813135921, true, 50, 5); last(&root) }, "wcdb_get_messages_by_type"),
        vec![json!(1), json!("s"), json!(244813135921_i64), json!(1), json!(50), json!(5)]
    );
    assert_eq!(
        echoed(&{ let _ = w.session_message_type_stats_batch("[]", "{}"); last(&root) }, "wcdb_get_session_message_type_stats_batch"),
        vec![json!(1), json!("[]"), json!("{}")]
    );
    assert_eq!(echoed(&{ let _ = w.list_tables("message", "/db"); last(&root) }, "wcdb_list_tables"), vec![json!(1), json!("message"), json!("/db")]);
    assert_eq!(echoed(&{ let _ = w.table_schema("k", "/db", "t"); last(&root) }, "wcdb_get_table_schema").len(), 4);
    assert_eq!(
        echoed(&{ let _ = w.export_table_snapshot("k", "/db", "t", "/out"); last(&root) }, "wcdb_export_table_snapshot"),
        vec![json!(1), json!("k"), json!("/db"), json!("t"), json!("/out")]
    );
    assert_eq!(echoed(&{ let _ = w.import_table_snapshot("k", "/db", "t", "/in"); last(&root) }, "wcdb_import_table_snapshot").len(), 5);
    assert_eq!(
        echoed(&{ let _ = w.import_table_snapshot_with_schema("k", "/db", "t", "/in", "CREATE TABLE t(a)"); last(&root) }, "wcdb_import_table_snapshot_with_schema"),
        vec![json!(1), json!("k"), json!("/db"), json!("t"), json!("/in"), json!("CREATE TABLE t(a)")]
    );
    assert_eq!(echoed(&{ let _ = w.message_table_columns("/db", "t"); last(&root) }, "wcdb_get_message_table_columns").len(), 3);
    assert_eq!(echoed(&{ let _ = w.message_table_time_range("/db", "t"); last(&root) }, "wcdb_get_message_table_time_range").len(), 3);
    assert_eq!(echoed(&{ let _ = w.message_table_time_range("/db", "t"); last(&root) }, "wcdb_get_message_table_time_range")[2], json!("t"));
    assert_eq!(echoed(&{ let _ = w.resolve_image_hardlink("md5", "/acct"); last(&root) }, "wcdb_resolve_image_hardlink"), vec![json!(1), json!("md5"), json!("/acct")]);
    assert_eq!(echoed(&{ let _ = w.resolve_video_hardlink_md5("md5", "/db"); last(&root) }, "wcdb_resolve_video_hardlink_md5"), vec![json!(1), json!("md5"), json!("/db")]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn list_and_status_wrappers() {
    let (w, root) = setup("lists");
    assert_eq!(echoed(&{ let _ = w.list_message_dbs(); last(&root) }, "wcdb_list_message_dbs"), vec![json!(1)]);
    assert_eq!(echoed(&{ let _ = w.list_media_dbs(); last(&root) }, "wcdb_list_media_dbs"), vec![json!(1)]);
    assert_eq!(echoed(&{ let _ = w.db_status(); last(&root) }, "wcdb_get_db_status"), vec![json!(1)]);
    let read = w.mark_all_sessions_read().unwrap();
    assert_eq!(echoed(&read, "wcdb_mark_all_sessions_read"), vec![json!(1)]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn string_output_wrappers_return_raw_text() {
    let (w, root) = setup("raw");
    let raw = w.emoticon_cdn_url("/db", "abc").unwrap();
    assert!(raw.contains("wcdb_get_emoticon_cdn_url") && raw.contains("abc"));
    assert!(w.emoticon_caption("/db", "abc").unwrap().contains("wcdb_get_emoticon_caption"));
    assert!(w.emoticon_caption_strict("abc").unwrap().contains("wcdb_get_emoticon_caption_strict"));
    let voice = w.voice_data("s", 100, 5, 9007199254740993, "[\"c\"]").unwrap();
    assert!(voice.contains("wcdb_get_voice_data") && voice.contains("9007199254740993") && voice.contains("100") && voice.contains("\"s\""), "{voice}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn special_shape_wrappers() {
    let (w, root) = setup("special");
    assert_eq!(w.message_count("s").unwrap(), 42);
    let cursor = w.open_message_cursor("s", 500, false, 0, 0, false).unwrap();
    assert_eq!(cursor, 7);
    let lite = w.open_message_cursor("s", 500, true, 1, 2, true).unwrap();
    assert_eq!(lite, 7);
    let (batch, more) = w.fetch_message_batch(cursor).unwrap();
    assert_eq!(batch["fn"], "wcdb_fetch_message_batch");
    assert_eq!(batch["args"], json!([1, 7]));
    assert!(!more);
    w.close_message_cursor(cursor).unwrap();
    let (media, more) = w.scan_media_stream("[\"s\"]", 3, 10, 20, 100, 0).unwrap();
    assert_eq!(media["args"], json!([1, "[\"s\"]", 3, 10, 20, 100, 0]));
    assert!(!more);
    assert!(w.logs().unwrap()["fn"] == "wcdb_get_logs");
    assert!(w.monitor_pipe_name().unwrap().contains("wcdb_get_monitor_pipe_name"));
    w.start_monitor_pipe().unwrap();
    w.stop_monitor_pipe().unwrap();
    w.cloud_init(60).unwrap();
    w.cloud_report("{}").unwrap();
    w.cloud_stop().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unsupported_shapes_and_missing_symbols_are_errors() {
    let (w, root) = setup("errors");
    assert!(w.invoke_json("wcdb_get_message_by_id", &[Arg::I32(1), Arg::I32(2), Arg::I32(3), Arg::I32(4), Arg::I32(5), Arg::I32(6), Arg::I32(7)]).is_err());
    assert!(w.invoke_json("wcdb_does_not_exist", &[]).is_err());
    let _ = std::fs::remove_dir_all(root);
}
