//! What the views draw with: the [`Style`] of a run, pieces of text in colours, lines fitted to the terminal,
//! tables, cards, bars and the numbers in them.

use chrono::{DateTime, Local};
use weflow_core::colour::{Paint, Role, Stream};
use weflow_core::locale::Lang;
use weflow_core::render::display_width;

/// The columns a view fits its lines to when stdout is not a terminal and `COLUMNS` does not say.
const DEFAULT_WIDTH: usize = 100;
/// The fewest columns a view fits its lines to, however narrow the terminal says it is.
const MIN_WIDTH: usize = 40;
/// The columns between two columns of a table, and between a card's labels and values.
const GAP: usize = 2;
/// The fewest columns a table squeezes a column of text to before it lets the line run past the width.
const MIN_COLUMN: usize = 8;
/// What a cut piece of text ends with.
const ELLIPSIS: &str = "…";

/// The characters bars and rules are drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Glyphs {
    /// Block and box-drawing characters.
    Unicode,
    /// ASCII only (`--ascii`), for a font without the others.
    Ascii,
}

impl Glyphs {
    /// A filled cell of a bar.
    fn full(self) -> &'static str {
        match self {
            Self::Unicode => "█",
            Self::Ascii => "#",
        }
    }

    /// An empty cell of a bar.
    fn empty(self) -> &'static str {
        match self {
            Self::Unicode => "░",
            Self::Ascii => ".",
        }
    }

    /// A piece of a horizontal rule.
    pub(crate) fn rule(self) -> &'static str {
        match self {
            Self::Unicode => "─",
            Self::Ascii => "-",
        }
    }

    /// What a line under another one starts with to say it belongs to it (a quote under its reply).
    pub(crate) fn branch(self) -> &'static str {
        match self {
            Self::Unicode => "└ ",
            Self::Ascii => "> ",
        }
    }

    /// The heights of a sparkline, lowest first.
    fn levels(self) -> [&'static str; 8] {
        match self {
            Self::Unicode => ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"],
            Self::Ascii => ["_", ".", "-", "~", "=", "+", "*", "#"],
        }
    }
}

/// How the views of a run draw: colours, characters, width, language, and the time "today" is counted from.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Style {
    /// The colours of stdout, where a view's lines go.
    pub(crate) paint: Paint,
    /// The colours of stderr, where its note goes.
    pub(crate) note_paint: Paint,
    /// The characters of bars and rules.
    pub(crate) glyphs: Glyphs,
    /// The columns a line may take.
    pub(crate) width: usize,
    /// The language of the words.
    pub(crate) lang: Lang,
    /// Now, which says what "today" and "yesterday" are.
    pub(crate) now: DateTime<Local>,
}

impl Style {
    /// The style of this run: the streams' colours, the terminal's width, the run's language and the time now.
    pub(crate) fn of_run(glyphs: Glyphs) -> Self {
        Self {
            paint: Paint::of(Stream::Stdout),
            note_paint: Paint::of(Stream::Stderr),
            glyphs,
            width: terminal_width(),
            lang: weflow_core::locale::current(),
            now: Local::now(),
        }
    }

    /// `en` or `zh`, by the style's language.
    pub(crate) fn tr(&self, en: &'static str, zh: &'static str) -> &'static str {
        match self.lang {
            Lang::En => en,
            Lang::Zh => zh,
        }
    }

    /// A count with its unit: `123,456 条`, `1 message`, `2 messages`.
    pub(crate) fn count(&self, n: i64, one: &str, many: &str, zh_unit: &str) -> String {
        match self.lang {
            Lang::Zh => format!("{} {zh_unit}", digits(n)),
            Lang::En => format!("{} {}", digits(n), if n == 1 { one } else { many }),
        }
    }

    /// What goes between a name and what it says: `：` in Chinese, `: ` in English.
    pub(crate) fn colon(&self) -> &'static str {
        self.tr(": ", "：")
    }
}

/// The columns of stdout when it is a terminal, else `COLUMNS`, else [`DEFAULT_WIDTH`]; at least [`MIN_WIDTH`].
fn terminal_width() -> usize {
    terminal_size::terminal_size_of(std::io::stdout())
        .map(|(w, _)| usize::from(w.0))
        .or_else(|| std::env::var("COLUMNS").ok()?.trim().parse().ok())
        .unwrap_or(DEFAULT_WIDTH)
        .max(MIN_WIDTH)
}

/// A piece of a line, and the role it is shown as (`None`: the terminal's own colour).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Span {
    text: String,
    role: Option<Role>,
}

/// Pieces of text shown one after another: a line, or a cell of a table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Text {
    spans: Vec<Span>,
}

impl Text {
    /// Text in the terminal's own colour.
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self::default().then(text)
    }

    /// Text shown as `role`.
    pub(crate) fn styled(role: Role, text: impl Into<String>) -> Self {
        Self::default().then_as(role, text)
    }

    /// This, then `text` in the terminal's own colour.
    pub(crate) fn then(mut self, text: impl Into<String>) -> Self {
        self.push(None, text.into());
        self
    }

    /// This, then `text` shown as `role`.
    pub(crate) fn then_as(mut self, role: Role, text: impl Into<String>) -> Self {
        self.push(Some(role), text.into());
        self
    }

    /// This, then `other`.
    pub(crate) fn append(mut self, other: Text) -> Self {
        for span in other.spans {
            self.push(span.role, span.text);
        }
        self
    }

    fn push(&mut self, role: Option<Role>, text: String) {
        if text.is_empty() {
            return;
        }
        match self.spans.last_mut() {
            Some(last) if last.role == role => last.text.push_str(&text),
            _ => self.spans.push(Span { text, role }),
        }
    }

    /// Whether there is nothing to show.
    pub(crate) fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// The columns it takes.
    pub(crate) fn width(&self) -> usize {
        self.spans.iter().map(|s| display_width(&s.text)).sum()
    }

    /// Its words without colours.
    #[cfg(test)]
    pub(crate) fn plain_text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    /// The first `width` columns of it, ending with `…` when something was cut.
    pub(crate) fn cut(&self, width: usize) -> Text {
        if self.width() <= width {
            return self.clone();
        }
        let room = width.saturating_sub(display_width(ELLIPSIS));
        let mut out = Text::default();
        let mut used = 0;
        'spans: for span in &self.spans {
            let mut piece = String::new();
            for c in span.text.chars() {
                let w = display_width(c.encode_utf8(&mut [0; 4]));
                if used + w > room {
                    out.push(span.role, piece);
                    break 'spans;
                }
                used += w;
                piece.push(c);
            }
            out.push(span.role, piece);
        }
        if width > 0 {
            out.push(Some(Role::Muted), ELLIPSIS.to_string());
        }
        out
    }

    /// It in `paint`'s colours.
    pub(crate) fn show(&self, paint: Paint) -> String {
        self.spans
            .iter()
            .map(|s| match s.role {
                Some(role) => paint.paint(role, &s.text),
                None => s.text.clone(),
            })
            .collect()
    }
}

/// A message's words: the `[kind]` they start with (`[图片]`, `[链接]`), as the chat window labels a message that is not
/// text, soft teal; the rest as it is.
pub(crate) fn with_kind(words: &str) -> Text {
    if words.starts_with('[') {
        if let Some(end) = words.find(']') {
            let (kind, rest) = words.split_at(end + 1);
            return Text::styled(Role::Accent, kind).then(rest);
        }
    }
    Text::plain(words)
}

/// `text` on one line: line breaks and tabs as spaces, a run of spaces as one, the ends trimmed, and without
/// control characters, which a terminal does not show but which would count in the width (a name of `DEL`s).
pub(crate) fn one_line(text: &str) -> String {
    text.split_whitespace()
        .map(|word| word.chars().filter(|c| !c.is_control()).collect::<String>())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `text` in lines of at most `width` columns: its own line breaks kept, a long line broken between words (any two
/// CJK characters are a place to break), a word longer than a line broken where it reaches the end; tabs as spaces,
/// other control characters left out.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let text: String = text
        .chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            '\n' => Some('\n'),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        let mut used = 0;
        for word in words(paragraph.trim_end()) {
            let w = display_width(word);
            if used + w > width && used > 0 {
                lines.push(line.trim_end().to_string());
                line.clear();
                used = 0;
                if word.trim().is_empty() {
                    continue;
                }
            }
            if w > width {
                for c in word.chars() {
                    let cw = display_width(c.encode_utf8(&mut [0; 4]));
                    if used + cw > width && used > 0 {
                        lines.push(std::mem::take(&mut line));
                        used = 0;
                    }
                    line.push(c);
                    used += cw;
                }
                continue;
            }
            line.push_str(word);
            used += w;
        }
        lines.push(line.trim_end().to_string());
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// The words of `text`: a run of characters other than spaces that are not wide, each wide character on its own,
/// and each run of spaces.
fn words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut kind = None;
    for (i, c) in text.char_indices() {
        let this = if c.is_whitespace() {
            0
        } else if display_width(c.encode_utf8(&mut [0; 4])) > 1 {
            2
        } else {
            1
        };
        if kind.is_some() && (kind != Some(this) || this == 2) {
            out.push(&text[start..i]);
            start = i;
        }
        kind = Some(this);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// `123456` → `123,456`.
pub(crate) fn digits(n: i64) -> String {
    let text = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(text.len() + text.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (i, c) in text.chars().enumerate() {
        if i > 0 && (text.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `part` of `whole` in hundredths, rounded; 0 without a whole.
fn share(part: i64, whole: i64, of: i64) -> i64 {
    if whole <= 0 || part <= 0 {
        return 0;
    }
    let scaled =
        (i128::from(part) * i128::from(of) * 2 + i128::from(whole)) / (2 * i128::from(whole));
    i64::try_from(scaled).unwrap_or(of)
}

/// `part` of `whole` as a whole percent: `41%`.
pub(crate) fn percent(part: i64, whole: i64) -> String {
    format!("{}%", share(part, whole, 100))
}

/// A bar of `cells` cells, `part` of `whole` of them filled: the filled cells soft teal, the rest grey.
pub(crate) fn bar(part: i64, whole: i64, cells: usize, style: &Style) -> Text {
    let filled = usize::try_from(share(part, whole, i64::try_from(cells).unwrap_or(0)))
        .unwrap_or(0)
        .min(cells);
    Text::styled(Role::Accent, style.glyphs.full().repeat(filled))
        .then_as(Role::Muted, style.glyphs.empty().repeat(cells - filled))
}

/// One character per value, as high as the value is against the largest; the lowest height for nothing.
pub(crate) fn sparkline(values: &[i64], style: &Style) -> Text {
    let levels = style.glyphs.levels();
    let top = values.iter().copied().max().unwrap_or(0);
    let last = i64::try_from(levels.len() - 1).unwrap_or(0);
    let line: String = values
        .iter()
        .map(|&v| {
            levels[usize::try_from(share(v, top, last))
                .unwrap_or(0)
                .min(levels.len() - 1)]
        })
        .collect();
    Text::styled(Role::Accent, line)
}

/// How a column of a table lines its cells up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align {
    /// Text, flush left; the columns squeezed first when a line is too long.
    Left,
    /// Numbers, flush right.
    Right,
}

/// Rows of cells under a grey header, lined up in columns two spaces apart and fitted to the width: a column with
/// nothing in any row is left out, and when a line would be too long the widest text columns are cut (with `…`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Table {
    headers: Vec<&'static str>,
    aligns: Vec<Align>,
    caps: Vec<Option<usize>>,
    rows: Vec<Vec<Text>>,
}

impl Table {
    /// A table of these columns: each a header and how its cells line up.
    pub(crate) fn new(columns: &[(&'static str, Align)]) -> Self {
        Self {
            headers: columns.iter().map(|(h, _)| *h).collect(),
            aligns: columns.iter().map(|(_, a)| *a).collect(),
            caps: vec![None; columns.len()],
            rows: Vec::new(),
        }
    }

    /// The same table with the cells of `column` cut to `width` columns, however wide the terminal is.
    pub(crate) fn cap(mut self, column: usize, width: usize) -> Self {
        if let Some(cap) = self.caps.get_mut(column) {
            *cap = Some(width);
        }
        self
    }

    /// Adds a row; a missing cell is empty.
    pub(crate) fn row(&mut self, cells: Vec<Text>) {
        self.rows.push(cells);
    }

    /// Whether it has no rows.
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The header and the rows, each a line.
    pub(crate) fn lines(&self, style: &Style) -> Vec<String> {
        let empty = Text::default();
        let cell = |row: &[Text], i: usize| row.get(i).unwrap_or(&empty).clone();
        let shown: Vec<usize> = (0..self.headers.len())
            .filter(|&i| self.rows.iter().any(|r| !cell(r, i).is_empty()))
            .collect();
        let mut widths: Vec<usize> = shown
            .iter()
            .map(|&i| {
                let cells = self
                    .rows
                    .iter()
                    .map(|r| cell(r, i).width())
                    .max()
                    .unwrap_or(0);
                let cells = self.caps[i].map_or(cells, |cap| cells.min(cap));
                cells.max(display_width(self.headers[i]))
            })
            .collect();
        self.squeeze(&shown, &mut widths, style.width);
        let line = |cells: Vec<Text>| {
            let last = cells.len().saturating_sub(1);
            let parts: Vec<String> = cells
                .iter()
                .zip(shown.iter().zip(&widths))
                .enumerate()
                .map(|(n, (text, (&i, &w)))| {
                    let text = text.cut(w);
                    let pad = " ".repeat(w - text.width().min(w));
                    match self.aligns[i] {
                        Align::Right => format!("{pad}{}", text.show(style.paint)),
                        Align::Left if n == last => text.show(style.paint),
                        Align::Left => format!("{}{pad}", text.show(style.paint)),
                    }
                })
                .collect();
            parts.join(&" ".repeat(GAP)).trim_end().to_string()
        };
        let mut lines = vec![line(
            shown
                .iter()
                .map(|&i| Text::styled(Role::Muted, self.headers[i]))
                .collect(),
        )];
        for row in &self.rows {
            lines.push(line(shown.iter().map(|&i| cell(row, i)).collect()));
        }
        lines
    }

    /// Narrows the widest text columns until a line fits `width`, none below [`MIN_COLUMN`] or its header.
    fn squeeze(&self, shown: &[usize], widths: &mut [usize], width: usize) {
        loop {
            let total = widths.iter().sum::<usize>() + GAP * widths.len().saturating_sub(1);
            if total <= width {
                return;
            }
            let floor = |n: usize| MIN_COLUMN.max(display_width(self.headers[shown[n]]));
            let widest = (0..shown.len())
                .filter(|&n| self.aligns[shown[n]] == Align::Left && widths[n] > floor(n))
                .max_by_key(|&n| widths[n]);
            let Some(n) = widest else { return };
            widths[n] = widths[n].saturating_sub(total - width).max(floor(n));
        }
    }
}

/// Labels and their values, the labels grey and lined up: one thing's facts.
#[derive(Clone, Debug, Default)]
pub(crate) struct Card {
    rows: Vec<(String, Vec<Text>)>,
}

impl Card {
    /// Adds `label` with the value `value`, unless the value is empty.
    pub(crate) fn field(&mut self, label: &str, value: Text) {
        if !value.is_empty() {
            self.rows.push((label.to_string(), vec![value]));
        }
    }

    /// Adds `label` with a value of several lines, unless there are none.
    pub(crate) fn lines_field(&mut self, label: &str, lines: Vec<Text>) {
        if !lines.is_empty() {
            self.rows.push((label.to_string(), lines));
        }
    }

    /// The rows, each `indent` columns in; a value that does not fit is cut.
    pub(crate) fn lines(&self, indent: usize, style: &Style) -> Vec<String> {
        let label_width = self
            .rows
            .iter()
            .map(|(l, _)| display_width(l))
            .max()
            .unwrap_or(0);
        let room = style
            .width
            .saturating_sub(indent + label_width + GAP)
            .max(MIN_COLUMN);
        let lead = " ".repeat(indent);
        let mut out = Vec::new();
        for (label, values) in &self.rows {
            for (n, value) in values.iter().enumerate() {
                let label = if n == 0 { label.as_str() } else { "" };
                let pad = " ".repeat(label_width - display_width(label) + GAP);
                out.push(format!(
                    "{lead}{}{pad}{}",
                    style.paint.paint(Role::Muted, label),
                    value.cut(room).show(style.paint)
                ));
            }
        }
        out
    }
}

/// The note under a view, its parts joined by ` · `, grey: what was left out and how to see more. It goes to stderr,
/// so a file the view is written to holds only the view.
pub(crate) fn note(parts: &[String], style: &Style) -> Option<String> {
    let parts: Vec<&str> = parts
        .iter()
        .map(String::as_str)
        .filter(|p| !p.is_empty())
        .collect();
    (!parts.is_empty()).then(|| {
        format!(
            "{}\n",
            style.note_paint.paint(Role::Muted, &parts.join(" · "))
        )
    })
}

/// The note's part that says `--verbose` shows the fields the view leaves out.
pub(crate) fn verbose_hint(style: &Style) -> String {
    style
        .tr("--verbose shows every field", "--verbose 显示全部字段")
        .to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::TimeZone;

    /// A plain style of `width` columns in `lang`, its now 2026-11-23 12:00 local time.
    pub(crate) fn style(lang: Lang, width: usize) -> Style {
        Style {
            paint: Paint::plain(),
            note_paint: Paint::plain(),
            glyphs: Glyphs::Unicode,
            width,
            lang,
            now: Local.with_ymd_and_hms(2026, 11, 23, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn numbers_get_their_thousands_marked() {
        assert_eq!(digits(0), "0");
        assert_eq!(digits(999), "999");
        assert_eq!(digits(1000), "1,000");
        assert_eq!(digits(123_456), "123,456");
        assert_eq!(digits(-1_234_567), "-1,234,567");
        assert_eq!(percent(410, 1000), "41%");
        assert_eq!(percent(1, 0), "0%");
    }

    #[test]
    fn a_cut_ends_with_an_ellipsis_and_keeps_wide_characters_whole() {
        let text = Text::plain("示例群聊").then_as(Role::Muted, " 群");
        assert_eq!(text.cut(20), text);
        assert_eq!(text.cut(6).plain_text(), "示例…");
        assert_eq!(text.cut(6).width(), 5);
        assert_eq!(Text::plain("abcdef").cut(4).plain_text(), "abc…");
    }

    #[test]
    fn wrapping_breaks_between_words_and_anywhere_in_cjk() {
        assert_eq!(wrap("hello world again", 11), ["hello world", "again"]);
        assert_eq!(wrap("一二三四五", 4), ["一二", "三四", "五"]);
        assert_eq!(wrap("a\nb", 10), ["a", "b"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 4), [""]);
        assert_eq!(wrap("a\tb\u{7f}c\r\nd", 10), ["a bc", "d"]);
    }

    #[test]
    fn one_line_leaves_out_what_a_terminal_does_not_show() {
        assert_eq!(one_line(" a\n b\t\tc "), "a b c");
        assert_eq!(one_line("\u{7f}\u{7f}\u{7f}\u{7f}"), "");
        assert_eq!(one_line("x\u{7f} y"), "x y");
    }

    #[test]
    fn tables_line_up_drop_empty_columns_and_fit_the_width() {
        let style = style(Lang::En, 30);
        let mut table = Table::new(&[
            ("n", Align::Right),
            ("name", Align::Left),
            ("empty", Align::Left),
            ("said", Align::Left),
        ]);
        table.row(vec![
            Text::plain("1"),
            Text::plain("小测."),
            Text::default(),
            Text::plain("a long line that does not fit"),
        ]);
        table.row(vec![Text::plain("12"), Text::plain("Bob")]);
        assert_eq!(
            table.lines(&style),
            [
                " n  name   said",
                " 1  小测.  a long line that d…",
                "12  Bob",
            ]
        );
    }

    #[test]
    fn bars_and_sparklines_scale_to_their_whole() {
        let style = style(Lang::En, 80);
        assert_eq!(
            bar(78, 100, 20, &style).plain_text(),
            "████████████████░░░░"
        );
        assert_eq!(bar(0, 100, 4, &style).plain_text(), "░░░░");
        assert_eq!(bar(5, 0, 4, &style).plain_text(), "░░░░");
        assert_eq!(sparkline(&[0, 7, 14], &style).plain_text(), "▁▅█");
        let ascii = Style {
            glyphs: Glyphs::Ascii,
            ..style
        };
        assert_eq!(bar(1, 2, 4, &ascii).plain_text(), "##..");
    }

    #[test]
    fn cards_line_their_labels_up() {
        let style = style(Lang::Zh, 40);
        let mut card = Card::default();
        card.field("成员", Text::plain("42 人"));
        card.field("空的", Text::default());
        card.lines_field(
            "存放位置",
            vec![Text::plain("message_0.db"), Text::plain("message_1.db")],
        );
        assert_eq!(
            card.lines(2, &style),
            [
                "  成员      42 人",
                "  存放位置  message_0.db",
                "            message_1.db",
            ]
        );
    }

    #[test]
    fn a_note_joins_its_parts_and_skips_empty_ones() {
        let style = style(Lang::En, 80);
        assert_eq!(
            note(&["a".into(), String::new(), "b".into()], &style).as_deref(),
            Some("a · b\n")
        );
        assert_eq!(note(&[String::new()], &style), None);
    }
}
