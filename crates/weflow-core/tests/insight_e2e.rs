//! The AI insights against the fake model service: the connection test, triggers, the silence scan and the
//! footprint's summary, in both API formats.
#![cfg(target_os = "linux")]
mod common;

use serde_json::json;
use weflow_core::insight::RecordFilters;
use weflow_fake_model::{FakeModel, KEY};

/// The model API settings of the fake in `format` with `model`, and insights on for the conversations of `filter`.
fn hub_with_ai(
    tag: &str,
    fake: &FakeModel,
    format: &str,
    model: &str,
    filter: &[&str],
) -> (
    weflow_native::fixture::TestDir,
    weflow_core::services::ServiceHub,
) {
    let filter: Vec<serde_json::Value> = filter.iter().map(|s| json!(s)).collect();
    let base = if format == "anthropic" {
        fake.anthropic_base_url()
    } else {
        fake.openai_base_url()
    };
    let (format, model) = (format.to_string(), model.to_string());
    common::mock_hub_with(tag, move |p| {
        p.ai_model_api_format = Some(format);
        p.ai_model_api_base_url = Some(base);
        p.ai_model_api_key = Some(KEY.into());
        p.ai_model_api_model = Some(model);
        p.ai_insight_enabled = Some(true);
        p.extra
            .insert("ai_insight_filter_mode".into(), json!("whitelist"));
        p.extra
            .insert("ai_insight_filter_list".into(), json!(filter));
        p.extra
            .insert("ai_insight_allow_context".into(), json!(true));
        p.extra.insert("ai_footprint_enabled".into(), json!(true));
    })
}

#[tokio::test]
async fn connection_test_says_where_it_asked_and_what_came_back() {
    let fake = FakeModel::start().unwrap();
    let (_root, hub) = hub_with_ai("ins-test", &fake, "openai", "fake-chat", &[]);
    let r = hub.insight_test_connection().await.unwrap();
    assert!(r.message.unwrap().contains("connection successful"));
    let probe = r.probe.unwrap();
    assert!(
        probe.endpoint.ends_with("/v1/chat/completions"),
        "{probe:?}"
    );
    assert_eq!(
        (probe.model.as_str(), probe.reply.as_str()),
        ("fake-chat", "connection successful")
    );
    let request = &fake.requests()[0];
    assert_eq!(request.path, "/v1/chat/completions", "no second /v1");
    assert_eq!(request.json().unwrap()["model"], "fake-chat");
}

#[tokio::test]
async fn trigger_test_builds_context_and_stores_a_record() {
    let fake = FakeModel::start().unwrap();
    let (_root, hub) = hub_with_ai("ins-trigger", &fake, "openai", "fake-chat", &["wxid_bob"]);
    hub.insight_trigger_test().await.unwrap();
    let body = fake.requests()[0].json().unwrap();
    let user = body["messages"][1]["content"].as_str().unwrap();
    assert!(
        user.contains("Recent chat history"),
        "context section present: {user}"
    );
    assert!(user.contains("Give your insight"), "{user}");
    assert!(user.contains("Current system time"), "{user}");

    let list = hub.insight_list_records(&RecordFilters::default());
    assert_eq!((list.total, list.unread_count), (1, 1));
    let rec = &list.records[0];
    assert_eq!(rec.session_id, "wxid_bob");
    assert_eq!(rec.trigger_reason, "test");
    assert!(!rec.insight.is_empty());
    let id = rec.id.clone();
    let full = serde_json::to_value(hub.insight_get_record(&id).unwrap()).unwrap();
    let log = &full["record"]["log"];
    assert_eq!(
        (log["format"].as_str(), log["model"].as_str()),
        (Some("openai"), Some("fake-chat"))
    );
    assert!(log["endpoint"]
        .as_str()
        .unwrap()
        .ends_with("/v1/chat/completions"));
    assert_eq!(log["finalInsight"], rec.insight.as_str());
    assert_eq!(hub.insight_mark_record_read(&id).unwrap().id, id);
    assert_eq!(
        hub.insight_list_records(&RecordFilters::default())
            .unread_count,
        0
    );
    assert_eq!(hub.insight_today_stats().len(), 1);
    assert_eq!(
        hub.insight_clear_records(&RecordFilters::default())
            .unwrap()
            .removed,
        1
    );
}

#[tokio::test]
async fn an_anthropic_model_writes_insights_too() {
    let fake = FakeModel::start().unwrap();
    let (_root, hub) = hub_with_ai(
        "ins-anthropic",
        &fake,
        "anthropic",
        "fake-claude",
        &["wxid_bob"],
    );
    let r = hub.insight_trigger_session("wxid_bob").await.unwrap();
    let request = &fake.requests()[0];
    assert_eq!(request.path, "/v1/messages");
    let body = request.json().unwrap();
    assert!(
        body["system"].as_str().unwrap().contains("Insight"),
        "{body}"
    );
    let id = r.record.unwrap().id;
    let full = serde_json::to_value(hub.insight_get_record(&id).unwrap()).unwrap();
    assert_eq!(full["record"]["log"]["format"], "anthropic");
}

#[tokio::test]
async fn a_failed_trigger_says_why() {
    let fake = FakeModel::start().unwrap();
    let (_root, hub) = hub_with_ai("ins-fail", &fake, "openai", "fake-quota", &["wxid_bob"]);
    let message = hub
        .insight_trigger_test()
        .await
        .unwrap_err()
        .message
        .clone();
    assert!(
        message.contains("HTTP 429") && message.contains("insufficient_quota"),
        "{message}"
    );
    assert_eq!(hub.insight_list_records(&RecordFilters::default()).total, 0);

    let (_root2, skip) = hub_with_ai("ins-fail-skip", &fake, "openai", "fake-skip", &["wxid_bob"]);
    let message = skip
        .insight_trigger_session("wxid_bob")
        .await
        .unwrap_err()
        .message
        .clone();
    assert!(message.contains("SKIP"), "{message}");
    assert!(
        !message.contains("ai_insight_allow_context"),
        "the chat was sent: {message}"
    );
}

#[tokio::test]
async fn silence_scan_and_skip_answers() {
    let fake = FakeModel::start().unwrap();
    let (_root, hub) = hub_with_ai("ins-silence", &fake, "openai", "fake-chat", &["wxid_bob"]);
    let mut got = Vec::new();
    let n = hub.insight_silence_scan(&mut |r| got.push(r.clone())).await;
    assert_eq!(n, 1, "only the whitelisted silent private chat");
    assert_eq!(got[0].trigger_reason, "silence");
    assert!(got[0].log.user_prompt.contains("days since you contacted"));

    let (_root2, hub2) = hub_with_ai("ins-skip", &fake, "anthropic", "fake-skip", &["wxid_bob"]);
    assert_eq!(hub2.insight_silence_scan(&mut |_| {}).await, 0);
    assert_eq!(
        hub2.insight_list_records(&RecordFilters::default()).total,
        0,
        "SKIP answers are dropped"
    );
}

#[tokio::test]
async fn footprint_summary_formats_the_prompt() {
    let fake = FakeModel::start().unwrap();
    let (_root, hub) = hub_with_ai("ins-foot", &fake, "anthropic", "fake-claude", &[]);
    let payload = json!({
        "range_label": "Last 7 days",
        "summary": { "private_inbound_people": 5, "private_outbound_people": 4, "private_reply_rate": 0.8, "mention_count": 2, "mention_group_count": 1 },
        "private_segments": [{ "display_name": "Bob", "incoming_count": 3, "outgoing_count": 2, "message_count": 5, "replied": true }],
        "mention_groups": [{ "display_name": "Team", "count": 2 }]
    });
    let r = hub.insight_footprint_summary(&payload).await.unwrap();
    assert!(r.insight.unwrap().contains("steady pace"));
    let body = fake.requests()[0].json().unwrap();
    let user = body["messages"][0]["content"].as_str().unwrap();
    assert!(user.contains("Reply rate: 80.0%"), "{user}");
    assert!(
        user.contains("1. Bob (in 3/out 2/total 5/replied)"),
        "{user}"
    );
    assert!(user.contains("1. Team (@-mentioned me 2 times)"), "{user}");
}
