//! What tells a client the server is there (issue #221): `GET /health`, and `GET /health/events`, the same as a
//! stream of server-sent events that a page keeps open instead of asking again and again.
//!
//! - The stream starts with `event: hello`, whose data is `/health`'s; then `event: heartbeat` every 15 s, whose
//!   data is `{"data": …}` (see [`DataKey`]). A heartbeat is an event, not a comment line: `EventSource` reports
//!   no comment, and a page needs something it can see to tell a stream still alive from a half-open one.
//! - Like `/health`, it needs no token (an `EventSource` cannot send a header, and a token in the address ends up
//!   in logs and history) and is behind the `Host` check. It starts no work: no push engine, only the heartbeat.
//! - `/api/push/messages` starts with the same `hello` and sends the same heartbeat, so a page listening to it
//!   needs no second stream.
//! - When the server stops on Ctrl+C, each of these streams ends with `event: bye` (see [`Farewell`]), so a page
//!   can tell a server that stopped from one it lost.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{header, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use tokio::sync::watch;

use super::API_VERSION;

/// Says `bye` on every open event stream when the server stops.
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
    /// Every open stream sends `bye` and ends; returns once they all did, or `within` passed.
    pub async fn say(&self, within: Duration) {
        self.stopping.send_replace(true);
        let deadline = Instant::now() + within;
        while self.open.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // the last `bye` handed over is still on its way out
        if self.open.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// A stream that is open: it ends with `bye` once [`Farewell::say`] is called.
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
    /// Resolves when the server stops.
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

/// How often a stream sends `heartbeat`; `WEFLOW_SSE_HEARTBEAT_MS` sets another for tests.
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

/// `data` of `/health` (issue #198): changes when the configuration does (so with the account the commands read)
/// and with each run, so a client knows when what it read before may be another account's. A keyed hash, its key
/// drawn at start: it says nothing of the configuration itself.
#[derive(Clone)]
pub(super) struct DataKey {
    seed: std::collections::hash_map::RandomState,
    config: PathBuf,
}

impl DataKey {
    pub(super) fn new(config: PathBuf) -> Self {
        Self {
            seed: Default::default(),
            config,
        }
    }

    pub(super) fn get(&self) -> String {
        use std::hash::BuildHasher;
        let config = std::fs::read(&self.config).unwrap_or_default();
        format!("{:016x}", self.seed.hash_one(&config))
    }
}

/// The body of `/health`, and the data of `hello`.
pub(super) fn health(data: &DataKey) -> Value {
    json!({
        "status": "ok",
        "apiVersion": API_VERSION,
        "version": env!("WEFLOW_VERSION"),
        "data": data.get(),
    })
}

pub(super) fn hello(data: &DataKey) -> Bytes {
    Bytes::from(format!("event: hello\ndata: {}\n\n", health(data)))
}

pub(super) fn beat(data: &DataKey) -> Bytes {
    Bytes::from(format!(
        "event: heartbeat\ndata: {}\n\n",
        json!({ "data": data.get() })
    ))
}

/// An event stream of the command API, with its headers.
pub(super) fn event_stream(body: Body) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache, no-transform")
        .header("x-accel-buffering", "no")
        .body(body)
        .unwrap()
}

/// `GET /health/events`.
pub(super) fn events(data: DataKey, farewell: &Farewell) -> Response {
    let mut listening = farewell.listen();
    let stream = async_stream::stream! {
        yield Ok::<_, Infallible>(hello(&data));
        let mut every = tokio::time::interval(heartbeat());
        every.tick().await;
        loop {
            tokio::select! {
                _ = every.tick() => yield Ok(beat(&data)),
                _ = listening.stopping() => {
                    yield Ok(bye());
                    break;
                }
            }
        }
    };
    event_stream(Body::from_stream(stream))
}
