//! Chinese `--help` text and clap parse errors. English stays the default; Chinese is used when the language
//! resolves to Chinese (see `weflow_core::locale`).

use clap::builder::StyledStr;
use clap::{Command, CommandFactory, Error, FromArgMatches};
use weflow_core::locale::{self, translate_with, Lang};

include!("help_zh.rs");

/// The value of a global option given as `--name value` or `--name=value`.
fn option_value(args: &[String], name: &str) -> Option<String> {
    let eq = format!("{name}=");
    let mut it = args.iter().skip(1);
    while let Some(arg) = it.next() {
        if arg == "--" {
            break;
        }
        if let Some(v) = arg.strip_prefix(&eq) {
            return Some(v.to_string());
        }
        if arg == name {
            return it.next().cloned();
        }
    }
    None
}

fn parse_lang(value: &str) -> Option<Lang> {
    match value {
        "zh" => Some(Lang::Zh),
        "en" => Some(Lang::En),
        _ => None,
    }
}

/// Settle the output language before clap runs (help is printed during parsing). Precedence: `--lang`,
/// `WEFLOW_LANG`, the language saved in the config file, then the system.
fn prescan_lang(args: &[String]) {
    if let Some(lang) = option_value(args, "--lang").as_deref().and_then(parse_lang) {
        locale::set(lang);
        return;
    }
    if std::env::var("WEFLOW_LANG").is_ok_and(|v| !v.trim().is_empty()) {
        return;
    }
    let path = option_value(args, "--config")
        .map(std::path::PathBuf::from)
        .or_else(weflow_core::config::default_config_path);
    let saved = path
        .and_then(|p| weflow_core::config::ConfigStore::load(&p).ok())
        .and_then(|c| c.lang);
    if let Some(lang) = saved.as_deref().and_then(parse_lang) {
        locale::set(lang);
    }
}

/// `weflow --lang zh` on its own (no command) saves the language in the config file.
fn save_language_if_alone(args: &[String]) {
    // `--config <path>` may accompany it; anything else (a command, other options) means a normal run.
    let mut rest: Vec<&str> = Vec::new();
    let mut it = args.iter().skip(1);
    while let Some(arg) = it.next() {
        if arg == "--config" {
            it.next();
        } else if !arg.starts_with("--config=") {
            rest.push(arg);
        }
    }
    let alone = match rest.as_slice() {
        [flag, _] => *flag == "--lang",
        [one] => one.starts_with("--lang="),
        _ => false,
    };
    if !alone {
        return;
    }
    let Some(value) = option_value(args, "--lang") else {
        return;
    };
    let Some(lang) = parse_lang(&value) else {
        return;
    };
    let path = option_value(args, "--config")
        .map(std::path::PathBuf::from)
        .or_else(weflow_core::config::default_config_path);
    let result = path
        .clone()
        .ok_or_else(|| "failed to locate platform config directory".to_string())
        .and_then(|path| {
            let mut config =
                weflow_core::config::ConfigStore::load(&path).map_err(|e| e.to_string())?;
            config.lang = Some(value.clone());
            config.save(&path).map_err(|e| e.to_string())?;
            Ok(path)
        });
    match result {
        Ok(path) => {
            let message = match lang {
                Lang::Zh => "已将语言设置为中文",
                Lang::En => "Language set to English",
            };
            println!("{message} ({})", path.display());
            std::process::exit(0);
        }
        Err(err) => {
            eprint!(
                "{}",
                weflow_core::render::render_error(&weflow_core::locale::localize(err), None)
            );
            std::process::exit(3);
        }
    }
}

fn tr_str(text: &str) -> Option<String> {
    translate_with(HELP, text)
}

fn translate_styled(text: Option<&StyledStr>) -> Option<StyledStr> {
    text.and_then(|t| tr_str(&t.to_string()))
        .map(StyledStr::from)
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
    save_language_if_alone(&args);
    prescan_lang(&args);
    if locale::current() == Lang::En {
        return T::parse_from_clap();
    }
    let cmd = localize_command(T::command());
    match cmd
        .try_get_matches_from(&args)
        .and_then(|m| T::from_arg_matches(&m))
    {
        Ok(value) => value,
        Err(err) => exit_with(err),
    }
}

fn exit_with(err: Error) -> ! {
    use clap::error::ErrorKind;
    let text = localize_rendered(&err.to_string());
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

trait ParseFromClap: Sized {
    fn parse_from_clap() -> Self;
}

impl<T: CommandFactory + FromArgMatches> ParseFromClap for T {
    fn parse_from_clap() -> Self {
        let mut matches = T::command().get_matches();
        match T::from_arg_matches_mut(&mut matches) {
            Ok(value) => value,
            Err(err) => err.format(&mut T::command()).exit(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        walk(&crate::Cli::command(), &mut missing);
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
}
