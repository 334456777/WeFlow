//! The views of the chat reads: the conversation list, the contacts, one contact, the contact counts and a
//! conversation's details.

use serde_json::Value;
use weflow_core::colour::Role;

use super::layout::{
    digits, note, one_line, verbose_hint, with_kind, Align, Card, Style, Table, Text,
};
use super::{number, text, when, Shown};

/// The columns a name takes in a list at most, however wide the terminal is.
const NAME_COLUMNS: usize = 24;

/// What kind of conversation `username` is, as a grey tag after its name: `群` for a group, `公众号` for an official
/// account, nothing for a person.
fn kind_tag(username: &str, style: &Style) -> &'static str {
    if username.ends_with("@chatroom") {
        style.tr("group", "群")
    } else if username.starts_with("gh_") {
        style.tr("official", "公众号")
    } else {
        ""
    }
}

/// `name`, cut to [`NAME_COLUMNS`], with its kind tag after it.
fn tagged_name(name: &str, username: &str, style: &Style) -> Text {
    let tag = kind_tag(username, style);
    let room = if tag.is_empty() {
        NAME_COLUMNS
    } else {
        NAME_COLUMNS - 1 - weflow_core::render::display_width(tag)
    };
    let name = match one_line(name) {
        // a name of nothing a terminal shows
        shown if shown.is_empty() => Text::styled(Role::Muted, username),
        shown => Text::plain(shown),
    }
    .cut(room);
    if tag.is_empty() {
        name
    } else {
        name.then(" ").then_as(Role::Muted, tag)
    }
}

/// `chat sessions`: one line a conversation, the newest first.
pub(super) fn sessions(data: &Value, limit: usize, style: &Style) -> Option<Shown> {
    let all = data.as_array()?;
    let shown = if limit > 0 {
        &all[..limit.min(all.len())]
    } else {
        all
    };
    let unread = shown.iter().any(|s| number(s, "unreadCount") > 0);
    let mut table = Table::new(&[
        (style.tr("time", "时间"), Align::Left),
        (style.tr("unread", "未读"), Align::Right),
        (style.tr("conversation", "会话"), Align::Left),
        (style.tr("last message", "最后一条"), Align::Left),
    ]);
    for session in shown {
        let username = text(session, "username");
        let time = match number(session, "lastTimestamp") {
            0 => number(session, "sortTimestamp"),
            t => t,
        };
        let count = number(session, "unreadCount");
        let unread_cell = match (unread, count) {
            (false, _) => Text::default(),
            (true, 0) => Text::styled(Role::Muted, "-"),
            (true, n) => Text::styled(Role::Highlight, digits(n)),
        };
        table.row(vec![
            Text::styled(Role::Muted, when::short(time, style)),
            unread_cell,
            tagged_name(text(session, "displayName"), username, style),
            last_message(session, style),
        ]);
    }
    let mut parts = vec![if shown.len() < all.len() {
        format!(
            "{} {} / {}",
            style.tr("conversations", "会话"),
            digits(i64::try_from(shown.len()).unwrap_or(0)),
            digits(i64::try_from(all.len()).unwrap_or(0))
        )
    } else {
        style.count(
            i64::try_from(all.len()).unwrap_or(0),
            "conversation",
            "conversations",
            "个会话",
        )
    }];
    if shown.len() < all.len() {
        parts.push(
            style
                .tr("--limit 0 lists all", "--limit 0 列出全部")
                .to_string(),
        );
    }
    parts.push(verbose_hint(style));
    Some(Shown::of(table.lines(style), note(&parts, style)))
}

/// A conversation's last message, after who sent it in a group: `我：[视频]`.
fn last_message(session: &Value, style: &Style) -> Text {
    let summary = with_kind(&one_line(text(session, "summary")));
    if !text(session, "username").ends_with("@chatroom") {
        return summary;
    }
    let sender = text(session, "lastMsgSender");
    let mine = !sender.is_empty() && sender == text(session, "selfWxid");
    let name = text(session, "lastSenderDisplayName");
    if mine {
        Text::styled(Role::Success, style.tr("Me", "我"))
            .then_as(Role::Muted, style.colon())
            .append(summary)
    } else if !name.is_empty() {
        Text::plain(one_line(name))
            .then_as(Role::Muted, style.colon())
            .append(summary)
    } else {
        summary
    }
}

/// A contact's name: the remark, then the nickname in grey when it is another name; when the name has nothing a
/// terminal shows, the WeChat ID or the username, grey.
fn contact_name(contact: &Value) -> Text {
    let shown = one_line(text(contact, "displayName"));
    let nickname = one_line(text(contact, "nickname"));
    if shown.is_empty() {
        let other = [text(contact, "alias"), text(contact, "username")]
            .into_iter()
            .find(|n| !n.is_empty())
            .unwrap_or("");
        return Text::styled(Role::Muted, one_line(other));
    }
    let remarked = !text(contact, "remark").is_empty();
    let name = Text::plain(shown.as_str()).cut(NAME_COLUMNS);
    if remarked && !nickname.is_empty() && nickname != shown {
        let room = NAME_COLUMNS.saturating_sub(name.width() + 1);
        if room >= 4 {
            return name
                .then(" ")
                .append(Text::styled(Role::Muted, nickname).cut(room));
        }
    }
    name
}

/// `chat contacts`: the friends, the groups, the official accounts and the former friends, a table each.
pub(super) fn contacts(data: &Value, style: &Style) -> Option<Shown> {
    let all = data.as_array()?;
    let kinds = [
        ("friend", style.tr("Friends", "好友")),
        ("group", style.tr("Groups", "群聊")),
        ("official", style.tr("Official accounts", "公众号")),
        ("former_friend", style.tr("Former friends", "已删好友")),
    ];
    let mut lines = Vec::new();
    for (kind, heading) in kinds {
        let of_kind: Vec<&Value> = all.iter().filter(|c| text(c, "type") == kind).collect();
        if of_kind.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!(
            "{} {}",
            style.paint.paint(Role::Strong, heading),
            style.paint.paint(
                Role::Muted,
                &digits(i64::try_from(of_kind.len()).unwrap_or(0))
            )
        ));
        let about = if kind == "official" {
            style.tr("about", "简介")
        } else {
            style.tr("signature", "签名")
        };
        let mut table = Table::new(&[
            (style.tr("name", "名称"), Align::Left),
            (style.tr("WeChat ID", "微信号"), Align::Left),
            (style.tr("region", "地区"), Align::Left),
            (style.tr("labels", "标签"), Align::Left),
            (about, Align::Left),
        ])
        .cap(0, NAME_COLUMNS)
        .cap(1, 18)
        .cap(2, 14);
        for contact in of_kind {
            let labels: Vec<&str> = contact["labels"]
                .as_array()
                .map(|l| l.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            table.row(vec![
                contact_name(contact),
                Text::plain(text(contact, "alias")),
                Text::plain(one_line(text(contact, "region"))),
                Text::styled(Role::Accent, labels.join(style.tr(", ", "、"))),
                Text::styled(Role::Muted, one_line(text(contact, "detailDescription"))),
            ]);
        }
        lines.extend(table.lines(style));
    }
    let parts = vec![
        style.count(
            i64::try_from(all.len()).unwrap_or(0),
            "contact",
            "contacts",
            "位联系人",
        ),
        verbose_hint(style),
    ];
    Some(Shown::of(lines, note(&parts, style)))
}

/// What a contact row of WeChat's `contact` table is: a group, an official account, a friend or someone else.
fn row_kind(row: &Value, style: &Style) -> &'static str {
    let username = text(row, "username");
    if username.ends_with("@chatroom") {
        style.tr("group chat", "群聊")
    } else if username.starts_with("gh_") {
        style.tr("official account", "公众号")
    } else if number(row, "local_type") == 1 {
        style.tr("friend", "好友")
    } else {
        style.tr("not a friend", "非好友")
    }
}

/// `chat contact`: the names of one contact row as WeChat keeps it.
pub(super) fn contact(data: &Value, style: &Style) -> Option<Shown> {
    let row = data.as_object()?;
    if row.is_empty() {
        return Some(Shown::of(
            vec![style.tr("No such contact", "没有这个联系人").to_string()],
            None,
        ));
    }
    let remark = text(data, "remark");
    let nickname = text(data, "nick_name");
    let title = [
        remark,
        nickname,
        text(data, "alias"),
        text(data, "username"),
    ]
    .into_iter()
    .find(|n| !n.is_empty())?;
    let mut card = Card::default();
    if !remark.is_empty() && nickname != title {
        card.field(
            style.tr("nickname", "昵称"),
            Text::plain(one_line(nickname)),
        );
    }
    card.field(
        style.tr("WeChat ID", "微信号"),
        Text::plain(text(data, "alias")),
    );
    card.field(style.tr("kind", "类型"), Text::plain(row_kind(data, style)));
    card.field(
        style.tr("description", "简介"),
        Text::plain(one_line(text(data, "description"))),
    );
    let mut lines = vec![style.paint.paint(Role::Strong, &one_line(title))];
    lines.extend(card.lines(2, style));
    Some(Shown::of(lines, note(&[verbose_hint(style)], style)))
}

/// `chat tab-counts`: how many contacts of each kind.
pub(super) fn tab_counts(data: &Value, style: &Style) -> Option<Shown> {
    data.as_object()?;
    let mut table = Table::new(&[("", Align::Left), ("", Align::Right)]);
    for (key, label) in [
        ("private", style.tr("Friends", "好友")),
        ("group", style.tr("Groups", "群聊")),
        ("official", style.tr("Official accounts", "公众号")),
        ("former_friend", style.tr("Former friends", "已删好友")),
    ] {
        table.row(vec![
            Text::styled(Role::Muted, label),
            Text::plain(digits(number(data, key))),
        ]);
    }
    // the header row of two empty headers is a blank line
    Some(Shown::of(table.lines(style).split_off(1), None))
}

/// `chat detail`: who the conversation is with, how many messages it has, over what time, and where they are kept.
pub(super) fn detail(data: &Value, style: &Style) -> Option<Shown> {
    data.as_object()?;
    let username = text(data, "wxid");
    let name = text(data, "displayName");
    let mut card = Card::default();
    if !text(data, "remark").is_empty() && text(data, "nickName") != name {
        card.field(
            style.tr("nickname", "昵称"),
            Text::plain(one_line(text(data, "nickName"))),
        );
    }
    card.field(
        style.tr("WeChat ID", "微信号"),
        Text::plain(text(data, "alias")),
    );
    if data.get("messageCount").is_some() {
        card.field(
            style.tr("messages", "消息"),
            Text::plain(style.count(number(data, "messageCount"), "message", "messages", "条")),
        );
    }
    let first = number(data, "firstMessageTime");
    let latest = number(data, "latestMessageTime");
    if first > 0 && latest > 0 {
        card.field(
            style.tr("span", "时间跨度"),
            Text::plain(format!("{} ~ {}", when::full(first), when::full(latest))),
        );
    }
    let tables: Vec<Text> = data["messageTables"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            Text::plain(text(t, "dbName")).then_as(
                Role::Muted,
                format!(
                    "  {}",
                    style.count(number(t, "count"), "message", "messages", "条")
                ),
            )
        })
        .collect();
    card.lines_field(style.tr("kept in", "存放位置"), tables);
    let mut lines = Vec::new();
    if !name.is_empty() {
        let kind = if username.ends_with("@chatroom") {
            style.tr(" (group chat)", "（群聊）")
        } else if username.starts_with("gh_") {
            style.tr(" (official account)", "（公众号）")
        } else {
            ""
        };
        lines.push(format!(
            "{}{}",
            style.paint.paint(Role::Strong, &one_line(name)),
            style.paint.paint(Role::Muted, kind)
        ));
    }
    lines.extend(card.lines(if name.is_empty() { 0 } else { 2 }, style));
    let hint = style.tr(
        "--verbose shows the wxid, the avatar and every other field",
        "--verbose 显示 wxid、头像地址等全部字段",
    );
    Some(Shown::of(lines, note(&[hint.to_string()], style)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::layout::tests::style;
    use chrono::{Local, TimeZone};
    use serde_json::json;
    use weflow_core::locale::Lang;

    fn at(d: u32, h: u32, m: u32) -> i64 {
        Local
            .with_ymd_and_hms(2026, 11, d, h, m, 0)
            .unwrap()
            .timestamp()
    }

    #[test]
    fn sessions_take_a_line_each_and_say_how_many_there_are() {
        let data = json!([
            { "username": "1@chatroom", "unreadCount": 0, "summary": "[视频]", "lastTimestamp": at(23, 11, 16),
              "displayName": "示例群聊", "lastMsgSender": "wxid_me", "selfWxid": "wxid_me",
              "avatarUrl": "https://example.invalid/a" },
            { "username": "wxid_b", "unreadCount": 3, "summary": "嗯嗯\n嗯", "lastTimestamp": at(22, 9, 0),
              "displayName": "小测.", "selfWxid": "wxid_me" },
            { "username": "wxid_c", "unreadCount": 0, "summary": "x", "lastTimestamp": at(20, 9, 0),
              "displayName": "c", "selfWxid": "wxid_me" },
        ]);
        let shown = sessions(&data, 2, &style(Lang::Zh, 80)).unwrap();
        assert_eq!(
            shown.body,
            "时间   未读  会话         最后一条\n\
             11:16     -  示例群聊 群  我：[视频]\n\
             昨天      3  小测.        嗯嗯 嗯\n"
        );
        assert_eq!(
            shown.note.as_deref(),
            Some("会话 2 / 3 · --limit 0 列出全部 · --verbose 显示全部字段\n")
        );
        assert!(!shown.body.contains("wxid") && !shown.body.contains("https"));
    }

    #[test]
    fn contacts_are_grouped_by_kind() {
        let data = json!([
            { "username": "wxid_a", "displayName": "阿甲", "remark": "阿甲", "nickname": "Abc", "alias": "abc1",
              "region": "某省 某市", "type": "friend" },
            { "username": "1@chatroom", "displayName": "示例群聊", "type": "group" },
        ]);
        let shown = contacts(&data, &style(Lang::Zh, 80)).unwrap();
        assert_eq!(
            shown.body,
            "好友 1\n名称      微信号  地区\n阿甲 Abc  abc1    某省 某市\n\n群聊 1\n名称\n示例群聊\n"
        );
    }

    #[test]
    fn a_detail_reads_as_a_card() {
        let data = json!({
            "wxid": "1@chatroom", "displayName": "示例群聊", "nickName": "示例群聊", "messageCount": 123_456,
            "firstMessageTime": Local.with_ymd_and_hms(2025, 9, 1, 8, 30, 0).unwrap().timestamp(),
            "latestMessageTime": at(23, 11, 16),
            "messageTables": [
                { "dbName": "message_0.db", "tableName": "Msg_x", "count": 23_456 },
                { "dbName": "message_1.db", "tableName": "Msg_x", "count": 100_000 },
            ],
        });
        let shown = detail(&data, &style(Lang::Zh, 80)).unwrap();
        assert_eq!(
            shown.body,
            "示例群聊（群聊）\n\
             \x20 消息      123,456 条\n\
             \x20 时间跨度  2025-09-01 08:30 ~ 2026-11-23 11:16\n\
             \x20 存放位置  message_0.db  23,456 条\n\
             \x20           message_1.db  100,000 条\n"
        );
    }

    #[test]
    fn tab_counts_line_their_numbers_up() {
        let data = json!({ "private": 123, "group": 5, "official": 12, "former_friend": 4 });
        assert_eq!(
            tab_counts(&data, &style(Lang::Zh, 80)).unwrap().body,
            "好友      123\n群聊        5\n公众号     12\n已删好友    4\n"
        );
    }

    #[test]
    fn an_answer_of_another_shape_is_left_to_the_plain_rendering() {
        let style = style(Lang::En, 80);
        assert_eq!(sessions(&json!({}), 0, &style), None);
        assert_eq!(detail(&json!([]), &style), None);
    }
}
