#![cfg(target_os = "linux")]
mod common;

#[test]
fn available_years_are_deduplicated_and_sorted_descending() {
    let (hub, _root) = common::mock_hub("rep-years");
    let v = hub.report_available_years().unwrap();
    assert_eq!(v["years"], serde_json::json!([2024, 2023]));
    assert_eq!(v["strategy"], "native");
}

#[test]
fn annual_report_matches_the_desktop_shape() {
    let (hub, _root) = common::mock_hub("rep-annual");
    let r = hub.report_annual(2024).unwrap();
    assert_eq!(r["year"], 2024);
    assert_eq!(r["totalMessages"], 300);
    assert_eq!(r["totalFriends"], 2);
    assert_eq!(r["coreFriends"][0]["username"], "wxid_bob");
    assert_eq!(r["coreFriends"][0]["displayName"], "Bobby");
    assert_eq!(r["coreFriends"][0]["messageCount"], 220);
    assert_eq!(r["monthlyTopFriends"].as_array().unwrap().len(), 12);
    assert_eq!(r["monthlyTopFriends"][1]["displayName"], "Bobby");
    assert_eq!(r["monthlyTopFriends"][1]["messageCount"], 200);
    assert_eq!(r["monthlyTopFriends"][3]["displayName"], "暂无");
    assert_eq!(r["peakDay"]["date"], "2024-02-10");
    assert_eq!(r["peakDay"]["messageCount"], 90);
    assert_eq!(r["peakDay"]["topFriend"], "Bobby");
    assert_eq!(r["peakDay"]["topFriendCount"], 70);
    assert_eq!(r["longestStreak"]["days"], 4);
    assert_eq!(r["longestStreak"]["startDate"], "2024-02-08");
    assert_eq!(r["longestStreak"]["endDate"], "2024-02-11");
    assert_eq!(r["activityHeatmap"]["data"][0][2], 1);
    assert_eq!(r["activityHeatmap"]["data"][0][23], 2);
    assert_eq!(r["midnightKing"]["count"], 30);
    assert_eq!(r["midnightKing"]["percentage"], 75.0);
    assert_eq!(r["mutualFriend"]["displayName"], "Bobby", "only contacts with >= 50 messages each way qualify");
    assert_eq!(r["mutualFriend"]["ratio"], 1.2);
    assert_eq!(r["socialInitiative"]["initiatedChats"], 35);
    assert_eq!(r["socialInitiative"]["receivedChats"], 25);
    assert_eq!(r["socialInitiative"]["initiativeRate"], 58.3);
    assert_eq!(r["socialInitiative"]["topInitiatedFriend"], "Bobby");
    assert_eq!(r["responseSpeed"]["avgResponseTime"], 105);
    assert_eq!(r["responseSpeed"]["fastestFriend"], "Bobby");
    assert_eq!(r["responseSpeed"]["fastestTime"], 60);
    assert_eq!(r["topPhrases"][0]["phrase"], "ok");
    assert_eq!(r["snsStats"]["totalPosts"], 12);
    assert_eq!(r["snsStats"]["topLikers"][0]["displayName"], "Bobby");
    assert_eq!(r["lostFriend"]["username"], "wxid_bob");
    assert_eq!(r["lostFriend"]["earlyCount"], 210);
    assert_eq!(r["lostFriend"]["lateCount"], 10);
    assert_eq!(r["lostFriend"]["periodDesc"], "2024年上半年");
}

#[test]
fn annual_report_for_all_years_reports_year_zero() {
    let (hub, _root) = common::mock_hub("rep-annual-all");
    let r = hub.report_annual(0).unwrap();
    assert_eq!(r["year"], 0);
    assert_eq!(r["totalMessages"], 300);
}

#[test]
fn dual_report_builds_first_chat_phrases_and_passthrough() {
    let (hub, _root) = common::mock_hub("rep-dual");
    let r = hub.report_dual("wxid_bob", 2024, &[]).unwrap();
    assert_eq!(r["year"], 2024);
    assert_eq!(r["friendUsername"], "wxid_bob");
    assert_eq!(r["friendName"], "Bobby");
    assert_eq!(r["stats"]["totalMessages"], 500);
    assert_eq!(r["stats"]["totalWords"], 1234);
    assert_eq!(r["stats"]["imageCount"], 10);
    assert!(r["stats"].get("myTopEmojiMd5").is_none());
    assert!(r["firstChatMessages"].as_array().unwrap().len() >= 2);
    assert_eq!(r["yearFirstChat"]["friendName"], "Bobby");
    assert!(r["firstChat"]["createTime"].as_i64().unwrap() % 1000 == 0);
    assert_eq!(r["topPhrases"].as_array().unwrap().len(), 3);
    let mine: Vec<&str> = r["myExclusivePhrases"].as_array().unwrap().iter().map(|p| p["phrase"].as_str().unwrap()).collect();
    assert_eq!(mine, vec!["haha", "skip"]);
    assert_eq!(r["friendExclusivePhrases"][0]["phrase"], "mine");
    assert_eq!(r["heatmap"], serde_json::json!([[1]]));
    assert_eq!(r["initiative"]["mine"], 3);
    assert_eq!(r["streak"]["days"], 5);

    let filtered = hub.report_dual("wxid_bob", 0, &["skip".to_string()]).unwrap();
    assert_eq!(filtered["year"], 0);
    assert!(filtered["yearFirstChat"].is_null());
    assert_eq!(filtered["topPhrases"].as_array().unwrap().len(), 2);
    assert!(!filtered["myExclusivePhrases"].as_array().unwrap().iter().any(|p| p["phrase"] == "skip"));
}
