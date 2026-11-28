//! 让客户端知道服务还在（#221）：`GET /health`，以及内容相同、以服务器发送事件（server-sent events）推送的
//! `GET /health/events`，网页让它一直开着，不必反复请求。
//!
//! - 事件流以 `event: hello` 开始，数据与 `/health` 相同；之后每 15 秒一个 `event: heartbeat`，
//!   数据为 `{"data": …}`（见 [`Status::data`]）。心跳是事件而不是注释行：`EventSource` 不报告注释，
//!   网页需要看得见的东西来区分仍然活着的事件流和半开的连接。
//! - 与 `/health` 一样不需要令牌（`EventSource` 不能发送请求头，而地址里的令牌会留在日志和浏览历史中），
//!   同样经过 `Host` 检查。它不启动任何工作：没有推送引擎，只有心跳。
//! - `/api/push/messages` 以同样的 `hello` 开始，发送同样的心跳，监听它的网页不需要第二个事件流。
//!   `/jobs/{id}/events` 以同样的间隔发送数据为空的 `heartbeat`。
//! - 服务被要求停止时（Ctrl+C 或 `crate::stop` 接受的其他停止请求：SIGTERM、SIGHUP、关闭控制台窗口、
//!   Windows 注销或关机），`/health/events` 和 `/api/push/messages` 以 `event: bye` 结束（见 [`Farewell`]），
//!   网页因此能区分已停止的服务和失去连接的服务。

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};
use tokio::sync::watch;

use super::API_VERSION;

/// 服务停止时在每个打开的事件流上发送 `bye`。
#[derive(Clone)]
pub struct Farewell {
    stopping: Arc<watch::Sender<bool>>,
    open: Arc<AtomicUsize>,
}

impl Default for Farewell {
    fn default() -> Self {
        Self {
            stopping: Arc::new(watch::channel(false).0),
            open: Default::default(),
        }
    }
}

impl Farewell {
    /// 让每个打开的事件流发送 `bye` 并结束；全部结束或过了 `within` 后返回。
    pub async fn say(&self, within: Duration) {
        self.stopping.send_replace(true);
        let deadline = Instant::now() + within;
        while self.open.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // 最后交出去的 `bye` 可能还没发出
        if self.open.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// 登记一个打开的事件流：调用 [`Farewell::say`] 后，它以 `bye` 结束。
    pub(super) fn listen(&self) -> Listening {
        self.open.fetch_add(1, Ordering::SeqCst);
        Listening {
            stopping: self.stopping.subscribe(),
            open: self.open.clone(),
        }
    }
}

pub(super) struct Listening {
    stopping: watch::Receiver<bool>,
    open: Arc<AtomicUsize>,
}

impl Listening {
    /// 服务停止时完成。
    pub(super) async fn stopping(&mut self) {
        let _ = self.stopping.wait_for(|stopping| *stopping).await;
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.open.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(super) fn bye() -> Bytes {
    Bytes::from(format!(
        "event: bye\ndata: {}\n\n",
        json!({ "reason": "stopped" })
    ))
}

/// 事件流发送 `heartbeat` 的间隔，默认 15 秒；测试用 `WEFLOW_SSE_HEARTBEAT_MS` 另设。
pub(super) fn heartbeat() -> Duration {
    static EVERY: OnceLock<Duration> = OnceLock::new();
    *EVERY.get_or_init(|| {
        std::env::var("WEFLOW_SSE_HEARTBEAT_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|ms| *ms > 0)
            .map_or(Duration::from_secs(15), Duration::from_millis)
    })
}

/// `/health` 和 `hello` 报告的服务状态。
#[derive(Clone)]
pub(super) struct Status {
    /// 启动时随机生成，所以 `data` 在两次运行之间不同，也不透露配置的内容。
    seed: std::collections::hash_map::RandomState,
    /// 服务启动时命令读取的配置文件：找不到当前使用的配置文件时，`data` 按它计算。
    config: PathBuf,
    /// 服务的运行方式（#214）：各项上限和见解引擎是否在运行，不含账号的任何信息。
    server: Arc<Value>,
}

/// `/health` 报告的服务运行方式。
pub(super) struct ServerInfo {
    pub(super) max_tasks: usize,
    pub(super) max_reads: usize,
    pub(super) max_media: usize,
    pub(super) insight: bool,
}

impl Status {
    pub(super) fn new(config: PathBuf, server: &ServerInfo) -> Self {
        Self {
            seed: Default::default(),
            config,
            server: Arc::new(json!({
                "limits": {
                    "maxTasks": server.max_tasks,
                    "maxReads": server.max_reads,
                    "maxMedia": server.max_media,
                },
                "insight": server.insight,
            })),
        }
    }

    /// `data`（#198）：配置（因而命令读取的账号）变化时和每次启动时都会改变，
    /// 客户端由此知道之前读到的内容可能属于另一个账号。配置指命令当前读取的文件，包括它的路径和内容：
    /// 服务运行期间，`config set config_path` 可以换成另一个文件。它是带密钥的哈希，密钥在启动时随机生成，
    /// 不透露配置本身的任何内容。
    pub(super) fn data(&self) -> String {
        use std::hash::BuildHasher;
        let path =
            weflow_core::config::default_config_path().unwrap_or_else(|| self.config.clone());
        let content = std::fs::read(&path).unwrap_or_default();
        format!("{:016x}", self.seed.hash_one((&path, &content)))
    }

    /// `/health` 的回答，也是 `hello` 的数据。
    pub(super) fn health(&self) -> Value {
        let mut body = json!({
            "status": "ok",
            "apiVersion": API_VERSION,
            "version": env!("WEFLOW_VERSION"),
            "data": self.data(),
            // 服务端回答所用的语言，WebUI 也用它显示界面（#439）
            "lang": super::server_lang(),
            // 服务端所在计算机的系统：`windows`、`macos` 或 `linux`
            "platform": std::env::consts::OS,
        });
        if let (Some(body), Some(server)) = (body.as_object_mut(), self.server.as_object()) {
            body.extend(server.clone());
        }
        body
    }

    pub(super) fn hello(&self) -> Bytes {
        Bytes::from(format!("event: hello\ndata: {}\n\n", self.health()))
    }

    pub(super) fn beat(&self) -> Bytes {
        Bytes::from(format!(
            "event: heartbeat\ndata: {}\n\n",
            json!({ "data": self.data() })
        ))
    }
}

/// 命令 API 的事件流回答：`no-transform` 和 `x-accel-buffering: no` 让代理（如 nginx）不缓冲、不改写事件流。
pub(super) fn event_stream(body: Body) -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache, no-transform"),
            ),
            (
                HeaderName::from_static("x-accel-buffering"),
                HeaderValue::from_static("no"),
            ),
        ],
        body,
    )
        .into_response()
}

/// `GET /health/events` 的事件流。
pub(super) fn events(status: Status, farewell: &Farewell) -> Response {
    let mut listening = farewell.listen();
    let stream = async_stream::stream! {
        yield Ok::<_, Infallible>(status.hello());
        let mut every = tokio::time::interval(heartbeat());
        every.tick().await;
        loop {
            tokio::select! {
                _ = every.tick() => yield Ok(status.beat()),
                () = listening.stopping() => {
                    yield Ok(bye());
                    break;
                }
            }
        }
    };
    event_stream(Body::from_stream(stream))
}
