//! `/api/<command path>` (issue #140): reads the WebUI asks for all the time, run in the server's own process on a
//! hub that stays open, so the databases are not opened again for every request. Parameters and answers are
//! those of `/<command path>`, from the same parsing and the same handler; only the language is the server's, as
//! it is set once per process.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use clap::{Command, FromArgMatches};
use serde_json::Value;
use weflow_core::error::{AppError, AppResult};
use weflow_core::services::ServiceHub;

/// The commands with an `/api/` route; `tests::the_list_matches_the_cli` keeps it in step with
/// `runs_in_process`.
pub(super) const COMMANDS: &[&str] = &[
    "chat sessions",
    "chat messages",
    "chat latest",
    "chat message",
    "chat contact",
    "chat contacts",
    "chat counts",
    "chat search",
    "group members",
];

/// When a file was last written, as far as telling a change goes.
type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &std::path::Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

struct Open {
    hub: ServiceHub,
    config: PathBuf,
    stamp: Stamp,
}

/// The hub of the in-process routes, built again when the config file changes (`PUT /config/set`, another
/// profile…). Its databases stay open across hubs: `weflow-native` keeps them per path.
#[derive(Default)]
pub(super) struct Hub {
    open: Mutex<Option<Open>>,
}

impl Hub {
    fn get(&self) -> AppResult<ServiceHub> {
        let mut open = self.open.lock().unwrap();
        let before = match open.as_ref() {
            Some(o) => {
                let now = stamp(&o.config);
                if now == o.stamp {
                    return Ok(o.hub.clone());
                }
                now
            }
            None => None,
        };
        let (hub, config) = crate::open_hub()?;
        // a change while the config was read is caught by the next request
        let stamp = if open.is_some() {
            before
        } else {
            stamp(&config)
        };
        *open = Some(Open {
            hub: hub.clone(),
            config,
            stamp,
        });
        Ok(hub)
    }
}

fn command_tree() -> Command {
    static TREE: OnceLock<Command> = OnceLock::new();
    TREE.get_or_init(crate::i18n::build_command::<crate::Cli>)
        .clone()
}

/// The `--json` document and exit code of `weflow --json <argv>`, run in this process; `None` when the command
/// line does not parse, so the request goes to a child process, which reports that as the terminal does.
pub(super) async fn run(hub: &Hub, argv: &[String]) -> Option<(Value, u8)> {
    let line = ["weflow", "--json"]
        .into_iter()
        .map(String::from)
        .chain(argv.iter().cloned());
    let cli = command_tree()
        .try_get_matches_from(line)
        .ok()
        .and_then(|m| crate::Cli::from_arg_matches(&m).ok())
        .filter(|cli| crate::runs_in_process(&cli.command))?;
    let hub = match hub.get() {
        Ok(hub) => hub,
        Err(err) => return Some(crate::json_document(Err(err))),
    };
    let handle = tokio::runtime::Handle::current();
    let done = tokio::task::spawn_blocking(move || {
        crate::json_document(handle.block_on(crate::run_in_process(&cli, &hub)))
    })
    .await;
    Some(done.unwrap_or_else(|err| {
        crate::json_document(Err(AppError::runtime(format!("the command failed: {err}"))))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_matches_the_cli() {
        let endpoints = super::super::Endpoints::new();
        let mut root = <crate::Cli as clap::CommandFactory>::command();
        root.build();
        for name in COMMANDS {
            let ep = endpoints
                .0
                .get(*name)
                .unwrap_or_else(|| panic!("{name} is not a command"));
            assert_eq!(
                ep.verbs(),
                [super::super::Verb::Get],
                "{name} is not a read"
            );
            // the command with every required argument filled in
            let mut sub = &root;
            for p in &ep.path {
                sub = sub.find_subcommand(p).unwrap();
            }
            let mut argv: Vec<String> = ep.path.clone();
            let mut positionals = Vec::new();
            for a in sub.get_arguments().filter(|a| a.is_required_set()) {
                match a.get_long() {
                    Some(long) => argv.push(format!("--{long}=1")),
                    None => positionals.push("1".to_string()),
                }
            }
            if !positionals.is_empty() {
                argv.push("--".into());
                argv.extend(positionals);
            }
            let cli = command_tree()
                .try_get_matches_from(
                    ["weflow", "--json"]
                        .into_iter()
                        .map(String::from)
                        .chain(argv),
                )
                .ok()
                .and_then(|m| crate::Cli::from_arg_matches(&m).ok())
                .unwrap_or_else(|| panic!("{name} does not parse"));
            assert!(
                crate::runs_in_process(&cli.command),
                "{name} does not run in process"
            );
        }
    }
}
