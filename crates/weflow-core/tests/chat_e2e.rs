mod common;

use serde_json::json;
use weflow_core::services::ResourceQuery;
use weflow_native::fixture::{ContactSpec, Fixture, MsgSpec, SessionSpec, VoiceSpec, T0};

fn session(username: &'static str) -> SessionSpec {
    SessionSpec {
        username,
        summary: "",
        last_timestamp: T0,
        unread: 0,
        last_msg_type: 3,
    }
}

const MD5_A: &str = "aabbccddeeff00112233445566778899";
const MD5_B: &str = "11223344556677889900aabbccddeeff";

fn image(local_id: i64, at: i64, md5: &str) -> MsgSpec {
    MsgSpec::text(
        local_id,
        "wxid_bob",
        T0 + at,
        &format!("<msg><img md5=\"{md5}\"/></msg>"),
    )
    .of_type(3)
}

fn images_world(f: &Fixture) {
    f.session_db(&[session("wxid_bob")]);
    f.contact_db(
        &[ContactSpec {
            remark: "Bobby",
            ..ContactSpec::new("wxid_bob", 1, "Bob")
        }],
        &[],
    );
    // the same image row sits in both shards (a re-synced shard), plus a newer one only in shard 1
    f.message_shard(0, &[("wxid_bob", vec![image(1, 0, MD5_A)])]);
    f.message_shard(
        1,
        &[("wxid_bob", vec![image(1, 0, MD5_A), image(2, 100, MD5_B)])],
    );
}

#[test]
fn resources_and_images_are_deduplicated() {
    let (hub, _root, _f) = common::custom_hub("chat-res", images_world);
    let images = serde_json::to_value(hub.chat_all_images("wxid_bob").unwrap()).unwrap();
    let list = images["images"].as_array().unwrap();
    assert_eq!(
        list.len(),
        2,
        "the duplicated row (same ids and time) must collapse"
    );
    assert_eq!(list[0]["imageMd5"], MD5_B, "newest first");
    assert_eq!(list[1]["imageMd5"], MD5_A);

    let q = ResourceQuery {
        session_id: Some("wxid_bob".into()),
        types: vec!["image".into()],
        limit: 10,
        ..Default::default()
    };
    let r = hub.chat_resources(&q).unwrap();
    assert_eq!(r["items"].as_array().unwrap().len(), 2);
    assert_eq!(r["hasMore"], false);
    assert_eq!(r["nextCursor"], serde_json::Value::Null);
    assert_eq!(r["items"][0]["sessionDisplayName"], "Bobby");
    assert_eq!(r["items"][0]["resourceType"], "image");

    let paged = hub
        .chat_resources(&ResourceQuery {
            limit: 1,
            ..q.clone()
        })
        .unwrap();
    assert_eq!(paged["items"].as_array().unwrap().len(), 1);
    assert_eq!(paged["hasMore"], true);
    // the page after ends in the row both shards keep: it is listed once, and nothing comes after it
    let rest = hub
        .chat_resources(&ResourceQuery {
            limit: 1,
            before: paged["nextCursor"].as_str().map(str::to_string),
            ..q.clone()
        })
        .unwrap();
    assert_eq!(rest["items"].as_array().unwrap().len(), 1);
    assert_eq!(rest["items"][0]["localId"], 1);
    assert_eq!(rest["hasMore"], false);
}

fn app(local_id: i64, at: i64, xml_type: u32) -> MsgSpec {
    MsgSpec::text(
        local_id,
        "wxid_bob",
        T0 + at,
        &format!("<msg><appmsg><title>f{local_id}</title><type>{xml_type}</type></appmsg></msg>"),
    )
    .of_type(49)
}

fn resources_world(f: &Fixture) {
    f.session_db(&[
        SessionSpec {
            last_timestamp: T0 + 300,
            ..session("wxid_bob")
        },
        SessionSpec {
            last_timestamp: T0 + 250,
            ..session("wxid_amy")
        },
        SessionSpec {
            last_timestamp: T0 + 50,
            ..session("room@chatroom")
        },
    ]);
    f.message_shard(
        0,
        &[
            (
                "wxid_bob",
                vec![
                    image(1, 100, MD5_A),
                    image(2, 200, MD5_B),
                    app(3, 150, 6),
                    // links are not files, and there are more of them than a page: they are read past
                    app(4, 160, 5),
                    app(5, 170, 5),
                    app(6, 180, 5),
                    MsgSpec::text(7, "wxid_bob", T0 + 300, "<msg><videomsg/></msg>").of_type(43),
                ],
            ),
            (
                "wxid_amy",
                vec![
                    // the same place as Bob's picture: the conversations are listed by id
                    image(2, 200, MD5_A),
                    MsgSpec::text(3, "wxid_amy", T0 + 250, "<msg><voicemsg/></msg>").of_type(34),
                    app(4, 120, 6),
                ],
            ),
            ("room@chatroom", vec![image(1, 50, MD5_A)]),
        ],
    );
    f.message_shard(1, &[("wxid_bob", vec![image(1, 100, MD5_A)])]);
}

fn listed(r: &serde_json::Value) -> Vec<String> {
    r["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| format!("{}#{}", i["sessionId"].as_str().unwrap(), i["localId"]))
        .collect()
}

#[test]
fn resources_page_across_conversations_from_a_cursor() {
    let (hub, _root, _f) = common::custom_hub("chat-res-pages", resources_world);
    let all = hub
        .chat_resources(&ResourceQuery {
            limit: 100,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        listed(&all),
        [
            "wxid_bob#7",
            "wxid_amy#3",
            "wxid_amy#2",
            "wxid_bob#2",
            "wxid_bob#3",
            "wxid_amy#4",
            "wxid_bob#1",
            "room@chatroom#1",
        ]
    );
    assert_eq!(all["hasMore"], false);
    for limit in 1..=3 {
        let mut got = Vec::new();
        let mut before = None;
        loop {
            let page = hub
                .chat_resources(&ResourceQuery {
                    limit,
                    before: before.clone(),
                    ..Default::default()
                })
                .unwrap();
            got.extend(listed(&page));
            if page["hasMore"] == false {
                assert_eq!(page["nextCursor"], serde_json::Value::Null);
                break;
            }
            let last = page["items"].as_array().unwrap().last().unwrap();
            assert_eq!(page["nextCursor"], last["_cursor"]);
            before = page["nextCursor"].as_str().map(str::to_string);
        }
        assert_eq!(got, listed(&all), "pages of {limit}");
    }

    let files = hub
        .chat_resources(&ResourceQuery {
            types: vec!["file".into()],
            limit: 1,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(listed(&files), ["wxid_bob#3"]);
    assert_eq!(files["hasMore"], true);
    let ranged = hub
        .chat_resources(&ResourceQuery {
            begin: T0 + 110,
            end: T0 + 200,
            limit: 100,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        listed(&ranged),
        ["wxid_amy#2", "wxid_bob#2", "wxid_bob#3", "wxid_amy#4"]
    );

    let err = hub
        .chat_resources(&ResourceQuery {
            before: Some("1.2.3".into()),
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(err.code, "invalid_arguments");
}

#[test]
fn anti_revoke_sessions_skip_official_accounts() {
    let (hub, _root) = common::mock_hub("chat-anti");
    let v = hub.chat_anti_revoke_sessions().unwrap();
    assert!(v
        .as_array()
        .unwrap()
        .iter()
        .all(|s| !s["username"].as_str().unwrap_or("").starts_with("gh_")));
}

#[test]
fn group_hint_roundtrip_and_transfer_names() {
    let (hub, _root) = common::mock_hub("chat-hint");
    assert_eq!(
        serde_json::to_value(hub.group_hint("room1@chatroom").unwrap()).unwrap(),
        json!({})
    );
    hub.set_group_hint("room1@chatroom", 9).unwrap();
    let h = hub.group_hint("room1@chatroom").unwrap();
    assert_eq!((h.count, h.source.as_deref()), (Some(9), Some("disk")));
    let t = hub
        .chat_transfer_names("room1@chatroom", "wxid_bob", "wxid_me")
        .unwrap();
    assert_eq!(t.payer_name, "Bob in room");
}

#[test]
fn voice_messages_decode_from_silk_to_a_cached_wav() {
    let (created, server) = (1_700_000_100_i64, 9_007_199_254_740_993_i64);
    let (hub, _root, _f) = common::custom_hub("voice", |f| {
        f.session_db(&[session("wxid_bob")]);
        // a real SILK stream made by the vendored encoder: one second of a 440 Hz tone
        let silk = weflow_silk::testenc::encode_tone(24000);
        f.media_db(
            0,
            &[VoiceSpec {
                chat: "wxid_bob",
                create_time: created,
                local_id: 5,
                svr_id: server,
                data: silk,
                index: "0",
            }],
        );
    });
    // strong key (createTime + serverId): no localId lookup needed
    let wav = hub
        .voice_data(
            "wxid_bob",
            "5",
            Some(created),
            Some("9007199254740993"),
            Some("wxid_bob"),
        )
        .unwrap();
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 24000);
    let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
    assert_eq!(wav.len(), 44 + data_len);
    assert!(
        data_len > 2 * 24000 / 2,
        "roughly a second of 16-bit audio, got {data_len} bytes"
    );
    let samples: Vec<i16> = wav[44..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    assert!(
        samples.iter().any(|s| s.abs() > 2000),
        "decoded samples carry the tone"
    );

    // cached under <cache>/Voices/<session>_<createTime>_<localId>.wav and served from there next time
    let key_file = hub.voice_cache_dir().join("wxid_bob_1700000100_5.wav");
    assert!(key_file.exists(), "{}", key_file.display());
    std::fs::write(&key_file, b"cached-wav").unwrap();
    assert_eq!(
        hub.voice_data("wxid_bob", "5", Some(created), Some("1"), None)
            .unwrap(),
        b"cached-wav"
    );

    // the key without a timestamp is what `resolveVoiceCache` looks at
    assert!(!hub.voice_resolve_cache("wxid_bob", "5").has_cache);
    assert!(hub.voice_data("wxid_bob", "abc", None, None, None).is_err());
    // a voice that is not in the media database is an error that explains what to do
    let missing = hub
        .voice_data(
            "wxid_bob",
            "99",
            Some(created + 500),
            Some("4242"),
            Some("wxid_bob"),
        )
        .unwrap_err();
    assert!(
        missing.message.contains("voice data not found"),
        "{}",
        missing.message
    );

    let prepared = hub
        .voice_preload(
            "wxid_bob",
            &[json!({"localId": 9, "createTime": 1700000300, "serverId": "77"})],
        )
        .unwrap();
    assert!(prepared.success);
}

#[test]
fn parsed_pages_keep_the_cursors_and_parse_each_kind() {
    let (hub, _root, _f) = common::custom_hub("parsed", |f| {
        f.session_db(&[session("wxid_bob")]);
        f.message_shard(
            0,
            &[(
                "wxid_bob",
                vec![
                    MsgSpec::text(1, "wxid_bob", T0 + 10, "hello"),
                    image(2, 20, MD5_A),
                    MsgSpec::text(
                        3,
                        "wxid_bob",
                        T0 + 30,
                        "<msg><appmsg><title>Trip plan.pdf</title><type>6</type>\
                         <appattach><totallen>2048</totallen><fileext>pdf</fileext></appattach></appmsg></msg>",
                    )
                    .of_type(49),
                ],
            )],
        );
    });
    let raw = hub.message_page("wxid_bob", 10, 0, None, None).unwrap();
    let parsed = hub.parsed_message_page(raw.clone());
    let rows = parsed.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    // the same rows, newest first, each with the cursor of its raw row
    for (p, r) in rows.iter().zip(raw.as_array().unwrap()) {
        assert_eq!(p["_cursor"], r["_cursor"]);
        assert_eq!(p["localId"], r["local_id"]);
    }
    assert_eq!(rows[0]["localType"], 49);
    assert_eq!(rows[0]["fileName"], "Trip plan.pdf");
    assert_eq!(rows[0]["fileSize"], 2048);
    assert_eq!(rows[1]["localType"], 3);
    assert_eq!(rows[1]["imageMd5"], MD5_A);
    assert_eq!(rows[2]["parsedContent"], "hello");
    assert_eq!(rows[2]["senderUsername"], "wxid_bob");
    // the next page from a parsed row's cursor
    let older = hub
        .message_page("wxid_bob", 10, 0, rows[1]["_cursor"].as_str(), None)
        .unwrap();
    assert_eq!(older.as_array().unwrap().len(), 1);
}

/// (local_id, database) of each row of a page.
fn places(page: &serde_json::Value) -> Vec<(i64, String)> {
    page.as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["local_id"].as_i64().unwrap(),
                r["db_name"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn cursor_of(page: &serde_json::Value, i: usize) -> String {
    page[i]["_cursor"].as_str().unwrap().to_string()
}

#[test]
fn pages_from_a_cursor_follow_the_offset_order() {
    let (hub, _root, _f) = common::custom_hub("cursor", |f| {
        f.session_db(&[session("wxid_bob")]);
        let msg = |id: i64, at: i64| MsgSpec::text(id, "wxid_bob", at, &format!("m{id}"));
        f.message_shard(
            0,
            &[("wxid_bob", (1..=12).map(|i| msg(i, T0 + i * 10)).collect())],
        );
        f.message_shard(
            1,
            &[(
                "wxid_bob",
                vec![
                    // the very key of the first database's message 5
                    msg(5, T0 + 50),
                    msg(13, T0 + 130),
                    // without a sort_seq: keyed by create_time * 1000
                    msg(14, T0 + 140).with_sort_seq(0),
                    msg(15, T0 + 135).with_sort_seq(0),
                    msg(16, T0 + 160),
                    msg(17, T0 + 170).with_sort_seq((T0 + 170) * 1000 + 1),
                ],
            )],
        );
    });
    let page = |limit: i32, offset: i32, before: Option<&str>, after: Option<&str>| {
        hub.message_page("wxid_bob", limit, offset, before, after)
            .unwrap()
    };
    let all = page(0, 0, None, None);
    let order = places(&all);
    assert_eq!(order.len(), 18);
    // equal keys: the first database's message comes first
    let fives: Vec<usize> = (0..order.len()).filter(|i| order[*i].0 == 5).collect();
    assert_eq!(fives.len(), 2);
    assert_eq!(
        (&order[fives[0]].1[..], &order[fives[1]].1[..]),
        ("message_0", "message_1")
    );

    // walking back from the first page, 4 at a time, lists every message once, in order
    let mut walked = places(&page(4, 0, None, None));
    let mut last = page(4, 0, None, None);
    loop {
        let next = page(
            4,
            0,
            Some(&cursor_of(&last, 3.min(last.as_array().unwrap().len() - 1))),
            None,
        );
        if next.as_array().unwrap().is_empty() {
            break;
        }
        walked.extend(places(&next));
        last = next;
    }
    assert_eq!(walked, order);

    // and forward from the oldest one, 4 at a time, each page newest first
    let mut forward: Vec<(i64, String)> = Vec::new();
    let mut from = cursor_of(&all, order.len() - 1);
    loop {
        let next = page(4, 0, None, Some(&from));
        let got = places(&next);
        if got.is_empty() {
            break;
        }
        forward.splice(0..0, got);
        from = cursor_of(&next, 0);
    }
    assert_eq!(forward[..], order[..order.len() - 1]);

    // a page from a cursor is the page at the same offset
    for at in [1, 4, 8, 11, 15] {
        assert_eq!(
            places(&page(5, 0, Some(&cursor_of(&all, at - 1)), None)),
            places(&page(5, at as i32, None, None)),
            "after row {at}"
        );
    }
    // every row has a cursor, also `chat latest`'s
    assert!(hub.message_page("wxid_bob", 2, 0, None, None).unwrap()[1]["_cursor"].is_string());
    let err = hub
        .message_page("wxid_bob", 2, 0, Some("1.2.3"), None)
        .unwrap_err();
    assert_eq!(err.code, "invalid_arguments");
}

#[test]
fn a_message_looked_up_parsed_is_the_one_of_its_page() {
    // a media list knows a message by its local id and time; opening it there needs the page's `_cursor` (#200)
    let (hub, _root, _f) = common::custom_hub("lookup-parsed", |f| {
        f.session_db(&[session("wxid_bob")]);
        let msg = |id: i64, at: i64, text: &str| MsgSpec::text(id, "wxid_bob", at, text);
        f.message_shard(
            0,
            &[("wxid_bob", vec![msg(1, T0, "one"), msg(2, T0 + 10, "two")])],
        );
        f.message_shard(1, &[("wxid_bob", vec![msg(2, T0 + 20, "other two")])]);
    });
    let page = hub.parsed_message_page(hub.message_page("wxid_bob", 10, 0, None, None).unwrap());
    let in_page = |text: &str| {
        page.as_array()
            .unwrap()
            .iter()
            .find(|m| m["parsedContent"] == text)
            .unwrap()
            .clone()
    };
    for (at, text) in [(T0 + 10, "two"), (T0 + 20, "other two")] {
        let found = hub.parsed_lookup(hub.chat_message_by_id("wxid_bob", 2, Some(at)).unwrap());
        assert_eq!(found, in_page(text), "{text}");
        assert!(found["_cursor"].is_string());
    }
    assert!(hub.chat_message_by_id("wxid_bob", 9999, None).is_err());
    assert!(hub
        .parsed_lookup(serde_json::json!({ "row": null }))
        .is_null());
}

#[test]
fn a_page_since_a_time_follows_the_chat_order_where_it_is_off_the_send_time() {
    // WeChat orders a chat by sort_seq, the send time in milliseconds plus a little; a message may be ordered a
    // second or two off its own send time (issue #242)
    let t = T0 + 100;
    let (hub, _root, _f) = common::custom_hub("since-drift", |f| {
        f.session_db(&[session("wxid_bob")]);
        let msg = |id: i64, at: i64| MsgSpec::text(id, "wxid_bob", at, &format!("m{id}"));
        f.message_shard(
            0,
            &[(
                "wxid_bob",
                vec![
                    msg(1, t - 10),
                    // sent before t, ordered after it
                    msg(2, t - 1).with_sort_seq(t * 1000 + 1),
                    // sent at t, ordered before it
                    msg(3, t).with_sort_seq(t * 1000 - 1),
                    msg(4, t + 10),
                ],
            )],
        );
        f.message_shard(1, &[("wxid_bob", vec![msg(5, t + 20), msg(6, t - 20)])]);
    });
    let ids = |page: &serde_json::Value| -> Vec<i64> {
        page.as_array()
            .unwrap()
            .iter()
            .map(|r| r["local_id"].as_i64().unwrap())
            .collect()
    };
    let newest_first = ids(&hub.message_page("wxid_bob", 10, 0, None, None).unwrap());
    assert_eq!(newest_first, vec![5, 4, 2, 3, 1, 6], "the chat order");
    // the page starts where t falls in that order: 2 opens it, 3 sits right above it
    let since = hub.message_page_since("wxid_bob", 10, t).unwrap();
    assert_eq!(ids(&since), vec![5, 4, 2]);
    // pages on both ways join it without a gap or a repeat, in small pages too
    let mut seen = ids(&since);
    let mut cursor = since[2]["_cursor"].as_str().unwrap().to_string();
    loop {
        let older = hub
            .message_page("wxid_bob", 1, 0, Some(&cursor), None)
            .unwrap();
        let Some(last) = older.as_array().unwrap().last() else {
            break;
        };
        cursor = last["_cursor"].as_str().unwrap().to_string();
        seen.extend(ids(&older));
    }
    assert_eq!(seen, newest_first);
    let small = hub.message_page_since("wxid_bob", 1, t).unwrap();
    assert_eq!(ids(&small), vec![2]);
    let newer = hub
        .message_page("wxid_bob", 10, 0, None, small[0]["_cursor"].as_str())
        .unwrap();
    assert_eq!(ids(&newer), vec![5, 4]);
    // a moment exactly at an order position: that message opens the page
    assert_eq!(
        ids(&hub.message_page_since("wxid_bob", 1, t + 10).unwrap()),
        vec![4]
    );
}

#[test]
fn a_page_since_a_time_starts_at_its_first_message_and_hits_page_from_theirs() {
    let (hub, _root, _f) = common::custom_hub("since", |f| {
        f.session_db(&[session("wxid_bob")]);
        let msg = |id: i64, at: i64, text: &str| MsgSpec::text(id, "wxid_bob", at, text);
        f.message_shard(
            0,
            &[(
                "wxid_bob",
                (1..=8)
                    .map(|i| msg(i, T0 + i * 10, &format!("m{i}")))
                    .collect(),
            )],
        );
        f.message_shard(
            1,
            &[(
                "wxid_bob",
                vec![
                    // the same second as message 4 of the first database, and one without a sort_seq
                    msg(9, T0 + 40, "m9 trip"),
                    msg(10, T0 + 55, "m10 trip").with_sort_seq(0),
                ],
            )],
        );
    });
    let ids = |page: &serde_json::Value| -> Vec<i64> {
        page.as_array()
            .unwrap()
            .iter()
            .map(|r| r["local_id"].as_i64().unwrap())
            .collect()
    };
    // the earliest messages sent at T0 + 40 or later, newest first: both of that second, then the next ones
    let since = hub.message_page_since("wxid_bob", 4, T0 + 40).unwrap();
    assert_eq!(ids(&since), vec![10, 5, 9, 4]);
    // between two messages: the next one is the first
    assert_eq!(
        ids(&hub.message_page_since("wxid_bob", 2, T0 + 51).unwrap()),
        vec![6, 10]
    );
    // after the last message: nothing; before the first: the oldest
    assert!(ids(&hub.message_page_since("wxid_bob", 5, T0 + 81).unwrap()).is_empty());
    assert_eq!(
        ids(&hub.message_page_since("wxid_bob", 1, T0).unwrap()),
        vec![1]
    );
    // the page pages on both ways from its own cursors
    let older = hub
        .message_page("wxid_bob", 10, 0, since[3]["_cursor"].as_str(), None)
        .unwrap();
    assert_eq!(ids(&older), vec![3, 2, 1]);
    let newer = hub
        .message_page("wxid_bob", 10, 0, None, since[0]["_cursor"].as_str())
        .unwrap();
    assert_eq!(ids(&newer), vec![8, 7, 6]);

    // a hit carries its message's cursor and conversation, parsed or not
    let hits = hub.search("trip", Some("wxid_bob"), 10, 0, 0, 0).unwrap();
    assert_eq!(ids(&hits), vec![10, 9]);
    let parsed = hub.parsed_message_page(hits.clone());
    for (p, h) in parsed
        .as_array()
        .unwrap()
        .iter()
        .zip(hits.as_array().unwrap())
    {
        assert!(h["_cursor"].is_string());
        assert_eq!(p["_cursor"], h["_cursor"]);
        assert_eq!(p["_session_id"], "wxid_bob");
    }
    assert_eq!(parsed[1]["parsedContent"], "m9 trip");
    // the messages around a hit: older from its cursor, newer from it too
    let around = hub
        .message_page("wxid_bob", 2, 0, None, hits[1]["_cursor"].as_str())
        .unwrap();
    assert_eq!(ids(&around), vec![10, 5]);
}
