mod common;

use serde_json::json;
use weflow_core::services::ResourceQuery;
use weflow_native::fixture::{ContactSpec, Fixture, MsgSpec, SessionSpec, VoiceSpec, T0};

fn session(username: &'static str) -> SessionSpec {
    SessionSpec { username, summary: "", last_timestamp: T0, unread: 0, last_msg_type: 3 }
}

const MD5_A: &str = "aabbccddeeff00112233445566778899";
const MD5_B: &str = "11223344556677889900aabbccddeeff";

fn image(local_id: i64, at: i64, md5: &str) -> MsgSpec {
    MsgSpec::text(local_id, "wxid_bob", T0 + at, &format!("<msg><img md5=\"{md5}\"/></msg>")).of_type(3)
}

fn images_world(f: &Fixture) {
    f.session_db(&[session("wxid_bob")]);
    f.contact_db(&[ContactSpec { remark: "Bobby", ..ContactSpec::new("wxid_bob", 1, "Bob") }], &[]);
    // the same image row sits in both shards (a re-synced shard), plus a newer one only in shard 1
    f.message_shard(0, &[("wxid_bob", vec![image(1, 0, MD5_A)])]);
    f.message_shard(1, &[("wxid_bob", vec![image(1, 0, MD5_A), image(2, 100, MD5_B)])]);
}

#[test]
fn resources_and_images_are_deduplicated() {
    let (hub, _root, _f) = common::custom_hub("chat-res", images_world);
    let images = hub.chat_all_images("wxid_bob").unwrap();
    let list = images["images"].as_array().unwrap();
    assert_eq!(list.len(), 2, "the duplicated row (same ids and time) must collapse");
    assert_eq!(list[0]["imageMd5"], MD5_B, "newest first");
    assert_eq!(list[1]["imageMd5"], MD5_A);

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
fn voice_messages_decode_from_silk_to_a_cached_wav() {
    let (created, server) = (1_700_000_100_i64, 9_007_199_254_740_993_i64);
    let (hub, _root, _f) = common::custom_hub("voice", |f| {
        f.session_db(&[session("wxid_bob")]);
        // a real SILK stream made by the vendored encoder: one second of a 440 Hz tone
        let silk = weflow_silk::testenc::encode_tone(24000);
        f.media_db(0, &[VoiceSpec { chat: "wxid_bob", create_time: created, local_id: 5, svr_id: server, data: silk, index: "0" }]);
    });
    // strong key (createTime + serverId): no localId lookup needed
    let wav = hub.voice_data("wxid_bob", "5", Some(created), Some("9007199254740993"), Some("wxid_bob")).unwrap();
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 24000);
    let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
    assert_eq!(wav.len(), 44 + data_len);
    assert!(data_len > 2 * 24000 / 2, "roughly a second of 16-bit audio, got {data_len} bytes");
    let samples: Vec<i16> = wav[44..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    assert!(samples.iter().any(|s| s.abs() > 2000), "decoded samples carry the tone");

    // cached under <cache>/Voices/<session>_<createTime>_<localId>.wav and served from there next time
    let key_file = hub.voice_cache_dir().join("wxid_bob_1700000100_5.wav");
    assert!(key_file.exists(), "{}", key_file.display());
    std::fs::write(&key_file, b"cached-wav").unwrap();
    assert_eq!(hub.voice_data("wxid_bob", "5", Some(created), Some("1"), None).unwrap(), b"cached-wav");

    // the key without a timestamp is what `resolveVoiceCache` looks at
    assert_eq!(hub.voice_resolve_cache("wxid_bob", "5")["hasCache"], false);
    assert!(hub.voice_data("wxid_bob", "abc", None, None, None).is_err());
    // a voice that is not in the media database is an error that explains what to do
    let missing = hub.voice_data("wxid_bob", "99", Some(created + 500), Some("4242"), Some("wxid_bob")).unwrap_err();
    assert!(missing.message.contains("voice data not found"), "{}", missing.message);

    let prepared = hub.voice_preload("wxid_bob", &[json!({"localId": 9, "createTime": 1700000300, "serverId": "77"})]).unwrap();
    assert_eq!(prepared["success"], true);
}
