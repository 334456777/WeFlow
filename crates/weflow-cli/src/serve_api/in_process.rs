//! `/api/<command path>` (issue #140): reads the WebUI asks for all the time, run in the server's own process on a
//! hub that stays open, so the databases are not opened again for every request. Parameters and answers are
//! those of `/<command path>`: the request is read into the same typed read the command runs (`weflow_core::reads`,
//! #397), without a command line; only the language is the server's, as it is set once per process.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde_json::Value;
use weflow_core::error::{AppError, AppResult};
use weflow_core::reads::{image_variant, MediaRead, MessagePage, Read};
use weflow_core::services::ServiceHub;

use super::{find_param, flag_value, Endpoint, Kind, Params};

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
    "chat statuses",
    "chat tab-counts",
    "group list",
    "group ranking",
    "group hours",
    "group media",
    "group member",
    "group member-messages",
    "analytics overall",
    "analytics rankings",
    "analytics time",
    "analytics excluded",
    "analytics exclude-candidates",
    "biz accounts",
    "biz messages",
    "biz pay-records",
    "insight footprint",
    "insight get",
    "report annual years",
    "cache list",
    "config get",
    "config profiles",
    "runtime info",
    "db detect",
    "db test",
    "db wxid",
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
    (
        "analytics excluded",
        "`set`, which saves the list, runs as the route without `/api/`.",
        "`set` 会保存名单，按不带 `/api/` 的路由执行。",
    ),
];

/// How long two writes of a file can leave it the same time: a tick of the coarsest clock of a common file system
/// (2 s on FAT), and a margin.
const COARSE_TICK: Duration = Duration::from_secs(3);

/// What tells a change of the config file (#402): its time and size, and, while that time may still be shared by a
/// later write, its content. Two writes within one tick of a coarse clock leave the same time, and a switch between
/// two profiles whose names are as long the same size; once a tick has gone by, a write changes the time.
struct Stamp {
    modified: SystemTime,
    len: u64,
    content: u64,
    /// The content was found the same after a tick had gone by: the time and size tell from now on.
    settled: bool,
}

fn digest(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Whether `modified` is older than a tick.
fn past_a_tick(modified: SystemTime) -> bool {
    SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age >= COARSE_TICK)
}

impl Stamp {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta.modified().ok()?;
        let content = digest(&std::fs::read(path).ok()?);
        Some(Self {
            modified,
            len: meta.len(),
            content,
            settled: past_a_tick(modified),
        })
    }

    /// Whether the file at `path` is still the one stamped. Its content is read only until it has settled.
    fn holds(&mut self, path: &Path) -> bool {
        let Some(meta) = std::fs::metadata(path).ok() else {
            return false;
        };
        if meta.modified().ok() != Some(self.modified) || meta.len() != self.len {
            return false;
        }
        if self.settled {
            return true;
        }
        // the time is read before the content: a tick gone by then has gone by for what is read after
        let past = past_a_tick(self.modified);
        let same = std::fs::read(path).is_ok_and(|bytes| digest(&bytes) == self.content);
        self.settled = same && past;
        same
    }
}

struct Open {
    hub: ServiceHub,
    config: PathBuf,
    stamp: Option<Stamp>,
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
    /// Builds the hub in the background, which opens the databases of a chat page (see [`hub_and_generation`]), so
    /// the first request finds them ready.
    ///
    /// [`hub_and_generation`]: Self::hub_and_generation
    pub(super) fn warm(self: &std::sync::Arc<Self>) {
        let hub = self.clone();
        std::thread::spawn(move || hub.get());
    }

    fn get(&self) -> AppResult<ServiceHub> {
        self.hub_and_generation().map(|(hub, _)| hub)
    }

    /// The hub, and which one it is: the number changes when the hub is built again.
    pub(super) fn hub_and_generation(&self) -> AppResult<(ServiceHub, u64)> {
        let mut open = self.open.lock().expect("lock poisoned");
        // another config file chosen (`config set config_path`) is a change too
        let in_use = weflow_core::config::default_config_path();
        let before = match open.as_mut() {
            Some(o) => {
                let holds = in_use.as_ref() == Some(&o.config)
                    && match o.stamp.as_mut() {
                        Some(stamp) => stamp.holds(&o.config),
                        None => std::fs::metadata(&o.config).is_err(),
                    };
                if holds {
                    return Ok((o.hub.clone(), o.generation));
                }
                in_use.as_deref().and_then(Stamp::of)
            }
            None => None,
        };
        let (hub, config) = crate::open_hub()?;
        // a change while the config was read is caught by the next request
        let stamp = if open.is_some() && in_use.as_ref() == Some(&config) {
            before
        } else {
            Stamp::of(&config)
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

/// A request's parameters by the argument of the command they set, as the texts its command line would carry
/// ([`Endpoint::argv`] writes the same ones): read into a [`Read`] or a [`MediaRead`] here, without writing a command
/// line or parsing one (#397). They take only what the command line takes, and read it as clap does; anything else is
/// [`Refused`]: the server then parses its command line with clap and answers clap's error, as the terminal prints
/// it (#432), or, when clap takes it, sends the request to a child process. The parameters have passed
/// [`Endpoint::check`].
pub(super) struct Given {
    /// The texts of each argument by its name; a switch has one per time it is set.
    texts: BTreeMap<String, Vec<String>>,
}

/// A request the typed reads do not take: one the command line refuses, or a part of a command that does not run in
/// process (`chat detail` without `fast`, `sns timeline` with its media).
#[derive(Debug)]
pub(super) struct Refused;

type Taken<T> = Result<T, Refused>;

impl Given {
    pub(super) fn new(endpoint: &Endpoint, params: &Params) -> Self {
        let mut texts: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (key, values) in params {
            let Some(param) = find_param(endpoint, key) else {
                continue;
            };
            let all = texts.entry(param.name.clone()).or_default();
            if param.kind == Kind::Flag {
                let set = values.iter().filter(|v| flag_value(v) == Some(true));
                all.extend(set.map(|_| "true".to_string()));
            } else {
                all.extend(super::texts(param, values));
            }
        }
        Self { texts }
    }

    fn all(&self, name: &str) -> &[String] {
        self.texts.get(name).map_or(&[], Vec::as_slice)
    }

    fn given(&self, name: &str) -> bool {
        !self.all(name).is_empty()
    }

    /// An argument of one value: absent, or given once (clap refuses it twice).
    fn one(&self, name: &str) -> Taken<Option<&str>> {
        match self.all(name) {
            [] => Ok(None),
            [value] => Ok(Some(value)),
            _ => Err(Refused),
        }
    }

    fn text(&self, name: &str) -> Taken<Option<String>> {
        Ok(self.one(name)?.map(str::to_string))
    }

    fn required(&self, name: &str) -> Taken<String> {
        self.text(name)?.ok_or(Refused)
    }

    /// A number as clap reads it: Rust's own parsing, so `+3` is 3 and an empty value none.
    fn number<T: std::str::FromStr>(&self, name: &str) -> Taken<Option<T>> {
        self.one(name)?
            .map(|text| text.parse().map_err(|_| Refused))
            .transpose()
    }

    fn number_or<T: std::str::FromStr>(&self, name: &str, default: T) -> Taken<T> {
        Ok(self.number(name)?.unwrap_or(default))
    }

    /// A switch, set at most once (clap refuses it twice).
    fn switch(&self, name: &str) -> Taken<bool> {
        match self.all(name).len() {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Refused),
        }
    }

    fn list(&self, name: &str) -> Vec<String> {
        self.all(name).to_vec()
    }

    /// `name`, when given, comes without any of `others` (clap's `conflicts_with`).
    fn without(&self, name: &str, others: &[&str]) -> Taken<()> {
        if self.given(name) && others.iter().any(|o| self.given(o)) {
            return Err(Refused);
        }
        Ok(())
    }

    /// The [`Read`] of a command of [`COMMANDS`], with the defaults of its arguments.
    pub(super) fn read(&self, command: &str) -> Taken<Read> {
        Ok(match command {
            "chat sessions" => Read::ChatSessions {
                limit: self.number_or("limit", 0)?,
            },
            "chat messages" => {
                self.without("before", &["offset", "after"])?;
                self.without("after", &["offset"])?;
                self.without("since", &["offset", "before", "after"])?;
                Read::ChatMessages(MessagePage {
                    session_id: self.required("session-id")?,
                    limit: self.number_or("limit", 50)?,
                    offset: self.number_or("offset", 0)?,
                    before: self.text("before")?,
                    after: self.text("after")?,
                    since: self.number("since")?,
                    parsed: self.switch("parsed")?,
                })
            }
            "chat latest" => Read::ChatLatest {
                session_id: self.required("session-id")?,
                limit: self.number_or("limit", 20)?,
            },
            "chat message" => {
                if self.given("create-time") && !self.given("local-id") {
                    return Err(Refused);
                }
                Read::ChatMessage {
                    session_id: self.required("session-id")?,
                    local_id: self.number("local-id")?,
                    server_id: self.text("server-id")?,
                    create_time: self.number("create-time")?,
                    parsed: self.switch("parsed")?,
                }
            }
            "chat contact" => Read::ChatContact {
                username: self.required("username")?,
            },
            "chat contacts" => Read::ChatContacts {
                parsed: self.switch("parsed")?,
            },
            "chat counts" => Read::ChatCounts {
                sessions: self.list("sessions"),
            },
            "chat search" => Read::ChatSearch {
                keyword: self.required("keyword")?,
                session_id: self.text("session-id")?,
                limit: self.number_or("limit", 50)?,
                offset: self.number_or("offset", 0)?,
                start: self.number_or("start", 0)?,
                end: self.number_or("end", 0)?,
                parsed: self.switch("parsed")?,
            },
            "chat dates" => Read::ChatDates {
                session_id: self.required("session-id")?,
            },
            "chat date-counts" => Read::ChatDateCounts {
                session_id: self.required("session-id")?,
            },
            "chat detail" => {
                // the rest of the details is slower to read: it runs in a child process
                if !self.switch("fast")? || self.switch("extra")? {
                    return Err(Refused);
                }
                Read::ChatDetailFast {
                    session_id: self.required("session-id")?,
                }
            }
            "chat resources" => Read::ChatResources {
                session: self.text("session")?,
                types: self.list("type"),
                start: self.text("start")?,
                end: self.text("end")?,
                limit: self.number_or("limit", 300)?,
                before: self.text("before")?,
            },
            "chat image-info" => Read::ChatImageInfo {
                session_id: self.required("session-id")?,
                msg_id: self.required("msg-id")?,
                create_time: self.number("create-time")?,
            },
            "group members" => Read::GroupMembers {
                chatroom_id: self.required("chatroom-id")?,
                counts: self.switch("counts")?,
                refresh: self.switch("refresh")?,
            },
            "sns timeline" => {
                // the media are downloaded: in a child process
                if self.switch("with-media")? {
                    return Err(Refused);
                }
                Read::SnsTimeline {
                    limit: self.number_or("limit", 20)?,
                    offset: self.number_or("offset", 0)?,
                    users: self.list("user"),
                    keyword: self.text("keyword")?,
                    start: self.number_or("start", 0)?,
                    end: self.number_or("end", 0)?,
                }
            }
            "sns users" => Read::SnsUsers,
            "sns stats" => Read::SnsStats {
                fast: self.switch("fast")?,
            },
            "sns post-counts" => Read::SnsPostCounts {
                user: self.text("user")?,
                prefer_cache: self.switch("prefer-cache")?,
            },
            "chat statuses" => Read::ChatStatuses {
                usernames: self.list("usernames"),
            },
            "chat tab-counts" => Read::ChatTabCounts,
            "group list" => Read::GroupList,
            "group ranking" => Read::GroupRanking {
                chatroom_id: self.required("chatroom-id")?,
                limit: self.number_or("limit", 20)?,
                start: self.text("start")?,
                end: self.text("end")?,
            },
            "group hours" => Read::GroupHours {
                chatroom_id: self.required("chatroom-id")?,
                start: self.text("start")?,
                end: self.text("end")?,
            },
            "group media" => Read::GroupMedia {
                chatroom_id: self.required("chatroom-id")?,
                start: self.text("start")?,
                end: self.text("end")?,
            },
            "group member" => Read::GroupMember {
                chatroom_id: self.required("chatroom-id")?,
                username: self.required("username")?,
                start: self.text("start")?,
                end: self.text("end")?,
            },
            "group member-messages" => Read::GroupMemberMessages {
                chatroom_id: self.required("chatroom-id")?,
                username: self.required("username")?,
                limit: self.number_or("limit", 50)?,
                cursor: self.number_or("cursor", 0)?,
                start: self.text("start")?,
                end: self.text("end")?,
            },
            "analytics overall" => Read::AnalyticsOverall {
                force: self.switch("force")?,
            },
            "analytics rankings" => Read::AnalyticsRankings {
                limit: self.number_or("limit", 20)?,
                start: self.text("start")?,
                end: self.text("end")?,
            },
            "analytics time" => Read::AnalyticsTime,
            "analytics excluded" => {
                // the list is saved: in a child process
                if self.given("set") {
                    return Err(Refused);
                }
                Read::AnalyticsExcluded
            }
            "analytics exclude-candidates" => Read::AnalyticsExcludeCandidates,
            "biz accounts" => Read::BizAccounts,
            "biz messages" => Read::BizMessages {
                username: self.required("username")?,
                limit: self.number_or("limit", 50)?,
                offset: self.number_or("offset", 0)?,
            },
            "biz pay-records" => Read::BizPayRecords {
                limit: self.number_or("limit", 50)?,
                offset: self.number_or("offset", 0)?,
            },
            "insight footprint" => Read::InsightFootprint {
                start: self.text("start")?,
                end: self.text("end")?,
            },
            "insight get" => Read::InsightGet {
                id: self.required("id")?,
            },
            "report annual years" => Read::ReportAnnualYears,
            "cache list" => Read::CacheList,
            "config get" => Read::ConfigGet {
                key: self.text("key")?,
            },
            "config profiles" => Read::ConfigProfiles,
            "runtime info" => Read::RuntimeInfo,
            "db detect" => Read::DbDetect,
            "db test" => Read::DbTest,
            "db wxid" => Read::DbWxid {
                root: self.text("root")?,
            },
            _ => return Err(Refused),
        })
    }

    /// The file a command of [`MEDIA`] asks for; its `out` is not the request's (the route refuses it).
    pub(super) fn media(&self, command: &str) -> Taken<Media> {
        Ok(match command {
            "chat image-data" => {
                const VARIANTS: &[&str] = &["best", "thumb", "normal", "hd"];
                self.without("thumb", &["variant"])?;
                let variant = self.text("variant")?.unwrap_or_else(|| "best".into());
                if !VARIANTS.contains(&variant.as_str()) {
                    return Err(Refused);
                }
                Media::Chat(MediaRead::ChatImage {
                    session_id: self.required("session-id")?,
                    msg_id: self.required("msg-id")?,
                    create_time: self.number("create-time")?,
                    variant: image_variant(if self.switch("thumb")? {
                        "thumb"
                    } else {
                        &variant
                    }),
                })
            }
            "chat voice-data" => Media::Chat(MediaRead::ChatVoice {
                session_id: self.required("session-id")?,
                msg_id: self.required("msg-id")?,
                create_time: self.number("create-time")?,
                server_id: self.text("server-id")?,
                sender: self.text("sender")?,
            }),
            "chat video-data" => Media::Chat(MediaRead::ChatVideo {
                session_id: self.required("session-id")?,
                msg_id: self.required("msg-id")?,
                create_time: self.number("create-time")?,
                thumb: self.switch("thumb")?,
            }),
            "sns media" => Media::Sns {
                url: self.required("url")?,
                key: self.text("key")?,
            },
            _ => return Err(Refused),
        })
    }
}

/// The file an `/api/` media route answers: a message's, or a Moments post's.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Media {
    Chat(MediaRead),
    Sns { url: String, key: Option<String> },
}

/// The places of an `/api/` request in its [`Line`](super::Line): admitted, and running.
pub(super) type Place = (
    tokio::sync::OwnedSemaphorePermit,
    tokio::sync::OwnedSemaphorePermit,
);

/// Runs `work` holding `place` until it ends, also when the request goes away first (#396, risk 2): a read cannot be
/// stopped halfway, so its place stays taken while it runs, and `--max-reads` counts every read still running.
pub(super) async fn held<T: Send + 'static>(
    place: Place,
    work: impl std::future::Future<Output = T> + Send + 'static,
) -> Result<T, tokio::task::JoinError> {
    tokio::spawn(async move {
        let _place = place;
        work.await
    })
    .await
}

/// The `--json` document and exit code of a [`Read`] run in this process, as a child process would print them.
pub(super) async fn run(hub: &Hub, read: Read) -> (Value, u8) {
    let hub = match hub.get() {
        Ok(hub) => hub,
        Err(err) => return crate::json_document(Err(err)),
    };
    let done = tokio::task::spawn_blocking(move || crate::json_document(read.run(&hub))).await;
    done.unwrap_or_else(|err| {
        crate::json_document(Err(AppError::runtime(format!("the command failed: {err}"))))
    })
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

/// The file of a [`MEDIA`] command, read in this process: the decrypted image, the decoded WAV or the video, or the
/// `--json` failure document and exit code.
pub(super) async fn media(hub: &Hub, media: Media) -> Result<MediaFile, (Value, u8)> {
    let outcome = match media {
        Media::Chat(read) => match hub.get() {
            Ok(hub) => read.fetch(hub).await,
            Err(err) => Err(err),
        },
        Media::Sns { url, key } => sns_media(hub, &url, key.as_deref()).await,
    };
    outcome.map_err(|err| crate::json_document(Err(err)))
}

/// `sns media` for a page (issue #202): an image or a video of a Moments post, fetched from WeChat's servers,
/// decrypted with `key` and kept in the cache, so it is fetched once. Only addresses of WeChat's media servers are
/// taken (see [`weflow_core::sns::is_wechat_media_url`]). A video is answered from its cached file, so a player can
/// ask for a part of it.
async fn sns_media(hub: &Hub, url: &str, key: Option<&str>) -> AppResult<MediaFile> {
    if !weflow_core::sns::is_wechat_media_url(url) {
        return Err(AppError::usage(
            "url must be an https address of WeChat's Moments media servers",
        ));
    }
    let fetched = hub.get()?.sns_fetch_media(url, key).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Command, FromArgMatches};

    #[test]
    fn a_config_written_twice_within_a_tick_of_a_coarse_clock_is_told_changed() {
        let dir = std::env::temp_dir().join(format!("weflow-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"profile":"aaaa"}"#).unwrap();
        let time = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut stamp = Stamp::of(&path).unwrap();
        assert!(stamp.holds(&path));
        // another profile of a name as long, written within the same tick: the same time and size
        std::fs::write(&path, r#"{"profile":"bbbb"}"#).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(time)
            .unwrap();
        assert!(!stamp.holds(&path));

        // written long ago and stamped since: the time and size tell, the content is not read again
        let old = SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let mut stamp = Stamp::of(&path).unwrap();
        assert!(stamp.settled && stamp.holds(&path));
        std::fs::write(&path, r#"{"profile":"cccc"}"#).unwrap();
        assert!(!stamp.holds(&path));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn command_tree() -> Command {
        static TREE: std::sync::OnceLock<Command> = std::sync::OnceLock::new();
        TREE.get_or_init(crate::i18n::build_command::<crate::Cli>)
            .clone()
    }

    /// The command line parsed by clap, as the child process does.
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
            // a command that also writes (`analytics excluded --set`) runs in process only in part
            let verbs = ep.verbs();
            assert!(
                verbs == [super::super::Verb::Get]
                    || (verbs[0] == super::super::Verb::Get
                        && IN_PART.iter().any(|(n, ..)| n == name)),
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
                crate::read_of(cli.command).is_ok(),
                "{name} does not run in process"
            );
        }
    }

    /// What an `/api/` read spent on its command line before it ran (#403): writing it from the parameters,
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
                let _ = std::hint::black_box(crate::read_of(cli.command));
            }
            let all = begin.elapsed() / RUNS;
            // what `/api/` does instead since #397
            let begin = std::time::Instant::now();
            for _ in 0..RUNS {
                let given = Given::new(ep, std::hint::black_box(&params));
                std::hint::black_box(given.read("chat messages").unwrap());
            }
            let typed = begin.elapsed() / RUNS;
            eprintln!(
                "round {round}: clone of the command tree {clone:?}, command line + clone + parse {all:?}, \
                 typed read {typed:?} (mean of {RUNS})"
            );
        }
    }

    /// What a request asks for, as read by clap (the child process's way) or by the typed reads.
    #[derive(Debug, PartialEq)]
    enum Asked {
        Read(Read),
        Media(Media),
    }

    fn by_clap(ep: &Endpoint, params: &Params) -> Option<Asked> {
        let name = ep.command_name();
        let media = MEDIA.contains(&name.as_str());
        let mut params = params.clone();
        if media {
            params.insert("out".into(), vec![Value::String("-".into())]);
        }
        let cli = parse(&ep.argv(&params).ok()?)?;
        if !media {
            return crate::read_of(cli.command).ok().map(Asked::Read);
        }
        match cli.command {
            crate::Commands::Chat(chat) => crate::media_of(chat.command)
                .ok()
                .map(|(m, _)| Asked::Media(Media::Chat(m))),
            crate::Commands::Sns(sns) => match sns.command {
                crate::SnsSubcommand::Media { url, key, .. } => {
                    Some(Asked::Media(Media::Sns { url, key }))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn by_types(ep: &Endpoint, params: &Params) -> Option<Asked> {
        ep.check(params).ok()?;
        let given = Given::new(ep, params);
        let name = ep.command_name();
        if MEDIA.contains(&name.as_str()) {
            given.media(&name).ok().map(Asked::Media)
        } else {
            given.read(&name).ok().map(Asked::Read)
        }
    }

    /// Requests made up from a fixed seed: each parameter of the command absent, given once or twice as text, or as
    /// a JSON value, under its name or with `_` for `-`.
    fn made_up(ep: &Endpoint, runs: usize, mut each: impl FnMut(&Params)) {
        const TEXTS: &[&str] = &[
            "1",
            "0",
            "-1",
            "+2",
            "abc",
            "",
            "true",
            "false",
            "best",
            "thumb",
            "hd",
            "2026-01-02",
            "1:2:3",
            "9999999999",
            "wxid_bob",
        ];
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d ^ ep.command_name().len() as u64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for _ in 0..runs {
            let mut params = Params::new();
            for p in ep.params.iter().filter(|p| p.name != "out") {
                let text = |next: &mut dyn FnMut(usize) -> usize| {
                    Value::String(TEXTS[next(TEXTS.len())].to_string())
                };
                let values = match next(6) {
                    0 | 1 => continue,
                    2 | 3 => vec![text(&mut next)],
                    4 => vec![text(&mut next), text(&mut next)],
                    _ => vec![match next(3) {
                        0 => Value::Bool(true),
                        1 => Value::from(3),
                        _ => Value::Array(vec![text(&mut next), text(&mut next)]),
                    }],
                };
                let key = if next(4) == 0 {
                    p.name.replace('-', "_")
                } else {
                    p.name.clone()
                };
                params.entry(key).or_default().extend(values);
            }
            each(&params);
        }
    }

    /// The typed reads of `/api/` take nothing the command line refuses, and read what they take as clap does: what
    /// they do not take goes to a child process, whose answer is the terminal's. The only requests clap takes and they
    /// do not give one positional argument twice and another none, which clap fills in by their order.
    #[test]
    fn typed_reads_take_what_clap_takes_and_read_it_the_same() {
        let endpoints = super::super::Endpoints::new();
        for name in COMMANDS.iter().chain(MEDIA) {
            let ep = endpoints.0.get(*name).unwrap();
            let mut taken = 0;
            made_up(ep, 1000, |params| {
                let typed = by_types(ep, params);
                let clap = by_clap(ep, params);
                match (&typed, &clap) {
                    (Some(asked), _) => {
                        assert_eq!(Some(asked), clap.as_ref(), "{name}: {params:?}");
                        taken += 1;
                    }
                    (None, Some(_)) => {
                        let given = Given::new(ep, params);
                        assert!(
                            ep.params
                                .iter()
                                .filter(|p| p.index.is_some() && p.kind == Kind::Single)
                                .any(|p| given.all(&p.name).len() > 1),
                            "{name}: only clap takes {params:?}"
                        );
                    }
                    (None, None) => {}
                }
            });
            assert!(taken > 0, "{name}: no request was taken");
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
