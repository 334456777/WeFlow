//! `GET /api/push/messages` (issue #140): new and recalled messages as server-sent events, for a page that would
//! otherwise poll `chat latest`. The desktop app's engine (`weflow_core::push`) finds them on the hub of the
//! in-process routes; it runs only while someone listens, checking once a second whether the account's session and
//! message databases changed.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use axum::body::{Body, Bytes};
use axum::response::Response;
use serde_json::json;
use weflow_core::push::{PushBroker, PushEngine};
use weflow_core::services::ServiceHub;

use super::in_process;
use super::status::{self, DataKey, Farewell};

/// How often the engine looks for new messages while someone listens.
const INTERVAL: Duration = Duration::from_secs(1);

pub(super) struct Push {
    broker: Arc<PushBroker>,
    listeners: Arc<AtomicUsize>,
    running: Arc<AtomicBool>,
}

impl Default for Push {
    fn default() -> Self {
        Self {
            broker: PushBroker::new(),
            listeners: Default::default(),
            running: Default::default(),
        }
    }
}

/// The folders of `db_storage` WeChat writes to when a message comes or is recalled.
const WATCHED: &[&str] = &["session", "message"];
/// A full look even when no file changed, in case a file system reports a change late.
const FULL_ROUND: Duration = Duration::from_secs(30);

/// Each file of the [`WATCHED`] folders with its time and size; a new message changes one of them.
type Stamp = Vec<(PathBuf, Option<SystemTime>, u64)>;

fn stamp(account: &Path) -> Stamp {
    let mut files = Vec::new();
    for dir in WATCHED {
        let Ok(entries) = std::fs::read_dir(account.join("db_storage").join(dir)) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            // not the entry's own metadata: on Windows that is the folder listing's copy, which lags behind the
            // writes to a file another program holds open
            if let Ok(meta) = std::fs::metadata(&path) {
                files.push((path, meta.modified().ok(), meta.len()));
            }
        }
    }
    files.sort();
    files
}

/// The engine of one hub. Looking for new messages reads every group's latest messages, about a tenth of a second
/// on a large account, so it is skipped while none of the watched files changed.
struct Watcher {
    engine: PushEngine,
    generation: u64,
    account: Option<PathBuf>,
    stamp: Stamp,
    last_full: Instant,
}

impl Watcher {
    fn start(hub: ServiceHub, generation: u64) -> Option<Self> {
        let account = hub.account_dir().ok();
        // taken first: a change while the baseline is read shows on the next round
        let stamp = account.as_deref().map(stamp).unwrap_or_default();
        let mut engine = PushEngine::new(hub);
        engine.bootstrap().then(|| Self {
            engine,
            generation,
            account,
            stamp,
            last_full: Instant::now(),
        })
    }

    /// The events of one round: none when nothing changed.
    fn round(&mut self) -> Vec<serde_json::Value> {
        if let Some(account) = &self.account {
            let now = stamp(account);
            if now == self.stamp && self.last_full.elapsed() < FULL_ROUND {
                return Vec::new();
            }
            self.stamp = now;
        }
        self.last_full = Instant::now();
        self.engine.flush(true)
    }
}

/// One open stream; the engine stops once the last one is gone.
struct Listener(Arc<AtomicUsize>);

impl Drop for Listener {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Push {
    /// Starts the engine's thread unless it runs already.
    fn watch(&self, hub: Arc<in_process::Hub>) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let (broker, listeners, running) = (
            self.broker.clone(),
            self.listeners.clone(),
            self.running.clone(),
        );
        std::thread::spawn(move || {
            let mut watcher: Option<Watcher> = None;
            loop {
                if listeners.load(Ordering::SeqCst) == 0 {
                    running.store(false, Ordering::SeqCst);
                    // a stream opened meanwhile found the thread still running and did not start another
                    if listeners.load(Ordering::SeqCst) == 0 || running.swap(true, Ordering::SeqCst)
                    {
                        return;
                    }
                }
                // not configured yet, or the account cannot be read: try again on the next round
                if let Ok((current, generation)) = hub.get_counted() {
                    match &mut watcher {
                        Some(w) if w.generation == generation => {
                            for payload in w.round() {
                                broker.broadcast(&payload);
                            }
                        }
                        // a new hub (another account or key): start from what it has now
                        _ => watcher = Watcher::start(current, generation),
                    }
                }
                std::thread::sleep(INTERVAL);
            }
        });
    }

    /// The event stream: `hello` (see `status`), `ready`, the kept events after `last_id` (all of them for 0), then
    /// each new one, with a `heartbeat` when it is quiet; `bye` when the server stops.
    pub(super) fn stream(
        &self,
        hub: Arc<in_process::Hub>,
        last_id: u64,
        data: DataKey,
        farewell: &Farewell,
    ) -> Response {
        let mut listening = farewell.listen();
        self.listeners.fetch_add(1, Ordering::SeqCst);
        let listener = Listener(self.listeners.clone());
        self.watch(hub);
        let mut rx = self.broker.subscribe();
        let replay = self.broker.replay_since(last_id);
        let stream = async_stream::stream! {
            let _listener = listener;
            yield Ok::<_, Infallible>(status::hello(&data));
            let ready = format!("event: ready\ndata: {}\n\n", json!({ "success": true }));
            yield Ok(Bytes::from(ready));
            let mut highest = last_id;
            for (id, body) in replay {
                highest = highest.max(id);
                yield Ok(Bytes::from(body.as_bytes().to_vec()));
            }
            let mut heartbeat = tokio::time::interval(status::heartbeat());
            heartbeat.tick().await;
            loop {
                tokio::select! {
                    msg = rx.recv() => match msg {
                        Ok((id, body)) => {
                            if id <= highest {
                                continue;
                            }
                            highest = id;
                            yield Ok(Bytes::from(body.as_bytes().to_vec()));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    },
                    _ = heartbeat.tick() => yield Ok(status::beat(&data)),
                    _ = listening.stopping() => {
                        yield Ok(status::bye());
                        break;
                    }
                }
            }
        };
        status::event_stream(Body::from_stream(stream))
    }
}
