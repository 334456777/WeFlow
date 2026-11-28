//! `/api/<命令路径>`（#140）：网页经常请求的读取，在服务进程内用一直打开的 `Hub` 执行，
//! 不必为每个请求重新打开数据库。参数和回答与 `/<命令路径>` 相同：请求不经过命令行，
//! 直接读成命令运行的同一个类型化读取（`weflow_core::reads`，#397）；
//! 只有语言是服务端的，因为语言在每个进程中只设置一次。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde_json::Value;
use weflow_core::error::{AppError, AppResult};
use weflow_core::reads::{image_variant, MediaRead, MessagePage, Read};
use weflow_core::services::{ServiceHub, MEMBER_MESSAGES_LIMIT};
use weflow_core::sync::MutexExt;

use super::endpoints::{find_param, flag_value, Endpoint, Kind, Params};

/// 有 `/api/` 路由的命令；`tests::the_list_matches_the_cli` 保证它与 `read_of` 一致。
pub(super) const COMMANDS: &[&str] = &[
    "chat sessions",
    "chat messages",
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
    "db detect",
    "db test",
    "db wxid",
];

/// [`COMMANDS`] 中只有一部分在服务进程内执行的命令，以及是哪一部分（英文、中文）；
/// 其余部分与不带 `/api/` 的路由一样执行，回答相同。
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

/// 两次写入可能让文件的修改时间相同的最长间隔：常见文件系统中最粗的时钟的一个刻度（FAT 上是 2 秒），再加一点余量。
const COARSE_TICK: Duration = Duration::from_secs(3);

/// 判断配置文件是否变化的依据（#402）：修改时间和大小；之后的写入仍可能得到相同修改时间的期间，还要比较内容。
///
/// 时钟粗糙时，一个刻度内的两次写入修改时间相同，在两个名字等长的档案之间切换时大小也相同；
/// 过了一个刻度后，写入一定会改变修改时间。
struct Stamp {
    modified: SystemTime,
    len: u64,
    content: u64,
    /// 过了一个刻度后内容仍然相同：此后只看修改时间和大小。
    settled: bool,
}

fn digest(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// `modified` 是否已经过了一个刻度。
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

    /// `path` 处的文件是否仍是记下状态时的那个；内容只在稳定之前读取。
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
        // 先判断是否过了一个刻度再读内容：判断时已经过了，之后读到的内容也一定在一个刻度之后
        let past = past_a_tick(self.modified);
        let same = std::fs::read(path).is_ok_and(|bytes| digest(&bytes) == self.content);
        self.settled = same && past;
        same
    }
}

/// 已经建好的 `ServiceHub`，以及建它时用的配置文件和文件的状态。
struct Open {
    hub: ServiceHub,
    config: PathBuf,
    stamp: Option<Stamp>,
    /// 建过的 `ServiceHub` 的计数，[`Hub::hub_and_generation`] 的调用者据此区分新建的和它原来的那个。
    generation: u64,
}

/// 进程内路由的 `ServiceHub`，配置文件变化（`PUT /config/set`、换档案等）时重建。
/// 数据库在重建前后保持打开：`weflow-native` 按路径保留它们。
#[derive(Default)]
pub(super) struct Hub {
    open: Mutex<Option<Open>>,
}

impl Hub {
    /// 在后台建好 `ServiceHub`，它会打开聊天页面用到的数据库（见 [`hub_and_generation`]），
    /// 第一个请求到来时它们已经就绪。
    ///
    /// [`hub_and_generation`]: Self::hub_and_generation
    pub(super) fn warm(self: &std::sync::Arc<Self>) {
        let hub = self.clone();
        std::thread::spawn(move || hub.get());
    }

    fn get(&self) -> AppResult<ServiceHub> {
        self.hub_and_generation().map(|(hub, _)| hub)
    }

    /// `ServiceHub` 及它的编号：重建时编号改变。配置文件读不出来、建不起 `ServiceHub` 时为 `Err`。
    pub(super) fn hub_and_generation(&self) -> AppResult<(ServiceHub, u64)> {
        let mut open = self.open.lock_or_panic();
        // 换了配置文件（`config set config_path`）也算变化
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
        let (hub, config) = crate::reads::open_hub()?;
        // 读取配置期间发生的变化由下一个请求发现
        let stamp = if open.is_some() && in_use.as_ref() == Some(&config) {
            before
        } else {
            Stamp::of(&config)
        };
        let generation = open.as_ref().map_or(0, |o| o.generation + 1);
        // 在后台打开聊天页面用到的数据库：否则最初的几次读取要等待派生密钥
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

/// 请求的参数，按它们设置的命令参数分组，写成命令行上的文字（与 [`Endpoint::argv`] 写的相同）：
/// 在这里读成 [`Read`] 或 [`MediaRead`]，不写出也不解析命令行（#397）。
///
/// 只接受命令行接受的内容，按 clap 的方式读取；其他的都是 [`Refused`]：服务这时用 clap 解析命令行，
/// 回答 clap 的错误，与终端打印的相同（#432）；clap 接受时，把请求交给子进程。参数已经通过了 [`Endpoint::check`]。
pub(super) struct Given {
    /// 各个参数按名字列出的文字；开关每设置一次就有一项。
    texts: BTreeMap<String, Vec<String>>,
}

/// 类型化读取不接受的请求：命令行会拒绝的请求，
/// 或者命令中不在服务进程内执行的部分（不带 `fast` 的 `chat detail`、带媒体的 `sns timeline`）。
#[derive(Debug)]
pub(super) struct Refused;

type Taken<T> = Result<T, Refused>;

impl Given {
    /// 从 `params` 中取出 `endpoint` 的参数；开关只记下设为真的次数。
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
                all.extend(super::endpoints::texts(param, values));
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

    /// 只接受一个值的参数：没有给，或者只给了一次（给两次时 clap 会拒绝）。
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

    /// 按 clap 的方式读取的数字：用 Rust 自己的解析，所以 `+3` 是 3，空值不是数字，被拒绝。
    fn number<T: std::str::FromStr>(&self, name: &str) -> Taken<Option<T>> {
        self.one(name)?
            .map(|text| text.parse().map_err(|_| Refused))
            .transpose()
    }

    /// 计数或时间，读法与 `numbers` 让 clap 读取的相同：负数是 [`Refused`]，由 clap 说明原因。
    fn non_negative<T: std::str::FromStr + Default + PartialOrd>(
        &self,
        name: &str,
    ) -> Taken<Option<T>> {
        match self.number::<T>(name)? {
            Some(number) if number < T::default() => Err(Refused),
            number => Ok(number),
        }
    }

    fn non_negative_or<T: std::str::FromStr + Default + PartialOrd>(
        &self,
        name: &str,
        default: T,
    ) -> Taken<T> {
        Ok(self.non_negative(name)?.unwrap_or(default))
    }

    /// 计数，读法与 `numbers::usize_in` 让 clap 读取的相同：超出 `range` 时是 [`Refused`]，由 clap 说明原因。
    fn within_or(
        &self,
        name: &str,
        range: &std::ops::RangeInclusive<usize>,
        default: usize,
    ) -> Taken<usize> {
        match self.number::<usize>(name)?.unwrap_or(default) {
            number if range.contains(&number) => Ok(number),
            _ => Err(Refused),
        }
    }

    /// 开关，最多设置一次（设置两次时 clap 会拒绝）。
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

    /// 给了 `name` 时，`others` 都不能给（clap 的 `conflicts_with`）。
    fn without(&self, name: &str, others: &[&str]) -> Taken<()> {
        if self.given(name) && others.iter().any(|o| self.given(o)) {
            return Err(Refused);
        }
        Ok(())
    }

    /// [`COMMANDS`] 中的命令对应的 [`Read`]，没有给的参数用命令的默认值。
    pub(super) fn read(&self, command: &str) -> Taken<Read> {
        Ok(match command {
            "chat sessions" => Read::ChatSessions {
                limit: self.non_negative_or("limit", 0)?,
            },
            "chat messages" => {
                self.without("before", &["offset", "after"])?;
                self.without("after", &["offset"])?;
                self.without("since", &["offset", "before", "after"])?;
                Read::ChatMessages(MessagePage {
                    session_id: self.required("session-id")?,
                    limit: self.non_negative_or("limit", 50)?,
                    offset: self.non_negative_or("offset", 0)?,
                    before: self.text("before")?,
                    after: self.text("after")?,
                    since: self.non_negative("since")?,
                    parsed: self.switch("parsed")?,
                })
            }
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
                session: self.text("session")?,
                limit: self.non_negative_or("limit", 50)?,
                offset: self.non_negative_or("offset", 0)?,
                start: self.non_negative_or("start", 0)?,
                end: self.non_negative_or("end", 0)?,
                parsed: self.switch("parsed")?,
            },
            "chat dates" => Read::ChatDates {
                session_id: self.required("session-id")?,
            },
            "chat date-counts" => Read::ChatDateCounts {
                session_id: self.required("session-id")?,
            },
            "chat detail" => {
                self.without("fast", &["extra"])?;
                // 其余的详情读得慢：在子进程中运行
                if !self.switch("fast")? {
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
                limit: self.non_negative_or("limit", 300)?,
                before: self.text("before")?,
            },
            "chat image-info" => Read::ChatImageInfo {
                session_id: self.required("session-id")?,
                local_id: self.required("local-id")?,
                create_time: self.number("create-time")?,
            },
            "group members" => Read::GroupMembers {
                chatroom_id: self.required("chatroom-id")?,
                counts: self.switch("counts")?,
            },
            "sns timeline" => {
                // 要下载媒体：在子进程中运行
                if self.switch("with-media")? {
                    return Err(Refused);
                }
                Read::SnsTimeline {
                    limit: self.non_negative_or("limit", 20)?,
                    offset: self.non_negative_or("offset", 0)?,
                    users: self.list("user"),
                    keyword: self.text("keyword")?,
                    start: self.non_negative_or("start", 0)?,
                    end: self.non_negative_or("end", 0)?,
                }
            }
            "sns users" => Read::SnsUsers,
            "sns stats" => Read::SnsStats {
                prefer_cache: self.switch("prefer-cache")?,
            },
            "sns post-counts" => Read::SnsPostCounts {
                user: self.text("user")?,
                prefer_cache: self.switch("prefer-cache")?,
            },
            "chat statuses" => Read::ChatStatuses {
                sessions: self.list("sessions"),
            },
            "chat tab-counts" => Read::ChatTabCounts,
            "group list" => Read::GroupList,
            "group ranking" => Read::GroupRanking {
                chatroom_id: self.required("chatroom-id")?,
                limit: self.non_negative_or("limit", 20)?,
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
                limit: self.within_or("limit", &MEMBER_MESSAGES_LIMIT, 50)?,
                cursor: self.non_negative_or("cursor", 0)?,
                start: self.text("start")?,
                end: self.text("end")?,
            },
            "analytics overall" => Read::AnalyticsOverall,
            "analytics rankings" => Read::AnalyticsRankings {
                limit: self.non_negative_or("limit", 20)?,
                start: self.text("start")?,
                end: self.text("end")?,
            },
            "analytics time" => Read::AnalyticsTime,
            "analytics excluded" => {
                // 要保存名单：在子进程中运行
                if self.given("set") {
                    return Err(Refused);
                }
                Read::AnalyticsExcluded
            }
            "analytics exclude-candidates" => Read::AnalyticsExcludeCandidates,
            "biz accounts" => Read::BizAccounts,
            "biz messages" => Read::BizMessages {
                username: self.required("username")?,
                limit: self.non_negative_or("limit", 50)?,
                offset: self.non_negative_or("offset", 0)?,
            },
            "biz pay-records" => Read::BizPayRecords {
                limit: self.non_negative_or("limit", 50)?,
                offset: self.non_negative_or("offset", 0)?,
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
            "db detect" => Read::DbDetect,
            "db test" => Read::DbTest,
            "db wxid" => Read::DbWxid {
                root: self.text("root")?,
            },
            _ => return Err(Refused),
        })
    }

    /// [`MEDIA`] 中的命令要取的文件；`out` 不由请求给出（路由会拒绝它）。
    pub(super) fn media(&self, command: &str) -> Taken<Media> {
        Ok(match command {
            "chat image-data" => {
                const VARIANTS: &[&str] = &["best", "thumb", "normal", "hd"];
                let variant = self.text("variant")?.unwrap_or_else(|| "best".into());
                if !VARIANTS.contains(&variant.as_str()) {
                    return Err(Refused);
                }
                Media::Chat(MediaRead::ChatImage {
                    session_id: self.required("session-id")?,
                    local_id: self.required("local-id")?,
                    create_time: self.number("create-time")?,
                    variant: image_variant(&variant),
                })
            }
            "chat voice-data" => Media::Chat(MediaRead::ChatVoice {
                session_id: self.required("session-id")?,
                local_id: self.required("local-id")?,
                create_time: self.number("create-time")?,
                server_id: self.text("server-id")?,
                sender: self.text("sender")?,
            }),
            "chat video-data" => Media::Chat(MediaRead::ChatVideo {
                session_id: self.required("session-id")?,
                local_id: self.required("local-id")?,
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

/// `/api/` 媒体路由回答的文件：消息的文件，或者朋友圈帖子的文件。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Media {
    Chat(MediaRead),
    Sns { url: String, key: Option<String> },
}

/// `/api/` 请求在它的 [`Line`](super::Line) 中占的位置：排队的名额和运行的空位。
pub(super) type Place = (
    tokio::sync::OwnedSemaphorePermit,
    tokio::sync::OwnedSemaphorePermit,
);

/// 占着 `place` 运行 `work` 直到它结束，请求先断开时也一样（#396 的风险 2）：读取无法中途停止，
/// 所以运行期间一直占着空位，`--max-reads` 统计的是所有仍在运行的读取。`work` panic 时为 `Err`。
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

/// 在服务进程内执行 [`Read`] 得到的 `--json` 文档和退出码，与子进程打印的相同。
pub(super) async fn run(hub: &Hub, read: Read) -> (Value, u8) {
    let hub = match hub.get() {
        Ok(hub) => hub,
        Err(err) => return crate::reads::json_document(Err(err)),
    };
    let done =
        tokio::task::spawn_blocking(move || crate::reads::json_document(read.run(&hub))).await;
    done.unwrap_or_else(|err| {
        crate::reads::json_document(Err(AppError::runtime(format!("the command failed: {err}"))))
    })
}

/// `/api/` 路由是直接回答文件本身的 `GET`、供页面显示的命令：消息的图片、语音或视频，或者朋友圈帖子的图片或视频。
/// 参数与命令相同，只是没有 `out`。
pub(super) const MEDIA: &[&str] = &[
    "chat image-data",
    "chat voice-data",
    "chat video-data",
    "sns media",
];

pub(super) use weflow_core::reads::{image_mime, MediaBody, MediaFile};

/// 在服务进程内读取的 [`MEDIA`] 命令的文件：解密后的图片、解码出的 WAV 或视频；
/// 失败时是 `--json` 的失败文档和退出码。
pub(super) async fn media(hub: &Hub, media: Media) -> Result<MediaFile, (Value, u8)> {
    let outcome = match media {
        Media::Chat(read) => match hub.get() {
            Ok(hub) => read.fetch(hub).await,
            Err(err) => Err(err),
        },
        Media::Sns { url, key } => sns_media(hub, &url, key.as_deref()).await,
    };
    outcome.map_err(|err| crate::reads::json_document(Err(err)))
}

/// 供页面使用的 `sns media`（#202）：朋友圈帖子的图片或视频，从微信的服务器下载，用 `key` 解密后保存在缓存中，
/// 所以只下载一次。只接受微信媒体服务器的地址（见 [`weflow_core::sns::is_wechat_media_url`]）。
/// 视频从缓存的文件回答，播放器可以只请求其中的一部分。
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
        let dir = weflow_native::fixture::TestDir::new("weflow-stamp");
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"profile":"aaaa"}"#).unwrap();
        let time = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut stamp = Stamp::of(&path).unwrap();
        assert!(stamp.holds(&path));
        // 在同一个刻度内写入名字等长的另一个档案：修改时间和大小都相同
        std::fs::write(&path, r#"{"profile":"bbbb"}"#).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(time)
            .unwrap();
        assert!(!stamp.holds(&path));

        // 很久以前写入、之后才记下状态：只看修改时间和大小，不再读取内容
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
    }

    fn command_tree() -> Command {
        static TREE: std::sync::OnceLock<Command> = std::sync::OnceLock::new();
        TREE.get_or_init(crate::i18n::build_command::<crate::cli::Cli>)
            .clone()
    }

    /// 与子进程一样用 clap 解析命令行 `argv`；clap 拒绝时为 `None`。
    fn parse(argv: &[String]) -> Option<crate::cli::Cli> {
        let line = ["weflow", "--json"]
            .into_iter()
            .map(String::from)
            .chain(argv.iter().cloned());
        command_tree()
            .try_get_matches_from(line)
            .ok()
            .and_then(|m| crate::cli::Cli::from_arg_matches(&m).ok())
    }

    /// [`COMMANDS`] 中只有带这些参数时才在服务进程内执行的命令；不带时，`/api/` 按不带 `/api/` 的路由执行它们。
    const ONLY_WITH: &[(&str, &str)] = &[("chat detail", "--fast")];

    #[test]
    fn the_list_matches_the_cli() {
        let endpoints = super::super::Endpoints::new();
        let mut root = <crate::cli::Cli as clap::CommandFactory>::command();
        root.build();
        for name in COMMANDS {
            let ep = endpoints
                .0
                .get(*name)
                .unwrap_or_else(|| panic!("{name} is not a command"));
            // 也会写入的命令（`analytics excluded --set`）只有一部分在服务进程内执行
            let verbs = ep.verbs();
            assert!(
                verbs == [super::super::Verb::Get]
                    || (verbs[0] == super::super::Verb::Get
                        && IN_PART.iter().any(|(n, ..)| n == name)),
                "{name} is not a read"
            );
            // 填好所有必需参数的命令
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
                .and_then(|m| crate::cli::Cli::from_arg_matches(&m).ok())
                .unwrap_or_else(|| panic!("{name} does not parse"));
            assert!(
                crate::reads::read_of(cli.command).is_ok(),
                "{name} does not run in process"
            );
        }
    }

    /// 测量把请求参数读成 `Read` 的两种方式的耗时（#403）：从参数写出命令行、克隆命令树再用 clap 解析，
    /// 以及 `/api/` 的类型化读取（#397）；克隆命令树的耗时也单独列出。每轮在 stderr 输出各项的平均耗时。
    /// 这是测量而不是检查，用 release 构建运行：
    /// `cargo test --release -p weflow-cli --bin weflow clap_round_trip -- --ignored --nocapture`。
    #[test]
    #[ignore]
    fn clap_round_trip() {
        const RUNS: u32 = 2000;
        let endpoints = super::super::Endpoints::new();
        let ep = endpoints.0.get("chat messages").unwrap();
        let mut params = Params::new();
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
                let _ = std::hint::black_box(crate::reads::read_of(cli.command));
            }
            let all = begin.elapsed() / RUNS;
            // `/api/` 的做法（#397）
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

    /// 请求要的读取，由 clap（子进程的读法）或类型化读取读出。
    #[derive(Debug, PartialEq)]
    enum Asked {
        Read(Read),
        Media(Media),
    }

    /// 按子进程的方式读请求：写出命令行再用 clap 解析；clap 拒绝时为 `None`。
    fn by_clap(ep: &Endpoint, params: &Params) -> Option<Asked> {
        let name = ep.command_name();
        let media = MEDIA.contains(&name.as_str());
        let mut params = params.clone();
        if media {
            params.insert("out".into(), vec![Value::String("-".into())]);
        }
        let cli = parse(&ep.argv(&params).ok()?)?;
        if !media {
            return crate::reads::read_of(cli.command).ok().map(Asked::Read);
        }
        match cli.command {
            crate::cli::Commands::Chat(chat) => crate::reads::media_of(chat.command)
                .ok()
                .map(|(m, _)| Asked::Media(Media::Chat(m))),
            crate::cli::Commands::Sns(sns) => match sns.command {
                crate::commands::sns::SnsSubcommand::Media { url, key, .. } => {
                    Some(Asked::Media(Media::Sns { url, key }))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// 按 `/api/` 的方式读请求：类型化读取；不接受时为 `None`。
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

    /// 用固定的种子编造 `runs` 个请求，逐个交给 `each`：命令的每个参数可能没有、以文字给出一次或两次，
    /// 或者给出一个 JSON 值，键用参数名或把 `-` 换成 `_`。
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
            usize::try_from(seed % n as u64).unwrap_or(0)
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

    /// `/api/` 的类型化读取不接受命令行会拒绝的请求，接受的请求读法与 clap 相同：不接受的交给子进程，
    /// 回答与终端的相同。clap 接受而类型化读取不接受的请求只有一种：某个位置参数给了两次、另一个没有给，
    /// clap 按顺序把值分给它们。
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

    /// `chat detail` 接受 `fast` 或 `extra`，不能同时给：命令行拒绝两者同时出现，`/api/chat/detail` 也一样。
    #[test]
    fn chat_detail_takes_fast_or_extra_but_not_both() {
        use serde_json::json;
        let both = ["chat", "detail", "--fast", "--extra", "--", "wxid_bob"].map(String::from);
        assert!(parse(&both).is_none());
        let endpoints = super::super::Endpoints::new();
        let ep = endpoints.0.get("chat detail").unwrap();
        let read = |pairs: &[(&str, Value)]| {
            Given::new(ep, &super::super::tests::params(pairs)).read("chat detail")
        };
        let session = ("session-id", json!("wxid_bob"));
        assert!(read(&[
            session.clone(),
            ("fast", json!(true)),
            ("extra", json!(true))
        ])
        .is_err());
        assert_eq!(
            read(&[session, ("fast", json!(true))]).unwrap(),
            Read::ChatDetailFast {
                session_id: "wxid_bob".into()
            }
        );
    }

    #[test]
    fn the_push_route_is_not_a_command() {
        // `/api/push/messages` 是事件流，不是命令
        assert!(COMMANDS
            .iter()
            .chain(MEDIA)
            .all(|c| !c.starts_with("push ")));
    }

    #[test]
    fn media_commands_parse_with_the_placeholder_out() {
        use crate::cli::Commands;
        use crate::commands::chat::ChatSubcommand;
        use crate::commands::sns::SnsSubcommand;
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
            // 消息的会话 ID 和本地 ID，或者朋友圈的地址
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
