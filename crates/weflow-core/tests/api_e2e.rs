mod common;

use weflow_core::api::Params;

fn params(pairs: &[(&str, &str)]) -> Params {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

#[test]
fn sessions_are_filtered_typed_and_named() {
    let (hub, _root) = common::mock_hub("api-sessions");
    let all = hub.api_sessions(&params(&[])).unwrap();
    let names: Vec<&str> = all["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["username"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"wxid_bob") && names.contains(&"room1@chatroom"));
    assert!(
        names.contains(&"gh_news"),
        "official accounts are appended from the contact list"
    );
    assert!(!names.contains(&"medianote"));
    let bob = all["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["username"] == "wxid_bob")
        .unwrap();
    assert_eq!(bob["displayName"], "Bobby");
    assert_eq!(bob["sessionType"], "private");
    assert_eq!(bob["unreadCount"], 2);
    let room = all["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["username"] == "room1@chatroom")
        .unwrap();
    assert_eq!(room["sessionType"], "group");
    assert_eq!(room["displayName"], "Project Room");

    let filtered = hub.api_sessions(&params(&[("keyword", "BOBBY")])).unwrap();
    assert_eq!(filtered["count"], 1);
    let limited = hub.api_sessions(&params(&[("limit", "1")])).unwrap();
    assert_eq!(limited["count"], 1);

    let chatlab = hub.api_sessions(&params(&[("format", "chatlab")])).unwrap();
    let first = &chatlab["sessions"][0];
    assert_eq!(first["platform"], "wechat");
    assert!(first["id"].is_string() && first["name"].is_string());
    let gh = chatlab["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "gh_news")
        .unwrap();
    assert_eq!(gh["type"], "channel");
}

#[test]
fn contacts_are_classified() {
    let (hub, _root) = common::mock_hub("api-contacts");
    let r = hub.api_contacts(&params(&[("limit", "50")])).unwrap();
    let list = r["contacts"].as_array().unwrap();
    let kind = |u: &str| {
        list.iter()
            .find(|c| c["username"] == u)
            .map(|c| c["type"].as_str().unwrap().to_string())
    };
    assert_eq!(kind("wxid_bob").as_deref(), Some("friend"));
    assert_eq!(kind("gh_news").as_deref(), Some("official"));
    assert_eq!(kind("room1@chatroom").as_deref(), Some("group"));
    assert_eq!(kind("medianote"), None);
    let bob = list.iter().find(|c| c["username"] == "wxid_bob").unwrap();
    assert_eq!(bob["displayName"], "Bobby");
    assert_eq!(bob["remark"], "Bobby");
    assert_eq!(bob["nickname"], "Bob");
    let kw = hub.api_contacts(&params(&[("keyword", "carol")])).unwrap();
    assert_eq!(kw["count"], 1);
}

#[test]
fn messages_endpoint_json_and_chatlab() {
    let (hub, _root) = common::mock_hub("api-messages");
    let rt = rt();
    let r = rt
        .block_on(hub.api_messages(
            &params(&[("talker", "wxid_bob"), ("limit", "2")]),
            "http://127.0.0.1:5031",
        ))
        .unwrap();
    assert_eq!(r["success"], true);
    assert_eq!(r["count"], 2);
    assert_eq!(r["hasMore"], true, "limit reached with rows left over");
    assert_eq!(r["messages"][0]["localId"], 5);
    assert_eq!(r["messages"][0]["content"], "see you");
    assert_eq!(r["messages"][0]["serverId"], "1005");
    assert_eq!(r["messages"][0]["senderUsername"], "wxid_bob");
    assert_eq!(
        r["messages"][1]["content"], "later, compressed",
        "zstd-compressed text from the second shard is readable"
    );
    assert_eq!(r["messages"][1]["isSend"], 1);
    assert_eq!(r["media"]["enabled"], false);

    let all = rt
        .block_on(hub.api_messages(
            &params(&[("talker", "wxid_bob"), ("limit", "100")]),
            "http://h",
        ))
        .unwrap();
    assert_eq!(all["count"], 5);
    assert_eq!(all["hasMore"], false);
    let skipped = rt
        .block_on(hub.api_messages(
            &params(&[("talker", "wxid_bob"), ("offset", "4")]),
            "http://h",
        ))
        .unwrap();
    assert_eq!(skipped["count"], 1);
    assert_eq!(skipped["messages"][0]["localId"], 1);

    let chatlab = rt
        .block_on(hub.api_messages(
            &params(&[("talker", "room1@chatroom"), ("format", "chatlab")]),
            "http://h",
        ))
        .unwrap();
    assert_eq!(chatlab["chatlab"]["version"], "0.0.2");
    assert_eq!(chatlab["meta"]["type"], "group");
    assert_eq!(chatlab["meta"]["groupId"], "room1@chatroom");
    assert_eq!(chatlab["meta"]["ownerId"], "wxid_me");
    let members = chatlab["members"].as_array().unwrap();
    assert!(members.iter().any(|m| m["platformId"] == "wxid_bob"
        && m["accountName"] == "Bobby"
        && m["groupNickname"] == "Bob in room"));
    let mine = members
        .iter()
        .find(|m| m["platformId"] == "wxid_me")
        .unwrap();
    assert_eq!(mine["accountName"], "我");
    let msgs = chatlab["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0]["type"], 0);
    assert_eq!(msgs[0]["content"], "ok");
    assert_eq!(msgs[0]["platformMessageId"], "1003");
    assert_eq!(msgs[2]["content"], "welcome");
}

#[test]
fn messages_endpoint_validates_input() {
    let (hub, _root) = common::mock_hub("api-errors");
    let rt = rt();
    let e = rt
        .block_on(hub.api_messages(&params(&[]), "http://h"))
        .unwrap_err();
    assert_eq!(
        (e.status, e.message.as_str()),
        (400, "Missing required parameter: talker")
    );
    let e = rt
        .block_on(hub.api_messages(&params(&[("talker", "a"), ("format", "xml")]), "http://h"))
        .unwrap_err();
    assert_eq!(
        (e.status, e.message.as_str()),
        (400, "Invalid format, supported: json/chatlab")
    );
    let e = hub.api_group_members(&params(&[])).unwrap_err();
    assert_eq!((e.status, e.message.as_str()), (400, "Missing chatroomId"));
}

#[test]
fn pull_endpoint_reports_sync_block() {
    let (hub, _root) = common::mock_hub("api-pull");
    let r = rt()
        .block_on(hub.api_pull_messages("wxid_bob", &params(&[("limit", "2")]), "http://h"))
        .unwrap();
    assert_eq!(r["messages"].as_array().unwrap().len(), 2);
    assert_eq!(r["sync"]["hasMore"], true);
    assert_eq!(r["sync"]["nextOffset"], 2);
    assert!(r["sync"]["nextSince"].is_i64());
    assert!(r["sync"]["watermark"].as_i64().unwrap() > 1_700_000_000);
    let all = rt()
        .block_on(hub.api_pull_messages("wxid_bob", &params(&[]), "http://h"))
        .unwrap();
    assert_eq!(all["sync"]["hasMore"], false);
    assert!(all["sync"].get("nextSince").is_none());
}

#[test]
fn group_members_endpoint() {
    let (hub, _root) = common::mock_hub("api-members");
    let r = hub
        .api_group_members(&params(&[
            ("chatroomId", "room1@chatroom"),
            ("includeMessageCounts", "1"),
        ]))
        .unwrap();
    assert_eq!(r["count"], 3);
    assert_eq!(r["fromCache"], false);
    assert_eq!(r["members"][0]["wxid"], "wxid_bob");
    assert_eq!(r["members"][0]["messageCount"], 1);
    assert_eq!(r["members"][0]["groupNickname"], "Bob in room");
    assert_eq!(r["members"][0]["isOwner"], true);
    let again = hub
        .api_group_members(&params(&[
            ("talker", "room1@chatroom"),
            ("withCounts", "true"),
        ]))
        .unwrap();
    assert_eq!(again["fromCache"], true);
}
