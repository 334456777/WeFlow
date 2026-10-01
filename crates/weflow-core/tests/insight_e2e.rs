#![cfg(target_os = "linux")]
mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::json;
use weflow_core::insight::RecordFilters;

/// Minimal OpenAI-compatible server; records each request body and answers `answer`.
fn fake_ai(answer: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = vec![0u8; 65536];
            let mut total = 0;
            loop {
                let n = s.read(&mut buf[total..]).unwrap_or(0);
                total += n;
                let text = String::from_utf8_lossy(&buf[..total]).to_string();
                if let Some(pos) = text.find("\r\n\r\n") {
                    let len: usize = text
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if total >= pos + 4 + len || n == 0 {
                        seen2.lock().unwrap().push(format!(
                            "{}\n{}",
                            &text[..pos],
                            &text[pos + 4..]
                        ));
                        break;
                    }
                } else if n == 0 {
                    break;
                }
            }
            let body = json!({ "choices": [{ "message": { "content": answer } }] }).to_string();
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
        }
    });
    (base, seen)
}

fn hub_with_ai(tag: &str, base: String, filter: &[&str]) -> weflow_core::services::ServiceHub {
    let filter: Vec<serde_json::Value> = filter.iter().map(|s| json!(s)).collect();
    let (hub, _root) = common::mock_hub_with(tag, move |p| {
        p.ai_model_api_base_url = Some(base);
        p.ai_model_api_key = Some("sk-test".into());
        p.ai_model_api_model = Some("test-model".into());
        p.ai_insight_enabled = Some(true);
        p.extra
            .insert("aiInsightFilterMode".into(), json!("whitelist"));
        p.extra.insert("aiInsightFilterList".into(), json!(filter));
        p.extra.insert("aiInsightAllowContext".into(), json!(true));
        p.extra.insert("aiFootprintEnabled".into(), json!(true));
    });
    hub
}

#[tokio::test]
async fn connection_test_uses_chat_completions_without_adding_v1() {
    let (base, seen) = fake_ai("connection successful");
    let hub = hub_with_ai("ins-test", base, &[]);
    let r = hub.insight_test_connection().await;
    assert_eq!(r["success"], true, "{r}");
    assert!(r["message"]
        .as_str()
        .unwrap()
        .contains("connection successful"));
    let req = seen.lock().unwrap()[0].clone();
    assert!(req.starts_with("POST /v1/chat/completions "), "{req}");
    assert!(req.to_lowercase().contains("authorization: bearer sk-test"));
    assert!(req.contains("\"model\":\"test-model\"") && req.contains("\"stream\":false"));
}

#[tokio::test]
async fn trigger_test_builds_context_and_stores_a_record() {
    let (base, seen) = fake_ai("Ask about the weekend plans.");
    let hub = hub_with_ai("ins-trigger", base, &["wxid_bob"]);
    let r = hub.insight_trigger_test().await;
    assert_eq!(r["success"], true, "{r}");
    let req = seen.lock().unwrap()[0].clone();
    assert!(
        req.contains("Recent chat history"),
        "context section present: {req}"
    );
    assert!(req.contains("Give your insight"), "{req}");
    assert!(req.contains("Current system time"), "{req}");

    let list = hub.insight_list_records(&RecordFilters::default());
    assert_eq!(list["total"], 1);
    assert_eq!(list["unreadCount"], 1);
    let rec = &list["records"][0];
    assert_eq!(rec["sessionId"], "wxid_bob");
    assert_eq!(rec["triggerReason"], "test");
    assert_eq!(rec["insight"], "Ask about the weekend plans.");
    let id = rec["id"].as_str().unwrap().to_string();
    let full = hub.insight_get_record(&id);
    assert_eq!(full["record"]["log"]["model"], "test-model");
    assert!(full["record"]["log"]["endpoint"]
        .as_str()
        .unwrap()
        .ends_with("/v1/chat/completions"));
    assert_eq!(hub.insight_mark_record_read(&id)["success"], true);
    assert_eq!(
        hub.insight_list_records(&RecordFilters::default())["unreadCount"],
        0
    );
    assert_eq!(hub.insight_today_stats().as_array().unwrap().len(), 1);
    assert_eq!(
        hub.insight_clear_records(&RecordFilters::default())["removed"],
        1
    );
}

#[tokio::test]
async fn silence_scan_and_skip_answers() {
    let (base, _seen) = fake_ai("Reach out, it has been quiet.");
    let hub = hub_with_ai("ins-silence", base, &["wxid_bob"]);
    let mut got = Vec::new();
    let n = hub.insight_silence_scan(&mut |r| got.push(r.clone())).await;
    assert_eq!(n, 1, "only the whitelisted silent private chat");
    assert_eq!(got[0].trigger_reason, "silence");
    assert!(got[0].log.user_prompt.contains("days since you contacted"));

    let (base2, _) = fake_ai("SKIP");
    let hub2 = hub_with_ai("ins-skip", base2, &["wxid_bob"]);
    assert_eq!(hub2.insight_silence_scan(&mut |_| {}).await, 0);
    assert_eq!(
        hub2.insight_list_records(&RecordFilters::default())["total"],
        0,
        "SKIP answers are dropped"
    );
}

#[tokio::test]
async fn footprint_summary_formats_the_prompt() {
    let (base, seen) = fake_ai("You chatted a lot. Reply to Bob sooner.");
    let hub = hub_with_ai("ins-foot", base, &[]);
    let payload = json!({
        "rangeLabel": "Last 7 days",
        "summary": { "private_inbound_people": 5, "private_outbound_people": 4, "private_reply_rate": 0.8, "mention_count": 2, "mention_group_count": 1 },
        "privateSegments": [{ "displayName": "Bob", "incoming_count": 3, "outgoing_count": 2, "message_count": 5, "replied": true }],
        "mentionGroups": [{ "displayName": "Team", "count": 2 }]
    });
    let r = hub.insight_footprint_summary(&payload).await;
    assert_eq!(r["success"], true, "{r}");
    assert_eq!(r["insight"], "You chatted a lot. Reply to Bob sooner.");
    let req = seen.lock().unwrap()[0].clone();
    assert!(req.contains("Reply rate: 80.0%"), "{req}");
    assert!(req.contains("1. Bob (in 3/out 2/total 5/replied)"), "{req}");
    assert!(req.contains("1. Team (@-mentioned me 2 times)"), "{req}");
}
