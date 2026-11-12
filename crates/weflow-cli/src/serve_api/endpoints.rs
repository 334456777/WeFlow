//! The routes, collected from the clap command tree: every leaf command with its parameters, and the method a
//! request must use.

use std::collections::BTreeMap;

use clap::{ArgAction, Command, CommandFactory};
use serde_json::Value;

use super::rules::{Rule, Verb, HIDDEN, METHODS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// `--force`: given as `true` / `false`.
    Flag,
    /// One value.
    Single,
    /// Any number of values (repeated query keys or a JSON array).
    Multi,
    /// Like `Multi`, but may also be given with no value at all (`analytics excluded --set`).
    MultiOrBare,
}

#[derive(Clone, Debug)]
pub(super) struct Param {
    /// The long option name, or the argument id in kebab case for positional arguments.
    pub(super) name: String,
    pub(super) kind: Kind,
    /// Position of a positional argument.
    pub(super) index: Option<usize>,
}

#[derive(Clone, Debug)]
pub(super) struct Endpoint {
    pub(super) path: Vec<String>,
    rule: Rule,
    pub(super) params: Vec<Param>,
}

/// Every leaf command of the CLI with its method rule and parameters.
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
        // a command missing from METHODS is left out here and reported by the tests
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

/// The arguments of a command that a request may set.
pub(super) fn api_args(cmd: &Command) -> impl Iterator<Item = &clap::Arg> {
    cmd.get_arguments().filter(|a| {
        // clap's own `--help` and `--version`, not an argument named `version` (`web use <VERSION>`)
        let built_in = matches!(
            a.get_action(),
            clap::ArgAction::Help
                | clap::ArgAction::HelpShort
                | clap::ArgAction::HelpLong
                | clap::ArgAction::Version
        );
        !a.is_global_set() && !a.is_hide_set() && !built_in
    })
}

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

/// Request parameters by name: query strings, overridden by the keys of a JSON body.
pub(super) type Params = BTreeMap<String, Vec<Value>>;

pub(super) fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

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

/// The parameter a request key names: the long option name, with `_` accepted for `-`.
pub(super) fn find_param<'a>(endpoint: &'a Endpoint, key: &str) -> Option<&'a Param> {
    let key = key.replace('_', "-");
    endpoint.params.iter().find(|p| p.name == key)
}

impl Endpoint {
    pub(super) fn command_name(&self) -> String {
        self.path.join(" ")
    }

    /// The verb a request with these parameters must use, or `None` when it cannot be told yet (the command
    /// then reports the missing argument itself).
    pub(super) fn verb(&self, params: &Params) -> Option<Verb> {
        match self.rule {
            Rule::Fixed(v) => Some(v),
            Rule::Arg { arg, without, with } => {
                let given = params.iter().any(|(k, values)| {
                    find_param(self, k).is_some_and(|p| p.name == arg)
                        && !values.iter().all(|v| v == &Value::Bool(false))
                });
                Some(if given { with } else { without })
            }
            Rule::Value { arg, values } => {
                let given = params
                    .iter()
                    .find(|(k, _)| find_param(self, k).is_some_and(|p| p.name == arg))
                    .and_then(|(_, v)| v.first())
                    .map(text_of)?;
                values
                    .iter()
                    .find(|(name, _)| *name == given)
                    .map(|(_, v)| *v)
            }
        }
    }

    pub(super) fn verbs(&self) -> Vec<Verb> {
        match self.rule {
            Rule::Fixed(v) => vec![v],
            Rule::Arg { without, with, .. } => vec![without, with],
            Rule::Value { values, .. } => {
                let mut out: Vec<Verb> = Vec::new();
                for (_, v) in values {
                    if !out.contains(v) {
                        out.push(*v);
                    }
                }
                out
            }
        }
    }

    /// Refuses what the command does not take: a name it has no parameter of, or a switch that is neither true nor
    /// false. The command itself checks the rest, or the typed reads of `/api/` (#397) do as it would.
    pub(super) fn check(&self, params: &Params) -> Result<(), String> {
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
        }
        Ok(())
    }

    /// The command line (after `weflow` and the global options) for these parameters. Options are written as
    /// `--name=value` and positional arguments after `--`, so values starting with `-` stay values. The command
    /// itself checks the result, exactly as on a terminal.
    pub(super) fn argv(&self, params: &Params) -> Result<Vec<String>, String> {
        self.check(params)?;
        let mut options = Vec::new();
        let mut positionals: Vec<(usize, Vec<String>)> = Vec::new();
        for (key, values) in params {
            // `check` has answered an unknown one with what is expected
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
        Ok(argv)
    }
}

/// The values of a parameter that is not a switch, as the command line carries them: each value as text, and the
/// items of a JSON array one by one where the parameter takes more than one.
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

    /// Query / body keys the server reads itself; no command argument may use them.
    const RESERVED_PARAMS: &[&str] = &["std", "return", "access_token", "fields"];

    /// First path segments that are not commands (the WebUI, later endpoints).
    const RESERVED_PATHS: &[&str] = &["api", "assets", "health", "jobs", "openapi.json", "ui"];

    #[test]
    fn file_commands_write_to_out() {
        let endpoints = Endpoints::new();
        for (name, _, _) in files::FILE_COMMANDS {
            let ep = endpoints
                .0
                .get(*name)
                .unwrap_or_else(|| panic!("{name} is not a command"));
            // `--out`, or the positional `out` of the group exports
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

    #[test]
    fn builds_the_command_line() {
        assert_eq!(argv("config list", &[]).unwrap(), ["config", "list"]);
        assert_eq!(
            argv(
                "chat latest",
                &[("session-id", json!("-a b")), ("limit", json!(5))]
            )
            .unwrap(),
            ["chat", "latest", "--limit=5", "--", "-a b"]
        );
        // snake case is accepted for kebab case
        assert_eq!(
            argv("chat latest", &[("session_id", json!("x"))]).unwrap(),
            ["chat", "latest", "--", "x"]
        );
        // repeated positional values, from repeated keys or an array
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
        // switches
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
        // an option that takes any number of values, none included
        assert_eq!(
            argv("analytics excluded", &[("set", json!(""))]).unwrap(),
            ["analytics", "excluded", "--set"]
        );
        assert_eq!(
            argv("analytics excluded", &[("set", json!(["a", "b"]))]).unwrap(),
            ["analytics", "excluded", "--set=a", "--set=b"]
        );
        // a JSON value for a single-valued argument is passed as JSON text
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

    #[test]
    fn methods_follow_the_arguments() {
        let endpoints = Endpoints::new();
        let ep = |p: &str| endpoints.0.get(p).unwrap();
        assert_eq!(ep("config list").verb(&Params::new()), Some(Get));
        assert_eq!(ep("analytics excluded").verb(&Params::new()), Some(Get));
        assert_eq!(
            ep("analytics excluded").verb(&params(&[("set", json!(["a"]))])),
            Some(Put)
        );
        assert_eq!(
            ep("sns block-delete").verb(&params(&[("action", json!("uninstall"))])),
            Some(Delete)
        );
        assert_eq!(ep("sns block-delete").verb(&Params::new()), None);
        assert_eq!(ep("sns block-delete").verbs(), [Get, Put, Delete]);
    }
}
