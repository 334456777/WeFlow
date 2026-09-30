#![cfg(target_os = "linux")]
mod common;

#[test]
fn members_panel_is_sorted_and_enriched() {
    let (hub, _root) = common::mock_hub("group-panel");
    let (entries, from_cache, _) = hub.group_members_panel("room1@chatroom", false, true).unwrap();
    assert!(!from_cache);
    let names: Vec<&str> = entries.iter().map(|e| e["username"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["wxid_bob", "wxid_me", "wxid_quiet"], "friends first, then by message count");
    assert_eq!(entries[0]["messageCount"], 30);
    assert_eq!(entries[0]["isFriend"], true);
    assert_eq!(entries[0]["groupNickname"], "Bob in room");
    assert_eq!(entries[0]["nickname"], "Bob");
    assert_eq!(entries[0]["displayName"], "Bobby");
    assert_eq!(entries[2]["isFriend"], false);
    assert_eq!(entries[2]["messageCount"], 5);
    let (_, cached, _) = hub.group_members_panel("room1@chatroom", false, true).unwrap();
    assert!(cached, "second call is served from the panel cache");
    let (without_counts, _, _) = hub.group_members_panel("room1@chatroom", true, false).unwrap();
    assert_eq!(without_counts[0]["messageCount"], 0);
    // the panel stored my own count as a hint
    assert_eq!(hub.get_group_hint("room1@chatroom").unwrap()["count"], 10);
}

#[test]
fn ranking_hours_and_media_mix() {
    let (hub, _root) = common::mock_hub("group-stats");
    let ranking = hub.group_message_ranking("room1@chatroom", 2, 0, 0).unwrap();
    assert_eq!(ranking.len(), 2);
    assert_eq!(ranking[0]["member"]["username"], "wxid_bob");
    assert_eq!(ranking[0]["member"]["displayName"], "Bobby");
    assert_eq!(ranking[0]["member"]["avatarUrl"], "https://example.com/bob.png");
    assert_eq!(ranking[0]["messageCount"], 30);
    assert_eq!(ranking[1]["member"]["username"], "wxid_me");

    let hours = hub.group_active_hours("room1@chatroom", 0, 0).unwrap();
    assert_eq!(hours["hourlyDistribution"]["21"], 7);
    assert_eq!(hours["hourlyDistribution"]["0"], 0);

    let media = hub.group_media_stats("room1@chatroom", 0, 0).unwrap();
    assert_eq!(media["total"], 44);
    let list = media["typeCounts"].as_array().unwrap();
    assert_eq!(list[0]["name"], "文本");
    assert_eq!(list.last().unwrap()["name"], "其他");
    assert_eq!(list.last().unwrap()["count"], 2);
}

#[test]
fn group_chats_and_full_members() {
    let (hub, _root) = common::mock_hub("group-list");
    let groups = hub.group_chats().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["username"], "room1@chatroom");
    assert_eq!(groups[0]["memberCount"], 3);
    let members = hub.group_members_full("room1@chatroom").unwrap();
    assert_eq!(members.len(), 3);
    assert_eq!(members[0]["groupNickname"], "Bob in room");
}

#[test]
fn member_analytics_and_exports() {
    let (hub, root) = common::mock_hub("group-export");
    let a = hub.group_member_analytics("room1@chatroom", "wxid_bob", 0, 0).unwrap();
    assert_eq!(a["statistics"]["totalMessages"], 1, "only Bob's message in the room counts");
    assert_eq!(a["statistics"]["textMessages"], 1);
    assert_eq!(a["statistics"]["firstMessageTime"], 1700000120);
    assert_eq!(a["statistics"]["activeDays"], 1);
    assert_eq!(a["timeDistribution"].as_object().unwrap().len(), 24);
    assert_eq!(a["commonPhrases"][0]["phrase"], "wxid_bob:third", "the sender prefix is only stripped before a newline");
    let me = hub.group_member_analytics("room1@chatroom", "wxid_me", 0, 0).unwrap();
    assert_eq!(me["statistics"]["totalMessages"], 1, "my own messages are matched through is_send");

    let csv = root.join("members.csv");
    let res = hub.group_export_members("room1@chatroom", &csv).unwrap();
    assert_eq!(res["count"], 3);
    let text = std::fs::read_to_string(&csv).unwrap();
    assert!(text.starts_with('\u{feff}'));
    assert!(text.contains("会话信息"));
    assert!(text.contains("wxid_bob"));
    assert!(text.contains("微信昵称,微信备注,群昵称,wxid,微信号"));

    let xlsx = root.join("members.xlsx");
    hub.group_export_members("room1@chatroom", &xlsx).unwrap();
    assert!(std::fs::metadata(&xlsx).unwrap().len() > 1000);

    let msgs = root.join("bob.csv");
    let res = hub.group_export_member_messages("room1@chatroom", "wxid_bob", &msgs, 0, 0).unwrap();
    assert_eq!(res["count"], 1);
    let csv_text = std::fs::read_to_string(&msgs).unwrap();
    assert!(csv_text.contains("序号,时间,发送者wxid,消息类型,内容"));
    assert!(csv_text.contains("wxid_bob") && csv_text.contains("文本") && csv_text.contains("third"), "{csv_text}");
    let page = hub.group_member_messages("room1@chatroom", "wxid_bob", 0, 0, 20, 0).unwrap();
    assert_eq!(page["hasMore"], false);
    assert_eq!(page["messages"].as_array().unwrap().len(), 1);
    assert_eq!(page["messages"][0]["parsedContent"], "third");
}
