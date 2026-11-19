//! How clap's help and parse errors read: its bracketed notes (`[default: 50]`) and the parenthesised wording of its
//! built-in help become words, and on a terminal the headings are bold and the `error:` label red.

use weflow_core::colour::{Paint, Role};
use weflow_core::locale::tr;

/// Clap's notes after an option's help, which it puts in square brackets, and the words they become.
fn note_labels() -> [(&'static str, &'static str); 4] {
    [
        ("[default: ", tr("default: ", "默认：")),
        ("[env: ", tr("environment variable: ", "环境变量：")),
        ("[possible values: ", tr("possible values: ", "可选值：")),
        ("[aliases: ", tr("aliases: ", "别名：")),
    ]
}

/// Clap's own help of its built-in options and subcommand, worded without brackets.
const BUILT_IN: [(&str, &str); 3] = [
    (
        "Print this message or the help of the given subcommand(s)",
        "Print this message or the help of the given subcommands",
    ),
    (
        "Print help (see more with '--help')",
        "Print help; --help shows more",
    ),
    (
        "Print help (see a summary with '-h')",
        "Print help; -h shows a summary",
    ),
];

/// `text` with clap's bracketed notes written as words after a semicolon (`Maximum number of results; default: 50`)
/// and its built-in help reworded.
pub(crate) fn plain_notes(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let mut line = line.to_string();
            for (en, plain) in BUILT_IN {
                line = line.replace(en, plain);
            }
            for (open, label) in note_labels() {
                while let Some(start) = line.find(open) {
                    let Some(len) = line[start..].find(']') else {
                        break;
                    };
                    let content = &line[start + open.len()..start + len];
                    let before = &line[..start];
                    // a note on a line of its own keeps its indent; after the help it follows a semicolon
                    let lead = if before.trim().is_empty() {
                        before.to_string()
                    } else {
                        format!("{}{}", before.trim_end(), tr("; ", "；"))
                    };
                    line = format!("{lead}{label}{content}{}", &line[start + len + 1..]);
                }
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Headings of clap's help that stand on lines of their own, in either language.
const HEADINGS: [&str; 6] = [
    "Commands:",
    "Options:",
    "Arguments:",
    "命令：",
    "选项：",
    "参数：",
];

/// `text`, clap's help or error, in the colours of `paint`: headings bold, the `error:` label red, tips and the
/// pointer to `--help` grey.
pub(crate) fn painted(text: &str, paint: Paint) -> String {
    if paint.is_plain() {
        return text.to_string();
    }
    let label = |line: &str, label: &str, role: Role| {
        line.strip_prefix(label)
            .map(|rest| format!("{}{rest}", paint.paint(role, label)))
    };
    text.split('\n')
        .map(|line| {
            if HEADINGS.contains(&line) {
                return paint.paint(Role::Strong, line);
            }
            if line.starts_with("For more information") || line.starts_with("更多信息") {
                return paint.paint(Role::Muted, line);
            }
            [
                ("Usage:", Role::Strong),
                ("用法：", Role::Strong),
                ("error:", Role::Error),
                ("错误：", Role::Error),
                ("  tip:", Role::Muted),
                ("  提示：", Role::Muted),
            ]
            .into_iter()
            .find_map(|(prefix, role)| label(line, prefix, role))
            .unwrap_or_else(|| line.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use weflow_core::colour::Depth;

    #[test]
    fn notes_become_words_after_a_semicolon() {
        let zh = weflow_core::locale::current() == weflow_core::locale::Lang::Zh;
        let (default, values, sep) = if zh {
            ("默认：", "可选值：", "；")
        } else {
            ("default: ", "possible values: ", "; ")
        };
        assert_eq!(
            plain_notes("      --limit <LIMIT>    Maximum number of results [default: 50]"),
            format!("      --limit <LIMIT>    Maximum number of results{sep}{default}50")
        );
        // a note on a line of its own, as in a parse error
        assert_eq!(
            plain_notes("  [possible values: en, zh]"),
            format!("  {values}en, zh")
        );
        assert_eq!(
            plain_notes("  help  Print this message or the help of the given subcommand(s)"),
            "  help  Print this message or the help of the given subcommands"
        );
        assert!(!plain_notes("a [default: 1] [env: X]").contains('['));
    }

    #[test]
    fn headings_are_bold_and_the_error_label_red() {
        let paint = Paint::new(Depth::Basic);
        let text = painted(
            "error: bad\n\nUsage: weflow x\n\nOptions:\n  --json  JSON",
            paint,
        );
        assert_eq!(
            text,
            "\x1b[1;31merror:\x1b[0m bad\n\n\x1b[1mUsage:\x1b[0m weflow x\n\n\x1b[1mOptions:\x1b[0m\n  --json  JSON"
        );
        assert_eq!(painted("Options:", Paint::plain()), "Options:");
    }
}
