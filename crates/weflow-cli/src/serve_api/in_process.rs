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
    "chat dates",
    "chat date-counts",
    "chat detail",
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

/// Commands whose `/api/` route is a `GET` that answers the file itself, for a page to show: the image, the voice or
/// the video of a message, or an image or video of a Moments post. The parameters are those of the command, but for
/// `out`.
pub(super) const MEDIA: &[&str] = &[
    "chat image-data",
    "chat voice-data",
    "chat video-data",
    "sns media",
];

/// A media file and its content type.
pub(super) struct MediaFile {
    pub body: MediaBody,
    pub mime: &'static str,
}

/// What a media file is made of: bytes decoded in memory, or a file on disk, sent as it is read (a video).
pub(super) enum MediaBody {
    Bytes(Vec<u8>),
    File(std::path::PathBuf),
}

/// The content type of an image, from its first bytes.
fn image_mime(bytes: &[u8]) -> &'static str {
    let ext = weflow_core::decrypt::detect_image_extension(bytes);
    mime_guess::from_ext(ext.trim_start_matches('.'))
        .first_raw()
        .filter(|m| m.starts_with("image/"))
        .unwrap_or("application/octet-stream")
}

/// The file a [`MEDIA`] command would write, read in this process: the decrypted image or the decoded WAV, or the
/// `--json` failure document and exit code. `None` when the command line does not parse.
pub(super) async fn media(hub: &Hub, argv: &[String]) -> Option<Result<MediaFile, (Value, u8)>> {
    use crate::{ChatSubcommand, Commands};
    let cli = parse(argv)?;
    let chat = match cli.command {
        Commands::Chat(chat) => chat,
        Commands::Sns(sns) => return Some(sns_media(hub, sns.command).await),
        _ => return None,
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
            thumb,
            ..
        } => if *thumb {
            hub.image_thumbnail_for_message(session_id, msg_id, *create_time)
        } else {
            hub.image_data_for_message(session_id, msg_id, *create_time)
        }
        .map(|bytes| MediaFile {
            mime: image_mime(&bytes),
            body: MediaBody::Bytes(bytes),
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
                body: MediaBody::Bytes(bytes),
                mime: "audio/wav",
            }),
        ChatSubcommand::VideoData {
            session_id,
            msg_id,
            create_time,
            thumb,
            ..
        } => hub
            .video_files_for_message(session_id, msg_id, *create_time)
            .and_then(|files| {
                let path = crate::video_file(&files, *thumb)?;
                if !*thumb {
                    return Ok(MediaFile {
                        body: MediaBody::File(path.to_path_buf()),
                        mime: "video/mp4",
                    });
                }
                // a poster is small: read, to tell its type
                let bytes = std::fs::read(path).map_err(|e| {
                    AppError::runtime(format!("failed to read {}: {e}", path.display()))
                })?;
                Ok(MediaFile {
                    mime: image_mime(&bytes),
                    body: MediaBody::Bytes(bytes),
                })
            }),
        _ => Err(AppError::runtime("this command has no media route")),
    })
    .await;
    let outcome =
        done.unwrap_or_else(|err| Err(AppError::runtime(format!("the command failed: {err}"))));
    Some(outcome.map_err(|err| crate::json_document(Err(err))))
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
                crate::runs_in_process(&cli.command),
                "{name} does not run in process"
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
