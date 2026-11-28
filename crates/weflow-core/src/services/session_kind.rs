//! What kind of conversation a session's username names, as the statistics, the annual report and the AI insights
//! tell a private chat from the rest.

/// The kind of conversation a session's username names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionKind {
    /// A chat with another person.
    Private,
    /// A group chat (`…@chatroom`).
    Group,
    /// The account's chat with itself.
    Own,
    /// No person: an official account (`gh_…`), the file helper, WeChat's system conversations and folders, and
    /// the WeCom and customer service accounts; also an empty username.
    Service,
}

/// The usernames of WeChat's system conversations and folders; a username that starts with one is one of them.
const SERVICE_PREFIXES: [&str; 15] = [
    "qqmail",
    "fmessage",
    "medianote",
    "floatbottle",
    "newsapp",
    "brandsessionholder",
    "brandservicesessionholder",
    "notifymessage",
    "opencustomerservicemsg",
    "notification_messages",
    "userexperience_alarm",
    "helper_folders",
    "placeholder_foldgroup",
    "@helper_folders",
    "@placeholder_foldgroup",
];

/// WeChat's own accounts that its contact table lists as friends (the voice notes, the drift bottle, the QQ
/// messages, QQ Mail, friend requests); the contact lists do not count them.
pub(super) const NOT_FRIENDS: [&str; 5] =
    ["medianote", "floatbottle", "qmessage", "qqmail", "fmessage"];

/// Whether `username` is an official account's (`gh_…`).
pub(super) fn is_official(username: &str) -> bool {
    username.starts_with("gh_")
}

/// Whether `username` is one of WeChat's system conversations or folders ([`SERVICE_PREFIXES`]).
pub(super) fn is_system_conversation(username: &str) -> bool {
    SERVICE_PREFIXES.iter().any(|p| username.starts_with(p))
}

/// Whether the contact `username` may be a friend: not empty, not a group, not an official account and not one of
/// [`NOT_FRIENDS`]. The contact's own type decides the rest.
pub(super) fn may_be_friend(username: &str) -> bool {
    !(username.is_empty()
        || username.contains("@chatroom")
        || is_official(username)
        || NOT_FRIENDS.contains(&username))
}

/// The kind of the conversation `username` names, for the account whose wxid without WeChat's suffix is `own_wxid`
/// ([`crate::config::clean_account_dir_name`]).
pub(super) fn session_kind(username: &str, own_wxid: &str) -> SessionKind {
    let lowered = username.to_lowercase();
    if username.is_empty() {
        SessionKind::Service
    } else if lowered == own_wxid.to_lowercase() {
        SessionKind::Own
    } else if username.contains("@chatroom") {
        SessionKind::Group
    } else if is_official(username)
        || username == "filehelper"
        || lowered == "weixin"
        || is_system_conversation(username)
        || username.contains("@kefu.openim")
        || username.contains("@openim")
        || username.contains("service_")
    {
        SessionKind::Service
    } else {
        SessionKind::Private
    }
}

/// Whether `username` names a chat with another person ([`SessionKind::Private`]).
pub(super) fn is_private_session(username: &str, own_wxid: &str) -> bool {
    session_kind(username, own_wxid) == SessionKind::Private
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_private_only_with_another_person() {
        let own = "wxid_me";
        for (username, kind) in [
            ("wxid_friend", SessionKind::Private),
            ("alice", SessionKind::Private),
            ("WXID_ME", SessionKind::Own),
            ("123@chatroom", SessionKind::Group),
            ("gh_abc", SessionKind::Service),
            ("filehelper", SessionKind::Service),
            ("weixin", SessionKind::Service),
            ("Weixin", SessionKind::Service),
            ("newsapp", SessionKind::Service),
            ("@placeholder_foldgroup", SessionKind::Service),
            ("abc@openim", SessionKind::Service),
            ("abc@kefu.openim", SessionKind::Service),
            ("service_abc", SessionKind::Service),
            ("", SessionKind::Service),
        ] {
            assert_eq!(session_kind(username, own), kind, "{username}");
            assert_eq!(
                is_private_session(username, own),
                kind == SessionKind::Private,
                "{username}"
            );
        }
    }

    #[test]
    fn a_friend_is_none_of_groups_official_accounts_and_wechats_own() {
        for maybe in ["wxid_friend", "alice", "weixin", "filehelper"] {
            assert!(may_be_friend(maybe), "{maybe}");
        }
        for not in [
            "",
            "1@chatroom",
            "gh_abc",
            "medianote",
            "fmessage",
            "qmessage",
        ] {
            assert!(!may_be_friend(not), "{not:?}");
        }
        assert!(is_official("gh_abc") && !is_official("wxid_gh_abc"));
        assert!(is_system_conversation("notifymessage") && !is_system_conversation("wxid_a"));
    }
}
