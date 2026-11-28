//! 每条命令使用的 HTTP 方法（[`METHODS`]），
//! 以及不能通过 HTTP 调用的命令（[`HIDDEN`]）和由服务进程自己回答的命令（[`OWN_STATE`]）。

use axum::http::Method;

use Verb::*;

/// 不能通过 HTTP 调用的命令。
pub(super) const HIDDEN: &[&str] = &["serve"];

/// 报告服务进程自己持有的状态、而不是文件内容的命令：服务运行的原图自动下载钩子，以及见解引擎记下的次数。
/// `serve` 自己回答这些命令（`own_state`），因为在子进程中运行的命令只能报告它自己的状态，其中没有这些（#1264）。
pub(super) const OWN_STATE: &[&str] = &["image auto-download status", "insight today-stats"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verb {
    Get,
    Post,
    Put,
    Delete,
}

impl Verb {
    pub(super) fn method(self) -> Method {
        match self {
            Get => Method::GET,
            Post => Method::POST,
            Put => Method::PUT,
            Delete => Method::DELETE,
        }
    }
}

/// 命令的 HTTP 方法怎样确定。
#[derive(Clone, Copy, Debug)]
pub(super) enum Rule {
    /// 固定用这个方法。
    Fixed(Verb),
    /// 给了参数 `arg` 时用 `with`，否则用 `without`（如 `analytics excluded --set`）。
    Arg {
        arg: &'static str,
        without: Verb,
        with: Verb,
    },
}

/// 每条命令的 HTTP 方法：`GET` 只读取（可能填充能够重建的缓存），`POST` 运行任务或写出文件，`PUT` 修改设置，
/// `DELETE` 删除设置、记录或缓存。`tests::every_command_has_a_method` 保证这张表与命令树一致。
pub(super) const METHODS: &[(&str, Rule)] = &[
    ("config path", Rule::Fixed(Get)),
    ("config list", Rule::Fixed(Get)),
    ("config profiles", Rule::Fixed(Get)),
    ("config get", Rule::Fixed(Get)),
    ("config set", Rule::Fixed(Put)),
    ("config unset", Rule::Fixed(Delete)),
    ("config clear", Rule::Fixed(Delete)),
    ("config remove-profile", Rule::Fixed(Delete)),
    ("db detect", Rule::Fixed(Get)),
    ("db scan", Rule::Fixed(Get)),
    ("db test", Rule::Fixed(Get)),
    ("db wxid", Rule::Fixed(Get)),
    ("key db", Rule::Fixed(Post)),
    ("key image", Rule::Fixed(Post)),
    ("key image-mem", Rule::Fixed(Post)),
    ("key processes", Rule::Fixed(Post)),
    ("chat sessions", Rule::Fixed(Get)),
    ("chat messages", Rule::Fixed(Get)),
    ("chat search", Rule::Fixed(Get)),
    ("chat contacts", Rule::Fixed(Get)),
    ("chat contact", Rule::Fixed(Get)),
    ("chat message", Rule::Fixed(Get)),
    ("chat dates", Rule::Fixed(Get)),
    ("chat date-counts", Rule::Fixed(Get)),
    ("chat counts", Rule::Fixed(Get)),
    ("chat statuses", Rule::Fixed(Get)),
    ("chat detail", Rule::Fixed(Get)),
    ("chat tab-counts", Rule::Fixed(Get)),
    ("chat export-stats", Rule::Fixed(Get)),
    (
        "chat group-hint",
        Rule::Arg {
            arg: "set",
            without: Get,
            with: Put,
        },
    ),
    ("chat resources", Rule::Fixed(Get)),
    ("chat images", Rule::Fixed(Get)),
    ("chat voice-messages", Rule::Fixed(Get)),
    ("chat media-stream", Rule::Fixed(Get)),
    ("chat transfer-names", Rule::Fixed(Get)),
    ("chat voice-data", Rule::Fixed(Post)),
    ("chat image-data", Rule::Fixed(Post)),
    ("chat image-info", Rule::Fixed(Get)),
    ("chat video-data", Rule::Fixed(Post)),
    ("chat voice-cache", Rule::Fixed(Get)),
    ("chat voice-preload", Rule::Fixed(Post)),
    ("chat emoji", Rule::Fixed(Post)),
    ("export sessions", Rule::Fixed(Post)),
    ("export contacts", Rule::Fixed(Post)),
    ("export footprint", Rule::Fixed(Post)),
    ("export media", Rule::Fixed(Post)),
    ("export messages", Rule::Fixed(Post)),
    ("analytics overall", Rule::Fixed(Get)),
    ("analytics rankings", Rule::Fixed(Get)),
    ("analytics time", Rule::Fixed(Get)),
    (
        "analytics excluded",
        Rule::Arg {
            arg: "set",
            without: Get,
            with: Put,
        },
    ),
    ("analytics exclude-candidates", Rule::Fixed(Get)),
    ("group list", Rule::Fixed(Get)),
    ("group members", Rule::Fixed(Get)),
    ("group ranking", Rule::Fixed(Get)),
    ("group hours", Rule::Fixed(Get)),
    ("group media", Rule::Fixed(Get)),
    ("group member", Rule::Fixed(Get)),
    ("group member-messages", Rule::Fixed(Get)),
    ("group export-member-messages", Rule::Fixed(Post)),
    ("group export-members", Rule::Fixed(Post)),
    ("report annual years", Rule::Fixed(Get)),
    ("report annual generate", Rule::Fixed(Post)),
    ("report dual generate", Rule::Fixed(Post)),
    ("sns timeline", Rule::Fixed(Get)),
    ("sns users", Rule::Fixed(Get)),
    ("sns debug-resource", Rule::Fixed(Get)),
    ("sns stats", Rule::Fixed(Get)),
    ("sns post-counts", Rule::Fixed(Get)),
    ("sns export", Rule::Fixed(Post)),
    ("sns media", Rule::Fixed(Post)),
    ("sns download-emoji", Rule::Fixed(Post)),
    ("sns download-image", Rule::Fixed(Post)),
    ("biz accounts", Rule::Fixed(Get)),
    ("biz messages", Rule::Fixed(Get)),
    ("biz pay-records", Rule::Fixed(Get)),
    ("image decrypt", Rule::Fixed(Post)),
    ("image resolve-cache", Rule::Fixed(Get)),
    ("image resolve-batch", Rule::Fixed(Post)),
    ("image auto-download start", Rule::Fixed(Post)),
    ("image auto-download status", Rule::Fixed(Get)),
    ("video info", Rule::Fixed(Get)),
    ("video parse-md5", Rule::Fixed(Get)),
    ("insight test", Rule::Fixed(Post)),
    ("insight models", Rule::Fixed(Get)),
    ("insight trigger", Rule::Fixed(Post)),
    ("insight records", Rule::Fixed(Get)),
    ("insight get", Rule::Fixed(Get)),
    ("insight mark-read", Rule::Fixed(Put)),
    ("insight clear", Rule::Fixed(Delete)),
    ("insight today-stats", Rule::Fixed(Get)),
    ("insight scan", Rule::Fixed(Post)),
    ("insight footprint", Rule::Fixed(Get)),
    ("insight footprint-summary", Rule::Fixed(Post)),
    ("backup create", Rule::Fixed(Post)),
    ("backup inspect", Rule::Fixed(Get)),
    ("backup restore", Rule::Fixed(Post)),
    ("cache list", Rule::Fixed(Get)),
    ("cache clear", Rule::Fixed(Delete)),
    ("cache clear-account", Rule::Fixed(Delete)),
    ("web info", Rule::Fixed(Get)),
    ("web list", Rule::Fixed(Get)),
    ("web install", Rule::Fixed(Post)),
    ("web use", Rule::Fixed(Put)),
    ("web update", Rule::Fixed(Post)),
    ("web prune", Rule::Fixed(Delete)),
    ("lang", Rule::Fixed(Put)),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve_api::endpoints::Endpoints;
    use clap::{Command, CommandFactory};

    fn leaves(cmd: &Command, path: &mut Vec<String>, out: &mut Vec<String>) {
        let subs: Vec<&Command> = cmd
            .get_subcommands()
            .filter(|c| c.get_name() != "help")
            .collect();
        if subs.is_empty() {
            out.push(path.join(" "));
        }
        for sub in subs {
            path.push(sub.get_name().to_string());
            leaves(sub, path, out);
            path.pop();
        }
    }

    fn all_leaves() -> Vec<String> {
        let mut cmd = crate::cli::Cli::command();
        cmd.build();
        let mut out = Vec::new();
        leaves(&cmd, &mut Vec::new(), &mut out);
        out
    }

    #[test]
    fn every_command_has_a_method() {
        let leaves = all_leaves();
        let missing: Vec<&String> = leaves
            .iter()
            .filter(|l| !HIDDEN.contains(&l.as_str()) && !METHODS.iter().any(|(p, _)| p == l))
            .collect();
        assert!(
            missing.is_empty(),
            "commands without an HTTP method: {missing:?}"
        );
        let stale: Vec<&str> = METHODS
            .iter()
            .map(|(p, _)| *p)
            .filter(|p| !leaves.iter().any(|l| l == p))
            .collect();
        assert!(
            stale.is_empty(),
            "METHODS lists unknown commands: {stale:?}"
        );
        let mut seen = std::collections::HashSet::new();
        for (p, _) in METHODS {
            assert!(seen.insert(*p), "{p} is listed twice");
        }
    }

    #[test]
    fn own_state_names_real_commands() {
        let leaves = all_leaves();
        for command in OWN_STATE {
            assert!(
                leaves.iter().any(|l| l == command),
                "{command} is no command"
            );
        }
    }

    #[test]
    fn rules_name_real_arguments() {
        let endpoints = Endpoints::new();
        for (path, rule) in METHODS {
            let arg = match rule {
                Rule::Fixed(_) => continue,
                Rule::Arg { arg, .. } => arg,
            };
            let ep = endpoints.0.get(*path).unwrap();
            assert!(
                ep.params.iter().any(|p| p.name == *arg),
                "{path} has no argument {arg}"
            );
        }
    }
}
