mod common;

#[test]
fn members_panel_is_sorted_and_enriched() {
    let (hub, _root) = common::mock_hub("group-panel");
    let (entries, from_cache, _) = hub
        .group_members_panel("room1@chatroom", false, true)
        .unwrap();
    assert!(!from_cache);
    let names: Vec<&str> = entries
        .iter()
        .map(|e| e["username"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["wxid_bob", "wxid_me", "wxid_quiet"],
        "friends first (bob, me), then non-friends"
    );
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
    let (_, cached, _) = hub
        .group_members_panel("room1@chatroom", false, true)
        .unwrap();
    assert!(cached, "second call is served from the panel cache");
    let (without_counts, _, _) = hub
        .group_members_panel("room1@chatroom", true, false)
        .unwrap();
    assert_eq!(without_counts[0]["messageCount"], 0);
    // the panel stored my own count as a hint
    assert_eq!(hub.get_group_hint("room1@chatroom").unwrap()["count"], 1);
}

#[test]
fn group_chats_and_full_members() {
    let (hub, _root) = common::mock_hub("group-list");
    let groups = hub.group_chats().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["username"], "room1@chatroom");
    assert_eq!(groups[0]["displayName"], "Project Room");
    assert_eq!(groups[0]["memberCount"], 3);
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
    let (hub, root) = common::mock_hub("group-export");
    let a = hub
        .group_member_analytics("room1@chatroom", "wxid_bob", 0, 0)
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
    let me = hub
        .group_member_analytics("room1@chatroom", "wxid_me", 0, 0)
        .unwrap();
    assert_eq!(
        me["statistics"]["totalMessages"], 1,
        "my own messages are matched through is_send"
    );
    assert_eq!(me["commonPhrases"][0]["phrase"], "thanks");

    let csv = root.join("members.csv");
    let res = hub.group_export_members("room1@chatroom", &csv).unwrap();
    assert_eq!(res["count"], 3);
    let text = std::fs::read_to_string(&csv).unwrap();
    assert!(text.starts_with('\u{feff}'));
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
    assert_eq!(res["count"], 1);
    let csv_text = std::fs::read_to_string(&msgs).unwrap();
    assert!(
        csv_text.contains("序号,时间,发送者wxid,消息类型,内容"),
        "{csv_text}"
    );
    assert!(
        csv_text.contains("wxid_bob") && csv_text.contains("文本") && csv_text.contains("welcome"),
        "{csv_text}"
    );
    let page = hub
        .group_member_messages("room1@chatroom", "wxid_bob", 0, 0, 20, 0)
        .unwrap();
    assert_eq!(page["hasMore"], false);
    assert_eq!(page["messages"].as_array().unwrap().len(), 1);
    assert_eq!(page["messages"][0]["parsedContent"], "welcome");
    assert_eq!(page["messages"][0]["senderUsername"], "wxid_bob");
}
