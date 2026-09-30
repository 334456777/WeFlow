#![cfg(target_os = "linux")]
mod common;

use serde_json::json;
use weflow_core::services::ResourceQuery;

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn single_message_lookups_normalize_rows() {
    let (hub, _root) = common::mock_hub("chat-single");
    let by_id = hub.chat_message_by_id("wxid_bob", 7).unwrap();
    assert_eq!(by_id["message"]["localId"], 7);
    assert_eq!(by_id["message"]["serverId"], "9007199254740993");
    assert_eq!(by_id["message"]["text"], "found by id");
    let by_svr = hub.chat_message_by_server_id("wxid_bob", "777").unwrap();
    assert_eq!(by_svr["message"]["isSend"], 1);
    assert_eq!(by_svr["message"]["senderUsername"], "wxid_me");
}

#[test]
fn date_and_count_queries() {
    let (hub, _root) = common::mock_hub("chat-dates");
    assert_eq!(hub.chat_dates("wxid_bob").unwrap()["dates"], json!(["2024-01-01", "2024-01-02"]));
    // days with a zero count are dropped
    assert_eq!(hub.chat_date_counts("wxid_bob").unwrap()["counts"], json!({"2024-01-01": 3, "2024-01-03": 5}));
    let counts = hub.chat_counts(&strs(&["wxid_bob", "room1@chatroom"])).unwrap();
    assert_eq!(counts["wxid_bob"], 120);
    assert_eq!(counts["room1@chatroom"], 55);
    assert_eq!(hub.chat_statuses(&strs(&["wxid_bob"])).unwrap()["wxid_bob"]["isFolded"], true);
    assert_eq!(hub.chat_tab_counts().unwrap()["group"], 1);
}

#[test]
fn detail_aggregates_table_stats() {
    let (hub, _root) = common::mock_hub("chat-detail");
    let d = hub.chat_detail("wxid_bob").unwrap();
    assert_eq!(d["displayName"], "Bobby");
    assert_eq!(d["avatarUrl"], "https://example.com/bob.png");
    assert_eq!(d["firstMessageTime"], 1690000000);
    assert_eq!(d["latestMessageTime"], 1700009999);
    let tables = d["messageTables"].as_array().unwrap();
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0]["dbName"], "message_0.db");
    assert_eq!(tables[0]["count"], 70);
}

#[test]
fn export_stats_with_relations() {
    let (hub, _root) = common::mock_hub("chat-stats");
    let s = hub.chat_export_stats(&strs(&["wxid_bob", "room1@chatroom"]), 0, 0, true).unwrap();
    let bob = &s["data"]["wxid_bob"];
    assert_eq!(bob["totalMessages"], 120);
    assert_eq!(bob["imageMessages"], 10);
    assert_eq!(bob["privateMutualGroups"], 1);
    let room = &s["data"]["room1@chatroom"];
    assert_eq!(room["groupMyMessages"], 17);
    assert_eq!(room["groupActiveSpeakers"], 3);
    assert_eq!(room["groupMemberCount"], 3);
}

#[test]
fn resources_and_images_are_deduplicated() {
    let (hub, _root) = common::mock_hub("chat-res");
    let images = hub.chat_all_images("wxid_bob").unwrap();
    let list = images["images"].as_array().unwrap();
    assert_eq!(list.len(), 2, "duplicate local_id 11 must collapse");
    assert_eq!(list[0]["imageMd5"], "aabbccddeeff00112233445566778899");
    assert_eq!(list[1]["imageDatName"], "3057aabbccddeeff0011223344556677");

    let q = ResourceQuery { session_id: Some("wxid_bob".into()), types: vec!["image".into()], limit: 10, ..Default::default() };
    let r = hub.chat_resources(&q).unwrap();
    assert_eq!(r["total"], 2);
    assert_eq!(r["hasMore"], false);
    assert_eq!(r["items"][0]["sessionDisplayName"], "Bobby");
    assert_eq!(r["items"][0]["resourceType"], "image");

    let paged = hub.chat_resources(&ResourceQuery { limit: 1, ..q.clone() }).unwrap();
    assert_eq!(paged["items"].as_array().unwrap().len(), 1);
    assert_eq!(paged["hasMore"], true);
}

#[test]
fn anti_revoke_sessions_skip_official_accounts() {
    let (hub, _root) = common::mock_hub("chat-anti");
    let v = hub.chat_anti_revoke_sessions().unwrap();
    assert!(v.as_array().unwrap().iter().all(|s| !s["username"].as_str().unwrap_or("").starts_with("gh_")));
}

#[test]
fn group_hint_roundtrip_and_transfer_names() {
    let (hub, _root) = common::mock_hub("chat-hint");
    assert_eq!(hub.get_group_hint("room1@chatroom").unwrap(), json!({}));
    hub.set_group_hint("room1@chatroom", 9).unwrap();
    let h = hub.get_group_hint("room1@chatroom").unwrap();
    assert_eq!(h["count"], 9);
    assert_eq!(h["source"], "disk");
    let t = hub.chat_transfer_names("room1@chatroom", "wxid_bob", "wxid_me").unwrap();
    assert_eq!(t["payerName"], "Bob in room");
}

#[test]
fn mark_read_reaches_the_library() {
    let (hub, _root) = common::mock_hub("chat-read");
    let v = hub.chat_mark_all_read().unwrap();
    assert_eq!(v["fn"], "wcdb_mark_all_sessions_read");
}

#[test]
fn voice_messages_decode_from_silk_to_a_cached_wav() {
    let (hub, _root) = common::mock_hub("voice");
    // strong key (createTime + serverId): no localId lookup needed
    let wav = hub.voice_data("wxid_bob", "5", Some(1700000100), Some("9007199254740993"), Some("wxid_bob")).unwrap();
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 24000);
    let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
    assert_eq!(wav.len(), 44 + data_len);
    assert!(data_len > 2 * 24000 / 10 / 2, "roughly 0.2 s of 16-bit audio, got {data_len} bytes");
    let samples: Vec<i16> = wav[44..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    assert!(samples.iter().any(|s| s.abs() > 2000), "decoded samples carry the tone");

    // cached under <cache>/Voices/<session>_<createTime>_<localId>.wav and served from there next time
    let key_file = hub.voice_cache_dir().join("wxid_bob_1700000100_5.wav");
    assert!(key_file.exists(), "{}", key_file.display());
    std::fs::write(&key_file, b"cached-wav").unwrap();
    assert_eq!(hub.voice_data("wxid_bob", "5", Some(1700000100), Some("1"), None).unwrap(), b"cached-wav");

    // the key without a timestamp is what `resolveVoiceCache` looks at
    assert_eq!(hub.voice_resolve_cache("wxid_bob", "5")["hasCache"], false);
    assert!(hub.voice_data("wxid_bob", "abc", None, None, None).is_err());

    let prepared = hub.voice_preload("wxid_bob", &[serde_json::json!({"localId": 9, "createTime": 1700000300, "serverId": "77"})]).unwrap();
    assert_eq!(prepared["success"], true);
}
