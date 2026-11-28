//! Messages as a chat log: a line a message under the day it was sent, who sent it by name, what it says as the
//! chat window shows it (`[图片]`, `[语音 12″]`, a file's size, the message a reply quotes), and where to read on.

use serde_json::Value;
use weflow_core::colour::Role;
use weflow_core::render::display_width;

use super::layout::{note, one_line, verbose_hint, with_kind, wrap, Align, Style, Table, Text};
use super::{number, text, when, Names, Shown};

/// The columns a sender's name takes at most.
const SENDER_COLUMNS: usize = 12;
/// The lines a message of a page takes at most; the rest is left to `--verbose`.
const MESSAGE_LINES: usize = 6;
/// The fewest columns a message's text is wrapped to, however narrow the terminal.
const TEXT_COLUMNS: usize = 20;
/// The columns of a day's rule at most.
const RULE_COLUMNS: usize = 44;
/// The types of a system notice (WeChat's own lines, a withdrawn message, a pat), shown grey without a sender.
const NOTICE_TYPES: [i64; 3] = [10000, 10002, 266_287_972_401];

/// Who sent `message`: `我` for the account itself, else their name in the chat, else their username; in a private
/// chat without a sender, the other party.
fn sender(message: &Value, session_id: &str, names: &Names, style: &Style) -> Text {
    if number(message, "isSend") == 1 {
        return Text::styled(Role::Success, style.tr("Me", "我"));
    }
    let username = match text(message, "senderUsername") {
        "" if !session_id.ends_with("@chatroom") => session_id,
        username => username,
    };
    match names.get(username) {
        Some(name) => Text::plain(one_line(name)),
        None if username.is_empty() => Text::styled(Role::Muted, "?"),
        None => Text::plain(username),
    }
}

/// What `message` says, as the chat window shows it, with a voice message's length and a file's size.
fn words_of(message: &Value) -> String {
    let content = message["parsedContent"].as_str().unwrap_or("").trim();
    let mut words = content.to_string();
    if let Some(seconds) = message["voiceDurationSeconds"].as_i64() {
        // the chat window labels a voice message `[语音消息]`, with its text or without (#1263)
        words = match content.strip_prefix("[语音消息]") {
            Some(rest) => format!("[语音 {seconds}″]{rest}"),
            None => format!("{content} {seconds}″"),
        };
    }
    if let Some(size) = message["fileSize"]
        .as_i64()
        .filter(|s| *s > 0 && text(message, "appMsgKind") == "file")
    {
        let size = crate::term::format_bytes(u64::try_from(size).unwrap_or(0));
        words = format!("{words} ({size})");
    }
    words
}

/// What a reply quotes, on a grey line of its own: `└ 引用 小测.：几点开周会？`.
fn quote(message: &Value, style: &Style) -> Option<Text> {
    let quoted = without_server_id(text(message, "quotedContent"));
    let who = one_line(text(message, "quotedSender"));
    if quoted.is_empty() && who.is_empty() {
        return None;
    }
    let mut line = format!("{}{}", style.glyphs.branch(), style.tr("quoting ", "引用 "));
    line.push_str(&who);
    if !quoted.is_empty() {
        if !who.is_empty() {
            line.push_str(style.colon());
        }
        line.push_str(&one_line(&quoted));
    }
    Some(Text::styled(Role::Muted, line))
}

/// `quoted` without the `__SVRID__<server id>__` a quote of a picture, video or voice names it by.
fn without_server_id(quoted: &str) -> String {
    let mut out = quoted.to_string();
    while let Some(start) = out.find("__SVRID__") {
        let after = start + "__SVRID__".len();
        let end = out[after..].find("__").map_or(out.len(), |e| after + e + 2);
        out.replace_range(start..end, "");
    }
    out.trim().to_string()
}

/// How a chat log is laid out: whether it has a sender column and how wide it is, and how many lines a message may
/// take.
struct Log<'a> {
    session_id: &'a str,
    names: &'a Names,
    style: &'a Style,
    senders: bool,
    sender_width: usize,
    max_lines: Option<usize>,
}

impl<'a> Log<'a> {
    fn new(
        messages: &[Value],
        session_id: &'a str,
        names: &'a Names,
        style: &'a Style,
        senders: bool,
    ) -> Self {
        let sender_width = if senders {
            messages
                .iter()
                .map(|m| sender(m, session_id, names, style).width())
                .max()
                .unwrap_or(0)
                .min(SENDER_COLUMNS)
        } else {
            0
        };
        Self {
            session_id,
            names,
            style,
            senders,
            sender_width,
            max_lines: Some(MESSAGE_LINES),
        }
    }

    /// The columns before a message's text.
    fn lead(&self) -> usize {
        5 + 2
            + if self.senders {
                self.sender_width + 2
            } else {
                0
            }
    }

    /// The lines of `messages`, a rule above each day.
    fn lines(&self, messages: &[Value]) -> Vec<String> {
        let mut out = Vec::new();
        let mut day = None;
        for message in messages {
            let time = number(message, "createTime");
            let this_day = when::local(time).map(|t| t.date_naive());
            if this_day.is_some() && this_day != day {
                if let Some(d) = this_day {
                    out.push(self.rule(&when::day_heading(d, self.style)));
                }
                day = this_day;
            }
            out.extend(self.message(message));
        }
        out
    }

    /// A day's rule: `──── 2026-11-23 周一 ────`, grey.
    fn rule(&self, heading: &str) -> String {
        let total = self.style.width.min(RULE_COLUMNS);
        let fill = total.saturating_sub(display_width(heading) + 2);
        let left = fill / 2;
        let rule = self.style.glyphs.rule();
        let line = format!(
            "{} {heading} {}",
            rule.repeat(left.max(2)),
            rule.repeat((fill - left).max(2))
        );
        self.style.paint.paint(Role::Muted, &line)
    }

    /// The lines of one message.
    fn message(&self, message: &Value) -> Vec<String> {
        let style = self.style;
        let paint = style.paint;
        let clock = paint.paint(
            Role::Muted,
            &format!("{:<5}", when::clock(number(message, "createTime"))),
        );
        let room = style.width.saturating_sub(self.lead()).max(TEXT_COLUMNS);
        let notice = NOTICE_TYPES.contains(&number(message, "localType"));
        let who = if self.senders && !notice {
            let who = sender(message, self.session_id, self.names, style).cut(self.sender_width);
            let pad = " ".repeat(self.sender_width - who.width());
            format!("{}{pad}  ", who.show(paint))
        } else if self.senders {
            " ".repeat(self.sender_width + 2)
        } else {
            String::new()
        };
        let blank = " ".repeat(self.lead());
        let words = words_of(message);
        let mut wrapped = wrap(&words, room);
        let all = wrapped.len();
        let cut = self.max_lines.is_some_and(|max| all > max);
        if let Some(max) = self.max_lines.filter(|_| cut) {
            wrapped.truncate(max - 1);
        }
        let mut out = Vec::with_capacity(wrapped.len() + 2);
        for (n, line) in wrapped.iter().enumerate() {
            let shown = if notice {
                Text::styled(Role::Muted, line.as_str())
            } else if n == 0 {
                with_kind(line)
            } else {
                Text::plain(line.as_str())
            };
            let lead = if n == 0 {
                format!("{clock}  {who}")
            } else {
                blank.clone()
            };
            out.push(
                format!("{lead}{}", shown.show(paint))
                    .trim_end()
                    .to_string(),
            );
        }
        if cut {
            let more = match style.lang {
                weflow_core::locale::Lang::Zh => format!("…（共 {all} 行，--verbose 显示全文）"),
                weflow_core::locale::Lang::En => {
                    format!("… ({all} lines in all; --verbose shows them)")
                }
            };
            out.push(format!("{blank}{}", paint.paint(Role::Muted, &more)));
        }
        if let Some(quote) = quote(message, style) {
            out.push(format!("{blank}{}", quote.cut(room).show(paint)));
        }
        out
    }
}

/// The heading of a chat log: the conversation's name, whether it is a group, how many messages.
fn heading(session_id: &str, count: usize, names: &Names, style: &Style) -> String {
    let name = names.get(session_id).map_or(session_id, String::as_str);
    let mut rest = String::new();
    if session_id.ends_with("@chatroom") {
        rest.push_str(" · ");
        rest.push_str(style.tr("group chat", "群聊"));
    }
    rest.push_str(" · ");
    rest.push_str(&style.count(
        i64::try_from(count).unwrap_or(0),
        "message",
        "messages",
        "条",
    ));
    format!(
        "{}{}",
        style.paint.paint(Role::Strong, &one_line(name)),
        style.paint.paint(Role::Muted, &rest)
    )
}

/// `chat messages`: a page of a conversation as a chat log, in the order of the page.
pub(super) fn page(
    data: &Value,
    session_id: &str,
    limit: i32,
    names: &Names,
    style: &Style,
) -> Option<Shown> {
    let messages = data.as_array()?;
    if messages.is_empty() {
        return Some(Shown::of(
            vec![style.tr("No messages", "没有消息").to_string()],
            None,
        ));
    }
    let mut lines = vec![
        heading(session_id, messages.len(), names, style),
        String::new(),
    ];
    lines.extend(Log::new(messages, session_id, names, style, true).lines(messages));
    let mut parts = Vec::new();
    let full = i32::try_from(messages.len()).is_ok_and(|n| limit > 0 && n >= limit);
    if let (true, Some(first), Some(last)) = (full, messages.first(), messages.last()) {
        let older = number(first, "createTime") >= number(last, "createTime");
        let cursor = text(last, "_cursor");
        if !cursor.is_empty() {
            let (words, flag) = if older {
                (style.tr("older", "更早的消息"), "--before")
            } else {
                (style.tr("newer", "更新的消息"), "--after")
            };
            parts.push(format!(
                "{words}{}weflow chat messages {session_id} {flag} {cursor}",
                style.colon()
            ));
        }
    }
    parts.push(verbose_hint(style));
    Some(Shown::of(lines, note(&parts, style)))
}

/// `chat message`: one message, all of its text.
pub(super) fn one(data: &Value, session_id: &str, names: &Names, style: &Style) -> Option<Shown> {
    if data.is_null() {
        return Some(Shown::of(
            vec![style.tr("No such message", "没有这条消息").to_string()],
            None,
        ));
    }
    data.as_object()?;
    let messages = std::slice::from_ref(data);
    let mut log = Log::new(messages, session_id, names, style, true);
    log.max_lines = None;
    let mut lines = vec![heading(session_id, 1, names, style)];
    lines.extend(log.lines(messages));
    Some(Shown::of(lines, note(&[verbose_hint(style)], style)))
}

/// `text` cut to `room` columns around the first `keyword` in it, the keyword soft yellow: a match far into a long
/// message is shown with `…` for what comes before it.
fn around(words: &str, keyword: &str, room: usize) -> Text {
    let lower = words.to_ascii_lowercase();
    let key = keyword.to_ascii_lowercase();
    let mut start = 0;
    if let Some(at) = lower.find(&key).filter(|_| !key.is_empty()) {
        let before = display_width(&words[..at]);
        if before > room / 3 {
            let mut skip = before - room / 3;
            for (i, c) in words.char_indices() {
                if skip == 0 || i >= at {
                    start = i;
                    break;
                }
                skip = skip.saturating_sub(display_width(c.encode_utf8(&mut [0; 4])));
                start = i + c.len_utf8();
            }
        }
    }
    let mut out = if start > 0 {
        Text::styled(Role::Muted, "…")
    } else {
        Text::default()
    };
    let rest = &words[start..];
    let rest_lower = &lower[start..];
    let mut from = 0;
    while let Some(at) = rest_lower[from..].find(&key).filter(|_| !key.is_empty()) {
        let at = from + at;
        out = out
            .then(&rest[from..at])
            .then_as(Role::Highlight, &rest[at..at + key.len()]);
        from = at + key.len();
    }
    out.then(&rest[from..])
}

/// `chat search`: a hit a line, with its conversation and sender, the keyword soft yellow.
pub(super) fn search(
    data: &Value,
    keyword: &str,
    limit: i32,
    offset: i32,
    names: &Names,
    style: &Style,
) -> Option<Shown> {
    let hits = data.as_array()?;
    let name = |username: &str| {
        names
            .get(username)
            .map_or(username, String::as_str)
            .to_string()
    };
    let mut table = Table::new(&[
        (style.tr("time", "时间"), Align::Left),
        (style.tr("conversation", "会话"), Align::Left),
        (style.tr("sender", "发送者"), Align::Left),
        (style.tr("message", "内容"), Align::Left),
    ])
    .cap(1, 16)
    .cap(2, SENDER_COLUMNS);
    let mut rows = Vec::new();
    for hit in hits {
        let session = text(hit, "_session_id");
        rows.push((
            Text::styled(Role::Muted, when::stamp(number(hit, "createTime"), style)),
            Text::plain(one_line(&name(session))),
            sender(hit, session, names, style),
            one_line(&words_of(hit)),
        ));
    }
    // the message column gets what the others leave of the width
    let others = rows
        .iter()
        .map(|(t, s, w, _)| (t.width(), s.width().min(16), w.width().min(SENDER_COLUMNS)))
        .fold((0, 0, 0), |a, b| (a.0.max(b.0), a.1.max(b.1), a.2.max(b.2)));
    let room = style
        .width
        .saturating_sub(others.0 + others.1 + others.2 + 6)
        .max(TEXT_COLUMNS);
    for (time, session, who, words) in rows {
        table.row(vec![time, session, who, around(&words, keyword, room)]);
    }
    let mut parts = vec![style.count(
        i64::try_from(hits.len()).unwrap_or(0),
        "hit",
        "hits",
        "条结果",
    )];
    if i32::try_from(hits.len()).is_ok_and(|n| limit > 0 && n >= limit) {
        let next = offset.saturating_add(limit);
        parts.push(match style.lang {
            weflow_core::locale::Lang::Zh => format!("--offset {next} 看后面的结果"),
            weflow_core::locale::Lang::En => format!("--offset {next} shows the next ones"),
        });
    }
    parts.push(verbose_hint(style));
    let lines = if table.is_empty() {
        vec![style.tr("Nothing found", "没有找到").to_string()]
    } else {
        table.lines(style)
    };
    Some(Shown::of(lines, note(&parts, style)))
}

/// `group member-messages`: one member's messages in a group, as a chat log without the sender.
pub(super) fn member(
    data: &Value,
    chatroom_id: &str,
    username: &str,
    limit: usize,
    names: &Names,
    style: &Style,
) -> Option<Shown> {
    let messages = data["messages"].as_array()?;
    let who = names.get(username).map_or(username, String::as_str);
    let group = names.get(chatroom_id).map_or(chatroom_id, String::as_str);
    let mut lines = vec![format!(
        "{}{}",
        style.paint.paint(Role::Strong, &one_line(who)),
        style.paint.paint(
            Role::Muted,
            &format!(
                "{}{} · {}",
                style.tr(" in ", " · "),
                one_line(group),
                style.count(
                    i64::try_from(messages.len()).unwrap_or(0),
                    "message",
                    "messages",
                    "条"
                )
            )
        )
    )];
    if messages.is_empty() {
        lines.push(style.tr("No messages", "没有消息").to_string());
    } else {
        lines.push(String::new());
        lines.extend(Log::new(messages, chatroom_id, names, style, false).lines(messages));
    }
    let mut parts = Vec::new();
    if data["hasMore"] == true {
        let next = number(data, "nextCursor");
        let words = style.tr("more", "更多");
        let limit = if limit == 50 {
            String::new()
        } else {
            format!(" --limit {limit}")
        };
        parts.push(format!(
            "{words}{}weflow group member-messages {chatroom_id} {username}{limit} --cursor {next}",
            style.colon()
        ));
    }
    parts.push(verbose_hint(style));
    Some(Shown::of(lines, note(&parts, style)))
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

    fn names() -> Names {
        [
            ("g@chatroom", "示例群聊"),
            ("wxid_a", "成员A"),
            ("wxid_b", "测试员."),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    #[test]
    fn a_page_reads_as_a_chat_log() {
        let data = json!([
            { "localType": 43, "createTime": at(23, 11, 16), "isSend": 1, "senderUsername": "wxid_me",
              "parsedContent": "[视频]", "rawContent": "<msg/>", "_cursor": "c1" },
            { "localType": 244_813_135_921_i64, "createTime": at(23, 11, 10), "isSend": 0,
              "senderUsername": "wxid_a", "parsedContent": "改到周四下午", "quotedContent": "几点开周会？",
              "quotedSender": "我", "_cursor": "c2" },
            { "localType": 34, "createTime": at(22, 23, 40), "isSend": 0, "senderUsername": "wxid_b",
              "parsedContent": "[语音消息]", "voiceDurationSeconds": 12, "_cursor": "c3" },
            { "localType": 10000, "createTime": at(22, 23, 39), "isSend": 0,
              "parsedContent": "成员A 拍了拍 测试员.", "_cursor": "c4" },
        ]);
        let shown = page(&data, "g@chatroom", 4, &names(), &style(Lang::Zh, 60)).unwrap();
        assert_eq!(
            shown.body,
            "示例群聊 · 群聊 · 4 条\n\
             \n\
             ─────────── 2026-11-23 周一 今天 ───────────\n\
             11:16  我       [视频]\n\
             11:10  成员A    改到周四下午\n\
             \x20               └ 引用 我：几点开周会？\n\
             ─────────── 2026-11-22 周日 昨天 ───────────\n\
             23:40  测试员.  [语音 12″]\n\
             23:39           成员A 拍了拍 测试员.\n"
        );
        assert_eq!(
            shown.note.as_deref(),
            Some("更早的消息：weflow chat messages g@chatroom --before c4 · --verbose 显示全部字段\n")
        );
    }

    #[test]
    fn a_long_message_is_cut_to_a_few_lines() {
        let long: Vec<String> = (1..=10).map(|n| format!("line {n}")).collect();
        let data = json!([{ "localType": 1, "createTime": at(23, 9, 0), "isSend": 1,
                             "parsedContent": long.join("\n") }]);
        let shown = page(&data, "wxid_a", 50, &names(), &style(Lang::En, 60)).unwrap();
        assert!(shown.body.contains("line 5\n"), "{}", shown.body);
        assert!(!shown.body.contains("line 6"), "{}", shown.body);
        assert!(shown
            .body
            .contains("(10 lines in all; --verbose shows them)"));
        // a page shorter than the limit is the last one
        assert_eq!(shown.note.as_deref(), Some("--verbose shows every field\n"));
    }

    #[test]
    fn a_hit_is_shown_around_its_keyword() {
        let words = format!("{}开会改到周四", "很长的开头".repeat(10));
        let shown = around(&words, "开会", 20);
        assert!(
            shown.plain_text().starts_with('…'),
            "{}",
            shown.plain_text()
        );
        assert!(shown.plain_text().contains("开会改到周四"));
        assert_eq!(
            around("Hello World", "world", 40).plain_text(),
            "Hello World"
        );
    }

    #[test]
    fn a_quote_of_a_picture_drops_its_server_id() {
        assert_eq!(without_server_id("__SVRID__123__"), "");
        assert_eq!(without_server_id("看 __SVRID__9__这个"), "看 这个");
    }
}
