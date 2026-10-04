//! Adversarial sender cases use only synthetic encrypted accounts.
mod common;

use serde_json::Value;
use weflow_core::api::ApiMediaOptions;
use weflow_core::export_msg::DisplayPref;
use weflow_core::services::MessageExportRequest;
use weflow_native::fixture::{ContactSpec, MsgSpec, SessionSpec, T0};

#[tokio::test]
async fn sender_prefilter_preserves_revoker_identity_across_shards_and_paths() {
    const ROOM: &str = "room_synthetic@chatroom";
    let (hub, root, _fixture) = common::custom_hub("sender-context", |f| {
        f.session_db(&[SessionSpec {
            username: ROOM,
            summary: "synthetic",
            last_timestamp: T0 + 60,
            unread: 0,
            last_msg_type: 1,
        }]);
        f.contact_db(
            &[
                ContactSpec::new("wxid_me", 1, "Synthetic owner"),
                ContactSpec::new("wxid_bob", 1, "Synthetic B"),
                ContactSpec::new("wxid_carol", 1, "Synthetic C"),
                ContactSpec::new(ROOM, 2, "Synthetic room"),
            ],
            &[],
        );
        let revoke = "<sysmsg type=\"revokemsg\"><revokemsg><session>room_synthetic@chatroom</session><fromusername>wxid_carol</fromusername><replacemsg>synthetic revoke</replacemsg></revokemsg></sysmsg>";
        f.message_shard(
            0,
            &[(
                ROOM,
                vec![
                    MsgSpec::text(1, "wxid_bob", T0, "synthetic B text"),
                    MsgSpec::text(2, "wxid_bob", T0 + 1, revoke).of_type(10000),
                    MsgSpec::text(3, "wxid_bob", T0 + 2, "你撤回了一条消息").of_type(10000),
                    MsgSpec::text(4, "", T0 + 3, "synthetic system").of_type(10000),
                    MsgSpec::text(5, "wxid_carol", T0 + 4, "synthetic C text"),
                ],
            )],
        );
        f.message_shard(
            1,
            &[(
                ROOM,
                vec![
                    MsgSpec::text(6, "wxid_bob", T0 + 4, revoke)
                        .of_type(266287972401)
                        .compressed(),
                    MsgSpec::text(7, "wxid_me", T0 + 5, "synthetic own text"),
                ],
            )],
        );
    });
    let mut req = MessageExportRequest {
        session_id: ROOM.into(),
        format: "chatlab".into(),
        start: None,
        end: None,
        sender: None,
        display_pref: DisplayPref::Remark,
        excel_compact: false,
    };
    let all_path = root.join("all.json");
    hub.export_messages(&req, &all_path).unwrap();
    let all: Value = serde_json::from_slice(&std::fs::read(all_path).unwrap()).unwrap();
    let all = all["messages"].as_array().unwrap();
    assert_eq!(all.len(), 7);
    let media = ApiMediaOptions {
        enabled: true,
        ..Default::default()
    };
    for (index, (sender, count)) in [
        ("wxid_bob", 1),
        ("wxid_carol", 3),
        ("wxid_me", 2),
        (ROOM, 1),
    ]
    .into_iter()
    .enumerate()
    {
        req.sender = Some(sender.into());
        let expected: Vec<Value> = all
            .iter()
            .filter(|m| {
                weflow_native::native_db::same_identity(m["sender"].as_str().unwrap(), sender)
            })
            .cloned()
            .collect();
        assert_eq!(expected.len(), count);
        for collect in [false, true] {
            let out = root.join(format!("filtered-{index}-{collect}.json"));
            if collect {
                hub.export_messages_with_media(&req, &out, &media)
                    .await
                    .unwrap_or_else(|e| panic!("sender={sender} collect={collect}: {e:?}"));
            } else {
                hub.export_messages(&req, &out)
                    .unwrap_or_else(|e| panic!("sender={sender} collect={collect}: {e:?}"));
            }
            let actual: Value = serde_json::from_slice(&std::fs::read(out).unwrap()).unwrap();
            assert_eq!(actual["messages"], Value::Array(expected.clone()));
        }
    }
}
