//! 在后台运行的命令（`Prefer: respond-async`）：作业的生命周期，与 HTTP 无关（#398）。
//!
//! 作业先排队，有空位后运行，结束于成功、失败或取消；结束状态只写一次，与 `result` 事件一起写入，
//! 之后到来的事情（迟到的进度行、第二次取消）都不会改变它。作业的事件会保存下来，供晚订阅或重连的客户端读取。
//! 呈现作业的路由在 [`super::job_routes`]。作业只保存在内存中，已结束的作业保留最近的 [`KEEP_FINISHED`] 个。
//!
//! 作业在子进程中运行命令（见 [`run_child`]）：长时间运行的任务或导出崩溃时不会拖垮服务，
//! 取消作业时终止进程，一定能释放它占用的内存和文件。

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt};
use tokio::sync::{broadcast, watch, Semaphore};
use weflow_core::sync::MutexExt;

use super::child::status_for;

/// `GET /jobs` 保留的已结束作业数，更早的被忘掉。
const KEEP_FINISHED: usize = 100;

/// 每个作业保存的事件数，供晚订阅或重连的客户端读取。
const KEEP_EVENTS: usize = 1000;

/// 被取消的自行结束的命令（如 `key db`）有多长时间自己结束，超时后被终止。
const CLEAN_UP: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(rename = "JobStatus")]
pub(super) enum Status {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl Status {
    pub(super) fn finished(self) -> bool {
        matches!(self, Status::Succeeded | Status::Failed | Status::Cancelled)
    }
}

/// `/jobs` 回答的一个作业（`Prefer: respond-async`，#398）：运行的命令、进度和结束的情况。
/// 时间都是 Unix 毫秒。
#[derive(serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(rename = "Job")]
pub(super) struct JobView {
    pub(super) id: String,
    pub(super) status: Status,
    /// 作业运行的命令行，第一项是 `weflow`（如 `weflow --json --progress export messages …`）。
    pub(super) command: Vec<String>,
    pub(super) created_at: u64,
    pub(super) started_at: Option<u64>,
    pub(super) finished_at: Option<u64>,
    pub(super) exit_code: Option<i32>,
    /// 同步请求会得到的 HTTP 状态。
    pub(super) http_status: Option<u16>,
    /// 最后一个 `progress` 事件。
    pub(super) progress: Option<Value>,
    /// 以 `return=file` 启动的作业成功后写出的文件，用 `GET /jobs/{id}/file` 取得。
    pub(super) file: Option<JobFile>,
    /// 命令的 `--json` 文档；只出现在 `GET /jobs/{id}`、`DELETE /jobs/{id}` 的回答和 `result` 事件中。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) result: Option<Value>,
}

/// 作业写出的文件。
#[derive(serde::Serialize, schemars::JsonSchema)]
pub(super) struct JobFile {
    /// 文件名。
    pub(super) name: Option<String>,
    /// 字节数；读不到文件时为 `null`。
    pub(super) size: Option<u64>,
}

/// 作业的一个事件：序号（服务器发送事件的 `id`）、名称和 JSON 数据。
#[derive(Clone, Debug)]
pub(super) struct Event {
    pub seq: u64,
    pub name: String,
    pub data: String,
}

impl Event {
    /// 是否是作业的最后一个事件 `result`。
    pub(super) fn is_result(&self) -> bool {
        self.name == "result"
    }
}

/// 作业会变化的部分，由 `Job::state` 的锁保护。
struct State {
    status: Status,
    started: Option<u64>,
    finished: Option<u64>,
    exit_code: Option<i32>,
    http_status: Option<u16>,
    /// 最后一个 `progress` 事件。
    progress: Option<Value>,
    /// 命令的 `--json` 文档（或者服务自己的错误文档）。
    result: Option<Value>,
    events: VecDeque<Event>,
    next_seq: u64,
    /// 以 `?return=file` 启动并成功的作业，命令写出的文件。
    file: Option<PathBuf>,
}

/// 一个作业：命令、状态、事件，以及取消它的开关。
pub(super) struct Job {
    pub id: String,
    command: Vec<String>,
    created: u64,
    state: Mutex<State>,
    sender: broadcast::Sender<Event>,
    cancel: watch::Sender<bool>,
    /// 以 `?return=file` 启动的作业，命令把文件写到哪里。
    file_out: Option<super::FileOut>,
}

/// 订阅作业的事件时得到的：它最后看到的事件之后保存的事件，以及作业尚未结束时 `live` 送来的新事件。
pub(super) struct Subscription {
    pub replay: Vec<Event>,
    pub live: broadcast::Receiver<Event>,
    pub ended: bool,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl Job {
    fn new(command: Vec<String>, file_out: Option<super::FileOut>) -> Self {
        let (sender, _) = broadcast::channel(256);
        let (cancel, _) = watch::channel(false);
        Self {
            id: super::generate_token()[..16].to_string(),
            command,
            created: now_ms(),
            state: Mutex::new(State {
                status: Status::Queued,
                started: None,
                finished: None,
                exit_code: None,
                http_status: None,
                progress: None,
                result: None,
                events: VecDeque::new(),
                next_seq: 1,
                file: None,
            }),
            sender,
            cancel,
            file_out,
        }
    }

    /// 不含命令结果的作业 JSON。
    pub(super) fn to_json(&self) -> Value {
        self.json_of(&self.state.lock_or_panic(), false)
    }

    /// 含命令结果的作业 JSON；还没有结果时 `result` 为 `null`。
    pub(super) fn to_json_with_result(&self) -> Value {
        self.json_of(&self.state.lock_or_panic(), true)
    }

    fn json_of(&self, st: &State, full: bool) -> Value {
        json!(JobView {
            id: self.id.clone(),
            status: st.status,
            command: self.command.clone(),
            created_at: self.created,
            started_at: st.started,
            finished_at: st.finished,
            exit_code: st.exit_code,
            http_status: st.http_status,
            progress: st.progress.clone(),
            file: st.file.as_ref().map(|path| JobFile {
                name: path.file_name().map(|n| n.to_string_lossy().into_owned()),
                size: std::fs::metadata(path).map(|m| m.len()).ok(),
            }),
            result: full.then(|| st.result.clone().unwrap_or(Value::Null)),
        })
    }

    pub(super) fn status(&self) -> Status {
        self.state.lock_or_panic().status
    }

    /// 作业成功并写出了文件时，这个文件。
    pub(super) fn file(&self) -> Option<PathBuf> {
        self.state.lock_or_panic().file.clone()
    }

    /// 在持有 `st` 的锁时记下一个事件并发给订阅者，所以订阅者看到的每个事件，要么在保存的事件中，要么是新事件。
    fn emit(&self, st: &mut State, name: &str, data: String) {
        let event = Event {
            seq: st.next_seq,
            name: name.to_string(),
            data,
        };
        st.next_seq += 1;
        if st.events.len() == KEEP_EVENTS {
            st.events.pop_front();
        }
        st.events.push_back(event.clone());
        // 没有订阅者也没关系
        let _ = self.sender.send(event);
    }

    /// 从排队变为运行；这期间被取消的作业保持取消，返回 `false`。
    fn set_running(&self) -> bool {
        let mut st = self.state.lock_or_panic();
        if st.status != Status::Queued {
            return false;
        }
        st.status = Status::Running;
        st.started = Some(now_ms());
        self.emit(
            &mut st,
            "status",
            json!({ "type": "status", "status": "running" }).to_string(),
        );
        true
    }

    /// 命令在 stderr 写出的一行：NDJSON 事件用它的 `type` 作为事件名，没有有效 `type` 的 JSON 对象成为 `message`
    /// 事件，其他文字成为 `stderr` 事件。作业结束后到来的行被丢弃。
    fn stderr_line(&self, line: &str) {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            return;
        }
        let mut st = self.state.lock_or_panic();
        if st.status.finished() {
            return;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(value @ Value::Object(_)) => {
                let name = value["type"]
                    .as_str()
                    .filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_graphic()))
                    .unwrap_or("message")
                    .to_string();
                if name == "progress" {
                    st.progress = Some(value.clone());
                }
                self.emit(&mut st, &name, value.to_string());
            }
            _ => {
                let data = json!({ "type": "stderr", "text": line }).to_string();
                self.emit(&mut st, "stderr", data);
            }
        }
    }

    /// 结束作业，只结束一次：在同一把锁下写入状态、结果和 `result` 事件。
    /// 作业已经结束时返回 `false`，什么也不改变。
    fn finish(
        &self,
        status: Status,
        exit_code: Option<i32>,
        http_status: u16,
        result: Value,
    ) -> bool {
        debug_assert!(status.finished());
        let file = match &self.file_out {
            Some(out) if status == Status::Succeeded => {
                let mut path = super::files::written_file(&result, out.field, &out.out);
                if out.dir.is_some() {
                    path = super::files::add_extension(&path);
                }
                Some(path).filter(|p| p.is_file())
            }
            _ => None,
        };
        let mut st = self.state.lock_or_panic();
        if st.status.finished() {
            return false;
        }
        if file.is_none() {
            self.remove_folder();
        }
        st.file = file;
        st.status = status;
        st.finished = Some(now_ms());
        st.exit_code = exit_code;
        st.http_status = Some(http_status);
        st.result = Some(result);
        let doc = self.json_of(&st, true).to_string();
        self.emit(&mut st, "result", doc);
        true
    }

    /// `after`（服务器发送事件的 `Last-Event-ID`）之后的事件，以及新的事件。
    pub(super) fn subscribe(&self, after: u64) -> Subscription {
        let st = self.state.lock_or_panic();
        Subscription {
            replay: st
                .events
                .iter()
                .filter(|e| e.seq > after)
                .cloned()
                .collect(),
            live: self.sender.subscribe(),
            ended: st.status.finished(),
        }
    }

    /// `after` 之后保存的事件，供落后的订阅者补读。
    pub(super) fn kept_after(&self, after: u64) -> Vec<Event> {
        let st = self.state.lock_or_panic();
        st.events
            .iter()
            .filter(|e| e.seq > after)
            .cloned()
            .collect()
    }

    /// 删除服务端为作业的文件选定的文件夹（`files_dir` 下请求自己的文件夹）；请求自己给的 `out` 保留。
    fn remove_folder(&self) {
        if let Some(dir) = self.file_out.as_ref().and_then(|f| f.dir.as_ref()) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// 服务的全部作业，按启动的先后排列。
pub(super) struct Jobs {
    list: Mutex<VecDeque<Arc<Job>>>,
}

/// [`Jobs::cancel_or_forget`] 做了什么。
#[must_use]
pub(super) enum Ended {
    /// 作业原本在排队或运行，现在已经结束（或者在等待时间内没有结束）。
    Cancelled,
    /// 作业早已结束：把它忘掉。
    Forgotten,
}

impl Jobs {
    pub(super) fn new() -> Self {
        Self {
            list: Mutex::new(VecDeque::new()),
        }
    }

    pub(super) fn get(&self, id: &str) -> Option<Arc<Job>> {
        self.list
            .lock_or_panic()
            .iter()
            .find(|j| j.id == id)
            .cloned()
    }

    pub(super) fn all(&self) -> Vec<Arc<Job>> {
        self.list.lock_or_panic().iter().cloned().collect()
    }

    /// 已结束的作业超过 [`KEEP_FINISHED`] 个时，忘掉最早的几个，并删除服务端为它们选定的文件夹。
    fn prune(&self) {
        let mut list = self.list.lock_or_panic();
        let mut finished = list.iter().filter(|j| j.status().finished()).count();
        list.retain(|j| {
            if finished > KEEP_FINISHED && j.status().finished() {
                finished -= 1;
                j.remove_folder();
                false
            } else {
                true
            }
        });
    }

    /// 让 `args`（`weflow` 之后的部分）排队，`limit` 有空位时在子进程中运行，返回新的作业。
    /// 带 `file_out`（`?return=file`）时，作业保留命令写出的文件，供 `GET /jobs/{id}/file` 取得。
    pub(super) fn start(
        self: &Arc<Self>,
        exe: PathBuf,
        args: Vec<String>,
        lang: &'static str,
        limit: Arc<Semaphore>,
        file_out: Option<super::FileOut>,
    ) -> Arc<Job> {
        let command = std::iter::once("weflow".to_string())
            .chain(args.iter().cloned())
            .collect();
        let job = Arc::new(Job::new(command, file_out));
        self.list.lock_or_panic().push_back(job.clone());
        let jobs = self.clone();
        let running = job.clone();
        let cancelled = job.cancel.subscribe();
        tokio::spawn(async move {
            run(&running, limit, cancelled, |job, cancelled| {
                run_child(job, exe, args, lang, cancelled)
            })
            .await;
            jobs.prune();
        });
        job
    }

    /// 取消排队或运行中的作业，最多等 10 秒让它结束；作业已经结束时忘掉它，并删除服务端为它选定的文件夹。
    pub(super) async fn cancel_or_forget(&self, job: &Arc<Job>) -> Ended {
        let mut events = job.sender.subscribe();
        if job.status().finished() {
            self.list.lock_or_panic().retain(|j| !Arc::ptr_eq(j, job));
            job.remove_folder();
            return Ended::Forgotten;
        }
        let _ = job.cancel.send(true);
        // 子进程被终止（自行结束的命令有 `CLEAN_UP` 的时间自己结束）；等作业记下它的结束
        let wait = async {
            while !job.status().finished() {
                match events.recv().await {
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), wait).await;
        Ended::Cancelled
    }
}

/// 被取消的作业的结果：`user_interrupt` 错误。
pub(super) fn cancelled_result() -> Value {
    json!({
        "success": false,
        "error": { "code": "user_interrupt", "message": "the job was cancelled" }
    })
}

/// 作业的一生：排队直到 `limit` 给出空位，然后运行 `work`，再结束；任何时候都可能被取消。
/// `work` 自己结束作业；运行中的取消也由 `work` 处理，它从 `cancelled` 得知取消。
async fn run<'a, W, F>(
    job: &'a Job,
    limit: Arc<Semaphore>,
    mut cancelled: watch::Receiver<bool>,
    work: W,
) where
    W: FnOnce(&'a Job, watch::Receiver<bool>) -> F,
    F: std::future::Future<Output = ()> + 'a,
{
    #[expect(clippy::expect_used, reason = "作业用的信号量从不关闭")]
    let _place = tokio::select! {
        place = limit.acquire_owned() => place.expect("the semaphore is never closed"),
        _ = cancelled.wait_for(|c| *c) => {
            job.finish(Status::Cancelled, None, 499, cancelled_result());
            return;
        }
    };
    if !job.set_running() {
        return;
    }
    work(job, cancelled).await;
}

/// 在子进程中运行作业的命令：stderr 的每一行成为作业的事件，stdout 成为结果。取消时终止进程；
/// 自行结束的命令先关闭它的 stdin，[`CLEAN_UP`] 内没有结束才终止。
async fn run_child(
    job: &Job,
    exe: PathBuf,
    args: Vec<String>,
    lang: &'static str,
    mut cancelled: watch::Receiver<bool>,
) {
    let mut child = match super::child::spawn(&exe, &args, lang).await {
        Ok(c) => c,
        Err(err) => {
            let doc = json!({
                "success": false,
                "error": { "code": "runtime_error", "message": format!("failed to run weflow: {err}") }
            });
            job.finish(Status::Failed, None, 500, doc);
            return;
        }
    };
    #[expect(clippy::expect_used, reason = "`spawn` 为 stdout 和 stderr 建了管道")]
    let (mut stdout, stderr) = (
        child.stdout.take().expect("stdout is piped"),
        child.stderr.take().expect("stderr is piped"),
    );
    // `Child::wait` 会先关闭 stdin，这会让自行结束的命令（`key db`，见 `spawn`）立即结束：
    // 所以先取出 stdin，保留到作业被取消
    let stdin = child.stdin.take();
    let out = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf).await;
        buf
    });
    // stderr 在单独的任务里读取，各行交到这里处理，这里可以借用 `job`
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let reader = tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(stderr);
        let mut line = Vec::new();
        let mut tail = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&line).into_owned();
                    if tail.len() == 20 {
                        tail.remove(0);
                    }
                    tail.push(text.clone());
                    let _ = tx.send(text);
                }
            }
        }
        tail.concat()
    });
    let forward = async {
        while let Some(line) = rx.recv().await {
            job.stderr_line(&line);
        }
    };
    let waited = tokio::select! {
        (status, ()) = async { tokio::join!(child.wait(), forward) } => Some(status),
        _ = cancelled.wait_for(|c| *c) => None,
    };
    let Some(status) = waited else {
        // 让自行结束的命令自己结束（#287），过一段时间仍没有结束才终止它
        let asked = stdin.is_some();
        drop(stdin);
        if !asked || tokio::time::timeout(CLEAN_UP, child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
        job.finish(Status::Cancelled, None, 499, cancelled_result());
        return;
    };
    let stdout = out.await.unwrap_or_default();
    let stderr_tail = reader.await.unwrap_or_default();
    // 被信号终止时没有退出码
    let exit = status.ok().and_then(|s| s.code()).unwrap_or(-1);
    match serde_json::from_slice::<Value>(&stdout) {
        Ok(doc) => {
            let http = status_for(&doc, exit);
            let state = if http == axum::http::StatusCode::OK {
                Status::Succeeded
            } else {
                Status::Failed
            };
            job.finish(state, Some(exit), http.as_u16(), doc);
        }
        Err(_) => {
            let doc = json!({
                "success": false,
                "error": {
                    "code": "runtime_error",
                    "message": "the command did not print a JSON document",
                    "details": { "exitCode": exit, "stderr": stderr_tail }
                }
            });
            job.finish(Status::Failed, Some(exit), 500, doc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> Job {
        let job = Job::new(vec!["weflow".into()], None);
        assert!(job.set_running());
        job
    }

    fn succeeded(data: i64) -> Value {
        json!({ "success": true, "data": data })
    }

    #[test]
    fn stderr_lines_become_events() {
        let job = job();
        job.stderr_line(r#"{"type":"progress","stage":"s","current":1,"total":2}"#);
        job.stderr_line("warning: something\n");
        job.stderr_line("\n");
        job.stderr_line(r#"{"no":"type"}"#);
        let st = job.state.lock().unwrap();
        let names: Vec<&str> = st.events.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["status", "progress", "stderr", "message"]);
        assert_eq!(st.progress.as_ref().unwrap()["current"], 1);
        assert_eq!(
            serde_json::from_str::<Value>(&st.events[2].data).unwrap()["text"],
            "warning: something"
        );
        assert_eq!(
            st.events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn events_are_capped() {
        let job = job();
        for i in 0..KEEP_EVENTS + 5 {
            job.stderr_line(&format!("line {i}"));
        }
        let st = job.state.lock().unwrap();
        assert_eq!(st.events.len(), KEEP_EVENTS);
        assert_eq!(st.events.front().unwrap().seq, 7);
    }

    #[test]
    fn a_finished_job_reports_its_result() {
        let job = job();
        assert!(job.finish(Status::Succeeded, Some(0), 200, succeeded(1)));
        let doc = job.to_json_with_result();
        assert_eq!(doc["status"], "succeeded");
        assert_eq!(doc["result"]["data"], 1);
        assert!(job.to_json().get("result").is_none());
        let st = job.state.lock().unwrap();
        let last = st.events.back().unwrap();
        assert!(last.is_result());
        assert_eq!(
            serde_json::from_str::<Value>(&last.data).unwrap()["status"],
            "succeeded"
        );
    }

    /// 结束状态只写一次：之后的第二次结束、迟到的进度行或取消都不改变它。
    #[test]
    fn the_end_of_a_job_is_written_once() {
        let job = job();
        job.stderr_line(r#"{"type":"progress","current":1}"#);
        assert!(job.finish(Status::Succeeded, Some(0), 200, succeeded(1)));
        let ended = job.to_json_with_result();
        let events = job.state.lock().unwrap().events.len();
        assert!(!job.finish(Status::Cancelled, None, 499, cancelled_result()));
        assert!(!job.finish(Status::Failed, Some(1), 500, succeeded(2)));
        job.stderr_line(r#"{"type":"progress","current":2}"#);
        job.stderr_line("late text");
        assert!(!job.set_running());
        assert_eq!(job.to_json_with_result(), ended);
        assert_eq!(job.state.lock().unwrap().events.len(), events);
    }

    /// 订阅者不论何时订阅都能收到 `result` 事件，在作业结束的那一刻订阅也一样（#396 的风险 1）：
    /// 另一个线程结束作业时反复订阅，发现作业已结束的订阅在保存的事件中有结果，其余的订阅把它作为新事件收到。
    #[test]
    fn a_subscriber_gets_the_result_however_close_to_the_end_it_comes() {
        for _ in 0..200 {
            let job = Arc::new(job());
            let go = Arc::new(std::sync::Barrier::new(2));
            let (ending, set_off) = (job.clone(), go.clone());
            let end = std::thread::spawn(move || {
                set_off.wait();
                ending.finish(Status::Succeeded, Some(0), 200, succeeded(1));
            });
            go.wait();
            let mut waiting = Vec::new();
            loop {
                let Subscription {
                    replay,
                    live,
                    ended,
                } = job.subscribe(0);
                if ended {
                    assert!(
                        replay.iter().any(Event::is_result),
                        "an ended job without its result: {} events kept",
                        replay.len()
                    );
                    break;
                }
                waiting.push(live);
            }
            end.join().unwrap();
            for mut live in waiting {
                let mut got = false;
                while let Ok(event) = live.try_recv() {
                    got |= event.is_result();
                }
                assert!(got, "a subscriber of the running job missed the result");
            }
        }
    }

    #[tokio::test]
    async fn a_queued_job_is_cancelled_before_it_runs_and_then_forgotten() {
        let jobs = Arc::new(Jobs::new());
        // 没有空位：作业排队等待
        let limit = Arc::new(Semaphore::new(0));
        let job = jobs.start(
            PathBuf::from("weflow-that-is-not-there"),
            vec!["export".into()],
            "en",
            limit,
            None,
        );
        tokio::task::yield_now().await;
        assert_eq!(job.status(), Status::Queued);
        assert!(matches!(
            jobs.cancel_or_forget(&job).await,
            Ended::Cancelled
        ));
        let doc = job.to_json_with_result();
        assert_eq!(doc["status"], "cancelled");
        assert_eq!(doc["httpStatus"], 499);
        assert_eq!(doc["startedAt"], Value::Null);
        assert!(matches!(
            jobs.cancel_or_forget(&job).await,
            Ended::Forgotten
        ));
        assert!(jobs.get(&job.id).is_none());
        assert_eq!(job.to_json_with_result(), doc);
    }

    #[tokio::test]
    async fn a_command_that_cannot_start_fails_its_job() {
        let jobs = Arc::new(Jobs::new());
        let job = jobs.start(
            PathBuf::from("weflow-that-is-not-there"),
            vec!["export".into()],
            "en",
            Arc::new(Semaphore::new(1)),
            None,
        );
        let mut live = job.subscribe(0).live;
        let ended = async {
            while !job.status().finished() {
                let _ = live.recv().await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), ended)
            .await
            .expect("the job ends");
        let doc = job.to_json_with_result();
        assert_eq!(doc["status"], "failed");
        assert_eq!(doc["result"]["error"]["code"], "runtime_error");
    }
}
