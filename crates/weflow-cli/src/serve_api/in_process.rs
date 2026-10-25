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
/// `read_of`.
pub(super) const COMMANDS: &[&str] = &[
    "chat sessions",
    "chat messages",
    "chat latest",
    "chat message",
    "chat contact",
    "chat contacts",
    "chat counts",
    "chat search",
    "chat dates",
    "chat date-counts",
    "chat detail",
    "chat resources",
    "chat image-info",
    "group members",
    "sns timeline",
    "sns users",
    "sns stats",
    "sns post-counts",
];

/// Commands of [`COMMANDS`] that run in process only in part, and which part (English, Chinese); the rest runs as
/// the route without `/api/` does, with the same answer.
pub(super) const IN_PART: &[(&str, &str, &str)] = &[
    (
        "chat detail",
        "Only `fast=true` runs in process.",
        "只有 `fast=true` 在服务进程内执行。",
    ),
    (
        "sns timeline",
        "`with-media=true`, which downloads the media, runs as the route without `/api/`.",
        "`with-media=true` 要下载媒体，按不带 `/api/` 的路由执行。",
    ),
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
    /// Counts the hubs built, so a user of [`Hub::get_counted`] tells a new hub from the one it had.
    generation: u64,
}

/// The hub of the in-process routes, built again when the config file changes (`PUT /config/set`, another
/// profile…). Its databases stay open across hubs: `weflow-native` keeps them per path.
#[derive(Default)]
pub(super) struct Hub {
    open: Mutex<Option<Open>>,
}

impl Hub {
    /// Builds the hub in the background, which opens the databases of a chat page (see [`get_counted`]), so the
    /// first request finds them ready.
    ///
    /// [`get_counted`]: Self::get_counted
    pub(super) fn warm(self: &std::sync::Arc<Self>) {
        let hub = self.clone();
        std::thread::spawn(move || hub.get());
    }

    fn get(&self) -> AppResult<ServiceHub> {
        self.get_counted().map(|(hub, _)| hub)
    }

    /// The hub, and which one it is: the number changes when the hub is built again.
    pub(super) fn get_counted(&self) -> AppResult<(ServiceHub, u64)> {
        let mut open = self.open.lock().unwrap();
        let before = match open.as_ref() {
            Some(o) => {
                let now = stamp(&o.config);
                if now == o.stamp {
                    return Ok((o.hub.clone(), o.generation));
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
        let generation = open.as_ref().map_or(0, |o| o.generation + 1);
        // the databases of a chat page, opened in the background: the first reads would wait for their keys
        let warm = hub.clone();
        std::thread::spawn(move || warm.warm_chat_databases());
        *open = Some(Open {
            hub: hub.clone(),
            config,
            stamp,
            generation,
        });
        Ok((hub, generation))
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

/// The `--json` document and exit code of `weflow --json <argv>`, run in this process as the [`Read`] it is; `None`
/// when the command line does not parse or is no read, so the request goes to a child process, which reports that as
/// the terminal does.
///
/// [`Read`]: weflow_core::reads::Read
pub(super) async fn run(hub: &Hub, argv: &[String]) -> Option<(Value, u8)> {
    let read = parse(argv).and_then(|cli| crate::read_of(&cli.command))?;
    let hub = match hub.get() {
        Ok(hub) => hub,
        Err(err) => return Some(crate::json_document(Err(err))),
    };
    let done = tokio::task::spawn_blocking(move || crate::json_document(read.run(&hub))).await;
    Some(done.unwrap_or_else(|err| {
        crate::json_document(Err(AppError::runtime(format!("the command failed: {err}"))))
    }))
}

/// Commands whose `/api/` route is a `GET` that answers the file itself, for a page to show: the image, the voice or
/// the video of a message, or an image or video of a Moments post. The parameters are those of the command, but for
/// `out`.
pub(super) const MEDIA: &[&str] = &[
    "chat image-data",
    "chat voice-data",
    "chat video-data",
    "sns media",
];

pub(super) use weflow_core::reads::{image_mime, MediaBody, MediaFile};

/// The file a [`MEDIA`] command would write, read in this process: the decrypted image, the decoded WAV or the video,
/// or the `--json` failure document and exit code. `None` when the command line does not parse.
pub(super) async fn media(hub: &Hub, argv: &[String]) -> Option<Result<MediaFile, (Value, u8)>> {
    use crate::Commands;
    let cli = parse(argv)?;
    let read = match cli.command {
        Commands::Chat(chat) => crate::media_of(&chat.command)?,
        Commands::Sns(sns) => return Some(sns_media(hub, sns.command).await),
        _ => return None,
    };
    let hub = match hub.get() {
        Ok(hub) => hub,
        Err(err) => return Some(Err(crate::json_document(Err(err)))),
    };
    Some(
        read.fetch(hub)
            .await
            .map_err(|err| crate::json_document(Err(err))),
    )
}

/// `sns media` for a page (issue #202): an image or a video of a Moments post, fetched from WeChat's servers,
/// decrypted with `key` and kept in the cache, so it is fetched once. Only addresses of WeChat's media servers are
/// taken (see [`weflow_core::sns::is_wechat_media_url`]). A video is answered from its cached file, so a player can
/// ask for a part of it.
async fn sns_media(hub: &Hub, command: crate::SnsSubcommand) -> Result<MediaFile, (Value, u8)> {
    let outcome = async {
        let crate::SnsSubcommand::Media { url, key, .. } = command else {
            return Err(AppError::runtime("this command has no media route"));
        };
        if !weflow_core::sns::is_wechat_media_url(&url) {
            return Err(AppError::usage(
                "url must be an https address of WeChat's Moments media servers",
            ));
        }
        let fetched = hub.get()?.sns_fetch_media(&url, key.as_deref()).await?;
        match (
            fetched.content_type.starts_with("video/"),
            fetched.cache_path,
            fetched.data,
        ) {
            (true, Some(path), _) => Ok(MediaFile {
                body: MediaBody::File(path),
                mime: "video/mp4",
            }),
            (false, _, Some(bytes)) => Ok(MediaFile {
                mime: image_mime(&bytes),
                body: MediaBody::Bytes(bytes),
            }),
            _ => Err(AppError::runtime("the media could not be read")),
        }
    }
    .await;
    outcome.map_err(|err| crate::json_document(Err(err)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Commands of [`COMMANDS`] that run in process only with these arguments; without them, `/api/` runs them as
    /// the route without `/api/` does.
    const ONLY_WITH: &[(&str, &str)] = &[("chat detail", "--fast")];

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
            argv.extend(
                ONLY_WITH
                    .iter()
                    .filter(|(n, _)| n == name)
                    .map(|(_, a)| a.to_string()),
            );
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
                crate::read_of(&cli.command).is_some(),
                "{name} does not run in process"
            );
        }
    }

    /// What an `/api/` read spends on its command line before it runs (#403): writing it from the parameters,
    /// cloning the command tree and parsing it. A measurement, not a check; run it in a release build with
    /// `cargo test --release -p weflow-cli --bin weflow clap_round_trip -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn clap_round_trip() {
        const RUNS: u32 = 2000;
        let endpoints = super::super::Endpoints::new();
        let ep = endpoints.0.get("chat messages").unwrap();
        let mut params = super::super::Params::new();
        for (k, v) in [
            ("session-id", "wxid_bob"),
            ("limit", "50"),
            ("before", "1:2:3"),
            ("parsed", "true"),
        ] {
            params.insert(k.into(), vec![Value::String(v.into())]);
        }
        assert!(parse(&ep.argv(&params).unwrap()).is_some());
        for round in 1..=3 {
            let begin = std::time::Instant::now();
            for _ in 0..RUNS {
                std::hint::black_box(command_tree());
            }
            let clone = begin.elapsed() / RUNS;
            let begin = std::time::Instant::now();
            for _ in 0..RUNS {
                let cli = parse(&ep.argv(std::hint::black_box(&params)).unwrap()).unwrap();
                std::hint::black_box(crate::read_of(&cli.command));
            }
            let all = begin.elapsed() / RUNS;
            eprintln!(
                "round {round}: clone of the command tree {clone:?}, command line + clone + parse {all:?} \
                 (mean of {RUNS})"
            );
        }
    }

    #[test]
    fn the_push_route_is_not_a_command() {
        // `/api/push/messages` is the event stream
        assert!(COMMANDS
            .iter()
            .chain(MEDIA)
            .all(|c| !c.starts_with("push ")));
    }

    #[test]
    fn media_commands_parse_with_the_placeholder_out() {
        use crate::{ChatSubcommand, Commands, SnsSubcommand};
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
            argv.extend(["--out=-", "--"].map(String::from));
            // a message's conversation and id, or a Moments address
            let positionals: &[&str] = if ep.path[0] == "sns" {
                &["https://a.qpic.cn/x"]
            } else {
                &["wxid_bob", "3"]
            };
            argv.extend(positionals.iter().map(|p| p.to_string()));
            let cli = parse(&argv).unwrap_or_else(|| panic!("{name} does not parse"));
            let handled = match cli.command {
                Commands::Chat(chat) => matches!(
                    chat.command,
                    ChatSubcommand::ImageData { .. }
                        | ChatSubcommand::VoiceData { .. }
                        | ChatSubcommand::VideoData { .. }
                ),
                Commands::Sns(sns) => matches!(sns.command, SnsSubcommand::Media { .. }),
                _ => false,
            };
            assert!(handled, "{name} has no media route in `media`");
        }
    }
}
