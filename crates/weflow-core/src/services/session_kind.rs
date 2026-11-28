//! 会话的用户名指的是哪种会话：统计、年度报告和 AI 见解按这里区分私聊和其他会话。

/// 会话的用户名所指的会话种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionKind {
    /// 和另一个人的聊天。
    Private,
    /// 群聊（`…@chatroom`）。
    Group,
    /// 账号和自己的聊天。
    Own,
    /// 不是人：公众号（`gh_…`）、文件传输助手、微信的系统会话和文件夹、企业微信和客服账号；空的用户名也属于这一类。
    Service,
}

/// 微信的系统会话和文件夹的用户名；以其中之一开头的用户名都属于这一类。
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

/// 联系人表列为好友的微信自己的账号（语音记事本、漂流瓶、QQ 消息、QQ 邮箱、好友请求），联系人列表不算它们。
pub(super) const NOT_FRIENDS: [&str; 5] =
    ["medianote", "floatbottle", "qmessage", "qqmail", "fmessage"];

/// `username` 是否是公众号（`gh_…`）。
pub(super) fn is_official(username: &str) -> bool {
    username.starts_with("gh_")
}

/// `username` 是否是微信的系统会话或文件夹（[`SERVICE_PREFIXES`]）。
pub(super) fn is_system_conversation(username: &str) -> bool {
    SERVICE_PREFIXES.iter().any(|p| username.starts_with(p))
}

/// 联系人 `username` 是否可能是好友：不为空，不是群和公众号，也不在 [`NOT_FRIENDS`] 中。其余由联系人自己的类型决定。
pub(super) fn may_be_friend(username: &str) -> bool {
    !(username.is_empty()
        || username.contains("@chatroom")
        || is_official(username)
        || NOT_FRIENDS.contains(&username))
}

/// `username` 所指会话的种类；`own_wxid` 是账号的裸 wxid（[`crate::config::clean_account_dir_name`]）。
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

/// `username` 是否是和另一个人的私聊（[`SessionKind::Private`]）。
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
