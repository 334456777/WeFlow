//! Which HTTP method each command takes, and the commands that are refused.

use axum::http::Method;

use Verb::*;

/// Commands that are not reachable over HTTP.
pub(super) const HIDDEN: &[&str] = &["serve"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verb {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Verb {
    pub(super) fn method(self) -> Method {
        match self {
            Get => Method::GET,
            Post => Method::POST,
            Put => Method::PUT,
            Patch => Method::PATCH,
            Delete => Method::DELETE,
        }
    }
}

/// How the HTTP method of a command is chosen.
#[derive(Clone, Copy, Debug)]
pub(super) enum Rule {
    Fixed(Verb),
    /// `with` when the argument is given, `without` otherwise (`analytics excluded --set`).
    Arg {
        arg: &'static str,
        without: Verb,
        with: Verb,
    },
    /// Chosen by the value of an argument (`sns block-delete <action>`).
    Value {
        arg: &'static str,
        values: &'static [(&'static str, Verb)],
    },
}

/// The commands the read-only database layer always refuses (`unsupported`, HTTP 501), with the methods that run
/// them so: those that would write WeChat's databases, and `sns block-delete check`, which looks for a trigger the
/// layer does not read. `/openapi.json` marks them `x-weflow-refused` (#217) so a client need not offer them, and
/// gives them no `200` answer (#430). `refused_commands_are_marked_and_refused` (tests/serve_api.rs) checks both
/// against a running server.
const REFUSED: &[(&str, &[Verb])] = &[
    ("chat update-message", &[Patch]),
    ("chat delete-message", &[Delete]),
    ("chat mark-read", &[Post]),
    // looking for the trigger is refused like installing it (#644)
    ("chat anti-revoke check", &[Get]),
    ("chat anti-revoke install", &[Put]),
    ("chat anti-revoke uninstall", &[Delete]),
    ("sns block-delete", &[Get, Put, Delete]),
    ("sns delete", &[Delete]),
];

/// Whether `name` run with `verb` is one of the [`REFUSED`].
pub(super) fn refused(name: &str, verb: Verb) -> bool {
    REFUSED
        .iter()
        .any(|(n, verbs)| *n == name && verbs.contains(&verb))
}

/// The HTTP method of every command: `GET` reads (it may fill caches that can be rebuilt), `POST` runs a task or
/// writes files, `PUT` changes a setting, `DELETE` removes one. Commands that would write WeChat's databases keep
/// the method their action implies and are refused by the command itself (`unsupported`, HTTP 501).
/// `tests::every_command_has_a_method` keeps this table in step with the command tree.
pub(super) const METHODS: &[(&str, Rule)] = &[
    ("config path", Rule::Fixed(Get)),
    ("config list", Rule::Fixed(Get)),
    ("config profiles", Rule::Fixed(Get)),
    ("config get", Rule::Fixed(Get)),
    ("config set", Rule::Fixed(Put)),
    ("config unset", Rule::Fixed(Delete)),
    ("config clear", Rule::Fixed(Delete)),
    ("db detect", Rule::Fixed(Get)),
    ("db scan", Rule::Fixed(Get)),
    ("db test", Rule::Fixed(Get)),
    ("db open", Rule::Fixed(Get)),
    ("db wxid", Rule::Fixed(Get)),
    ("key db", Rule::Fixed(Post)),
    ("key image", Rule::Fixed(Post)),
    ("key scan-image", Rule::Fixed(Post)),
    ("chat sessions", Rule::Fixed(Get)),
    ("chat messages", Rule::Fixed(Get)),
    ("chat latest", Rule::Fixed(Get)),
    ("chat search", Rule::Fixed(Get)),
    ("chat contacts", Rule::Fixed(Get)),
    ("chat contact", Rule::Fixed(Get)),
    ("chat update-message", Rule::Fixed(Patch)),
    ("chat delete-message", Rule::Fixed(Delete)),
    ("chat anti-revoke sessions", Rule::Fixed(Get)),
    ("chat anti-revoke check", Rule::Fixed(Get)),
    ("chat anti-revoke install", Rule::Fixed(Put)),
    ("chat anti-revoke uninstall", Rule::Fixed(Delete)),
    ("chat message", Rule::Fixed(Get)),
    ("chat dates", Rule::Fixed(Get)),
    ("chat date-counts", Rule::Fixed(Get)),
    ("chat counts", Rule::Fixed(Get)),
    ("chat statuses", Rule::Fixed(Get)),
    ("chat detail", Rule::Fixed(Get)),
    ("chat mark-read", Rule::Fixed(Post)),
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
    ("chat voice", Rule::Fixed(Post)),
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
    (
        "sns block-delete",
        Rule::Value {
            arg: "action",
            values: &[("check", Get), ("install", Put), ("uninstall", Delete)],
        },
    ),
    ("sns delete", Rule::Fixed(Delete)),
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
    ("insight trigger", Rule::Fixed(Post)),
    ("insight records", Rule::Fixed(Get)),
    ("insight get", Rule::Fixed(Get)),
    ("insight mark-read", Rule::Fixed(Put)),
    ("insight clear", Rule::Fixed(Delete)),
    ("insight today-stats", Rule::Fixed(Get)),
    ("insight scan", Rule::Fixed(Post)),
    ("insight footprint", Rule::Fixed(Get)),
    ("insight footprint-summary", Rule::Fixed(Post)),
    ("runtime info", Rule::Fixed(Get)),
    ("runtime manifest", Rule::Fixed(Get)),
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
    fn rules_name_real_arguments() {
        let endpoints = Endpoints::new();
        for (path, rule) in METHODS {
            let arg = match rule {
                Rule::Fixed(_) => continue,
                Rule::Arg { arg, .. } | Rule::Value { arg, .. } => arg,
            };
            let ep = endpoints.0.get(*path).unwrap();
            assert!(
                ep.params.iter().any(|p| p.name == *arg),
                "{path} has no argument {arg}"
            );
        }
    }
}
