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

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{header, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};

use super::API_VERSION;

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
pub(super) fn events(data: DataKey) -> Response {
    let stream = async_stream::stream! {
        yield Ok::<_, Infallible>(hello(&data));
        let mut every = tokio::time::interval(heartbeat());
        every.tick().await;
        loop {
            every.tick().await;
            yield Ok(beat(&data));
        }
    };
    event_stream(Body::from_stream(stream))
}
