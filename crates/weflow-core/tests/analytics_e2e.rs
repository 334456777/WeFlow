#![cfg(target_os = "linux")]
mod common;

use serde_json::json;

#[test]
fn overall_statistics_follow_the_desktop_formulas() {
    let (hub, _root) = common::mock_hub("an-overall");
    let s = hub.analytics_overall_statistics(false).unwrap();
    assert_eq!(s["totalMessages"], 100);
    assert_eq!(s["textMessages"], 70);
    assert_eq!(s["imageMessages"], 10);
    assert_eq!(s["otherMessages"], 5, "total minus text/image/voice/video/emoji");
    assert_eq!(s["sentMessages"], 40);
    assert_eq!(s["firstMessageTime"], 1690000000);
    assert_eq!(s["activeDays"], 40, "the desktop app estimates 20 days per active month");
    assert_eq!(s["messageTypeCounts"]["47"], 7);
}

#[test]
fn rankings_and_time_distribution() {
    let (hub, _root) = common::mock_hub("an-rank");
    let r = hub.analytics_contact_rankings(10, 0, 0).unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0]["username"], "wxid_bob");
    assert_eq!(r[0]["displayName"], "Bobby");
    assert_eq!(r[0]["wechatId"], "bobby_id");
    assert_eq!(r[0]["messageCount"], 70);
    assert_eq!(r[0]["sentCount"], 30);
    assert_eq!(r[1]["username"], "wxid_carol");
    assert_eq!(r[1]["wechatId"], "", "wxid_ accounts without alias have no wechat id");
    assert_eq!(hub.analytics_contact_rankings(1, 0, 0).unwrap().len(), 1);

    let t = hub.analytics_time_distribution().unwrap();
    assert_eq!(t["hourlyDistribution"]["21"], 70);
    assert_eq!(t["hourlyDistribution"]["3"], 0);
    assert_eq!(t["weekdayDistribution"]["7"], 20, "Sunday moves from 0 to 7");
    assert_eq!(t["weekdayDistribution"]["1"], 30);
    assert_eq!(t["monthlyDistribution"]["2024-02"], 40);
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
    assert_eq!(hub.analytics_overall_statistics(true).unwrap()["totalMessages"], json!(100));
}
