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
