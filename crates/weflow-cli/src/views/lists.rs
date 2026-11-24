//! The views of the other lists: the groups, a group's members, the official accounts, and who posts on Moments.

use serde_json::Value;
use weflow_core::colour::Role;

use super::layout::{digits, note, one_line, verbose_hint, Align, Style, Table, Text};
use super::{number, text, when, Names, Shown};

/// The columns a name of a grid of names takes at most.
const GRID_NAME: usize = 20;

/// `group list`: the groups, the largest first.
pub(super) fn groups(data: &Value, style: &Style) -> Option<Shown> {
    let all = data.as_array()?;
    let mut table = Table::new(&[
        (style.tr("members", "成员"), Align::Right),
        (style.tr("group", "群聊"), Align::Left),
    ]);
    for group in all {
        let name = text(group, "displayName");
        let unnamed = name.is_empty() || name == text(group, "username");
        table.row(vec![
            Text::plain(digits(number(group, "memberCount"))),
            if unnamed {
                Text::styled(Role::Muted, style.tr("(no name)", "（未命名）"))
            } else {
                Text::plain(one_line(name))
            },
        ]);
    }
    let parts = vec![
        style.count(
            i64::try_from(all.len()).unwrap_or(0),
            "group",
            "groups",
            "个群聊",
        ),
        verbose_hint(style),
    ];
    Some(Shown::of(table.lines(style), note(&parts, style)))
}

/// `group members`: the owner first, then the friends, then by their messages.
pub(super) fn members(
    data: &Value,
    chatroom_id: &str,
    counts: bool,
    names: &Names,
    style: &Style,
) -> Option<Shown> {
    let members = data["members"].as_array()?;
    let mut table = Table::new(&[
        (style.tr("member", "成员"), Align::Left),
        (style.tr("group nickname", "群昵称"), Align::Left),
        (style.tr("WeChat ID", "微信号"), Align::Left),
        ("", Align::Left),
        (style.tr("messages", "消息"), Align::Right),
    ])
    .cap(0, 24)
    .cap(1, 20);
    for member in members {
        let nickname = one_line(text(member, "groupNickname"));
        // the name the group shows, else (when it has nothing a terminal shows) another name of theirs
        let shown = ["displayName", "groupNickname", "nickname", "username"]
            .into_iter()
            .map(|key| one_line(text(member, key)))
            .find(|name| !name.is_empty())
            .unwrap_or_default();
        let mut marks = Vec::new();
        if member["isOwner"] == true {
            marks.push(style.tr("owner", "群主"));
        }
        if member["isFriend"] == true {
            marks.push(style.tr("friend", "好友"));
        }
        let nickname = if nickname == shown {
            String::new()
        } else {
            nickname
        };
        table.row(vec![
            Text::plain(shown),
            Text::styled(Role::Muted, nickname),
            Text::plain(text(member, "alias")),
            Text::styled(Role::Accent, marks.join(" ")),
            if counts {
                Text::plain(digits(number(member, "messageCount")))
            } else {
                Text::default()
            },
        ]);
    }
    let group = names.get(chatroom_id).map_or(chatroom_id, String::as_str);
    let mut lines = vec![format!(
        "{}{}",
        style.paint.paint(Role::Strong, &one_line(group)),
        style.paint.paint(
            Role::Muted,
            &format!(
                " · {}",
                style.count(number(data, "count"), "member", "members", "位成员")
            )
        )
    )];
    lines.push(String::new());
    lines.extend(table.lines(style));
    let mut parts = Vec::new();
    if !counts {
        parts.push(
            style
                .tr("--counts adds their messages", "--counts 加上各自的消息数")
                .to_string(),
        );
    }
    parts.push(verbose_hint(style));
    Some(Shown::of(lines, note(&parts, style)))
}

/// `biz accounts`: the official accounts followed, with their WeChat ID and their last push.
pub(super) fn biz_accounts(data: &Value, style: &Style) -> Option<Shown> {
    let accounts = data["accounts"].as_array()?;
    let mut table = Table::new(&[
        (style.tr("official account", "公众号"), Align::Left),
        (style.tr("WeChat ID", "微信号"), Align::Left),
        (style.tr("last push", "最近推送"), Align::Left),
    ])
    .cap(0, 30);
    for account in accounts {
        let name = [
            text(account, "remark"),
            text(account, "nick_name"),
            text(account, "username"),
        ]
        .into_iter()
        .find(|n| !n.is_empty())
        .unwrap_or("");
        table.row(vec![
            Text::plain(one_line(name)),
            Text::plain(text(account, "alias")),
            Text::styled(Role::Muted, when::short(number(account, "lastTime"), style)),
        ]);
    }
    let parts = vec![
        style.count(
            i64::try_from(accounts.len()).unwrap_or(0),
            "official account",
            "official accounts",
            "个公众号",
        ),
        verbose_hint(style),
    ];
    Some(Shown::of(table.lines(style), note(&parts, style)))
}

/// `sns users`: who has posts on Moments, by name, in columns across the width.
pub(super) fn sns_users(data: &Value, names: &Names, style: &Style) -> Option<Shown> {
    let users = data["usernames"].as_array()?;
    let shown: Vec<Text> = users
        .iter()
        .filter_map(Value::as_str)
        .map(|u| match names.get(u) {
            Some(name) => Text::plain(one_line(name)).cut(GRID_NAME),
            None => Text::styled(Role::Muted, u).cut(GRID_NAME),
        })
        .collect();
    let cell = shown.iter().map(Text::width).max().unwrap_or(0) + 2;
    let per_line = (style.width / cell.max(1)).max(1);
    let lines: Vec<String> = shown
        .chunks(per_line)
        .map(|row| {
            row.iter()
                .map(|t| {
                    let pad = " ".repeat(cell - t.width());
                    format!("{}{pad}", t.show(style.paint))
                })
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    let lines = if lines.is_empty() {
        vec![style.tr("Nobody has posts", "没有人发过朋友圈").to_string()]
    } else {
        lines
    };
    let parts = vec![style.count(
        i64::try_from(shown.len()).unwrap_or(0),
        "person posts",
        "people post",
        // the account itself among them: not all of them are friends (#1158)
        "人发过朋友圈",
    )];
    Some(Shown::of(lines, note(&parts, style)))
}

/// `sns stats`: the posts on Moments, how many people posted them (the account itself among them), and how many are
/// mine.
pub(super) fn sns_stats(data: &Value, style: &Style) -> Option<Shown> {
    data.get("totalPosts")?;
    let line = match style.lang {
        weflow_core::locale::Lang::Zh => format!(
            "{}  动态 {} 条 · {} 人发过 · 其中你发了 {} 条",
            style.paint.paint(Role::Strong, "朋友圈"),
            digits(number(data, "totalPosts")),
            digits(number(data, "totalFriends")),
            digits(number(data, "myPosts"))
        ),
        weflow_core::locale::Lang::En => format!(
            "{}  {} posts · by {} people · {} of them mine",
            style.paint.paint(Role::Strong, "Moments"),
            digits(number(data, "totalPosts")),
            digits(number(data, "totalFriends")),
            digits(number(data, "myPosts"))
        ),
    };
    Some(Shown::of(vec![line], None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::layout::tests::style;
    use serde_json::json;
    use weflow_core::locale::Lang;

    #[test]
    fn groups_without_a_name_say_so() {
        let data = json!([
            { "username": "1@chatroom", "displayName": "示例群聊", "memberCount": 42 },
            { "username": "2@chatroom", "displayName": "2@chatroom", "memberCount": 0 },
        ]);
        let shown = groups(&data, &style(Lang::Zh, 80)).unwrap();
        assert_eq!(shown.body, "成员  群聊\n  42  示例群聊\n   0  （未命名）\n");
    }

    #[test]
    fn members_mark_the_owner_and_friends() {
        let data = json!({ "chatroomId": "1@chatroom", "count": 2, "updatedAt": 0, "members": [
            { "username": "a", "displayName": "样例123（演示）", "groupNickname": "样例123", "alias": "ab",
              "isOwner": true, "isFriend": true, "messageCount": 0, "nickname": "", "remark": "" },
            { "username": "b", "displayName": "Bob", "groupNickname": "Bob", "alias": "",
              "isOwner": false, "isFriend": false, "messageCount": 0, "nickname": "", "remark": "" },
        ]});
        let names: Names = [("1@chatroom".to_string(), "示例群聊".to_string())].into();
        let shown = members(&data, "1@chatroom", false, &names, &style(Lang::Zh, 80)).unwrap();
        assert_eq!(
            shown.body,
            "示例群聊 · 2 位成员\n\n\
             成员             群昵称   微信号\n\
             样例123（演示）  样例123  ab      群主 好友\n\
             Bob\n"
        );
    }

    #[test]
    fn moments_users_are_named_in_columns() {
        let data = json!({ "usernames": ["a", "b", "c"] });
        let names: Names = [("a", "阿甲"), ("b", "Bob")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let shown = sns_users(&data, &names, &style(Lang::En, 12)).unwrap();
        assert_eq!(shown.body, "阿甲  Bob\nc\n");
    }

    /// The people who posted include the account itself, so they are not called friends (#1158).
    #[test]
    fn moments_stats_count_people_not_friends() {
        let data = json!({ "totalPosts": 5, "totalFriends": 3, "myPosts": 2 });
        let zh = sns_stats(&data, &style(Lang::Zh, 80)).unwrap();
        assert!(
            zh.body.contains("动态 5 条 · 3 人发过 · 其中你发了 2 条"),
            "{}",
            zh.body
        );
        assert!(!zh.body.contains("好友"), "{}", zh.body);
        let en = sns_stats(&data, &style(Lang::En, 80)).unwrap();
        assert!(
            en.body.contains("5 posts · by 3 people · 2 of them mine"),
            "{}",
            en.body
        );
    }
}
