//! Exports read by several workers, page by page: every message once, in the order a single reader gives.
mod common;

use serde_json::Value;
use weflow_core::api::ApiMediaOptions;
use weflow_core::export_msg::DisplayPref;
use weflow_core::services::MessageExportRequest;
use weflow_native::fixture::{ContactSpec, MsgSpec, SessionSpec, T0};

const PEER: &str = "wxid_bob";

/// (create time, local id, shard, sender) of every synthetic message: two shards whose local ids collide and whose
/// times overlap, two messages per second, more than two cursor pages (2000 messages each).
fn messages() -> Vec<(i64, i64, u32, &'static str)> {
    let mut all = Vec::new();
    for local in 1..=3000 {
        let sender = if local % 3 == 0 { "wxid_me" } else { PEER };
        all.push((T0 + local / 2, local, 0, sender));
    }
    for local in 1..=2500 {
        let sender = if local % 4 == 0 { "wxid_me" } else { PEER };
        all.push((T0 + 1400 + local / 2, local, 1, sender));
    }
    all
}

fn text(local: i64, shard: u32) -> String {
    format!("s{shard}-{local}")
}

fn request(
    format: &str,
    start: Option<i64>,
    end: Option<i64>,
    sender: Option<&str>,
) -> MessageExportRequest {
    MessageExportRequest {
        session_id: PEER.into(),
        format: format.into(),
        start,
        end,
        sender: sender.map(str::to_string),
        display_pref: DisplayPref::Remark,
        excel_compact: false,
    }
}

fn contents(path: &std::path::Path) -> Vec<String> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap().to_string())
        .collect()
}

/// What a single reader exports: the cursor order (time, then local id, then shard), within `[start, end)`.
fn expected(start: Option<i64>, end: Option<i64>, sender: Option<&str>) -> Vec<String> {
    let mut all = messages();
    all.sort();
    all.into_iter()
        .filter(|(t, ..)| start.is_none_or(|s| *t >= s) && end.is_none_or(|e| *t < e))
        .filter(|(.., who)| sender.is_none_or(|s| s == *who))
        .map(|(_, local, shard, _)| text(local, shard))
        .collect()
}

#[tokio::test]
async fn multi_page_exports_keep_every_message_in_reader_order() {
    let (hub, root, _f) = common::custom_hub("export-pages", |f| {
        f.session_db(&[SessionSpec {
            username: PEER,
            summary: "",
            last_timestamp: T0 + 3000,
            unread: 0,
            last_msg_type: 1,
        }]);
        f.contact_db(
            &[
                ContactSpec::new("wxid_me", 1, "Me"),
                ContactSpec::new(PEER, 1, "Bob"),
            ],
            &[],
        );
        for shard in [0, 1] {
            let msgs: Vec<MsgSpec> = messages()
                .into_iter()
                .filter(|m| m.2 == shard)
                .map(|(t, local, _, who)| MsgSpec::text(local, who, t, &text(local, shard)))
                .collect();
            f.message_shard(shard, &[(PEER, msgs)]);
        }
    });

    let all = expected(None, None, None);
    assert_eq!(all.len(), 5500);
    let out = root.join("all.json");
    hub.export_messages(&request("chatlab", None, None, None), &out)
        .unwrap();
    assert_eq!(contents(&out), all);

    // the media path reads the same stream, then sorts by time and local id: the same order here
    let media = ApiMediaOptions {
        enabled: true,
        ..Default::default()
    };
    let out = root.join("collected.json");
    hub.export_messages_with_media(&request("chatlab", None, None, None), &out, &media)
        .await
        .unwrap();
    assert_eq!(contents(&out), all);

    // a range that starts and ends inside pages, and one sender
    let (start, end) = (Some(T0 + 700), Some(T0 + 2100));
    let out = root.join("range.json");
    hub.export_messages(&request("chatlab", start, end, None), &out)
        .unwrap();
    assert_eq!(contents(&out), expected(start, end, None));
    let out = root.join("sender.json");
    hub.export_messages(&request("chatlab", start, end, Some("wxid_me")), &out)
        .unwrap();
    let mine = expected(start, end, Some("wxid_me"));
    assert!(!mine.is_empty());
    assert_eq!(contents(&out), mine);
}
