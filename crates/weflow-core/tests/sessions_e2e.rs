//! The conversation list and the contacts of a synthetic account: which are listed, their names and their kinds.
mod common;

use weflow_core::contract::ContactKind;

#[test]
fn sessions_are_filtered_and_named() {
    let (_root, hub) = common::mock_hub("sessions-list");
    let sessions = hub.chat_sessions_list().unwrap();
    let find = |u: &str| sessions.iter().find(|s| s.username == u);
    let bob = find("wxid_bob").unwrap();
    assert_eq!(bob.display_name, "Bobby");
    assert_eq!(bob.unread_count, 2);
    assert_eq!(find("room1@chatroom").unwrap().display_name, "Project Room");
    assert!(find("medianote").is_none());
}

#[test]
fn names_and_avatars_come_from_the_database_whatever_the_cache_holds() {
    let (_root, hub) = common::mock_hub("sessions-desktop-contacts");
    // the contact cache the desktop app wrote, with names and avatars that are out of date (#1017)
    let cache = hub.image_cache_root().parent().unwrap().to_path_buf();
    let stale = serde_json::json!({
        "wxid_bob": { "displayName": "Stale Bob", "avatarUrl": "https://stale.invalid/bob.jpg" },
        "room1@chatroom": { "displayName": "Stale Room", "avatarUrl": "https://stale.invalid/room.jpg" },
    });
    std::fs::write(cache.join("contacts.json"), stale.to_string()).unwrap();
    let is_stale = |name: &str, avatar: Option<&str>| {
        name.starts_with("Stale") || avatar.is_some_and(|a| a.contains("stale.invalid"))
    };

    let sessions = hub.chat_sessions_list().unwrap();
    assert!(!sessions.is_empty());
    for s in &sessions {
        assert!(!is_stale(&s.display_name, s.avatar_url.as_deref()), "{s:?}");
    }
    let groups = hub.group_chats().unwrap();
    let room = groups
        .iter()
        .find(|g| g.username == "room1@chatroom")
        .unwrap();
    assert_eq!(room.display_name, "Project Room");
    assert!(
        !is_stale(&room.display_name, room.avatar_url.as_deref()),
        "{room:?}"
    );
}

#[test]
fn contacts_are_classified() {
    let (_root, hub) = common::mock_hub("contacts-list");
    let contacts = hub.chat_contacts_parsed().unwrap();
    let find = |u: &str| contacts.iter().find(|c| c.username == u);
    assert_eq!(find("wxid_bob").unwrap().kind, ContactKind::Friend);
    assert_eq!(find("gh_news").unwrap().kind, ContactKind::Official);
    assert_eq!(find("room1@chatroom").unwrap().kind, ContactKind::Group);
    assert!(find("medianote").is_none());
    let bob = find("wxid_bob").unwrap();
    assert_eq!(bob.display_name, "Bobby");
    assert_eq!(bob.remark.as_deref(), Some("Bobby"));
    assert_eq!(bob.nickname.as_deref(), Some("Bob"));
}
