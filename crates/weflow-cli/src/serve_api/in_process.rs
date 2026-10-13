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

fn parse(argv: &[String]) -> Option<crate::Cli> {
    let line = ["weflow", "--json"]
        .into_iter()
        .map(String::from)
        .chain(argv.iter().cloned());
    command_tree()
        .try_get_matches_from(line)
        .ok()
        .and_then(|m| crate::Cli::from_arg_matches(&m).ok())
}

/// The `--json` document and exit code of `weflow --json <argv>`, run in this process; `None` when the command
/// line does not parse, so the request goes to a child process, which reports that as the terminal does.
pub(super) async fn run(hub: &Hub, argv: &[String]) -> Option<(Value, u8)> {
    let cli = parse(argv).filter(|cli| crate::runs_in_process(&cli.command))?;
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

/// Commands whose `/api/` route is a `GET` that answers the file itself, for a page to show: the image or the voice
/// of a message. The parameters are those of the command, but for `out`.
pub(super) const MEDIA: &[&str] = &["chat image-data", "chat voice-data"];

/// A media file and its content type.
pub(super) struct MediaFile {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

/// The file a [`MEDIA`] command would write, read in this process: the decrypted image or the decoded WAV, or the
/// `--json` failure document and exit code. `None` when the command line does not parse.
pub(super) async fn media(hub: &Hub, argv: &[String]) -> Option<Result<MediaFile, (Value, u8)>> {
    use crate::{ChatSubcommand, Commands};
    let cli = parse(argv)?;
    let Commands::Chat(chat) = cli.command else {
        return None;
    };
    let hub = match hub.get() {
        Ok(hub) => hub,
        Err(err) => return Some(Err(crate::json_document(Err(err)))),
    };
    let done = tokio::task::spawn_blocking(move || match &chat.command {
        ChatSubcommand::ImageData {
            session_id,
            msg_id,
            create_time,
            ..
        } => hub
            .image_data_for_message(session_id, msg_id, *create_time)
            .map(|bytes| {
                let ext = weflow_core::decrypt::detect_image_extension(&bytes);
                let mime = mime_guess::from_ext(ext.trim_start_matches('.'))
                    .first_raw()
                    .filter(|m| m.starts_with("image/"))
                    .unwrap_or("application/octet-stream");
                MediaFile { bytes, mime }
            }),
        ChatSubcommand::VoiceData {
            session_id,
            msg_id,
            create_time,
            server_id,
            sender,
            ..
        } => hub
            .voice_data(
                session_id,
                msg_id,
                *create_time,
                server_id.as_deref(),
                sender.as_deref(),
            )
            .map(|bytes| MediaFile {
                bytes,
                mime: "audio/wav",
            }),
        _ => Err(AppError::runtime("this command has no media route")),
    })
    .await;
    let outcome =
        done.unwrap_or_else(|err| Err(AppError::runtime(format!("the command failed: {err}"))));
    Some(outcome.map_err(|err| crate::json_document(Err(err))))
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

    #[test]
    fn media_commands_parse_with_the_placeholder_out() {
        use crate::{ChatSubcommand, Commands};
        let endpoints = super::super::Endpoints::new();
        for name in MEDIA {
            let ep = endpoints
                .0
                .get(*name)
                .unwrap_or_else(|| panic!("{name} is not a command"));
            assert!(
                ep.params.iter().any(|p| p.name == "out"),
                "{name} has no `out`"
            );
            let mut argv = ep.path.clone();
            argv.extend(["--out=-", "--", "wxid_bob", "3"].map(String::from));
            let cli = parse(&argv).unwrap_or_else(|| panic!("{name} does not parse"));
            let Commands::Chat(chat) = cli.command else {
                panic!("{name} is not a chat command")
            };
            assert!(
                matches!(
                    chat.command,
                    ChatSubcommand::ImageData { .. } | ChatSubcommand::VoiceData { .. }
                ),
                "{name} has no media route in `media`"
            );
        }
    }
}
