//! Service-layer behaviour on the native database backend, against a synthetic account made of real
//! encrypted databases (`weflow_native::fixture::Fixture::standard`).

mod common;

use serde_json::{json, Value};
use weflow_core::export_msg::DisplayPref;
use weflow_core::services::MessageExportRequest;

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

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
fn connection_opens_the_encrypted_account() {
    let (hub, _root) = common::mock_hub("n-conn");
    let v = hub.db_test().unwrap();
    assert_eq!(v["connected"], true);
}

#[test]
fn a_wrong_key_is_reported_as_a_native_error() {
    let (hub, _root) = common::mock_hub_with("n-badkey", |_| {});
    // same account, different key
    let ctx = weflow_core::config::AppContext {
        home_dir: _root.join("home"),
        config_path: _root.join("home/config.json"),
        runtime_dir: _root.join("runtime"),
        version: "test".into(),
    };
    let bad = weflow_core::services::ServiceHub::new(
        ctx,
        weflow_core::config::ConfigStore::default(),
        None,
        Some(
            _root
                .join("data/wxid_me_ab12")
                .to_string_lossy()
                .to_string(),
        ),
        Some("11".repeat(32)),
        Some("wxid_me_ab12".into()),
    );
    let err = bad.db_test().unwrap_err();
    assert_eq!(err.code, "native_error");
    // ordinary commands check the key on the first database they read: same error, same exit code
    for err in [
        bad.contacts().unwrap_err(),
        bad.messages("wxid_bob", 10, 0).unwrap_err(),
        bad.sessions().unwrap_err(),
    ] {
        assert_eq!(err.code, "native_error");
        assert_eq!(err.exit_code, 4);
        assert!(err.message.contains("cannot decrypt"), "{}", err.message);
    }
    drop(hub);
}

#[test]
fn session_list_is_filtered_sorted_and_carries_unread_counts() {
    let (hub, _root) = common::mock_hub("n-sessions");
    let sessions = hub.chat_sessions_list().unwrap();
    let names: Vec<&str> = sessions
        .iter()
        .map(|s| s["username"].as_str().unwrap())
        .collect();
    // official accounts are not real conversations: their session row is replaced by the history placeholder
    assert_eq!(names, ["wxid_bob", "room1@chatroom", "gh_news"]);
    assert_eq!(sessions[0]["unreadCount"], 2);
    assert_eq!(sessions[0]["summary"], "see you");
    assert_ne!(sessions[2]["summary"], "daily");
}

#[test]
fn single_messages_are_normalised_across_shards_and_compression() {
    let (hub, _root) = common::mock_hub("n-single");
    let m = hub.chat_message_by_id("wxid_bob", 5).unwrap();
    assert_eq!(m["message"]["text"], "see you");
    assert_eq!(m["message"]["isSend"], 0);
    assert_eq!(m["message"]["senderUsername"], "wxid_bob");

    let mine = hub.chat_message_by_id("wxid_bob", 2).unwrap();
    assert_eq!(mine["message"]["isSend"], 1);
    assert_eq!(mine["message"]["senderUsername"], "wxid_me");

    // local_id 4 lives in shard 1 and is stored as a zstd blob
    let compressed = hub.chat_message_by_server_id("wxid_bob", "1004").unwrap();
    assert_eq!(compressed["message"]["text"], "later, compressed");
    assert!(
        hub.chat_message_by_id("wxid_bob", 99).is_err(),
        "unknown ids are an error"
    );
}

#[test]
fn dates_counts_statuses_and_tab_counts() {
    let (hub, _root) = common::mock_hub("n-dates");
    let dates = hub.chat_dates("wxid_bob").unwrap();
    let dates = dates["dates"].as_array().unwrap();
    assert_eq!(
        dates.len(),
        4,
        "five messages on four distinct days: {dates:?}"
    );

    let per_day = hub.chat_date_counts("wxid_bob").unwrap();
    let total: i64 = per_day["counts"]
        .as_object()
        .unwrap()
        .values()
        .map(|n| n.as_i64().unwrap())
        .sum();
    assert_eq!(total, 5);

    let counts = hub
        .chat_counts(&strs(&["wxid_bob", "room1@chatroom", "nobody"]))
        .unwrap();
    assert_eq!(
        (
            counts["wxid_bob"].as_i64(),
            counts["room1@chatroom"].as_i64(),
            counts["nobody"].as_i64()
        ),
        (Some(5), Some(3), Some(0))
    );

    let st = hub
        .chat_statuses(&strs(&["room1@chatroom", "wxid_bob"]))
        .unwrap();
    assert_eq!(
        st["room1@chatroom"],
        json!({"isFolded": true, "isMuted": true})
    );
    assert_eq!(st["wxid_bob"], json!({"isFolded": false, "isMuted": false}));

    let tabs = hub.chat_tab_counts().unwrap();
    assert_eq!(tabs["group"], 1);
}

#[test]
fn contacts_and_groups_resolve_names_owner_and_nicknames() {
    let (hub, _root) = common::mock_hub("n-contacts");
    let contact = hub.contact("wxid_bob").unwrap();
    assert_eq!(contact["nick_name"], "Bob");
    assert_eq!(contact["remark"], "Bobby");
    assert_eq!(hub.contacts().unwrap().as_array().unwrap().len(), 7);

    let members = hub.group_members_full("room1@chatroom").unwrap();
    let names: Vec<&str> = members
        .iter()
        .map(|m| m["username"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 3);
    for n in ["wxid_me", "wxid_bob", "wxid_quiet"] {
        assert!(names.contains(&n), "{n} missing from {names:?}");
    }
    let bob = members
        .iter()
        .find(|m| m["username"] == "wxid_bob")
        .unwrap();
    assert_eq!(bob["isOwner"], true, "{bob}");
    assert_eq!(bob["groupNickname"], "Bob in room", "{bob}");
}

#[test]
fn every_format_exports_five_messages_of_a_conversation() {
    let (hub, root) = common::mock_hub("n-formats");
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
        let result = hub
            .export_messages(&request("wxid_bob", format), &out)
            .unwrap_or_else(|e| panic!("{format}: {e}"));
        assert_eq!(result["count"], 5, "{format}");
        assert!(
            out.metadata().unwrap().len() > 100,
            "{format} output is empty"
        );
    }
}

#[test]
fn json_export_carries_names_senders_and_decoded_text() {
    let (hub, root) = common::mock_hub("n-json");
    let out = root.join("out/bob.json");
    hub.export_messages(&request("wxid_bob", "json"), &out)
        .unwrap();
    let doc: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let msgs = doc["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 5);
    let contents: Vec<&str> = msgs.iter().filter_map(|m| m["content"].as_str()).collect();
    assert!(
        contents.contains(&"hello") && contents.contains(&"later, compressed"),
        "{contents:?}"
    );
    let first = &msgs[0];
    assert_eq!(first["senderUsername"], "wxid_bob");
    assert_eq!(
        first["senderDisplayName"], "Bobby",
        "remark wins over nickname"
    );
    assert_eq!(first["isSend"], 0);
    let mine = msgs.iter().find(|m| m["content"] == "hi bob").unwrap();
    assert_eq!(mine["isSend"], 1);
    assert!(
        msgs.windows(2)
            .all(|w| w[0]["createTime"].as_i64() <= w[1]["createTime"].as_i64()),
        "oldest first"
    );
}

#[test]
fn group_export_uses_group_nicknames_and_strips_sender_prefixes() {
    let (hub, root) = common::mock_hub("n-group");
    let out = root.join("out/room.json");
    let mut req = request("room1@chatroom", "json");
    req.display_pref = DisplayPref::GroupNickname;
    hub.export_messages(&req, &out).unwrap();
    let doc: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let msgs = doc["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3);
    // WeChat stores group text as "<wxid>:\n<text>"; the export drops the "<wxid>:" part, as the desktop app does
    // (the newline that follows it is kept, hence the trim).
    let welcome = msgs
        .iter()
        .find(|m| m["content"].as_str().map(str::trim) == Some("welcome"))
        .expect("the sender prefix is stripped from group text");
    assert_eq!(welcome["senderUsername"], "wxid_bob");
    assert_eq!(welcome["senderDisplayName"], "Bob in room");
}

#[test]
fn date_range_and_sender_filters_apply() {
    let (hub, root) = common::mock_hub("n-filter");
    let mut req = request("wxid_bob", "txt");
    req.sender = Some("wxid_me".into());
    let out = root.join("out/mine.txt");
    let result = hub.export_messages(&req, &out).unwrap();
    assert_eq!(result["count"], 2, "only my two messages");
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(
        text.contains("hi bob") && text.contains("later, compressed") && !text.contains("hello"),
        "{text}"
    );
}

// ── statistics ──

#[test]
fn overall_statistics_count_private_chats_by_type_and_direction() {
    let (hub, _root) = common::mock_hub("n-overall");
    let s = hub.analytics_overall_statistics(false).unwrap();
    assert_eq!(
        s["totalMessages"], 5,
        "only private chats count: the group is excluded"
    );
    assert_eq!(
        (s["textMessages"].as_i64(), s["imageMessages"].as_i64()),
        (Some(4), Some(1))
    );
    assert_eq!(
        (s["sentMessages"].as_i64(), s["receivedMessages"].as_i64()),
        (Some(2), Some(3))
    );
    assert_eq!(s["firstMessageTime"], 1_700_000_000_i64);
    assert_eq!(s["lastMessageTime"], 1_700_000_000_i64 + 4 * 86_400);
    assert_eq!(s["messageTypeCounts"], json!({"1": 4, "3": 1}));
}

#[test]
fn chat_detail_lists_shard_tables_with_counts_and_times() {
    let (hub, _root) = common::mock_hub("n-detail");
    let d = hub.chat_detail("wxid_bob").unwrap();
    assert_eq!(d["displayName"], "Bobby");
    assert_eq!(d["avatarUrl"], "https://example.com/bob.png");
    assert_eq!(d["messageCount"], 5);
    assert_eq!(d["firstMessageTime"], 1_700_000_000_i64);
    assert_eq!(d["latestMessageTime"], 1_700_000_000_i64 + 4 * 86_400);
    let tables = d["messageTables"].as_array().unwrap();
    assert_eq!(tables.len(), 2);
    assert_eq!(
        (tables[0]["dbName"].as_str(), tables[0]["count"].as_i64()),
        (Some("message_0.db"), Some(3))
    );
    assert_eq!(
        (tables[1]["dbName"].as_str(), tables[1]["count"].as_i64()),
        (Some("message_1.db"), Some(2))
    );
}

#[test]
fn export_stats_report_type_counts_and_group_relations() {
    let (hub, _root) = common::mock_hub("n-exportstats");
    let s = hub
        .chat_export_stats(&strs(&["wxid_bob", "room1@chatroom"]), 0, 0, true)
        .unwrap();
    let bob = &s["data"]["wxid_bob"];
    assert_eq!(
        (
            bob["totalMessages"].as_i64(),
            bob["imageMessages"].as_i64(),
            bob["voiceMessages"].as_i64()
        ),
        (Some(5), Some(1), Some(0))
    );
    assert_eq!(bob["privateMutualGroups"], 1, "bob is a member of room1");
    let room = &s["data"]["room1@chatroom"];
    assert_eq!(room["totalMessages"], 3);
    assert_eq!(room["groupMyMessages"], 1);
    assert_eq!(room["groupActiveSpeakers"], 3);
    assert_eq!(room["groupMemberCount"], 3);
    assert_eq!(
        room["groupMutualFriends"], 1,
        "of bob/me/quiet only bob is a friend other than me"
    );
}

#[test]
fn group_ranking_hours_and_media_mix() {
    let (hub, _root) = common::mock_hub("n-groupstats");
    let ranking = hub
        .group_message_ranking("room1@chatroom", 10, 0, 0)
        .unwrap();
    assert_eq!(ranking.len(), 3);
    assert!(ranking.iter().all(|r| r["messageCount"] == 1));
    let names: Vec<&str> = ranking
        .iter()
        .map(|r| r["member"]["displayName"].as_str().unwrap())
        .collect();
    for n in ["Bobby", "Me Nick", "Quiet"] {
        assert!(names.contains(&n), "{n} missing from {names:?}");
    }
    let hours = hub.group_active_hours("room1@chatroom", 0, 0).unwrap();
    let total: i64 = hours["hourlyDistribution"]
        .as_object()
        .unwrap()
        .values()
        .map(|n| n.as_i64().unwrap())
        .sum();
    assert_eq!(total, 3);
    let media = hub.group_media_stats("room1@chatroom", 0, 0).unwrap();
    assert_eq!(media["total"], 3);
    assert_eq!(media["typeCounts"][0]["type"], 1);
}

#[test]
fn report_years_come_from_the_data() {
    let (hub, _root) = common::mock_hub("n-years");
    let y = hub.report_available_years().unwrap();
    assert_eq!(y["years"], json!([2023]));
    assert_eq!(y["strategy"], "native");
}

// ── Moments ──

#[test]
fn moments_timeline_is_enriched_with_names_avatars_and_fixed_urls() {
    let (hub, _root) = common::mock_hub("n-sns");
    let q = weflow_core::services::SnsTimelineQuery {
        limit: 10,
        offset: 0,
        usernames: vec![],
        keyword: None,
        start: 0,
        end: 0,
    };
    let posts = hub.sns_timeline_query(&q).unwrap();
    let descs: Vec<&str> = posts
        .iter()
        .map(|p| p["contentDesc"].as_str().unwrap())
        .collect();
    assert_eq!(
        descs,
        [
            "old style",
            "carol video",
            "second post",
            "my post",
            "bob trip"
        ]
    );

    let trip = posts
        .iter()
        .find(|p| p["contentDesc"] == "bob trip")
        .unwrap();
    assert_eq!(
        trip["nickname"], "Bobby",
        "remark wins, filled in from the contact book"
    );
    assert_eq!(trip["avatarUrl"], "https://example.com/bob.png");
    let media = trip["media"].as_array().unwrap();
    assert_eq!(media.len(), 2);
    assert_eq!(
        media[0]["url"], "https://url/1?token=TU1&idx=1",
        "http is upgraded and the token/idx are appended"
    );
    assert_eq!(media[0]["key"], "2");
    assert_eq!(trip["likes"], json!(["Me Nick", "Carol"]));
    // the unsigned snowflake that SQLite stores as a negative number
    assert_eq!(posts[0]["tid"], "18446744073709551611");

    let only_me = weflow_core::services::SnsTimelineQuery {
        usernames: vec!["wxid_me".into()],
        ..q.clone()
    };
    assert_eq!(hub.sns_timeline_query(&only_me).unwrap().len(), 2);
    let searched = weflow_core::services::SnsTimelineQuery {
        keyword: Some("video".into()),
        ..q
    };
    assert_eq!(hub.sns_timeline_query(&searched).unwrap().len(), 1);
}

#[test]
fn moments_users_and_export_stats() {
    let (hub, _root) = common::mock_hub("n-snsstats");
    assert_eq!(
        hub.sns_usernames_list().unwrap(),
        ["wxid_bob", "wxid_carol", "wxid_me"]
    );
    let s = hub.sns_export_stats(false).unwrap();
    assert_eq!(
        (
            s["totalPosts"].as_i64(),
            s["totalFriends"].as_i64(),
            s["myPosts"].as_i64()
        ),
        (Some(5), Some(3), Some(2))
    );
}

#[test]
fn operations_that_modify_wechat_databases_are_refused_clearly() {
    let (hub, _root) = common::mock_hub("n-readonly");
    for err in [
        hub.sns_block_delete_install().unwrap_err().into_message(),
        hub.sns_delete_post("1001").unwrap_err().into_message(),
        hub.chat_mark_all_read().unwrap_err().into_message(),
    ] {
        assert!(err.contains("read-only"), "{err}");
    }
}

// ── reports ──

#[test]
fn dual_report_combines_native_counts_with_service_side_details() {
    let (hub, _root) = common::mock_hub("n-dual");
    let d = hub.report_dual("wxid_bob", 0, &[]).unwrap();
    assert_eq!(
        (d["friendName"].as_str(), d["selfName"].as_str()),
        (Some("Bobby"), Some("Me Nick"))
    );
    let s = &d["stats"];
    // words: "hello" 5 + "hi bob" 6 + "later, compressed" 17 + "see you" 7; the image XML is not text
    assert_eq!(
        (
            s["totalMessages"].as_i64(),
            s["totalWords"].as_i64(),
            s["imageCount"].as_i64(),
            s["voiceCount"].as_i64(),
            s["emojiCount"].as_i64()
        ),
        (Some(5), Some(35), Some(1), Some(0), Some(0))
    );
    assert_eq!(
        d["initiative"],
        json!({"initiated": 1, "received": 3}),
        "conversations start at messages 1, 3, 4 and 5; only message 4 is mine"
    );
    assert_eq!(
        d["response"],
        json!({"avg": 60, "fastest": 60, "slowest": 60, "count": 1})
    );
    assert_eq!(d["streak"]["days"], 2);
    assert_eq!(d["firstChat"]["content"], "hello");
    assert_eq!(d["firstChatMessages"].as_array().unwrap().len(), 5);
    assert!(d["yearFirstChat"].is_null(), "year 0 means all years");
    let heat_total: i64 = d["heatmap"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r.as_array().unwrap())
        .map(|n| n.as_i64().unwrap())
        .sum();
    assert_eq!(heat_total, 5);
    assert_eq!(
        d["monthly"]
            .as_object()
            .unwrap()
            .values()
            .map(|n| n.as_i64().unwrap())
            .sum::<i64>(),
        5
    );
    assert_eq!(d["topPhrases"], json!([]), "nothing was typed twice");
}

#[test]
fn annual_report_uses_native_extras_and_native_moments() {
    let (hub, _root) = common::mock_hub("n-annual");
    let a = hub.report_annual(2023).unwrap();
    assert_eq!(
        (a["totalMessages"].as_i64(), a["totalFriends"].as_i64()),
        (Some(5), Some(1)),
        "private chats only"
    );
    let core = &a["coreFriends"][0];
    assert_eq!(
        (core["username"].as_str(), core["displayName"].as_str()),
        (Some("wxid_bob"), Some("Bobby"))
    );
    assert_eq!(
        (
            core["messageCount"].as_i64(),
            core["sentCount"].as_i64(),
            core["receivedCount"].as_i64()
        ),
        (Some(5), Some(2), Some(3))
    );
    assert_eq!(a["socialInitiative"]["initiatedChats"], 1);
    assert_eq!(a["socialInitiative"]["receivedChats"], 3);
    assert_eq!(a["longestStreak"]["days"], 2);
    assert_eq!(a["peakDay"]["messageCount"], 2);
    let november = &a["monthlyTopFriends"][10];
    assert_eq!(
        (
            november["month"].as_i64(),
            november["displayName"].as_str(),
            november["messageCount"].as_i64()
        ),
        (Some(11), Some("Bobby"), Some(5)),
        "the per-month ranking needs the native per-session monthly counts"
    );
    assert_eq!(a["monthlyTopFriends"][0]["messageCount"], 0);
    assert_eq!(
        a["snsStats"]["totalPosts"], 2,
        "the owner's own Moments posts"
    );
    assert_eq!(a["snsStats"]["topLikers"][0]["username"], "wxid_bob");
    // a year without messages is an empty report, not an error; year 0 covers everything
    assert_eq!(hub.report_annual(0).unwrap()["totalMessages"], 5);
}

#[test]
fn footprint_reports_private_chats_segments_and_mentions() {
    let (hub, _root) = common::mock_hub("n-footprint");
    let f = hub.footprint().unwrap();
    assert_eq!(
        f["summary"],
        json!({"private_inbound_people": 1, "private_replied_people": 1, "private_outbound_people": 1, "private_reply_rate": 1.0, "mention_count": 0, "mention_group_count": 0})
    );
    let bob = &f["private_sessions"][0];
    assert_eq!(
        (
            bob["incoming_count"].as_i64(),
            bob["outgoing_count"].as_i64(),
            bob["replied"].as_bool()
        ),
        (Some(3), Some(2), Some(true))
    );
    // messages 1+2 share a segment; 3, 4 and 5 are each more than an hour apart
    assert_eq!(f["private_segments"].as_array().unwrap().len(), 4);
    assert_eq!(f["private_segments"][0]["message_count"], 2);
    assert_eq!(f["diagnostics"]["truncated"], false);
    assert_eq!(
        hub.insight_footprint().unwrap()["summary"]["private_inbound_people"],
        1
    );
}

/// Protobuf bytes (length-delimited string fields) the way WeChat stores a contact's `extra_buffer`.
fn extra(fields: &[(u8, &str)]) -> &'static [u8] {
    let mut out = Vec::new();
    for (n, text) in fields {
        let tag = (*n as u32) << 3 | 2;
        if tag >= 0x80 {
            out.extend([(tag & 0x7f) as u8 | 0x80, (tag >> 7) as u8]);
        } else {
            out.push(tag as u8);
        }
        out.push(text.len() as u8);
        out.extend(text.as_bytes());
    }
    Box::leak(out.into_boxed_slice())
}

#[test]
fn contacts_carry_labels_signature_and_region_and_sort_by_pinyin() {
    use weflow_native::fixture::{ContactSpec, SessionSpec, T0};
    let (hub, _root, _f) = common::custom_hub("n-contact-extra", |f| {
        f.session_db(&[SessionSpec {
            username: "wxid_dummy",
            summary: "",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 1,
        }]);
        f.contact_db_with_labels(
            &[
                ContactSpec {
                    extra: extra(&[
                        (4, "hello sign"),
                        (5, "CN"),
                        (6, "Guangdong"),
                        (7, "Shenzhen"),
                        (30, "1,3"),
                    ]),
                    ..ContactSpec::new("wxid_zhang", 1, "张三")
                },
                ContactSpec {
                    extra: extra(&[(5, "US"), (6, "California")]),
                    ..ContactSpec::new("wxid_alice", 1, "Alice")
                },
                ContactSpec {
                    description: "my desc",
                    ..ContactSpec::new("wxid_li", 1, "李四")
                },
                ContactSpec::new("wxid_plain", 1, "Zed"),
            ],
            &[],
            &[(1, "Family"), (3, "Gym")],
        );
    });
    let list = hub.chat_contacts_list().unwrap();
    let names: Vec<&str> = list
        .iter()
        .map(|c| c["displayName"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["李四", "张三", "Alice", "Zed"],
        "no recent chat: pinyin order, Han before Latin"
    );
    let by = |u: &str| list.iter().find(|c| c["username"] == u).unwrap();
    let zhang = by("wxid_zhang");
    assert_eq!(zhang["labels"], json!(["Family", "Gym"]));
    assert_eq!(zhang["detailDescription"], "hello sign");
    assert_eq!(
        zhang["region"], "广东 深圳",
        "the domestic country is hidden and the names are Chinese"
    );
    assert_eq!(by("wxid_alice")["region"], "美国 California");
    assert_eq!(by("wxid_li")["detailDescription"], "my desc");
    let plain = by("wxid_plain");
    assert!(
        plain.get("labels").is_none()
            && plain.get("region").is_none()
            && plain.get("detailDescription").is_none()
    );
}

#[test]
fn clearing_account_data_removes_only_that_account_and_needs_a_scope() {
    let (hub, root) = common::mock_hub("n-clear");
    let base = hub.image_cache_root().parent().unwrap().to_path_buf();
    let put = |p: std::path::PathBuf| {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
        p
    };
    let mine = [
        put(base.join("Images/wxid_me/a.jpg")),
        put(base.join("Voices/wxid_me_ab12/v.wav")),
        put(base.join("wxid_me-moments.json")),
    ];
    let other = put(base.join("Emojis/wxid_other/e.gif"));
    let exports = root.join("exports");
    let mine_export = put(exports.join("wxid_me_chat.json"));
    let other_export = put(exports.join("someone_else.json"));

    assert!(
        hub.clear_current_account_data(false, &[]).is_err(),
        "nothing selected"
    );
    // what `cache clear-account` shows before asking is what goes
    let shown = hub
        .account_data_paths(true, std::slice::from_ref(&exports))
        .unwrap();
    let r = hub
        .clear_current_account_data(true, std::slice::from_ref(&exports))
        .unwrap();
    assert_eq!(r["profileReset"], true);
    assert_eq!(r["removedPaths"], json!(shown), "{r}");
    assert!(mine.iter().all(|p| !p.exists()), "{r}");
    assert!(!mine_export.exists());
    assert!(
        other.exists() && other_export.exists(),
        "other accounts and unrelated exports stay"
    );

    // exports only: caches stay, the profile is not reset
    let again = put(base.join("Images/wxid_me/b.jpg"));
    let r = hub.clear_current_account_data(false, &[exports]).unwrap();
    assert_eq!(r["profileReset"], false);
    assert!(again.exists());
}

#[test]
fn cache_parts_clear_on_their_own_and_all_together() {
    use weflow_core::services::CachePart;
    let (hub, _root) = common::mock_hub("n-cache-parts");
    let base = hub.image_cache_root().parent().unwrap().to_path_buf();
    let put = |p: std::path::PathBuf| {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"jpg").unwrap();
        p
    };
    let image = put(base.join("Images/wxid_bob/2023-11/x.jpg"));
    let rest = [
        put(base.join("Voices/v.wav")),
        put(base.join("Emojis/e.gif")),
        put(base.join("sns_cache/m.jpg")),
        put(base.join("analytics_cache.json")),
        put(base.join("api-media/s/x.jpg")),
        put(base.join("push-avatar-files/a.png")),
        put(hub.cache_part_paths(CachePart::Keys)[0].clone()),
    ];
    // the mock hub runs as version `test`: its runtime stays, other versions' go
    let current = put(hub.runtime_root().join("test/t/manifest.json"));
    let older = put(hub.runtime_root().join("2.0.0/t/manifest.json"));

    let list = hub.cache_list();
    let entry = |name: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|e| e["part"] == name)
            .unwrap()
            .clone()
    };
    assert_eq!(list.as_array().unwrap().len(), CachePart::ALL.len());
    assert_eq!(
        (
            entry("images")["bytes"].clone(),
            entry("images")["files"].clone()
        ),
        (json!(3), json!(1))
    );
    assert_eq!(entry("api")["files"], 2);
    assert_eq!(
        entry("runtime")["paths"],
        json!([hub.runtime_root().join("2.0.0")])
    );

    let r = hub.cache_clear(&[CachePart::Images]);
    assert_eq!(r["freedBytes"], 3, "{r}");
    assert!(!image.exists());
    assert!(rest.iter().all(|p| p.exists()) && older.exists());

    let r = hub.cache_clear(&CachePart::ALL);
    assert!(r.get("warnings").is_none(), "{r}");
    assert_eq!(r["freedBytes"], 3 * (rest.len() + 1), "{r}");
    assert!(rest.iter().all(|p| !p.exists()), "{r}");
    assert!(!older.exists());
    assert!(current.exists());
}
