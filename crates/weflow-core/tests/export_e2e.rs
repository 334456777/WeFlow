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

#[test]
fn plain_txt_names_senders_and_honours_sender_and_display_name() {
    let (hub, root, _f) = common::custom_hub("plaintxt", |f| {
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
        .export_messages_txt(
            "room1@chatroom",
            None,
            None,
            &root.join("all.txt"),
            None,
            None,
        )
        .unwrap();
    assert_eq!(r["count"], 3);
    assert_eq!(headers(&read("all.txt")), ["Bob in room", "Quiet", "Me"]);
    assert!(
        read("all.txt").contains("\n\nsecond\n\n"),
        "the sender prefix is stripped"
    );

    // --sender keeps one person's messages
    let r = hub
        .export_messages_txt(
            "room1@chatroom",
            None,
            None,
            &root.join("quiet.txt"),
            Some("wxid_quiet"),
            None,
        )
        .unwrap();
    assert_eq!(r["count"], 1);
    assert_eq!(headers(&read("quiet.txt")), ["Quiet"]);

    // --display-name remark
    hub.export_messages_txt(
        "room1@chatroom",
        None,
        None,
        &root.join("remark.txt"),
        None,
        Some(DisplayPref::Remark),
    )
    .unwrap();
    assert_eq!(headers(&read("remark.txt")), ["Bobby", "Quiet", "Me"]);
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
