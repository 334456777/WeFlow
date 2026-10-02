//! Chinese `--help` text and clap parse errors. English stays the default; Chinese is used when the language
//! resolves to Chinese (see `weflow_core::locale`).

use clap::builder::StyledStr;
use clap::{Arg, ArgAction, ArgMatches, Command, CommandFactory, Error, FromArgMatches};
use weflow_core::locale::{self, translate_with, Lang};

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
        "打印帮助（使用 '--help' 查看更多）",
    ),
    (
        "Print help (see a summary with '-h')",
        "打印帮助（使用 '-h' 查看摘要）",
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
    ("  [possible values: {}]", "  [可选值：{}]"),
    (
        "  tip: some similar values exist: {}",
        "  提示：存在相似的值：{}",
    ),
    (
        "  tip: a similar value exists: '{}'",
        "  提示：存在相似的值：'{}'",
    ),
    ("[possible values: {}]", "[可选值：{}]"),
    ("[default: {}]", "[默认值：{}]"),
    ("[env: {}]", "[环境变量：{}]"),
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
        ("[possible values: ", "[可选值："),
        ("[default: ", "[默认值："),
        ("[env: ", "[环境变量："),
        (
            "Print this message or the help of the given subcommand(s)",
            "打印此消息或指定子命令的帮助",
        ),
        (
            "Print help (see more with '--help')",
            "打印帮助（使用 '--help' 查看更多）",
        ),
        (
            "Print help (see a summary with '-h')",
            "打印帮助（使用 '-h' 查看摘要）",
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

/// Parse the command line; help and parse errors are printed in the active language.
pub fn parse<T: CommandFactory + FromArgMatches>() -> T {
    let args: Vec<String> = std::env::args().collect();
    prescan_lang();
    let mut cmd = build_command::<T>();
    if locale::current() == Lang::Zh {
        cmd = localize_command(cmd);
    }
    let parsed = cmd
        .try_get_matches_from(&args)
        .and_then(|m| match misplaced_flag(&args, &m) {
            Some(err) => Err(err),
            None => T::from_arg_matches(&m),
        });
    match parsed {
        Ok(value) => value,
        Err(err) => exit_with(err),
    }
}

/// `--json` and `--progress` belong after the command (`weflow config path --json`); in front of it they are
/// refused although clap would accept a global option anywhere.
fn misplaced_flag(args: &[String], matches: &ArgMatches) -> Option<Error> {
    let mut chain = Vec::new();
    let mut current = matches;
    while let Some((name, sub)) = current.subcommand() {
        chain.push(name.to_string());
        current = sub;
    }
    if chain.is_empty() {
        return None;
    }
    let (mut next, mut last) = (0, 0);
    for (i, arg) in args.iter().enumerate().skip(1) {
        if arg == "--" {
            break;
        }
        if next < chain.len() && *arg == chain[next] {
            last = i;
            next += 1;
        }
    }
    if next < chain.len() {
        return None;
    }
    let flag = args[1..last]
        .iter()
        .find(|a| matches!(a.as_str(), "--json" | "--progress"))?;
    Some(Error::raw(
        clap::error::ErrorKind::UnknownArgument,
        format!("unexpected argument '{flag}' found\n"),
    ))
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
    let mut text = options_last(&err.to_string());
    if locale::current() == Lang::Zh {
        text = localize_rendered(&text);
    }
    match err.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayVersion
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            let to_stderr = err.kind() == ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand;
            if to_stderr {
                eprint!("{text}");
            } else {
                print!("{text}");
            }
        }
        _ => eprint!("{text}"),
    }
    std::process::exit(err.exit_code());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn misplaced(args: &[&str]) -> bool {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let m = build_command::<crate::Cli>()
            .try_get_matches_from(&args)
            .unwrap();
        misplaced_flag(&args, &m).is_some()
    }

    #[test]
    fn json_and_progress_go_after_the_command() {
        assert!(!misplaced(&["weflow", "config", "path", "--json"]));
        assert!(!misplaced(&["weflow", "config", "path"]));
        assert!(!misplaced(&[
            "weflow",
            "chat",
            "sessions",
            "--progress",
            "--json"
        ]));
        assert!(misplaced(&["weflow", "--json", "config", "path"]));
        assert!(misplaced(&["weflow", "config", "--json", "path"]));
        assert!(misplaced(&["weflow", "--progress", "chat", "sessions"]));
    }

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
        walk(&build_command::<crate::Cli>(), &mut missing);
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
        walk(&build_command::<crate::Cli>(), "", &mut missing);
        assert!(
            missing.is_empty(),
            "no help text for:\n{}",
            missing.join("\n")
        );
    }
}
