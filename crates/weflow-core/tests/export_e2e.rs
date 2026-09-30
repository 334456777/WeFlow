#![cfg(target_os = "linux")]
mod common;

use serde_json::Value;
use weflow_core::export_msg::DisplayPref;
use weflow_core::services::MessageExportRequest;

fn request(session: &str, format: &str) -> MessageExportRequest {
    MessageExportRequest {
        session_id: session.into(),
        format: format.into(),
        start: None,
        end: None,
        sender: None,
        display_pref: DisplayPref::Remark,
        excel_compact: false,
    }
}

#[test]
fn every_format_exports_through_the_ffi_layer() {
    let (hub, root) = common::mock_hub("formats");
    for (format, file) in [
        ("json", "a.json"),
        ("arkme-json", "b.json"),
        ("chatlab", "c.json"),
        ("chatlab-jsonl", "d.jsonl"),
        ("excel", "e.xlsx"),
        ("txt", "f.txt"),
        ("weclone", "g.csv"),
        ("html", "h.html"),
        ("sql", "i.sql"),
    ] {
        let out = root.join("out").join(file);
        let result = hub.export_messages(&request("wxid_bob", format), &out).unwrap_or_else(|e| panic!("{format}: {e}"));
        assert_eq!(result["count"], 3, "{format}");
        assert!(out.metadata().unwrap().len() > 100, "{format} output is empty");
    }
}

#[test]
fn chatlab_content_matches_the_mock_conversation() {
    let (hub, root) = common::mock_hub("chatlab");
    let out = root.join("chat.json");
    hub.export_messages(&request("wxid_bob", "chatlab"), &out).unwrap();
    let v: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let msgs = v["messages"].as_array().unwrap();
    // chronological: hello (me), hi (Bob), image (Bob)
    assert_eq!(msgs[0]["content"], "hello, \"world\"");
    assert_eq!(msgs[0]["sender"], "wxid_me");
    assert_eq!(msgs[1]["content"], "hi <there>");
    assert_eq!(msgs[1]["accountName"], "Bobby");
    assert_eq!(msgs[2]["type"], 1);
    // 16+ digit server ids survive as exact strings
    assert_eq!(msgs[2]["platformMessageId"], "9007199254740993");
    assert_eq!(v["meta"]["name"], "Bobby");
}

#[test]
fn group_export_uses_group_nicknames_and_members() {
    let (hub, root) = common::mock_hub("group");
    let out = root.join("group.json");
    let mut req = request("room1@chatroom", "arkme-json");
    req.display_pref = DisplayPref::GroupNickname;
    hub.export_messages(&req, &out).unwrap();
    let v: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(v["session"]["type"], "群聊");
    let members = v["groupMembers"].as_array().unwrap();
    assert!(members.iter().any(|m| m["wxid"] == "wxid_bob" && m["groupNickname"] == "Bob in room"));
    assert!(members.iter().any(|m| m["wxid"] == "wxid_quiet"));
    let system = v["messages"].as_array().unwrap().iter().find(|m| m["localType"] == 10000).unwrap();
    assert_eq!(system["content"], "Bob joined");
}

#[test]
fn date_range_and_sender_filter_apply() {
    let (hub, root) = common::mock_hub("filters");
    let out = root.join("only.txt");
    let mut req = request("wxid_bob", "txt");
    req.start = Some(1_700_000_050);
    hub.export_messages(&req, &out).unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(!text.contains("hello"));
    assert!(text.contains("hi <there>"));

    let out2 = root.join("mine.txt");
    let mut req2 = request("wxid_bob", "txt");
    req2.sender = Some("wxid_me".into());
    let result = hub.export_messages(&req2, &out2).unwrap();
    assert_eq!(result["count"], 1);
}

#[test]
fn empty_range_is_an_error_and_bad_format_is_usage_error() {
    let (hub, root) = common::mock_hub("errors");
    let mut req = request("wxid_bob", "txt");
    req.start = Some(1_800_000_000);
    assert!(hub.export_messages(&req, &root.join("x.txt")).is_err());
    let err = hub.export_messages(&request("wxid_bob", "pdf"), &root.join("x.pdf")).unwrap_err();
    assert_eq!(err.exit_code, 2);
}
