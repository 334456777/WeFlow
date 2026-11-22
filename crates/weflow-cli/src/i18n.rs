//! Chinese `--help` text and clap parse errors. English stays the default; Chinese is used when the language
//! resolves to Chinese (see `weflow_core::locale`).

use clap::builder::StyledStr;
use clap::{Arg, ArgAction, Command, CommandFactory, Error, FromArgMatches};
use weflow_core::locale::{self, translate_with, Lang};

use crate::help_style::pick;

include!("help_zh.rs");

fn parse_lang(value: &str) -> Option<Lang> {
    match value {
        "zh" => Some(Lang::Zh),
        "en" => Some(Lang::En),
        _ => None,
    }
}

/// Settle the output language before clap runs (help is printed during parsing). Precedence: `WEFLOW_LANG`,
/// the language saved in the config file (`weflow lang`), then the system.
fn prescan_lang() {
    if std::env::var("WEFLOW_LANG").is_ok_and(|v| !v.trim().is_empty()) {
        return;
    }
    let saved = weflow_core::config::default_config_path()
        .and_then(|p| weflow_core::config::ConfigStore::load(&p).ok())
        .and_then(|c| c.lang);
    if let Some(lang) = saved.as_deref().and_then(parse_lang) {
        locale::set(lang);
    }
}

fn tr_str(text: &str) -> Option<String> {
    translate_with(HELP, text)
}

fn translate_styled(text: Option<&StyledStr>) -> Option<StyledStr> {
    text.and_then(|t| tr_str(&t.to_string()))
        .map(StyledStr::from)
}

/// The command tree with `-h` / `--help` and `-V` / `-v` / `--version` kept working but left out of the
/// option lists. A command with required arguments shows its help when it is given none at all (like a
/// command that needs a subcommand).
pub fn build_command<T: CommandFactory>() -> Command {
    fn hide_flags(cmd: Command, root: bool) -> Command {
        let names: Vec<_> = cmd
            .get_subcommands()
            .filter(|c| c.get_name() != "help")
            .map(|c| c.get_name().to_string())
            .collect();
        let mut cmd = cmd;
        if cmd.get_arguments().any(|a| a.is_required_set()) {
            cmd = cmd.arg_required_else_help(true);
        }
        let mut cmd = cmd.disable_help_flag(true).disable_version_flag(true).arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .action(ArgAction::Help)
                .hide(true),
        );
        if root {
            cmd = cmd.arg(
                Arg::new("version")
                    .short('V')
                    .short_alias('v')
                    .long("version")
                    .action(ArgAction::Version)
                    .hide(true),
            );
        }
        for name in names {
            cmd = cmd.mut_subcommand(name, |sub| hide_flags(sub, false));
        }
        cmd
    }
    hide_flags(T::command(), true)
}

/// Replace the help text of a command tree with its Chinese translation.
pub fn localize_command(mut cmd: Command) -> Command {
    if let Some(about) = translate_styled(cmd.get_about()) {
        cmd = cmd.about(about);
    }
    if let Some(long) = translate_styled(cmd.get_long_about()) {
        cmd = cmd.long_about(long);
    }
    let ids: Vec<_> = cmd.get_arguments().map(|a| a.get_id().clone()).collect();
    for id in ids {
        cmd = cmd.mut_arg(id, |mut arg| {
            if let Some(help) = translate_styled(arg.get_help()) {
                arg = arg.help(help);
            }
            if let Some(long) = translate_styled(arg.get_long_help()) {
                arg = arg.long_help(long);
            }
            arg
        });
    }
    let names: Vec<_> = cmd
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect();
    for name in names {
        cmd = cmd.mut_subcommand(name, localize_command);
    }
    cmd
}

/// Chinese wording for clap's built-in headings and messages.
const BUILTIN: &[(&str, &str)] = &[
    ("Usage:", "用法："),
    ("Commands:", "命令："),
    ("Options:", "选项："),
    ("Arguments:", "参数："),
    (
        "Print help (see more with '--help')",
        "打印帮助；--help 显示更多",
    ),
    (
        "Print help (see a summary with '-h')",
        "打印帮助；-h 显示摘要",
    ),
    ("Print help", "打印帮助"),
    ("Print version", "打印版本"),
    (
        "Print this message or the help of the given subcommand(s)",
        "打印此消息或指定子命令的帮助",
    ),
    (
        "For more information, try '--help'.",
        "更多信息请尝试 '--help'。",
    ),
    (
        "error: unrecognized subcommand '{}'",
        "错误：无法识别的子命令 '{}'",
    ),
    (
        "error: unexpected argument '{}' found",
        "错误：发现意外的参数 '{}'",
    ),
    (
        "error: the following required arguments were not provided:",
        "错误：缺少以下必需的参数：",
    ),
    (
        "error: invalid value '{}' for '{}': {}",
        "错误：'{1}' 的值 '{0}' 无效：{2}",
    ),
    (
        "error: invalid value '{}' for '{}'",
        "错误：'{1}' 的值 '{0}' 无效",
    ),
    (
        "error: a value is required for '{}' but none was supplied",
        "错误：'{}' 需要一个值，但没有提供",
    ),
    (
        "error: the argument '{}' cannot be used multiple times",
        "错误：参数 '{}' 不能重复使用",
    ),
    (
        "error: the argument '{}' cannot be used with '{}'",
        "错误：参数 '{0}' 不能与 '{1}' 同时使用",
    ),
    (
        "error: the argument '{}' cannot be used with one or more of the other specified arguments",
        "错误：参数 '{}' 不能与其他已指定的一个或多个参数同时使用",
    ),
    (
        "error: unexpected value '{}' for '{}' found; no more were expected",
        "错误：'{1}' 发现多余的值 '{0}'",
    ),
    (
        "error: '{}' requires a subcommand but one was not provided",
        "错误：'{}' 需要子命令，但未提供",
    ),
    (
        "error: equal sign is needed when assigning values to '{}'",
        "错误：给 '{}' 赋值时需要等号",
    ),
    (
        "error: invalid UTF-8 was detected in one or more arguments",
        "错误：一个或多个参数中检测到无效的 UTF-8",
    ),
    ("error: ", "错误："),
    (
        "  tip: a similar argument exists: '{}'",
        "  提示：存在相似的参数：'{}'",
    ),
    (
        "  tip: a similar subcommand exists: '{}'",
        "  提示：存在相似的子命令：'{}'",
    ),
    (
        "  tip: to pass '{}' as a value, use '{}'",
        "  提示：如需将 '{0}' 作为值传入，请使用 '{1}'",
    ),
    (
        "  tip: some similar values exist: {}",
        "  提示：存在相似的值：{}",
    ),
    (
        "  tip: a similar value exists: '{}'",
        "  提示：存在相似的值：'{}'",
    ),
    ("  Usage: {}", "  用法：{}"),
    ("Usage: {}", "用法：{}"),
];

/// Translate clap's already-rendered help or error text line by line, then the inline notes.
fn localize_rendered(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let line = translate_with(BUILTIN, line).unwrap_or_else(|| line.to_string());
            inline_notes(&line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn inline_notes(line: &str) -> String {
    let mut line = line.to_string();
    for (en, zh) in [
        (
            "Print this message or the help of the given subcommand(s)",
            "打印此消息或指定子命令的帮助",
        ),
        (
            "Print help (see more with '--help')",
            "打印帮助；--help 显示更多",
        ),
        (
            "Print help (see a summary with '-h')",
            "打印帮助；-h 显示摘要",
        ),
        ("Print help", "打印帮助"),
        ("Print version", "打印版本"),
        ("[OPTIONS]", "[选项]"),
        ("<COMMAND>", "<命令>"),
        ("invalid digit found in string", "包含无效的数字字符"),
        (
            "cannot parse integer from empty string",
            "不能把空字符串解析为整数",
        ),
        ("number too large to fit in target type", "数值过大"),
        ("number too small to fit in target type", "数值过小"),
        ("invalid float literal", "无效的小数"),
        (
            "provided string was not `true` or `false`",
            "必须是 `true` 或 `false`",
        ),
    ] {
        line = line.replace(en, zh);
    }
    line
}

/// Parse `args`, the command line from the program's name on; help and parse errors are printed in the active
/// language.
pub fn parse<T: CommandFactory + FromArgMatches>(args: Vec<std::ffi::OsString>) -> T {
    prescan_lang();
    match parse_in_language::<T, _>(&args) {
        Ok(value) => value,
        Err(err) => exit_with(err),
    }
}

/// Parses `args` (from the program's name on) as [`parse`] does, in the active language, without printing or
/// exiting: the command API reads a request's command line in its own process with it (#432).
pub fn try_parse_from<T: CommandFactory + FromArgMatches>(args: &[String]) -> Result<T, Error> {
    parse_in_language::<T, _>(args)
}

/// Parses `args` with the English command tree. A command line that parses shows none of the tree's help texts,
/// and translating all of them takes a few milliseconds of every run, so the tree is translated only when clap
/// answers with help or an error and the language is Chinese; `args` are then parsed again with it, which gives
/// the same answer in Chinese. The version shows no help text and is not parsed again.
fn parse_in_language<T, A>(args: &[A]) -> Result<T, Error>
where
    T: CommandFactory + FromArgMatches,
    A: Clone + Into<std::ffi::OsString>,
{
    use clap::error::ErrorKind;
    let parse = |cmd: Command| {
        cmd.try_get_matches_from(args.iter().cloned())
            .and_then(|m| T::from_arg_matches(&m))
    };
    match parse(build_command::<T>()) {
        Err(err) if err.kind() != ErrorKind::DisplayVersion && locale::current() == Lang::Zh => {
            parse(localize_command(build_command::<T>()))
        }
        parsed => parsed,
    }
}

/// What clap says of `err`, in the active language, the options last in a usage line and its bracketed notes in
/// words.
fn rendered(err: &Error) -> String {
    rendered_in(err, locale::current())
}

/// What clap says of `err` as [`rendered`] has it, in `lang`.
fn rendered_in(err: &Error, lang: Lang) -> String {
    let text = options_last(&err.to_string());
    let text = if lang == Lang::Zh {
        localize_rendered(&text)
    } else {
        text
    };
    crate::help_style::plain_notes(&text, lang)
}

/// The `--json` document of a parse error and its exit code, as `--json` prints them; `None` for help and the
/// version, which are no errors.
pub fn json_failure(err: &Error) -> Option<(serde_json::Value, i32)> {
    json_failure_in(err, locale::current())
}

/// The `--json` document of a parse error as [`json_failure`] has it, in `lang`.
fn json_failure_in(err: &Error, lang: Lang) -> Option<(serde_json::Value, i32)> {
    use clap::error::ErrorKind;
    if matches!(
        err.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        return None;
    }
    let text = rendered_in(err, lang);
    let message = if err.kind() == ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
        pick(
            lang,
            "a subcommand or argument is missing; see --help",
            "缺少子命令或参数；请查看 --help",
        )
        .to_string()
    } else {
        parse_error_message(&text, lang)
    };
    let mut failure = weflow_core::error::AppError::usage(message);
    if let Some(usage) = text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("Usage:") || line.starts_with("用法："))
    {
        failure.details = Some(serde_json::json!({ "usage": usage }));
    }
    #[expect(
        clippy::expect_used,
        reason = "a document of JSON values and strings always serializes"
    )]
    let doc = serde_json::to_value(weflow_core::output::failure(failure.payload()))
        .expect("a failure document serializes");
    Some((doc, err.exit_code()))
}

/// In every usage line the options come last (`weflow config set <KEY> <VALUE> [OPTIONS]`).
fn options_last(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            if line.trim_start().starts_with("Usage:") && line.contains(" [OPTIONS]") {
                format!(
                    "{} [OPTIONS]",
                    line.replacen(" [OPTIONS]", "", 1).trim_end()
                )
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn exit_with(err: Error) -> ! {
    use clap::error::ErrorKind;
    use weflow_core::output::{eprint, print};
    // help or a usage error that cannot be written has nowhere else to go; the exit code still says what happened
    if json_requested() {
        if let Some((doc, exit)) = json_failure(&err) {
            let _ = print(&format!("{doc}\n"));
            std::process::exit(exit);
        }
    }
    use crate::help_style::painted;
    use weflow_core::colour::{Paint, Stream};
    let text = rendered(&err);
    let _ = match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            print(&painted(&text, Paint::of(Stream::Stdout)))
        }
        _ => eprint(&painted(&text, Paint::of(Stream::Stderr))),
    };
    std::process::exit(err.exit_code());
}

/// `--json` anywhere before a `--` separator: parse errors then go to stdout as a JSON error, like every other error.
fn json_requested() -> bool {
    json_flag_in(std::env::args_os().skip(1))
}

fn json_flag_in(args: impl IntoIterator<Item = std::ffi::OsString>) -> bool {
    args.into_iter()
        .take_while(|arg| arg != "--")
        .any(|arg| arg == "--json")
}

/// The message of a parse error from clap's rendered `text` in `lang`: the first line without its `error: ` prefix,
/// then the lines indented under it up to the next empty line, which name the missing arguments or the possible
/// values; clap's tips come after that empty line and are left out. After a first line that ends in a colon the lines
/// are a list (`...were not provided: <PAYER>, <RECEIVER>`), after any other they are notes behind a semicolon.
fn parse_error_message(text: &str, lang: Lang) -> String {
    let mut lines = text.lines().skip_while(|line| line.trim().is_empty());
    let first = lines.next().map(str::trim).unwrap_or_default();
    let first = first
        .strip_prefix("error: ")
        .or_else(|| first.strip_prefix("错误："))
        .unwrap_or(first);
    let details: Vec<&str> = lines
        .take_while(|line| !line.trim().is_empty())
        .map(str::trim)
        .filter(|line| !line.starts_with("tip:") && !line.starts_with("提示："))
        .collect();
    if details.is_empty() {
        first.to_string()
    } else if first.ends_with(':') {
        format!("{first} {}", details.join(", "))
    } else if first.ends_with('：') {
        format!("{first}{}", details.join(", "))
    } else {
        let note = pick(lang, "; ", "；");
        format!("{first}{note}{}", details.join(note))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_come_last_in_usage_lines() {
        assert_eq!(
            options_last("Usage: weflow config set [OPTIONS] <KEY> <VALUE>\n\nOptions:"),
            "Usage: weflow config set <KEY> <VALUE> [OPTIONS]\n\nOptions:"
        );
        assert_eq!(
            options_last("Usage: weflow export messages [OPTIONS] --out <OUT> <SESSION_ID>"),
            "Usage: weflow export messages --out <OUT> <SESSION_ID> [OPTIONS]"
        );
        assert_eq!(
            options_last("Usage: weflow config set <KEY> <VALUE>"),
            "Usage: weflow config set <KEY> <VALUE>"
        );
    }

    #[test]
    fn json_flag_is_found_before_the_separator_only() {
        let args = |list: &[&str]| {
            list.iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>()
        };
        assert!(json_flag_in(args(&[
            "chat", "sessions", "--bogus", "--json"
        ])));
        assert!(!json_flag_in(args(&["chat", "sessions", "--bogus"])));
        assert!(!json_flag_in(args(&["config", "set", "k", "--", "--json"])));
    }

    #[test]
    fn parse_error_message_is_the_first_line_without_prefix() {
        assert_eq!(
            parse_error_message(
                "error: unexpected argument '--bogus' found\n\nUsage: weflow chat",
                Lang::En
            ),
            "unexpected argument '--bogus' found"
        );
        assert_eq!(
            parse_error_message(
                "错误：发现意外的参数 '--bogus'\n\n用法：weflow chat",
                Lang::Zh
            ),
            "发现意外的参数 '--bogus'"
        );
        // the tips after an empty line are no part of the message
        assert_eq!(
            parse_error_message(
                "error: unexpected argument '--limt' found\n\n  tip: a similar argument exists: '--limit'\n\nUsage: weflow chat",
                Lang::En
            ),
            "unexpected argument '--limt' found"
        );
    }

    /// The indented lines under the first line say which arguments are missing and which values are possible (#1041).
    #[test]
    fn parse_error_message_keeps_the_lines_under_the_first_line() {
        assert_eq!(
            parse_error_message(
                "error: the following required arguments were not provided:\n  <PAYER>\n  <RECEIVER>\n\nUsage: weflow chat transfer-names <CHATROOM_ID> <PAYER> <RECEIVER>\n\nFor more information, try '--help'.",
                Lang::En
            ),
            "the following required arguments were not provided: <PAYER>, <RECEIVER>"
        );
        assert_eq!(
            parse_error_message(
                "错误：缺少以下必需的参数：\n  <PAYER>\n  <RECEIVER>\n\n用法：weflow chat transfer-names <CHATROOM_ID> <PAYER> <RECEIVER>",
                Lang::Zh
            ),
            "缺少以下必需的参数：<PAYER>, <RECEIVER>"
        );
        assert_eq!(
            parse_error_message(
                "error: invalid value 'bogus' for '--variant <VARIANT>'\n  possible values: best, thumb, normal, hd\n\n  tip: a similar value exists: 'best'\n\nUsage: weflow chat image-data",
                Lang::En
            ),
            "invalid value 'bogus' for '--variant <VARIANT>'; possible values: best, thumb, normal, hd"
        );
        assert_eq!(
            parse_error_message(
                "错误：'--variant <VARIANT>' 的值 'bogus' 无效\n  可选值：best, thumb, normal, hd\n\n用法：weflow chat image-data",
                Lang::Zh
            ),
            "'--variant <VARIANT>' 的值 'bogus' 无效；可选值：best, thumb, normal, hd"
        );
    }

    /// The error clap gives for `args` (without the program's name).
    fn parse_failure(args: &[&str]) -> Error {
        let line: Vec<String> = std::iter::once("weflow")
            .chain(args.iter().copied())
            .map(String::from)
            .collect();
        match try_parse_from::<crate::cli::Cli>(&line) {
            Ok(_) => panic!("{args:?} parses"),
            Err(err) => err,
        }
    }

    /// The `error.message` of the `--json` document of the error of `args`, in `lang`.
    fn json_message(args: &[&str], lang: Lang) -> String {
        let (doc, exit) = json_failure_in(&parse_failure(args), lang).expect("a parse error");
        assert_eq!(exit, 2, "{args:?}");
        assert_eq!(doc["error"]["code"], "invalid_arguments", "{args:?}");
        doc["error"]["message"]
            .as_str()
            .expect("a message")
            .to_string()
    }

    /// `--json` names the arguments a command is missing, not only that some are (#1041).
    #[test]
    fn the_json_error_names_the_missing_arguments() {
        let args = ["chat", "transfer-names", "a"];
        assert_eq!(
            json_message(&args, Lang::En),
            "the following required arguments were not provided: <PAYER>, <RECEIVER>"
        );
        assert_eq!(
            json_message(&args, Lang::Zh),
            "缺少以下必需的参数：<PAYER>, <RECEIVER>"
        );
    }

    /// `--json` gives the values an argument takes when it is given another (#1041).
    #[test]
    fn the_json_error_lists_the_possible_values() {
        let line = |value: &'static str| ["chat", "image-data", "wxid_a", "1", "--variant", value];
        assert_eq!(
            json_message(&line("bogus"), Lang::En),
            "invalid value 'bogus' for '--variant <VARIANT>'; possible values: best, thumb, normal, hd"
        );
        assert_eq!(
            json_message(&line("bogus"), Lang::Zh),
            "'--variant <VARIANT>' 的值 'bogus' 无效；可选值：best, thumb, normal, hd"
        );
        // clap's tip for a value close to a possible one is no part of the message
        assert_eq!(
            json_message(&line("thum"), Lang::En),
            "invalid value 'thum' for '--variant <VARIANT>'; possible values: best, thumb, normal, hd"
        );
        assert_eq!(
            json_message(&line("thum"), Lang::Zh),
            "'--variant <VARIANT>' 的值 'thum' 无效；可选值：best, thumb, normal, hd"
        );
    }

    /// An error of one line stays that line.
    #[test]
    fn the_json_error_of_one_line_is_that_line() {
        assert_eq!(
            json_message(&["chat", "sessions", "--bogus"], Lang::En),
            "unexpected argument '--bogus' found"
        );
        assert_eq!(
            json_message(&["chat", "sessions", "--bogus"], Lang::Zh),
            "发现意外的参数 '--bogus'"
        );
    }

    /// Every English help text in the command tree must have a Chinese translation.
    #[test]
    fn all_help_text_is_translated() {
        fn walk(cmd: &Command, missing: &mut Vec<String>) {
            let mut check = |text: Option<&StyledStr>| {
                if let Some(t) = text {
                    let t = t.to_string();
                    if tr_str(&t).is_none() {
                        missing.push(t);
                    }
                }
            };
            check(cmd.get_about());
            check(cmd.get_long_about());
            for arg in cmd.get_arguments() {
                check(arg.get_help());
                check(arg.get_long_help());
                for v in arg.get_possible_values() {
                    check(v.get_help());
                }
            }
            for sub in cmd.get_subcommands() {
                walk(sub, missing);
            }
        }
        let mut missing = Vec::new();
        walk(&build_command::<crate::cli::Cli>(), &mut missing);
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "untranslated help text:\n{}",
            missing
                .iter()
                .map(|m| format!("{m:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// Every English text of the translations is a help text of the command tree: an entry none uses is stale.
    #[test]
    fn every_translation_is_used() {
        fn walk(cmd: &Command, texts: &mut Vec<String>) {
            let mut keep = |text: Option<&StyledStr>| texts.extend(text.map(StyledStr::to_string));
            keep(cmd.get_about());
            keep(cmd.get_long_about());
            for arg in cmd.get_arguments() {
                keep(arg.get_help());
                keep(arg.get_long_help());
                for v in arg.get_possible_values() {
                    keep(v.get_help());
                }
            }
            for sub in cmd.get_subcommands() {
                walk(sub, texts);
            }
        }
        let mut texts = Vec::new();
        walk(&build_command::<crate::cli::Cli>(), &mut texts);
        let unused: Vec<&str> = HELP
            .iter()
            .map(|(en, _)| *en)
            .filter(|en| {
                !texts
                    .iter()
                    .any(|text| translate_with(&[(*en, "")], text).is_some())
            })
            .collect();
        assert!(
            unused.is_empty(),
            "translations no help text uses:\n{}",
            unused
                .iter()
                .map(|m| format!("{m:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// Every subcommand and every non-global option or argument needs a help text.
    #[test]
    fn every_command_and_argument_has_help() {
        fn walk(cmd: &Command, path: &str, missing: &mut Vec<String>) {
            for arg in cmd.get_arguments() {
                let id = arg.get_id().as_str();
                if arg.is_global_set() || id == "help" || id == "version" {
                    continue;
                }
                if arg.get_help().is_none() && arg.get_long_help().is_none() {
                    missing.push(format!("{path} --{id}"));
                }
            }
            for sub in cmd.get_subcommands() {
                let name = sub.get_name();
                if name == "help" {
                    continue;
                }
                let sub_path = format!("{path} {name}").trim().to_string();
                if sub.get_about().is_none() && sub.get_long_about().is_none() {
                    missing.push(format!("{sub_path} (command)"));
                }
                walk(sub, &sub_path, missing);
            }
        }
        let mut missing = Vec::new();
        walk(&build_command::<crate::cli::Cli>(), "", &mut missing);
        assert!(
            missing.is_empty(),
            "no help text for:\n{}",
            missing.join("\n")
        );
    }
}
