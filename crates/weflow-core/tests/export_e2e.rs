mod common;

use serde_json::Value;
use weflow_core::export_msg::DisplayPref;
use weflow_core::services::MessageExportRequest;
use weflow_native::fixture::{ContactSpec, MsgSpec, RoomSpec, SessionSpec, T0};

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

fn session(username: &'static str) -> SessionSpec {
    SessionSpec {
        username,
        summary: "",
        last_timestamp: T0,
        unread: 0,
        last_msg_type: 1,
    }
}

#[test]
fn chatlab_content_keeps_quotes_markup_and_big_server_ids() {
    let (hub, root, _f) = common::custom_hub("chatlab", |f| {
        f.session_db(&[session("wxid_bob")]);
        f.contact_db(
            &[
                ContactSpec::new("wxid_me", 1, "Me"),
                ContactSpec {
                    remark: "Bobby",
                    ..ContactSpec::new("wxid_bob", 1, "Bob")
                },
            ],
            &[],
        );
        let mut picture = MsgSpec::text(
            3,
            "wxid_bob",
            T0 + 120,
            "<msg><img md5=\"aabbccddeeff00112233445566778899\"/></msg>",
        )
        .of_type(3);
        picture.server_id = 9_007_199_254_740_993; // 2^53 + 1: not representable as a double
        f.message_shard(
            0,
            &[(
                "wxid_bob",
                vec![
                    MsgSpec::text(1, "wxid_me", T0, "hello, \"world\""),
                    MsgSpec::text(2, "wxid_bob", T0 + 60, "hi <there>"),
                    picture,
                    // every media kind is stored as `<msg>...` XML and must keep its own ChatLab type
                    MsgSpec::text(4, "wxid_bob", T0 + 180, "<msg><voicemsg length=\"1000\"/></msg>").of_type(34),
                    MsgSpec::text(5, "wxid_bob", T0 + 240, "<msg><videomsg md5=\"aabbccddeeff00112233445566778899\"/></msg>").of_type(43),
                    MsgSpec::text(6, "wxid_bob", T0 + 300, "<msg><emoji md5=\"aabbccddeeff00112233445566778899\"/></msg>").of_type(47),
                    MsgSpec::text(7, "wxid_bob", T0 + 360, "<msg><appmsg><title>t</title><type>5</type><url>http://x</url></appmsg></msg>").of_type(49),
                    MsgSpec::text(8, "wxid_bob", T0 + 420, "<msg><appmsg><title>f</title><type>6</type></appmsg></msg>").of_type(49),
                ],
            )],
        );
    });
    let out = root.join("chat.json");
    hub.export_messages(&request("wxid_bob", "chatlab"), &out)
        .unwrap();
    let v: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let msgs = v["messages"].as_array().unwrap();
    // chronological: hello (me), hi (Bob), image (Bob)
    assert_eq!(msgs[0]["content"], "hello, \"world\"");
    assert_eq!(msgs[0]["sender"], "wxid_me");
    assert_eq!(msgs[1]["content"], "hi <there>");
    assert_eq!(msgs[1]["accountName"], "Bobby");
    let types: Vec<i64> = msgs.iter().map(|m| m["type"].as_i64().unwrap()).collect();
    // text, text, image, voice, video, sticker, link, file
    assert_eq!(
        types,
        [0, 0, 1, 2, 3, 5, 7, 4],
        "media messages are not links"
    );
    // 16+ digit server ids survive as exact strings
    assert_eq!(msgs[2]["platformMessageId"], "9007199254740993");
    assert_eq!(v["meta"]["name"], "Bobby");
}

#[test]
fn group_export_uses_group_nicknames_members_and_system_messages() {
    let (hub, root, _f) = common::custom_hub("group", |f| {
        f.session_db(&[session("room1@chatroom")]);
        f.contact_db(
            &[
                ContactSpec::new("wxid_me", 1, "Me"),
                ContactSpec {
                    remark: "Bobby",
                    ..ContactSpec::new("wxid_bob", 1, "Bob")
                },
                ContactSpec::new("wxid_quiet", 3, "Quiet"),
                ContactSpec::new("room1@chatroom", 2, "Room"),
            ],
            &[RoomSpec {
                username: "room1@chatroom",
                owner: "wxid_bob",
                members: &[
                    ("wxid_me", ""),
                    ("wxid_bob", "Bob in room"),
                    ("wxid_quiet", ""),
                ],
            }],
        );
        f.message_shard(
            0,
            &[(
                "room1@chatroom",
                vec![
                    MsgSpec::text(
                        1,
                        "room1@chatroom",
                        T0,
                        "<sysmsg type=\"x\"><plain>Bob joined</plain></sysmsg>",
                    )
                    .of_type(10000),
                    MsgSpec::text(2, "wxid_bob", T0 + 60, "wxid_bob:\nhello all"),
                    MsgSpec::text(3, "wxid_me", T0 + 120, "welcome"),
                ],
            )],
        );
    });
    let out = root.join("group.json");
    let mut req = request("room1@chatroom", "arkme-json");
    req.display_pref = DisplayPref::GroupNickname;
    hub.export_messages(&req, &out).unwrap();
    let v: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(v["session"]["type"], "群聊");
    let members = v["groupMembers"].as_array().unwrap();
    assert!(members
        .iter()
        .any(|m| m["wxid"] == "wxid_bob" && m["groupNickname"] == "Bob in room"));
    assert!(members.iter().any(|m| m["wxid"] == "wxid_quiet"));
    let msgs = v["messages"].as_array().unwrap();
    let system = msgs.iter().find(|m| m["localType"] == 10000).unwrap();
    assert!(
        ["[System: Bob joined]", "[系统: Bob joined]"]
            .contains(&system["content"].as_str().unwrap()),
        "system messages are wrapped: {}",
        system["content"]
    );
    // arkme-json lists each sender once and refers to it by `senderID`
    let senders = v["senders"].as_array().unwrap();
    let bob = senders.iter().find(|s| s["wxid"] == "wxid_bob").unwrap();
    assert_eq!(
        bob["displayName"], "Bob in room",
        "the group nickname wins with GroupNickname"
    );
    assert_eq!(bob["remark"], "Bobby");
    let hello = msgs
        .iter()
        .find(|m| m["content"].as_str().map(str::trim) == Some("hello all"))
        .expect("sender prefix stripped");
    assert_eq!(hello["senderID"], bob["senderID"]);
}

fn txt_request(sender: Option<&str>, display_pref: DisplayPref) -> MessageExportRequest {
    MessageExportRequest {
        sender: sender.map(str::to_string),
        display_pref,
        ..request("room1@chatroom", "txt")
    }
}

#[test]
fn txt_names_senders_and_honours_sender_and_display_name() {
    let (hub, root, _f) = common::custom_hub("txt", |f| {
        f.session_db(&[session("room1@chatroom")]);
        f.contact_db(
            &[
                ContactSpec::new("wxid_me", 1, "Me"),
                ContactSpec {
                    remark: "Bobby",
                    ..ContactSpec::new("wxid_bob", 1, "Bob")
                },
                ContactSpec::new("wxid_quiet", 3, "Quiet"),
                ContactSpec::new("room1@chatroom", 2, "Room"),
            ],
            &[RoomSpec {
                username: "room1@chatroom",
                owner: "wxid_bob",
                members: &[
                    ("wxid_me", ""),
                    ("wxid_bob", "Bob in room"),
                    ("wxid_quiet", ""),
                ],
            }],
        );
        f.message_shard(
            0,
            &[(
                "room1@chatroom",
                vec![
                    MsgSpec::text(1, "wxid_bob", T0, "wxid_bob:\nfirst"),
                    MsgSpec::text(2, "wxid_quiet", T0 + 60, "wxid_quiet:\nsecond"),
                    MsgSpec::text(3, "wxid_me", T0 + 120, "third"),
                ],
            )],
        );
    });
    let read = |name: &str| std::fs::read_to_string(root.join(name)).unwrap();
    let headers = |text: &str| -> Vec<String> {
        text.lines()
            .filter_map(|l| {
                l.split_once(" '")
                    .map(|(_, n)| n.trim_end_matches('\'').to_string())
            })
            .collect()
    };

    // default naming: group nickname, then remark, then nickname (no remark: the nickname, not the wxid)
    let r = hub
        .export_messages(
            &txt_request(None, DisplayPref::GroupNickname),
            &root.join("all.txt"),
        )
        .unwrap();
    assert_eq!(r["count"], 3);
    assert_eq!(headers(&read("all.txt")), ["Bob in room", "Quiet", "Me"]);
    assert!(
        read("all.txt").contains("'\nsecond\n\n"),
        "the sender prefix is stripped"
    );

    // --sender keeps one person's messages
    let r = hub
        .export_messages(
            &txt_request(Some("wxid_quiet"), DisplayPref::GroupNickname),
            &root.join("quiet.txt"),
        )
        .unwrap();
    assert_eq!(r["count"], 1);
    assert_eq!(headers(&read("quiet.txt")), ["Quiet"]);

    // --display-name remark
    hub.export_messages(
        &txt_request(None, DisplayPref::Remark),
        &root.join("remark.txt"),
    )
    .unwrap();
    assert_eq!(headers(&read("remark.txt")), ["Bobby", "Quiet", "Me"]);

    // SQL and WeClone are written as the messages are read, too: header first, then one entry per message
    let sql_req = |sender: Option<&str>| MessageExportRequest {
        format: "sql".into(),
        ..txt_request(sender, DisplayPref::GroupNickname)
    };
    let r = hub
        .export_messages(&sql_req(None), &root.join("all.sql"))
        .unwrap();
    assert_eq!(
        (r["count"].as_i64(), r["format"].as_str()),
        (Some(3), Some("sql"))
    );
    let sql = read("all.sql");
    assert!(sql.starts_with("CREATE TABLE IF NOT EXISTS \"messages\""));
    assert_eq!(sql.matches("INSERT INTO \"messages\"").count(), 3);
    assert!(sql.contains("'second'"));
    let r = hub
        .export_messages(&sql_req(Some("wxid_quiet")), &root.join("quiet.sql"))
        .unwrap();
    assert_eq!(r["count"], 1);
    assert_eq!(read("quiet.sql").matches("INSERT INTO").count(), 1);

    let weclone = MessageExportRequest {
        format: "weclone".into(),
        ..txt_request(None, DisplayPref::GroupNickname)
    };
    hub.export_messages(&weclone, &root.join("all.csv"))
        .unwrap();
    let csv = read("all.csv");
    assert!(csv.starts_with("\u{feff}id,MsgSvrID,type_name,"));
    let rows: Vec<&str> = csv.lines().skip(1).collect();
    assert_eq!(rows.len(), 3, "{csv}");
    assert!(
        rows[0].starts_with("1,") && rows[2].starts_with("3,"),
        "{csv}"
    );

    // an empty range writes no file at all
    let mut empty = sql_req(None);
    empty.start = Some(1_900_000_000);
    assert!(hub.export_messages(&empty, &root.join("none.sql")).is_err());
    assert!(!root.join("none.sql").exists());

    // every format is written while the messages are read: all of them produce a file, and no spool file stays
    for format in weflow_core::services::MESSAGE_EXPORT_FORMATS.split(',') {
        let format = format.trim();
        let out = root.join(format!("all-{format}.out"));
        let req = MessageExportRequest {
            format: format.into(),
            ..txt_request(None, DisplayPref::GroupNickname)
        };
        let r = hub.export_messages(&req, &out).unwrap();
        assert_eq!(r["count"], 3, "{format}");
        assert!(std::fs::metadata(&out).unwrap().len() > 0, "{format}");
        let mut none = req;
        none.start = Some(1_900_000_000);
        let empty = root.join(format!("none-{format}.out"));
        assert!(hub.export_messages(&none, &empty).is_err(), "{format}");
        assert!(!empty.exists(), "{format}: an empty range writes no file");
    }
    let leftovers: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "spool files left behind: {leftovers:?}"
    );
    let json: serde_json::Value =
        serde_json::from_str(&read("all-json.out")).expect("streamed JSON is valid");
    assert_eq!(json["session"]["messageCount"], 3);
    assert_eq!(json["messages"].as_array().unwrap().len(), 3);
    let chatlab: serde_json::Value =
        serde_json::from_str(&read("all-chatlab.out")).expect("streamed ChatLab is valid");
    assert_eq!(chatlab["messages"].as_array().unwrap().len(), 3);
    assert!(
        read("all-html.out").contains("3 条消息") || read("all-html.out").contains("3 messages")
    );
}

#[test]
fn empty_range_is_an_error_and_bad_format_is_usage_error() {
    let (hub, root) = common::mock_hub("errors");
    let mut req = request("wxid_bob", "txt");
    req.start = Some(1_800_000_000);
    assert!(hub.export_messages(&req, &root.join("x.txt")).is_err());
    let err = hub
        .export_messages(&request("wxid_bob", "pdf"), &root.join("x.pdf"))
        .unwrap_err();
    assert_eq!(err.exit_code, 2);
}

#[tokio::test]
async fn an_export_whose_keys_break_the_sort_seq_rule_starts_over_in_the_right_order() {
    // the same sort_seq for two messages, but message 2 was sent first: keys taken from the sort_seq index list them
    // 1, 2, the rows 2, 1; the export finds out on its first page and starts over
    let seq = (T0 + 1) * 1000;
    let (hub, root, _) = common::custom_hub("keys-rule", |f| {
        f.session_db(&[session("wxid_bob")]);
        f.contact_db(&[ContactSpec::new("wxid_bob", 1, "Bob")], &[]);
        f.message_shard(
            0,
            &[(
                "wxid_bob",
                vec![
                    MsgSpec::text(1, "wxid_bob", T0 + 9, "later").with_sort_seq(seq),
                    MsgSpec::text(2, "wxid_bob", T0 + 1, "first").with_sort_seq(seq),
                    MsgSpec::text(3, "wxid_bob", T0 + 20, "last"),
                ],
            )],
        );
    });
    let texts = |path: &std::path::Path| -> Vec<String> {
        let doc: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        doc["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["content"].as_str().unwrap().to_string())
            .collect()
    };
    let out = root.join("stream.json");
    hub.export_messages(&request("wxid_bob", "chatlab"), &out)
        .unwrap();
    assert_eq!(texts(&out), ["first", "later", "last"]);

    // an export with media reads the messages first: the same, on a hub that has not met this table yet
    let (hub, root2, _) = common::custom_hub("keys-rule-media", |f| {
        f.session_db(&[session("wxid_bob")]);
        f.contact_db(&[ContactSpec::new("wxid_bob", 1, "Bob")], &[]);
        f.message_shard(
            0,
            &[(
                "wxid_bob",
                vec![
                    MsgSpec::text(1, "wxid_bob", T0 + 9, "later").with_sort_seq(seq),
                    MsgSpec::text(2, "wxid_bob", T0 + 1, "first").with_sort_seq(seq),
                    MsgSpec::text(3, "wxid_bob", T0 + 20, "last"),
                ],
            )],
        );
    });
    let media = weflow_core::api::ApiMediaOptions {
        enabled: true,
        images: true,
        ..Default::default()
    };
    let out = root2.join("collected.json");
    hub.export_messages_with_media(&request("wxid_bob", "chatlab"), &out, &media)
        .await
        .unwrap();
    assert_eq!(texts(&out), ["first", "later", "last"]);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
}
