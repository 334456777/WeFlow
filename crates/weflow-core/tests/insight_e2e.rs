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
    common::mock_hub_with(tag, ai_settings(fake, format, model, filter))
}

/// The settings [`hub_with_ai`] gives the profile.
fn ai_settings(
    fake: &FakeModel,
    format: &str,
    model: &str,
    filter: &[&str],
) -> impl FnOnce(&mut weflow_core::config::ProfileConfig) {
    let filter: Vec<serde_json::Value> = filter.iter().map(|s| json!(s)).collect();
    let base = if format == "anthropic" {
        fake.anthropic_base_url()
    } else {
        fake.openai_base_url()
    };
    let (format, model) = (format.to_string(), model.to_string());
    move |p| {
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
    }
}

/// A hub over the standard account in `root`, as a new process would make it: the state of an earlier hub over the
/// same folder is not shared, only the files.
fn hub_over(
    root: &std::path::Path,
    fixture: &weflow_native::fixture::Fixture,
    settings: impl FnOnce(&mut weflow_core::config::ProfileConfig),
) -> weflow_core::services::ServiceHub {
    common::hub_for_account(
        root,
        &fixture.account_dir,
        &fixture.key_hex(),
        "wxid_me_ab12",
        settings,
    )
}

/// The messages a conversation had when the engine started are not activity, neither on the first passes nor after a
/// restart: no model is asked and no insight written (#1140).
#[tokio::test]
async fn activity_starts_from_the_messages_there_when_the_engine_starts() {
    let fake = FakeModel::start().unwrap();
    let root = common::temp_dir("ins-activity-start");
    let fixture = weflow_native::fixture::Fixture::standard(&root.join("data"));
    let filter = ["wxid_bob", "room1@chatroom"];
    for _ in 0..2 {
        // a new hub is a restart
        let hub = hub_over(
            &root,
            &fixture,
            ai_settings(&fake, "openai", "fake-chat", &filter),
        );
        for _ in 0..3 {
            hub.insight_analyze_activity(&mut |_| {}).await;
        }
    }
    assert!(fake.requests().is_empty(), "{:?}", fake.requests());
    let hub = hub_over(
        &root,
        &fixture,
        ai_settings(&fake, "openai", "fake-chat", &filter),
    );
    assert_eq!(hub.insight_list_records(&RecordFilters::default()).total, 0);

    // the same with the conversations the engine picks itself, the recent ones that no blacklist leaves out
    let hub = hub_over(&root, &fixture, |p| {
        ai_settings(&fake, "openai", "fake-chat", &[])(p);
        p.extra
            .insert("ai_insight_filter_mode".into(), json!("blacklist"));
    });
    for _ in 0..3 {
        hub.insight_analyze_activity(&mut |_| {}).await;
    }
    assert!(fake.requests().is_empty(), "{:?}", fake.requests());
}

/// A message newer than the engine's start is activity once; after a restart the cooldown, read from the records,
/// keeps it from being written again (#1140).
#[tokio::test]
async fn a_new_message_is_activity_once_across_restarts() {
    use weflow_native::fixture::{ContactSpec, MsgSpec, SessionSpec};
    let fake = FakeModel::start().unwrap();
    let root = common::temp_dir("ins-activity-new");
    let fixture = weflow_native::fixture::Fixture::new(&root.join("data"), "wxid_me_ab12");
    // after the engine's start, which is now: the clock of the test and of the hub are the same
    let later = chrono::Utc::now().timestamp() + 600;
    fixture.contact_db(
        &[
            ContactSpec::new("wxid_me", 1, "Me Nick"),
            ContactSpec::new("wxid_bob", 1, "Bob"),
        ],
        &[],
    );
    fixture.session_db(&[SessionSpec {
        username: "wxid_bob",
        summary: "news",
        last_timestamp: later,
        unread: 1,
        last_msg_type: 1,
    }]);
    fixture.message_shard(
        0,
        &[(
            "wxid_bob",
            vec![
                MsgSpec::text(1, "wxid_bob", later - 86_400, "yesterday"),
                MsgSpec::text(2, "wxid_bob", later, "news"),
            ],
        )],
    );
    let settings = || ai_settings(&fake, "openai", "fake-chat", &["wxid_bob"]);

    let hub = hub_over(&root, &fixture, settings());
    let mut got = Vec::new();
    for _ in 0..3 {
        hub.insight_analyze_activity(&mut |r| got.push(r.clone()))
            .await;
    }
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(
        (got[0].session_id.as_str(), got[0].trigger_reason.as_str()),
        ("wxid_bob", "activity")
    );
    assert_eq!(fake.requests().len(), 1);

    let restarted = hub_over(&root, &fixture, settings());
    for _ in 0..3 {
        restarted
            .insight_analyze_activity(&mut |r| got.push(r.clone()))
            .await;
    }
    assert_eq!(
        got.len(),
        1,
        "the cooldown holds across the restart: {got:?}"
    );
    assert_eq!(fake.requests().len(), 1);
}

/// 冷却期间到的新消息不触发活动见解，冷却结束后再触发一次；名单的两种模式相同（#1340）。
#[tokio::test]
async fn a_message_during_the_cooldown_is_activity_when_it_ends() {
    use weflow_native::fixture::{ContactSpec, MsgSpec, SessionSpec};
    for mode in ["whitelist", "blacklist"] {
        let fake = FakeModel::start().unwrap();
        let root = common::temp_dir(&format!("ins-activity-cooldown-{mode}"));
        let fixture = weflow_native::fixture::Fixture::new(&root.join("data"), "wxid_me_ab12");
        fixture.contact_db(
            &[
                ContactSpec::new("wxid_me", 1, "Me Nick"),
                ContactSpec::new("wxid_bob", 1, "Bob"),
            ],
            &[],
        );
        // 时间都在引擎启动（即现在）之后
        let first = chrono::Utc::now().timestamp() + 600;
        let second = first + 60;
        let write_chat = |messages: Vec<MsgSpec>, last: i64| {
            fixture.session_db(&[SessionSpec {
                username: "wxid_bob",
                summary: "news",
                last_timestamp: last,
                unread: 1,
                last_msg_type: 1,
            }]);
            fixture.message_shard(0, &[("wxid_bob", messages)]);
        };
        let settings = |cooldown: u32| {
            // whitelist 列出 Bob；blacklist 的名单为空，引擎自己选会话
            let list: &[&str] = if mode == "whitelist" {
                &["wxid_bob"]
            } else {
                &[]
            };
            let base = ai_settings(&fake, "openai", "fake-chat", list);
            move |p: &mut weflow_core::config::ProfileConfig| {
                base(p);
                p.extra.insert("ai_insight_filter_mode".into(), json!(mode));
                p.extra
                    .insert("ai_insight_cooldown_minutes".into(), json!(cooldown));
            }
        };

        write_chat(vec![MsgSpec::text(1, "wxid_bob", first, "news")], first);
        let hub = hub_over(&root, &fixture, settings(120));
        let mut got = Vec::new();
        hub.insight_analyze_activity(&mut |r| got.push(r.clone()))
            .await;
        assert_eq!(got.len(), 1, "{mode}: {got:?}");

        // 冷却期间来了一条新消息，之后重启：引擎自己选会话时，会话列表要隔 15 分钟才重读，
        // 重启后马上读到它；冷却时间从记录里算
        write_chat(
            vec![
                MsgSpec::text(1, "wxid_bob", first, "news"),
                MsgSpec::text(2, "wxid_bob", second, "more news"),
            ],
            second,
        );
        let hub = hub_over(&root, &fixture, settings(120));
        for _ in 0..2 {
            hub.insight_analyze_activity(&mut |r| got.push(r.clone()))
                .await;
        }
        assert_eq!(got.len(), 1, "{mode}: still cooling: {got:?}");

        // 冷却结束：服务每次都重读配置文件，把冷却时间改成 0 写进去
        let mut config = weflow_core::config::ConfigStore::default();
        let p = config.profiles.get_mut("default").unwrap();
        p.db_path = Some(fixture.account_dir.to_string_lossy().to_string());
        p.decrypt_key = Some(fixture.key_hex());
        p.wxid = Some("wxid_me_ab12".into());
        settings(0)(p);
        config.save(&root.join("home/config.json")).unwrap();
        for _ in 0..2 {
            hub.insight_analyze_activity(&mut |r| got.push(r.clone()))
                .await;
        }
        assert_eq!(
            got.len(),
            2,
            "{mode}: the message waited out the cooldown: {got:?}"
        );
        assert_eq!(fake.requests().len(), 2, "{mode}");
    }
}

/// The conversations the engine picks itself, with no list to follow, are people: the file helper and an official
/// account are newer in the list and silent as long, yet `insight trigger` without a conversation, the silence scan
/// and the activity scan pass them by (#1249).
#[tokio::test]
async fn the_engine_picks_people_itself_not_the_file_helper_or_official_accounts() {
    use weflow_native::fixture::{ContactSpec, MsgSpec, SessionSpec};
    let fake = FakeModel::start().unwrap();
    let root = common::temp_dir("ins-picks-people");
    let now = chrono::Utc::now().timestamp();
    let silent = now - 10 * 86_400;
    // after the engine's start, which is now: the clock of the test and of the hub are the same
    let later = now + 600;
    // the file helper first in the list, then the official account, then the person
    let ids = [("filehelper", 2), ("gh_news", 1), ("wxid_bob", 0)];
    let account = |root: &std::path::Path, base: i64| {
        let fixture = weflow_native::fixture::Fixture::new(&root.join("data"), "wxid_me_ab12");
        fixture.contact_db(
            &[
                ContactSpec::new("wxid_me", 1, "Me Nick"),
                ContactSpec::new("filehelper", 1, "File Transfer"),
                ContactSpec::new("gh_news", 1, "News"),
                ContactSpec::new("wxid_bob", 1, "Bob"),
            ],
            &[],
        );
        let sessions: Vec<SessionSpec<'_>> = ids
            .iter()
            .map(|&(id, newer)| SessionSpec {
                username: id,
                summary: "hi",
                last_timestamp: base + newer,
                unread: 1,
                last_msg_type: 1,
            })
            .collect();
        fixture.session_db(&sessions);
        let messages: Vec<_> = ids
            .iter()
            .map(|&(id, newer)| (id, vec![MsgSpec::text(1, id, base + newer, "hi")]))
            .collect();
        fixture.message_shard(0, &messages);
        fixture
    };
    let blacklist = |p: &mut weflow_core::config::ProfileConfig| {
        ai_settings(&fake, "openai", "fake-chat", &[])(p);
        p.extra
            .insert("ai_insight_filter_mode".into(), json!("blacklist"));
    };

    let fixture = account(&root, silent);
    let hub = hub_over(&root, &fixture, blacklist);
    hub.insight_trigger_test().await.unwrap();
    assert_eq!(hub.insight_silence_scan(&mut |_| {}).await, 1);
    let written: Vec<_> = hub
        .insight_list_records(&RecordFilters::default())
        .records
        .into_iter()
        .map(|r| (r.session_id, r.trigger_reason))
        .collect();
    assert_eq!(written.len(), 2, "{written:?}");
    assert!(
        written.iter().all(|(id, _)| id == "wxid_bob"),
        "{written:?}"
    );

    let root = common::temp_dir("ins-picks-people-activity");
    let fixture = account(&root, later);
    let hub = hub_over(&root, &fixture, blacklist);
    let mut got = Vec::new();
    for _ in 0..4 {
        hub.insight_analyze_activity(&mut |r| got.push(r.session_id.clone()))
            .await;
    }
    assert_eq!(got, ["wxid_bob"]);
}

/// A silence is told once: a second scan, and a scan after a restart, write nothing more while no message ends it
/// (#1140).
#[tokio::test]
async fn a_silence_is_told_once() {
    let fake = FakeModel::start().unwrap();
    let root = common::temp_dir("ins-silence-once");
    let fixture = weflow_native::fixture::Fixture::standard(&root.join("data"));
    let settings = || ai_settings(&fake, "openai", "fake-chat", &["wxid_bob"]);
    let hub = hub_over(&root, &fixture, settings());
    assert_eq!(hub.insight_silence_scan(&mut |_| {}).await, 1);
    assert_eq!(hub.insight_silence_scan(&mut |_| {}).await, 0);
    let restarted = hub_over(&root, &fixture, settings());
    assert_eq!(restarted.insight_silence_scan(&mut |_| {}).await, 0);
    assert_eq!(fake.requests().len(), 1);
    assert_eq!(
        restarted
            .insight_list_records(&RecordFilters::default())
            .total,
        1
    );
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
        // 账号所有者发过消息的 6 个私聊里，有 2 个对方从没发过消息：回复的人数是双方都发过的 4 个（#1348）
        "summary": { "private_inbound_people": 5, "private_outbound_people": 6, "private_replied_people": 4, "private_reply_rate": 0.8, "mention_count": 2, "mention_group_count": 1 },
        "private_segments": [{ "display_name": "Bob", "incoming_count": 3, "outgoing_count": 2, "message_count": 5, "replied": true }],
        "mention_groups": [{ "display_name": "Team", "count": 2 }]
    });
    let r = hub.insight_footprint_summary(&payload).await.unwrap();
    assert!(r.insight.unwrap().contains("steady pace"));
    let body = fake.requests()[0].json().unwrap();
    let user = body["messages"][0]["content"].as_str().unwrap();
    // 回复率 = 回复的人数 / 有聊天的人数 = 4 / 5
    assert!(user.contains("People chatted with: 5\n"), "{user}");
    assert!(user.contains("People I replied to: 4\n"), "{user}");
    assert!(user.contains("Reply rate: 80.0%"), "{user}");
    assert!(user.contains("@-mentions of me: 2\n"), "{user}");
    assert!(user.contains("Groups involved: 1\n"), "{user}");
    assert!(
        user.contains("1. Bob (in 3/out 2/total 5/replied)"),
        "{user}"
    );
    assert!(user.contains("1. Team (@-mentioned me 2 times)"), "{user}");
}
