//! 从 clap 的命令树收集的路由：每个叶子命令及其参数、请求必须使用的方法，以及把请求的参数还原成命令行。

use std::collections::BTreeMap;

use clap::{ArgAction, Command, CommandFactory};
use serde_json::Value;

use super::rules::{Rule, Verb, HIDDEN, METHODS};

/// 参数接受值的方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// 开关，如 `--force`：给出 `true` 或 `false`。
    Flag,
    /// 一个值。
    Single,
    /// 任意多个值（重复的查询参数或 JSON 数组）。
    Multi,
    /// 与 `Multi` 相同，但也可以完全不带值（如 `analytics excluded --set`）。
    MultiOrBare,
}

/// 命令的一个参数。
#[derive(Clone, Debug)]
pub(super) struct Param {
    /// 长选项名；位置参数是参数 ID 的 kebab-case 形式。
    pub(super) name: String,
    pub(super) kind: Kind,
    /// 位置参数的序号；选项为 `None`。
    pub(super) index: Option<usize>,
}

/// 一条命令的路由：命令路径、确定方法的规则和参数。
#[derive(Clone, Debug)]
pub(super) struct Endpoint {
    pub(super) path: Vec<String>,
    rule: Rule,
    pub(super) params: Vec<Param>,
}

/// CLI 的每个叶子命令及其确定方法的规则和参数，以命令名（以空格连接的命令路径）为键。
pub(super) struct Endpoints(pub(super) BTreeMap<String, Endpoint>);

impl Endpoints {
    pub(super) fn new() -> Self {
        let mut cmd = crate::cli::Cli::command();
        cmd.build();
        let mut out = BTreeMap::new();
        collect(&cmd, &mut Vec::new(), &mut out);
        Self(out)
    }

    pub(super) fn get(&self, path: &[String]) -> Option<&Endpoint> {
        self.0.get(&path.join(" "))
    }
}

/// 把 `cmd` 下的叶子命令收集进 `out`，跳过 `help` 和 [`HIDDEN`] 中的命令；`path` 是 `cmd` 的命令路径。
fn collect(cmd: &Command, path: &mut Vec<String>, out: &mut BTreeMap<String, Endpoint>) {
    let subs: Vec<&Command> = cmd
        .get_subcommands()
        .filter(|c| c.get_name() != "help")
        .collect();
    if subs.is_empty() {
        let key = path.join(" ");
        if HIDDEN.contains(&key.as_str()) {
            return;
        }
        // METHODS 中没有的命令在这里略过，由测试报告
        let Some(rule) = METHODS.iter().find(|(p, _)| *p == key).map(|(_, r)| *r) else {
            return;
        };
        out.insert(
            key,
            Endpoint {
                path: path.clone(),
                rule,
                params: params_of(cmd),
            },
        );
        return;
    }
    for sub in subs {
        path.push(sub.get_name().to_string());
        collect(sub, path, out);
        path.pop();
    }
}

fn params_of(cmd: &Command) -> Vec<Param> {
    api_args(cmd).map(param_of).collect()
}

/// 请求可以设置的命令参数：不含全局参数、隐藏的参数和 clap 自带的 `--help`、`--version`。
pub(super) fn api_args(cmd: &Command) -> impl Iterator<Item = &clap::Arg> {
    cmd.get_arguments().filter(|a| {
        // clap 自带的 `--help` 和 `--version`，而不是名为 `version` 的参数（`web use <VERSION>`）
        let built_in = matches!(
            a.get_action(),
            ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong | ArgAction::Version
        );
        !a.is_global_set() && !a.is_hide_set() && !built_in
    })
}

/// clap 的参数 `a` 对应的 [`Param`]：名字、接受值的方式和位置。
pub(super) fn param_of(a: &clap::Arg) -> Param {
    let name = a
        .get_long()
        .map(str::to_string)
        .unwrap_or_else(|| a.get_id().as_str().replace('_', "-"));
    let kind = match a.get_action() {
        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count => Kind::Flag,
        _ => {
            let range = a
                .get_num_args()
                .unwrap_or(clap::builder::ValueRange::SINGLE);
            if range.min_values() == 0 && range.takes_values() {
                Kind::MultiOrBare
            } else if range.max_values() > 1 || matches!(a.get_action(), ArgAction::Append) {
                Kind::Multi
            } else {
                Kind::Single
            }
        }
    };
    Param {
        name,
        kind,
        index: a.get_index(),
    }
}

/// 按名字排列的请求参数：查询参数，再用 JSON 请求体的键覆盖。
pub(super) type Params = BTreeMap<String, Vec<Value>>;

/// 参数值的文字：字符串取它本身，其他值写成 JSON。
pub(super) fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// 开关的值：布尔值；数字，非 0 为真；字符串，不区分大小写，`true`、`1`、`yes` 和空字符串为真，
/// `false`、`0`、`no` 为假。其他值为 `None`。
pub(super) fn flag_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_i64().map(|n| n != 0),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "" | "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// 请求的键对应的参数：长选项名，`_` 也可以代替 `-`。
pub(super) fn find_param<'a>(endpoint: &'a Endpoint, key: &str) -> Option<&'a Param> {
    let key = key.replace('_', "-");
    endpoint.params.iter().find(|p| p.name == key)
}

impl Endpoint {
    /// 命令名：以空格连接的命令路径，如 `chat messages`。
    pub(super) fn command_name(&self) -> String {
        self.path.join(" ")
    }

    /// 带这些参数的请求必须使用的方法。
    pub(super) fn verb(&self, params: &Params) -> Verb {
        match self.rule {
            Rule::Fixed(v) => v,
            Rule::Arg { arg, without, with } => {
                let given = params.iter().any(|(k, values)| {
                    find_param(self, k).is_some_and(|p| p.name == arg)
                        && !values.iter().all(|v| v == &Value::Bool(false))
                });
                if given {
                    with
                } else {
                    without
                }
            }
        }
    }

    /// 命令可能使用的所有方法，供遍历路由的测试使用。
    #[cfg(test)]
    pub(super) fn verbs(&self) -> Vec<Verb> {
        match self.rule {
            Rule::Fixed(v) => vec![v],
            Rule::Arg { without, with, .. } => vec![without, with],
        }
    }

    /// 拒绝命令不接受的参数：命令没有的参数名、既不是真也不是假的开关值，
    /// 以及只接受一个值的位置参数给了多个值（在命令行上，第二个值会落到下一个参数；
    /// 重复给出的选项由 clap 自己拒绝）。其余由命令自己检查，或者由 `/api/` 的类型化读取（#397）按命令的方式检查。
    pub(super) fn check(&self, params: &Params) -> Result<(), String> {
        let mut singles: BTreeMap<&str, usize> = BTreeMap::new();
        for (key, values) in params {
            let Some(param) = find_param(self, key) else {
                let known: Vec<&str> = self.params.iter().map(|p| p.name.as_str()).collect();
                return Err(if known.is_empty() {
                    format!(
                        "unknown parameter `{key}`: `{}` takes no parameters",
                        self.command_name()
                    )
                } else {
                    format!(
                        "unknown parameter `{key}` for `{}`; expected one of: {}",
                        self.command_name(),
                        known.join(", ")
                    )
                });
            };
            if param.kind == Kind::Flag && values.iter().any(|v| flag_value(v).is_none()) {
                return Err(format!(
                    "parameter `{}` is a switch: pass true or false",
                    param.name
                ));
            }
            if param.kind == Kind::Single && param.index.is_some() {
                // `session-id` 和 `session_id` 指同一个参数
                let given = singles.entry(param.name.as_str()).or_default();
                *given += values.len();
                if *given > 1 {
                    return Err(format!(
                        "parameter `{}` takes one value, not several",
                        param.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// 这些参数对应的命令行（`weflow` 和全局选项之后的部分）。选项写成 `--name=value`，位置参数放在 `--` 之后，
    /// 所以以 `-` 开头的值仍是值。命令自己检查结果，与在终端上完全一样。
    /// 参数不被接受（见 [`Endpoint::check`]）或含 NUL 字节时，`Err` 是说明。
    pub(super) fn argv(&self, params: &Params) -> Result<Vec<String>, String> {
        self.check(params)?;
        let mut options = Vec::new();
        let mut positionals: Vec<(usize, Vec<String>)> = Vec::new();
        for (key, values) in params {
            // 未知的参数已由 `check` 回答，并列出了可用的参数
            let param =
                find_param(self, key).ok_or_else(|| format!("unknown parameter `{key}`"))?;
            if let Some(index) = param.index {
                positionals.push((index, texts(param, values)));
                continue;
            }
            match param.kind {
                Kind::Flag => {
                    for v in values {
                        if flag_value(v) == Some(true) {
                            options.push(format!("--{}", param.name));
                        }
                    }
                }
                Kind::MultiOrBare if texts(param, values).iter().all(String::is_empty) => {
                    options.push(format!("--{}", param.name));
                }
                _ => {
                    for text in texts(param, values) {
                        options.push(format!("--{}={text}", param.name));
                    }
                }
            }
        }
        let mut argv = self.path.clone();
        argv.extend(options);
        if !positionals.is_empty() {
            positionals.sort_by_key(|(index, _)| *index);
            argv.push("--".into());
            argv.extend(positionals.into_iter().flat_map(|(_, v)| v));
        }
        // 命令行不能含 NUL 字节：在这里拒绝，而不是等到子进程无法启动（#822）
        if argv.iter().any(|arg| arg.contains('\0')) {
            return Err("a parameter holds a NUL byte, which a command line cannot carry".into());
        }
        Ok(argv)
    }
}

/// 非开关参数的值，写成命令行上的样子：每个值写成文字；参数接受多个值时，JSON 数组的各项逐个列出。
pub(super) fn texts(param: &Param, values: &[Value]) -> Vec<String> {
    values
        .iter()
        .flat_map(|v| match v {
            Value::Array(items) if param.kind != Kind::Single => {
                items.iter().map(text_of).collect()
            }
            other => vec![text_of(other)],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve_api::files;
    use crate::serve_api::rules::Verb::*;
    use crate::serve_api::tests::params;
    use serde_json::json;

    /// 服务自己读取的查询参数和请求体的键；命令的参数都不能用这些名字。
    const RESERVED_PARAMS: &[&str] = &["std", "return", "access_token", "fields"];

    /// 不是命令的路径第一段：WebUI 和服务自己的路由。
    const RESERVED_PATHS: &[&str] = &["api", "assets", "health", "jobs", "openapi.json", "ui"];

    #[test]
    fn file_commands_write_to_out() {
        let endpoints = Endpoints::new();
        for (name, _, _) in files::FILE_COMMANDS {
            let ep = endpoints
                .0
                .get(*name)
                .unwrap_or_else(|| panic!("{name} is not a command"));
            // `--out`，或者群导出命令的位置参数 `out`
            assert!(
                ep.params.iter().any(|p| p.name == "out"),
                "{name} has no out"
            );
        }
    }

    #[test]
    fn reserved_names_stay_free() {
        let mut cmd = crate::cli::Cli::command();
        cmd.build();
        for sub in cmd.get_subcommands() {
            assert!(
                !RESERVED_PATHS.contains(&sub.get_name()),
                "command `{}` takes a reserved path",
                sub.get_name()
            );
        }
        for ep in Endpoints::new().0.values() {
            for p in &ep.params {
                assert!(
                    !RESERVED_PARAMS.contains(&p.name.as_str())
                        && !RESERVED_PARAMS.contains(&p.name.replace('-', "_").as_str()),
                    "{} uses the reserved parameter {}",
                    ep.command_name(),
                    p.name
                );
            }
        }
    }

    fn argv(path: &str, pairs: &[(&str, Value)]) -> Result<Vec<String>, String> {
        let endpoints = Endpoints::new();
        endpoints.0.get(path).unwrap().argv(&params(pairs))
    }

    /// 值中的 NUL 字节被拒绝：带着它的子进程无法启动（#822）。
    #[test]
    fn a_nul_byte_is_refused() {
        let err = argv("chat messages", &[("session-id", json!("a\0b"))]).unwrap_err();
        assert!(err.contains("NUL"), "{err}");
    }

    #[test]
    fn builds_the_command_line() {
        assert_eq!(argv("config list", &[]).unwrap(), ["config", "list"]);
        assert_eq!(
            argv(
                "chat messages",
                &[("session-id", json!("-a b")), ("limit", json!(5))]
            )
            .unwrap(),
            ["chat", "messages", "--limit=5", "--", "-a b"]
        );
        // 可以用 snake_case 代替 kebab-case
        assert_eq!(
            argv("chat messages", &[("session_id", json!("x"))]).unwrap(),
            ["chat", "messages", "--", "x"]
        );
        // 位置参数的多个值：来自重复的键或数组
        assert_eq!(
            argv(
                "chat counts",
                &[("sessions", json!("a")), ("sessions", json!("b"))]
            )
            .unwrap(),
            ["chat", "counts", "--", "a", "b"]
        );
        assert_eq!(
            argv("chat counts", &[("sessions", json!(["a", "微信"]))]).unwrap(),
            ["chat", "counts", "--", "a", "微信"]
        );
        // 开关
        assert_eq!(
            argv(
                "chat detail",
                &[
                    ("session-id", json!("s")),
                    ("fast", json!(true)),
                    ("extra", json!("false"))
                ]
            )
            .unwrap(),
            ["chat", "detail", "--fast", "--", "s"]
        );
        assert!(argv("chat detail", &[("fast", json!("maybe"))]).is_err());
        // 接受任意多个值（也可以没有值）的选项
        assert_eq!(
            argv("analytics excluded", &[("set", json!(""))]).unwrap(),
            ["analytics", "excluded", "--set"]
        );
        assert_eq!(
            argv("analytics excluded", &[("set", json!(["a", "b"]))]).unwrap(),
            ["analytics", "excluded", "--set=a", "--set=b"]
        );
        // 单值参数的 JSON 值按 JSON 文字传入
        assert_eq!(
            argv(
                "chat voice-preload",
                &[
                    ("session-id", json!("s")),
                    ("messages-json", json!([{"localId": 1}]))
                ]
            )
            .unwrap(),
            ["chat", "voice-preload", "--", "s", r#"[{"localId":1}]"#]
        );
        let err = argv("config list", &[("bogus", json!(1))]).unwrap_err();
        assert!(err.contains("bogus"), "{err}");
    }

    /// 只接受一个值的位置参数给了两次时拒绝：第二个值会落到下一个参数。
    #[test]
    fn a_single_value_given_twice_is_refused() {
        for pairs in [
            vec![("session-id", json!("a")), ("session-id", json!("3"))],
            vec![("session-id", json!("a")), ("session_id", json!("3"))],
        ] {
            let err = argv("chat voice-data", &pairs).unwrap_err();
            assert!(err.contains("takes one value"), "{pairs:?}: {err}");
        }
        // 重复给出的选项交给 clap，由它拒绝并给出用法
        assert_eq!(
            argv(
                "chat voice-data",
                &[
                    ("session-id", json!("a")),
                    ("local-id", json!("1")),
                    ("out", json!("x")),
                    ("out", json!("y")),
                ]
            )
            .unwrap(),
            ["chat", "voice-data", "--out=x", "--out=y", "--", "a", "1"]
        );
        // 数组本身是一个值，按 JSON 文字传入
        assert_eq!(
            argv(
                "chat voice-data",
                &[("session-id", json!(["a", "3"])), ("local-id", json!("1"))]
            )
            .unwrap(),
            ["chat", "voice-data", "--", r#"["a","3"]"#, "1"]
        );
    }

    #[test]
    fn methods_follow_the_arguments() {
        let endpoints = Endpoints::new();
        let ep = |p: &str| endpoints.0.get(p).unwrap();
        assert_eq!(ep("config list").verb(&Params::new()), Get);
        assert_eq!(ep("analytics excluded").verb(&Params::new()), Get);
        assert_eq!(
            ep("analytics excluded").verb(&params(&[("set", json!(["a"]))])),
            Put
        );
        assert_eq!(ep("analytics excluded").verbs(), [Get, Put]);
    }
}
