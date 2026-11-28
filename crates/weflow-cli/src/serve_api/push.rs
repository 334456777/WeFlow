//! `GET /api/push/messages`（#140）：以服务器发送事件推送新消息和撤回的消息，网页不必轮询 `chat messages`。
//! 移植自桌面版的推送引擎（`weflow_core::push`）在进程内路由的 `Hub` 上查找这些消息；引擎只在有人监听时运行，
//! 每秒检查一次账号的会话库和消息库是否变化。

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use axum::body::{Body, Bytes};
use axum::response::Response;
use serde_json::json;
use tokio::sync::broadcast;
use weflow_core::push::{PushBroker, PushEngine};
use weflow_core::services::ServiceHub;

use super::in_process;
use super::status::{self, Farewell, Status};

/// 有人监听时，引擎每隔多久检查一次新消息。
const INTERVAL: Duration = Duration::from_secs(1);

/// `/api/push/messages` 的状态：给事件编号并保存事件的 broker、监听者的数量，以及引擎的线程是否在运行。
pub(super) struct Push {
    broker: Arc<PushBroker>,
    listeners: Arc<AtomicUsize>,
    running: Arc<AtomicBool>,
}

impl Default for Push {
    fn default() -> Self {
        Self {
            // 事件 ID 从当前的 Unix 毫秒时间起编号：带着上次运行的 `Last-Event-ID`
            // 回到重启后的服务的页面不会漏掉事件
            broker: PushBroker::from_now(),
            listeners: Default::default(),
            running: Default::default(),
        }
    }
}

/// 收到或撤回消息时微信写入的 `db_storage` 子文件夹。
const WATCHED: &[&str] = &["session", "message"];
/// 即使没有文件变化，也每隔这么久完整查找一次，以防文件系统迟报变化。
const FULL_ROUND: Duration = Duration::from_secs(30);

/// [`WATCHED`] 文件夹中每个文件的修改时间和大小；新消息会改变其中之一。
type Stamp = Vec<(PathBuf, Option<SystemTime>, u64)>;

/// 账号目录 `account` 的 [`Stamp`]，按路径排序。
fn stamp(account: &Path) -> Stamp {
    let mut files = Vec::new();
    for dir in WATCHED {
        let Ok(entries) = std::fs::read_dir(account.join("db_storage").join(dir)) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            // 不用目录项自带的元数据：Windows 上那是目录列表里的副本，另一个程序打开着文件写入时，它会滞后
            if let Ok(meta) = std::fs::metadata(&path) {
                files.push((path, meta.modified().ok(), meta.len()));
            }
        }
    }
    files.sort();
    files
}

/// 一个 `Hub` 的推送引擎。查找新消息要读取每个会话的最新消息，大账号上约需 0.1 秒，
/// 所以监视的文件都没有变化时跳过查找。
struct Watcher {
    engine: PushEngine,
    generation: u64,
    account: Option<PathBuf>,
    stamp: Stamp,
    last_full: Instant,
}

impl Watcher {
    /// 以 `hub` 中现有的内容为基线启动引擎；读不出基线时为 `None`。
    fn start(hub: ServiceHub, generation: u64) -> Option<Self> {
        let account = hub.account_dir().ok();
        // 先记下文件的状态：读取基线期间发生的变化会在下一轮看到
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

    /// 一轮查找得到的事件：没有变化时为空。
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

/// 一个打开的事件流；最后一个关闭后，引擎停止。
struct Listener(Arc<AtomicUsize>);

impl Drop for Listener {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Push {
    /// 引擎的线程还没运行时启动它。
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
                    // 这期间打开的事件流可能看到线程仍在运行，因而没有另起一个：这时由这个线程继续运行
                    if listeners.load(Ordering::SeqCst) == 0 || running.swap(true, Ordering::SeqCst)
                    {
                        return;
                    }
                }
                // 还没有配置，或者读不了账号：下一轮再试
                if let Ok((current, generation)) = hub.hub_and_generation() {
                    match &mut watcher {
                        Some(w) if w.generation == generation => {
                            for payload in w.round() {
                                broker.broadcast(&payload);
                            }
                        }
                        // 换了新的 `Hub`（另一个账号或密钥）：以它现在的内容为基线重新开始
                        _ => watcher = Watcher::start(current, generation),
                    }
                }
                std::thread::sleep(INTERVAL);
            }
        });
    }

    /// 事件流：`hello`（见 `status`）、`ready`、`last_id` 之后保存的事件（为 0 时是全部），然后是每个新事件；
    /// 没有事件时发送 `heartbeat`，服务停止时发送 `bye`。
    pub(super) fn stream(
        &self,
        hub: Arc<in_process::Hub>,
        last_id: u64,
        server: Status,
        farewell: &Farewell,
    ) -> Response {
        let mut listening = farewell.listen();
        self.listeners.fetch_add(1, Ordering::SeqCst);
        let listener = Listener(self.listeners.clone());
        self.watch(hub);
        let broker = self.broker.clone();
        let mut rx = broker.subscribe();
        let replay = broker.replay_since(last_id);
        let stream = async_stream::stream! {
            let _listener = listener;
            yield Ok::<_, Infallible>(server.hello());
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
                    events = next_events(&broker, &mut rx, &mut highest) => match events {
                        Some(events) => {
                            for (_, body) in events {
                                yield Ok(Bytes::from(body.as_bytes().to_vec()));
                            }
                        }
                        None => break,
                    },
                    _ = heartbeat.tick() => yield Ok(server.beat()),
                    () = listening.stopping() => {
                        yield Ok(status::bye());
                        break;
                    }
                }
            }
        };
        status::event_stream(Body::from_stream(stream))
    }
}

/// broker 的一个事件：ID 和事件流发送的文字。
type Kept = (u64, Arc<String>);

/// 事件流接下来要发送的 broker 事件，每个都比 `highest` 新（`highest` 随之更新）：通常是 `rx` 收到的下一个；
/// `rx` 落后于 broker 的通道时，是 broker 保存的、`rx` 错过的那些。`rx` 收到已经发过的事件时为空；
/// broker 不在了时为 `None`。
async fn next_events(
    broker: &PushBroker,
    rx: &mut broadcast::Receiver<Kept>,
    highest: &mut u64,
) -> Option<Vec<Kept>> {
    match rx.recv().await {
        Ok((id, body)) => {
            if id <= *highest {
                return Some(Vec::new());
            }
            *highest = id;
            Some(vec![(id, body)])
        }
        Err(broadcast::error::RecvError::Lagged(_)) => {
            let missed = broker.replay_since(*highest);
            if let Some((id, _)) = missed.last() {
                *highest = (*highest).max(*id);
            }
            Some(missed)
        }
        Err(broadcast::error::RecvError::Closed) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 接收端落后于 broker 通道的事件流，从 broker 保存的事件中补发错过的，再接着发送新事件，
    /// 每个只发一次，顺序不变。
    #[tokio::test]
    async fn a_stream_that_falls_behind_misses_no_event() {
        let broker = PushBroker::new();
        let mut rx = broker.subscribe();
        // 比通道能容纳的多，比 broker 保存的少
        let count = 600u64;
        for n in 0..count {
            broker.broadcast(&json!({ "n": n }));
        }
        let mut highest = 0;
        let mut sent = Vec::new();
        while sent.last() != Some(&count) {
            let events = next_events(&broker, &mut rx, &mut highest)
                .await
                .expect("the broker is there");
            sent.extend(events.into_iter().map(|(id, _)| id));
        }
        assert_eq!(sent, (1..=count).collect::<Vec<_>>());

        // 通道里还留着的事件都已发过；下一个新事件只来一次
        broker.broadcast(&json!({ "n": count }));
        let mut after = Vec::new();
        while after.is_empty() {
            after = next_events(&broker, &mut rx, &mut highest)
                .await
                .expect("the broker is there");
        }
        let ids: Vec<u64> = after.into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, [count + 1]);
    }
}
