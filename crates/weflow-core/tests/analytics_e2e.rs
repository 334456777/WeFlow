mod common;

use serde_json::{json, Value};

fn sum(v: &Value) -> i64 {
    v.as_object().unwrap().values().map(|n| n.as_i64().unwrap()).sum()
}

#[test]
fn rankings_and_time_distribution() {
    let (hub, _root) = common::mock_hub("an-rank");
    let r = hub.analytics_contact_rankings(10, 0, 0).unwrap();
    assert_eq!(r.len(), 1, "only private chats with messages are ranked: bob");
    assert_eq!(r[0]["username"], "wxid_bob");
    assert_eq!(r[0]["displayName"], "Bobby");
    assert_eq!(r[0]["wechatId"], "bobby_id");
    assert_eq!(r[0]["messageCount"], 5);
    assert_eq!(r[0]["sentCount"], 2);
    assert_eq!(hub.analytics_contact_rankings(1, 0, 0).unwrap().len(), 1);
    assert!(hub.analytics_contact_rankings(0, 0, 0).unwrap().is_empty(), "a limit of 0 returns nothing");

    let t = hub.analytics_time_distribution().unwrap();
    // hours and weekdays are local time, so check totals and shape instead of a particular hour
    assert_eq!(t["hourlyDistribution"].as_object().unwrap().len(), 24);
    assert_eq!(sum(&t["hourlyDistribution"]), 5);
    let weekdays = t["weekdayDistribution"].as_object().unwrap();
    assert!(weekdays.keys().all(|k| ("1"..="7").contains(&k.as_str())), "Sunday is 7, not 0: {weekdays:?}");
    assert_eq!(sum(&t["weekdayDistribution"]), 5);
    assert_eq!(t["monthlyDistribution"]["2023-11"], 5);
}

#[test]
fn exclusion_list_roundtrip_and_candidates() {
    let (hub, root) = common::mock_hub("an-exclude");
    assert!(hub.analytics_excluded_usernames().unwrap().is_empty());
    let cand = hub.analytics_exclude_candidates().unwrap();
    assert!(cand.iter().any(|c| c["username"] == "wxid_bob" && c["displayName"] == "Bobby" && c["wechatId"] == "bobby_id"));
    assert!(!cand.iter().any(|c| c["username"] == "room1@chatroom"));

    let saved = hub.analytics_set_excluded_usernames(&["  WXID_Bob ".into(), "wxid_bob".into(), "".into(), "Ghost".into()]).unwrap();
    assert_eq!(saved, vec!["wxid_bob", "ghost"]);
    assert_eq!(hub.analytics_excluded_usernames().unwrap(), saved);
    let persisted = std::fs::read_to_string(root.join("home/config.json")).unwrap();
    assert!(persisted.contains("analyticsExcludedUsernames"));

    // excluded sessions vanish from the aggregate, but stay selectable as candidates
    let err = hub.analytics_overall_statistics(true).unwrap_err();
    assert!(err.message.contains("no message sessions"), "{}", err.message);
    let cand = hub.analytics_exclude_candidates().unwrap();
    assert!(cand.iter().any(|c| c["username"] == "ghost"));
    assert!(cand.iter().any(|c| c["username"] == "wxid_bob"));
    assert_eq!(hub.analytics_set_excluded_usernames(&[]).unwrap(), Vec::<String>::new());
    assert_eq!(hub.analytics_overall_statistics(true).unwrap()["totalMessages"], json!(5));
}
