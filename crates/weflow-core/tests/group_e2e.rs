//! The group reads on a synthetic account: the members panel, the group list and one member's analytics and exports.
mod common;

#[test]
fn members_panel_is_sorted_and_enriched() {
    let (_root, hub) = common::mock_hub("group-panel");
    let (entries, _) = hub.group_members_panel("room1@chatroom", true).unwrap();
    let entries = serde_json::to_value(entries).unwrap();
    let names: Vec<&str> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["username"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["wxid_bob", "wxid_me", "wxid_quiet"],
        "the owner (bob), then the others by their messages and names"
    );
    // the account's own row is a friend's in WeChat's contacts; the account is not its own friend (#1079)
    assert_eq!(entries[1]["isFriend"], false, "{}", entries[1]);
    assert_eq!(entries[0]["messageCount"], 1);
    assert_eq!(entries[0]["isFriend"], true);
    assert_eq!(entries[0]["isOwner"], true);
    assert_eq!(entries[0]["groupNickname"], "Bob in room");
    assert_eq!(entries[0]["nickname"], "Bob");
    assert_eq!(entries[0]["displayName"], "Bobby");
    assert_eq!(entries[0]["alias"], "bobby_id");
    assert_eq!(
        entries[2]["isFriend"], false,
        "quiet is in the group but not a friend"
    );
    assert_eq!(entries[2]["messageCount"], 1);
    let (without_counts, _) = hub.group_members_panel("room1@chatroom", false).unwrap();
    assert_eq!(without_counts[0].message_count, 0);
    // the panel stored my own count as a hint
    assert_eq!(hub.group_hint("room1@chatroom").unwrap().count, Some(1));
}

#[test]
fn group_chats_and_full_members() {
    let (_root, hub) = common::mock_hub("group-list");
    let groups = hub.group_chats().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].username, "room1@chatroom");
    assert_eq!(groups[0].display_name, "Project Room");
    assert_eq!(groups[0].member_count, 3);
    let members = hub.group_members_full("room1@chatroom").unwrap();
    assert_eq!(members.len(), 3);
    let bob = members
        .iter()
        .find(|m| m["username"] == "wxid_bob")
        .unwrap();
    assert_eq!(bob["groupNickname"], "Bob in room");
    assert_eq!(bob["isOwner"], true);
    assert_eq!(members.iter().filter(|m| m["isOwner"] == true).count(), 1);
}

#[test]
fn member_analytics_and_exports() {
    let (root, hub) = common::mock_hub("group-export");
    let a = serde_json::to_value(
        hub.group_member_analytics("room1@chatroom", "wxid_bob", 0, 0)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        a["statistics"]["totalMessages"], 1,
        "only Bob's message in the room counts"
    );
    assert_eq!(a["statistics"]["textMessages"], 1);
    assert_eq!(a["statistics"]["firstMessageTime"], 1_700_000_010_i64);
    assert_eq!(a["statistics"]["activeDays"], 1);
    assert_eq!(a["timeDistribution"].as_object().unwrap().len(), 24);
    assert_eq!(
        a["commonPhrases"][0]["phrase"], "welcome",
        "the sender prefix and the line break are stripped"
    );
    let me = serde_json::to_value(
        hub.group_member_analytics("room1@chatroom", "wxid_me", 0, 0)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        me["statistics"]["totalMessages"], 1,
        "my own messages are matched through is_send"
    );
    assert_eq!(me["commonPhrases"][0]["phrase"], "thanks");

    let csv = root.join("members.csv");
    let res = hub.group_export_members("room1@chatroom", &csv).unwrap();
    assert_eq!(res.count, 3);
    let text = std::fs::read_to_string(&csv).unwrap();
    assert!(text.starts_with("\u{feff}会话信息\n"), "{text}");
    assert!(
        text.contains("会话信息") || text.contains("Session"),
        "{text}"
    );
    assert!(
        text.contains("Bob,Bobby,Bob in room,wxid_bob,bobby_id"),
        "{text}"
    );
    assert!(
        text.contains("微信昵称,微信备注,群昵称,wxid,微信号") || text.contains("wxid"),
        "{text}"
    );

    let xlsx = root.join("members.xlsx");
    hub.group_export_members("room1@chatroom", &xlsx).unwrap();
    assert!(std::fs::metadata(&xlsx).unwrap().len() > 1000);

    let msgs = root.join("bob.csv");
    let res = hub
        .group_export_member_messages("room1@chatroom", "wxid_bob", &msgs, 0, 0)
        .unwrap();
    assert_eq!(res.count, 1);
    let csv_text = std::fs::read_to_string(&msgs).unwrap();
    assert!(csv_text.starts_with("\u{feff}会话信息\n"), "{csv_text}");
    assert!(
        csv_text.contains("序号,时间,发送者wxid,消息类型,内容"),
        "{csv_text}"
    );
    assert!(
        csv_text.contains("wxid_bob") && csv_text.contains("文本") && csv_text.contains("welcome"),
        "{csv_text}"
    );
    let page = serde_json::to_value(
        hub.group_member_messages("room1@chatroom", "wxid_bob", 0, 0, 20, 0)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(page["hasMore"], false);
    assert_eq!(page["messages"].as_array().unwrap().len(), 1);
    assert_eq!(page["messages"][0]["parsedContent"], "welcome");
    assert_eq!(page["messages"][0]["senderUsername"], "wxid_bob");
}

/// The group exports write `.csv` or `.xlsx`, in any case, and refuse any other name before they read anything (#1136).
#[test]
fn group_exports_take_a_csv_or_xlsx_name_only() {
    let (root, hub) = common::mock_hub("group-export-names");
    for name in ["members.txt", "members.json", "members", "members.csv.bak"] {
        let out = root.join(name);
        let err = hub
            .group_export_members("room1@chatroom", &out)
            .unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.exit_code),
            ("invalid_arguments", 2),
            "{name}: {}",
            err.message
        );
        assert!(
            err.message.contains("is neither a .csv nor an .xlsx file"),
            "{name}: {}",
            err.message
        );
        let err = hub
            .group_export_member_messages("room1@chatroom", "wxid_bob", &out, 0, 0)
            .unwrap_err();
        assert_eq!(err.code, "invalid_arguments", "{name}: {}", err.message);
        assert!(!out.exists(), "{name}");
    }
    for name in ["members.CSV", "members.XLSX"] {
        let out = root.join(name);
        assert_eq!(
            hub.group_export_members("room1@chatroom", &out)
                .unwrap()
                .count,
            3
        );
        assert!(out.is_file(), "{name}");
    }
    // the case of the extension picks the format as the lower case does
    let csv = std::fs::read(root.join("members.CSV")).unwrap();
    assert_eq!(
        csv[..3],
        [0xEF, 0xBB, 0xBF],
        "a CSV after its byte order mark"
    );
    let xlsx = std::fs::read(root.join("members.XLSX")).unwrap();
    assert_eq!(xlsx[..4], *b"PK\x03\x04", "a workbook is a zip");
}

/// A message longer than a cell of a workbook holds (the XML of a forwarded chat, say) is cut to 32 000 characters
/// in a member's messages written as `.xlsx`, rather than failing the whole export (#1136).
#[test]
fn a_member_message_longer_than_a_cell_is_cut_in_the_workbook() {
    use weflow_native::fixture::{ContactSpec, MsgSpec, RoomSpec, SessionSpec, T0};
    let long = format!("wxid_bob:\n{}", "x".repeat(40_000));
    let (root, hub, _) = common::custom_hub("group-export-long", |f| {
        f.contact_db(
            &[
                ContactSpec::new("wxid_me", 1, "Me Nick"),
                ContactSpec::new("wxid_bob", 1, "Bob"),
                ContactSpec::new("room1@chatroom", 2, "Project Room"),
            ],
            &[RoomSpec {
                username: "room1@chatroom",
                owner: "wxid_bob",
                members: &[("wxid_me", ""), ("wxid_bob", "")],
            }],
        );
        f.session_db(&[SessionSpec {
            username: "room1@chatroom",
            summary: "forwarded",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 1,
        }]);
        f.message_shard(
            0,
            &[(
                "room1@chatroom",
                vec![MsgSpec::text(1, "wxid_bob", T0, &long)],
            )],
        );
    });
    let out = root.join("bob.xlsx");
    let res = hub
        .group_export_member_messages("room1@chatroom", "wxid_bob", &out, 0, 0)
        .unwrap();
    assert_eq!(res.count, 1);
    let mut book = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
    let mut strings = String::new();
    std::io::Read::read_to_string(
        &mut book.by_name("xl/sharedStrings.xml").unwrap(),
        &mut strings,
    )
    .unwrap();
    let longest = strings.split(|c| c != 'x').map(str::len).max().unwrap_or(0);
    assert_eq!(longest, 32_000, "the message is cut to 32 000 characters");
}

/// A reply's phrase and emojis are those of what it says, its title, and not of its XML, whose `<![CDATA[…]]>` is no
/// emoji (#1092).
#[test]
fn a_replys_phrase_and_emojis_come_from_its_text_not_its_xml() {
    use weflow_native::fixture::{ContactSpec, MsgSpec, RoomSpec, SessionSpec, T0};
    const REPLY: i64 = 244_813_135_921;
    let reply = concat!(
        "wxid_bob:\n<?xml version=\"1.0\"?><msg><appmsg appid=\"\" sdkver=\"0\">",
        "<title>好的[微笑]</title><des /><type>57</type>",
        "<appattach><cdnthumbaeskey><![CDATA[]]></cdnthumbaeskey></appattach>",
        "<refermsg><type>1</type><svrid>1001</svrid><fromusr>room1@chatroom</fromusr>",
        "<chatusr>wxid_me</chatusr><displayname>Me</displayname><content>明天见</content>",
        "<msgsource>&lt;msgsource&gt;&lt;silence&gt;&lt;![CDATA[0]]&gt;&lt;/silence&gt;&lt;/msgsource&gt;</msgsource>",
        "</refermsg></appmsg><fromusername>wxid_bob</fromusername>",
        "<appinfo><version>1</version><appname><![CDATA[null]]></appname></appinfo></msg>",
    );
    let (_root, hub, _f) = common::custom_hub("group-reply-emoji", |f| {
        f.contact_db(
            &[
                ContactSpec::new("wxid_me", 1, "Me"),
                ContactSpec::new("wxid_bob", 1, "Bob"),
                ContactSpec::new("room1@chatroom", 2, "Room"),
            ],
            &[RoomSpec {
                username: "room1@chatroom",
                owner: "wxid_bob",
                members: &[("wxid_me", ""), ("wxid_bob", "")],
            }],
        );
        f.session_db(&[SessionSpec {
            username: "room1@chatroom",
            summary: "",
            last_timestamp: T0 + 20,
            unread: 0,
            last_msg_type: 1,
        }]);
        f.message_shard(
            0,
            &[(
                "room1@chatroom",
                vec![
                    MsgSpec::text(1, "wxid_me", T0, "明天见"),
                    MsgSpec::text(2, "wxid_bob", T0 + 10, reply).of_type(REPLY),
                    // a bracket with a space or markup in it is no emoji
                    MsgSpec::text(
                        3,
                        "wxid_bob",
                        T0 + 20,
                        "wxid_bob:\n[微笑][微笑] [这个 链接] [<b>]",
                    ),
                ],
            )],
        );
    });
    let bob = serde_json::to_value(
        hub.group_member_analytics("room1@chatroom", "wxid_bob", 0, 0)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(bob["statistics"]["textMessages"], 2, "{bob}");
    assert_eq!(
        bob["commonEmojis"],
        serde_json::json!([{ "emoji": "[微笑]", "count": 3 }]),
        "the reply's emoji counts, the CDATA of its XML does not"
    );
    let phrases: Vec<&str> = bob["commonPhrases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["phrase"].as_str().unwrap())
        .collect();
    assert!(phrases.contains(&"好的[微笑]"), "{phrases:?}");
    assert!(!phrases.iter().any(|p| p.contains('<')), "{phrases:?}");
}

/// A page of a member's messages holds 1 to 100 of them: any other `limit` is a usage error (#1067).
#[test]
fn a_member_messages_limit_outside_1_to_100_is_a_usage_error() {
    let (_root, hub) = common::mock_hub("group-limit");
    let page =
        |limit: usize| hub.group_member_messages("room1@chatroom", "wxid_bob", 0, 0, limit, 0);
    for limit in [0, 101, 1000, usize::MAX] {
        let err = page(limit).unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.exit_code),
            ("invalid_arguments", 2),
            "limit {limit}"
        );
        assert_eq!(err.message, format!("invalid limit {limit}; use 1-100"));
    }
    // the ends of the range, and what is between them, answer the one message Bob has sent
    for limit in [1, 2, 50, 100] {
        let page = page(limit).unwrap();
        assert_eq!(page.messages.len(), 1, "limit {limit}");
        assert!(!page.has_more, "limit {limit}");
    }
    // the same through the typed read the command and the command API run
    let read = |limit: usize| {
        weflow_core::reads::Read::GroupMemberMessages {
            chatroom_id: "room1@chatroom".into(),
            username: "wxid_bob".into(),
            limit,
            cursor: 0,
            start: None,
            end: None,
        }
        .run(&hub)
    };
    assert_eq!(read(0).unwrap_err().code, "invalid_arguments");
    assert_eq!(read(101).unwrap_err().code, "invalid_arguments");
    assert_eq!(read(100).unwrap()["messages"].as_array().unwrap().len(), 1);
}
