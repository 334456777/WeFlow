//! The views of the statistics: counts with their share as a bar, the hours of the day as a sparkline, and the
//! rankings as tables.

use serde_json::Value;
use weflow_core::colour::Role;

use super::layout::{
    bar, digits, note, one_line, percent, sparkline, verbose_hint, Align, Card, Style, Table, Text,
};
use super::{number, text, when, Shown, Span};

/// The cells of a bar of shares.
const BAR_CELLS: usize = 20;
/// The cells of a bar of a weekday.
const WEEKDAY_CELLS: usize = 16;
/// The narrowest width bars are drawn at; below it only the numbers are shown.
const BARS_FROM: usize = 56;
/// The months `analytics time` shows at most, the latest ones.
const MONTHS: usize = 24;
/// The columns lines under a heading are indented by.
const INDENT: &str = "  ";

/// `lines`, each indented under a heading.
fn indented(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|l| {
            if l.is_empty() {
                l
            } else {
                format!("{INDENT}{l}")
            }
        })
        .collect()
}

/// A heading, bold.
fn heading(words: &str, style: &Style) -> String {
    style.paint.paint(Role::Strong, words)
}

/// Whether the width leaves room for bars.
fn bars(style: &Style) -> bool {
    style.width >= BARS_FROM
}

/// Kinds of message and how many of each, the most first but "other" last, with their share of `total` as a bar.
fn kinds(
    mut rows: Vec<(String, i64)>,
    other: Option<(String, i64)>,
    total: i64,
    style: &Style,
) -> Vec<String> {
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    rows.extend(other);
    let mut table = Table::new(&[
        (style.tr("kind", "类型"), Align::Left),
        (style.tr("count", "数量"), Align::Right),
        ("", Align::Left),
        (style.tr("share", "占比"), Align::Right),
    ]);
    for (name, n) in rows {
        table.row(vec![
            Text::plain(name),
            Text::plain(digits(n)),
            if bars(style) {
                bar(n, total, BAR_CELLS, style)
            } else {
                Text::default()
            },
            Text::styled(Role::Muted, percent(n, total)),
        ]);
    }
    table.lines(style)
}

/// The values of `map` by the hour, 0 to 23 (a missing hour has none).
fn by_hour(map: &Value) -> [i64; 24] {
    let mut hours = [0; 24];
    for (n, slot) in hours.iter_mut().enumerate() {
        *slot = map[n.to_string()].as_i64().unwrap_or(0);
    }
    hours
}

/// The hours of the day as a sparkline, the hours under it, and the busiest and quietest hour.
fn hours(map: &Value, style: &Style) -> Vec<String> {
    let hours = by_hour(map);
    let axis = "0     6     12    18   23";
    let mut lines = vec![
        sparkline(&hours, style).show(style.paint),
        style.paint.paint(Role::Muted, axis),
    ];
    let busiest = (0..24).max_by_key(|&h| (hours[h], std::cmp::Reverse(h)));
    let quietest = (0..24).min_by_key(|&h| (hours[h], h));
    if let (Some(most), Some(least)) = (busiest, quietest) {
        if hours[most] > 0 {
            let at = |h: usize| match style.lang {
                weflow_core::locale::Lang::Zh => format!("{h} 点（{} 条）", digits(hours[h])),
                weflow_core::locale::Lang::En => format!("{h}:00 ({})", digits(hours[h])),
            };
            lines.push(format!(
                "{}{}  {}{}",
                style
                    .paint
                    .paint(Role::Muted, style.tr("busiest ", "最多 ")),
                at(most),
                style
                    .paint
                    .paint(Role::Muted, style.tr("quietest ", "最少 ")),
                at(least)
            ));
        }
    }
    lines
}

/// Labelled counts with a bar each against the largest: the weekdays, the months.
fn against_largest(rows: &[(String, i64)], cells: usize, style: &Style) -> Vec<String> {
    let top = rows.iter().map(|(_, n)| *n).max().unwrap_or(0);
    let mut table = Table::new(&[("", Align::Left), ("", Align::Right), ("", Align::Left)]);
    for (label, n) in rows {
        table.row(vec![
            Text::styled(Role::Muted, label.as_str()),
            Text::plain(digits(*n)),
            if bars(style) {
                bar(*n, top, cells, style)
            } else {
                Text::default()
            },
        ]);
    }
    // the header row of empty headers is a blank line
    table.lines(style).split_off(1)
}

/// The messages counted, the days with one and the time they span: `12,345 条 · 活跃 300 天 · 2025-09-01 ~ 2026-11-23`.
fn totals(total: i64, days: i64, first: i64, last: i64, style: &Style) -> Text {
    let mut line = Text::plain(style.count(total, "message", "messages", "条"));
    if days > 0 {
        line = line.then_as(Role::Muted, " · ").then(match style.lang {
            weflow_core::locale::Lang::Zh => format!("活跃 {} 天", digits(days)),
            weflow_core::locale::Lang::En => format!(
                "{} active {}",
                digits(days),
                if days == 1 { "day" } else { "days" }
            ),
        });
    }
    if first > 0 && last > 0 {
        line = line.then_as(Role::Muted, " · ").then(format!(
            "{} ~ {}",
            when::date(first),
            when::date(last)
        ));
    }
    line
}

/// The kinds `analytics overall` and `group member` count, with their names.
fn counted_kinds(stats: &Value, style: &Style) -> (Vec<(String, i64)>, (String, i64)) {
    let kinds = [
        ("textMessages", style.tr("text", "文本")),
        ("emojiMessages", style.tr("sticker", "表情")),
        ("imageMessages", style.tr("image", "图片")),
        ("voiceMessages", style.tr("voice", "语音")),
        ("videoMessages", style.tr("video", "视频")),
    ];
    let rows = kinds
        .iter()
        .map(|(key, name)| (name.to_string(), number(stats, key)))
        .collect();
    (
        rows,
        (
            style.tr("other", "其他").to_string(),
            number(stats, "otherMessages"),
        ),
    )
}

/// `analytics overall`: the private chats' messages, who sent them and of which kinds.
pub(super) fn overall(data: &Value, style: &Style) -> Option<Shown> {
    data.get("totalMessages")?;
    let total = number(data, "totalMessages");
    let mut card = Card::default();
    card.field(
        style.tr("messages", "消息"),
        totals(
            total,
            number(data, "activeDays"),
            number(data, "firstMessageTime"),
            number(data, "lastMessageTime"),
            style,
        ),
    );
    let sent = number(data, "sentMessages");
    let received = number(data, "receivedMessages");
    let share = |n| {
        format!(
            "{}{}{}",
            style.tr(" (", "（"),
            percent(n, sent + received),
            style.tr(")", "）")
        )
    };
    card.field(
        style.tr("sent", "发出"),
        Text::plain(digits(sent))
            .then_as(Role::Muted, share(sent))
            .then_as(Role::Muted, " · ")
            .then_as(Role::Muted, style.tr("received ", "收到 "))
            .then(digits(received))
            .then_as(Role::Muted, share(received)),
    );
    let mut lines = vec![heading(style.tr("Private chats", "私聊整体统计"), style)];
    lines.extend(card.lines(2, style));
    lines.push(String::new());
    let (rows, other) = counted_kinds(data, style);
    lines.extend(indented(kinds(rows, Some(other), total, style)));
    let hint = style.tr(
        "--verbose shows the counts of each message type",
        "--verbose 显示按消息类型（localType）的明细",
    );
    Some(Shown::of(lines, note(&[hint.to_string()], style)))
}

/// `analytics time`: the private chats' messages by the hour, the weekday and the month.
pub(super) fn time(data: &Value, style: &Style) -> Option<Shown> {
    let hourly = data.get("hourlyDistribution")?;
    let mut lines = vec![heading(style.tr("By the hour", "按小时（0~23 点）"), style)];
    lines.extend(indented(hours(hourly, style)));
    lines.push(String::new());
    let weekdays = [
        style.tr("Mon", "周一"),
        style.tr("Tue", "周二"),
        style.tr("Wed", "周三"),
        style.tr("Thu", "周四"),
        style.tr("Fri", "周五"),
        style.tr("Sat", "周六"),
        style.tr("Sun", "周日"),
    ];
    let week: Vec<(String, i64)> = weekdays
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let n = (n + 1).to_string();
            (
                name.to_string(),
                data["weekdayDistribution"][n].as_i64().unwrap_or(0),
            )
        })
        .collect();
    lines.push(heading(style.tr("By the weekday", "按星期"), style));
    lines.extend(indented(against_largest(&week, WEEKDAY_CELLS, style)));
    let mut months: Vec<(String, i64)> = data["monthlyDistribution"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(0)))
                .collect()
        })
        .unwrap_or_default();
    months.sort();
    let all = months.len();
    let months = months.split_off(all.saturating_sub(MONTHS));
    let mut parts = Vec::new();
    if !months.is_empty() {
        lines.push(String::new());
        lines.push(heading(style.tr("By the month", "按月份"), style));
        lines.extend(indented(against_largest(&months, BAR_CELLS, style)));
        if all > months.len() {
            parts.push(match style.lang {
                weflow_core::locale::Lang::Zh => {
                    format!(
                        "只显示最近 {} 个月，--verbose 显示全部 {all} 个月",
                        months.len()
                    )
                }
                weflow_core::locale::Lang::En => format!(
                    "the latest {} months; --verbose shows all {all}",
                    months.len()
                ),
            });
        }
    }
    Some(Shown::of(lines, note(&parts, style)))
}

/// `Top 15 of 194 chats`, or `194 chats` when they are all there.
fn shown_of(
    shown: usize,
    all: usize,
    one: &str,
    many: &str,
    zh_unit: &str,
    style: &Style,
) -> String {
    let n = |x: usize| digits(i64::try_from(x).unwrap_or(0));
    if shown < all {
        match style.lang {
            weflow_core::locale::Lang::Zh => format!("前 {} / {} {zh_unit}", n(shown), n(all)),
            weflow_core::locale::Lang::En => format!("top {} of {} {many}", n(shown), n(all)),
        }
    } else {
        style.count(i64::try_from(all).unwrap_or(0), one, many, zh_unit)
    }
}

/// `analytics rankings`: the private chats with the most messages, a line each.
pub(super) fn rankings(data: &Value, limit: usize, span: &Span, style: &Style) -> Option<Shown> {
    let all = data.as_array()?;
    let shown = &all[..if limit > 0 {
        limit.min(all.len())
    } else {
        all.len()
    }];
    let mut table = Table::new(&[
        ("#", Align::Right),
        (style.tr("contact", "联系人"), Align::Left),
        (style.tr("messages", "消息"), Align::Right),
        (style.tr("sent", "发出"), Align::Right),
        (style.tr("received", "收到"), Align::Right),
        (style.tr("last", "最近一条"), Align::Left),
    ])
    .cap(1, 24);
    for (n, rank) in shown.iter().enumerate() {
        table.row(vec![
            Text::styled(Role::Muted, (n + 1).to_string()),
            Text::plain(one_line(text(rank, "displayName"))),
            Text::plain(digits(number(rank, "messageCount"))),
            Text::plain(digits(number(rank, "sentCount"))),
            Text::plain(digits(number(rank, "receivedCount"))),
            Text::styled(
                Role::Muted,
                when::short(number(rank, "lastMessageTime"), style),
            ),
        ]);
    }
    let mut parts = vec![
        shown_of(shown.len(), all.len(), "chat", "chats", "位联系人", style),
        span.words(style),
    ];
    if shown.len() < all.len() {
        parts.push(
            style
                .tr("--limit 0 lists all", "--limit 0 列出全部")
                .to_string(),
        );
    }
    Some(Shown::of(table.lines(style), note(&parts, style)))
}

/// `group ranking`: the members who sent the most, with their share of the group's messages.
pub(super) fn group_ranking(
    data: &Value,
    limit: usize,
    span: &Span,
    style: &Style,
) -> Option<Shown> {
    let all = data.as_array()?;
    let total: i64 = all.iter().map(|r| number(r, "messageCount")).sum();
    let shown = &all[..if limit > 0 {
        limit.min(all.len())
    } else {
        all.len()
    }];
    let mut table = Table::new(&[
        ("#", Align::Right),
        (style.tr("member", "成员"), Align::Left),
        (style.tr("messages", "消息"), Align::Right),
        (style.tr("share", "占比"), Align::Right),
    ])
    .cap(1, 24);
    for (n, rank) in shown.iter().enumerate() {
        let name = text(&rank["member"], "displayName");
        table.row(vec![
            Text::styled(Role::Muted, (n + 1).to_string()),
            if name.is_empty() {
                Text::styled(Role::Muted, style.tr("(unknown sender)", "（未知发送者）"))
            } else {
                Text::plain(one_line(name))
            },
            Text::plain(digits(number(rank, "messageCount"))),
            Text::styled(Role::Muted, percent(number(rank, "messageCount"), total)),
        ]);
    }
    let mut parts = vec![
        shown_of(shown.len(), all.len(), "member", "members", "位成员", style),
        span.words(style),
    ];
    if shown.len() < all.len() {
        parts.push(
            style
                .tr("--limit 0 lists all", "--limit 0 列出全部")
                .to_string(),
        );
    }
    Some(Shown::of(table.lines(style), note(&parts, style)))
}

/// `group hours`: a group's messages by the hour.
pub(super) fn group_hours(data: &Value, span: &Span, style: &Style) -> Option<Shown> {
    let hourly = data.get("hourlyDistribution")?;
    let mut lines = vec![heading(style.tr("By the hour", "按小时（0~23 点）"), style)];
    lines.extend(indented(hours(hourly, style)));
    Some(Shown::of(lines, note(&[span.words(style)], style)))
}

/// `group media`: a group's messages by kind.
pub(super) fn group_media(data: &Value, span: &Span, style: &Style) -> Option<Shown> {
    let counts = data["typeCounts"].as_array()?;
    let total = number(data, "total");
    let mut rows = Vec::new();
    let mut other = None;
    for count in counts {
        let row = (one_line(text(count, "name")), number(count, "count"));
        if number(count, "type") == -1 {
            other = Some(row);
        } else {
            rows.push(row);
        }
    }
    let mut lines =
        vec![Text::plain(style.count(total, "message", "messages", "条")).show(style.paint)];
    lines.push(String::new());
    lines.extend(kinds(rows, other, total, style));
    Some(Shown::of(lines, note(&[span.words(style)], style)))
}

/// `group member`: what one member sent, of which kinds, at which hours, and what they say most.
pub(super) fn group_member(data: &Value, span: &Span, style: &Style) -> Option<Shown> {
    let stats = data.get("statistics")?;
    let total = number(stats, "totalMessages");
    let mut card = Card::default();
    card.field(
        style.tr("messages", "消息"),
        totals(
            total,
            number(stats, "activeDays"),
            number(stats, "firstMessageTime"),
            number(stats, "lastMessageTime"),
            style,
        ),
    );
    let joined = |list: &Value, key: &str| -> Text {
        let parts: Vec<String> = list
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| format!("{} ×{}", one_line(text(p, key)), number(p, "count")))
            .collect();
        Text::plain(parts.join(style.tr(", ", "、")))
    };
    card.field(
        style.tr("says", "常说"),
        joined(&data["commonPhrases"], "phrase"),
    );
    card.field(
        style.tr("emoji", "常用表情"),
        joined(&data["commonEmojis"], "emoji"),
    );
    let mut lines = card.lines(0, style);
    lines.push(String::new());
    let (rows, other) = counted_kinds(stats, style);
    lines.extend(kinds(rows, Some(other), total, style));
    if let Some(hourly) = data.get("timeDistribution") {
        lines.push(String::new());
        lines.push(heading(style.tr("By the hour", "按小时（0~23 点）"), style));
        lines.extend(indented(hours(hourly, style)));
    }
    Some(Shown::of(
        lines,
        note(&[span.words(style), verbose_hint(style)], style),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::layout::tests::style;
    use serde_json::json;
    use weflow_core::locale::Lang;

    #[test]
    fn the_overall_numbers_have_their_share_drawn() {
        let data = json!({
            "totalMessages": 100, "textMessages": 78, "imageMessages": 4, "voiceMessages": 3, "videoMessages": 0,
            "emojiMessages": 10, "otherMessages": 5, "sentMessages": 41, "receivedMessages": 59,
            "firstMessageTime": null, "lastMessageTime": null, "activeDays": 3, "messageTypeCounts": { "1": 78 },
        });
        let shown = overall(&data, &style(Lang::Zh, 80)).unwrap();
        assert_eq!(
            shown.body,
            "私聊整体统计\n\
             \x20 消息  100 条 · 活跃 3 天\n\
             \x20 发出  41（41%） · 收到 59（59%）\n\
             \n\
             \x20 类型  数量                        占比\n\
             \x20 文本    78  ████████████████░░░░   78%\n\
             \x20 表情    10  ██░░░░░░░░░░░░░░░░░░   10%\n\
             \x20 图片     4  █░░░░░░░░░░░░░░░░░░░    4%\n\
             \x20 语音     3  █░░░░░░░░░░░░░░░░░░░    3%\n\
             \x20 视频     0  ░░░░░░░░░░░░░░░░░░░░    0%\n\
             \x20 其他     5  █░░░░░░░░░░░░░░░░░░░    5%\n"
        );
        // too narrow for bars: the numbers alone
        let narrow = overall(&data, &style(Lang::Zh, 40)).unwrap();
        assert!(!narrow.body.contains('█'), "{}", narrow.body);
    }

    #[test]
    fn the_hours_are_a_sparkline_with_the_busiest_and_quietest() {
        let mut hourly = serde_json::Map::new();
        for h in 0..24 {
            hourly.insert(h.to_string(), json!(if h == 20 { 1500 } else { 800 + h }));
        }
        hourly.insert("3".into(), json!(150));
        let data = json!({ "hourlyDistribution": hourly, "weekdayDistribution": { "7": 2, "1": 4 },
                           "monthlyDistribution": { "2026-06": 10, "2026-05": 5 } });
        let shown = time(&data, &style(Lang::Zh, 80)).unwrap();
        let lines: Vec<&str> = shown.body.lines().collect();
        assert_eq!(lines[0], "按小时（0~23 点）");
        assert_eq!(lines[1].chars().count(), 2 + 24);
        assert_eq!(lines[3], "  最多 20 点（1,500 条）  最少 3 点（150 条）");
        assert!(
            shown.body.contains("  周一  4  ████████████████\n"),
            "{}",
            shown.body
        );
        assert!(
            shown.body.contains("  周日  2  ████████░░░░░░░░\n"),
            "{}",
            shown.body
        );
        // the months in order
        let may = shown.body.find("2026-05").unwrap();
        assert!(shown.body.find("2026-06").unwrap() > may);
        assert_eq!(shown.note, None);
    }

    #[test]
    fn a_ranking_says_how_many_it_left_out() {
        let rank = |name: &str, n: i64| {
            json!({ "username": "u", "displayName": name, "messageCount": n,
                                               "sentCount": n / 2, "receivedCount": n - n / 2,
                                               "lastMessageTime": null })
        };
        let data = json!([rank("Alice Wong", 7000), rank("小测.", 5001), rank("c", 1)]);
        let shown = rankings(&data, 2, &Span::default(), &style(Lang::Zh, 80)).unwrap();
        assert_eq!(
            shown.body,
            "#  联系人       消息   发出   收到\n\
             1  Alice Wong  7,000  3,500  3,500\n\
             2  小测.       5,001  2,500  2,501\n"
        );
        assert_eq!(
            shown.note.as_deref(),
            Some("前 2 / 3 位联系人 · 全部时间 · --limit 0 列出全部\n")
        );
    }
}
