//! Commands run in the background (`Prefer: respond-async`): their lifecycle, apart from HTTP (#398). A job is queued,
//! runs once it has a place, and ends succeeded, failed or cancelled; its end is written once, with its `result`
//! event, and nothing that comes later (a late progress line, a second cancel) changes it. Its events are kept for
//! clients that subscribe late or reconnect. The routes that show jobs are in [`super::job_routes`]. Jobs live in
//! memory only; the most recent finished ones are kept for [`KEEP_FINISHED`] lookups.
//!
//! A job runs its command in a child process (see [`run_child`]): a long task or an export that crashes does not take
//! the server down, and cancelling it kills the process, which frees its memory and files for sure.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt};
use tokio::sync::{broadcast, watch, Semaphore};

use super::status_for;

/// Finished jobs kept for `GET /jobs`; older ones are forgotten.
const KEEP_FINISHED: usize = 100;

/// Events kept per job for clients that subscribe late or reconnect.
const KEEP_EVENTS: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Queued => "queued",
            Status::Running => "running",
            Status::Succeeded => "succeeded",
            Status::Failed => "failed",
            Status::Cancelled => "cancelled",
        }
    }

    pub(super) fn finished(self) -> bool {
        matches!(self, Status::Succeeded | Status::Failed | Status::Cancelled)
    }
}

/// One event of a job: its sequence number (the SSE `id`), name and JSON data.
#[derive(Clone, Debug)]
pub(super) struct Event {
    pub seq: u64,
    pub name: String,
    pub data: String,
}

impl Event {
    /// The last event of a job.
    pub(super) fn is_result(&self) -> bool {
        self.name == "result"
    }
}

struct State {
    status: Status,
    started: Option<u64>,
    finished: Option<u64>,
    exit_code: Option<i32>,
    http_status: Option<u16>,
    /// The last `progress` event.
    progress: Option<Value>,
    /// The `--json` document of the command (or the server's own error document).
    result: Option<Value>,
    events: VecDeque<Event>,
    next_seq: u64,
    /// The file the command wrote, for a job started with `?return=file` that succeeded.
    file: Option<PathBuf>,
}

pub(super) struct Job {
    pub id: String,
    command: Vec<String>,
    created: u64,
    state: Mutex<State>,
    sender: broadcast::Sender<Event>,
    cancel: watch::Sender<bool>,
    /// Where the command writes its file, for a job started with `?return=file`.
    file_out: Option<super::FileOut>,
}

/// What a subscriber of a job's events gets: the events kept after the one it last saw, then the new ones from
/// `live` unless the job had ended.
pub(super) struct Subscription {
    pub replay: Vec<Event>,
    pub live: broadcast::Receiver<Event>,
    pub ended: bool,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
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

    /// The job as JSON; `full` adds the command's result.
    pub(super) fn to_json(&self, full: bool) -> Value {
        self.json_of(&self.state.lock().unwrap(), full)
    }

    fn json_of(&self, st: &State, full: bool) -> Value {
        let mut out = json!({
            "id": self.id,
            "status": st.status.name(),
            "command": self.command,
            "createdAt": self.created,
            "startedAt": st.started,
            "finishedAt": st.finished,
            "exitCode": st.exit_code,
            "httpStatus": st.http_status,
            "progress": st.progress,
            "file": st.file.as_ref().map(|path| json!({
                "name": path.file_name().map(|n| n.to_string_lossy()),
                "size": std::fs::metadata(path).map(|m| m.len()).ok(),
            })),
        });
        if full {
            out["result"] = st.result.clone().unwrap_or(Value::Null);
        }
        out
    }

    pub(super) fn status(&self) -> Status {
        self.state.lock().unwrap().status
    }

    /// The file the job wrote, once it succeeded with one.
    pub(super) fn file(&self) -> Option<PathBuf> {
        self.state.lock().unwrap().file.clone()
    }

    /// Records an event and sends it to the subscribers, under the lock of `st`, so a subscriber sees each event
    /// either among those kept or as a new one.
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
        // no subscriber is fine
        let _ = self.sender.send(event);
    }

    /// Queued → running; a job cancelled meanwhile stays cancelled.
    fn set_running(&self) -> bool {
        let mut st = self.state.lock().unwrap();
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

    /// One line the command wrote on stderr: its NDJSON events keep their `type` as the event name, any other
    /// text becomes a `stderr` event. A line that comes after the end is dropped.
    fn stderr_line(&self, line: &str) {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            return;
        }
        let mut st = self.state.lock().unwrap();
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

    /// Ends the job, once: the status, the result and the `result` event under one lock. `false` when it had ended
    /// already, and nothing changes.
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
        let mut st = self.state.lock().unwrap();
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

    /// The events after `after` (an SSE `Last-Event-ID`), and the new ones.
    pub(super) fn subscribe(&self, after: u64) -> Subscription {
        let st = self.state.lock().unwrap();
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

    /// The events kept after `after`, for a subscriber that fell behind.
    pub(super) fn kept_after(&self, after: u64) -> Vec<Event> {
        let st = self.state.lock().unwrap();
        st.events
            .iter()
            .filter(|e| e.seq > after)
            .cloned()
            .collect()
    }

    /// Removes the folder the server chose for the job's file (`serve-files/<id>/`); a given `out` stays.
    fn remove_folder(&self) {
        if let Some(dir) = self.file_out.as_ref().and_then(|f| f.dir.as_ref()) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

pub(super) struct Jobs {
    list: Mutex<VecDeque<Arc<Job>>>,
}

/// What [`Jobs::cancel_or_forget`] did.
#[must_use]
pub(super) enum Ended {
    /// The job was queued or running, and has ended (or did not end within the wait).
    Cancelled,
    /// The job had ended: it is forgotten.
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
            .lock()
            .unwrap()
            .iter()
            .find(|j| j.id == id)
            .cloned()
    }

    pub(super) fn all(&self) -> Vec<Arc<Job>> {
        self.list.lock().unwrap().iter().cloned().collect()
    }

    /// Forgets the oldest finished jobs beyond [`KEEP_FINISHED`].
    fn prune(&self) {
        let mut list = self.list.lock().unwrap();
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

    /// Queues `args` (after `weflow`) and runs them in a child process once `limit` lets it; returns the new job.
    /// With `file_out` (`?return=file`), the job keeps the file the command wrote for `GET /jobs/{id}/file`.
    pub(super) fn start(
        self: &Arc<Self>,
        exe: PathBuf,
        args: Vec<String>,
        lang: Option<&'static str>,
        limit: Arc<Semaphore>,
        file_out: Option<super::FileOut>,
    ) -> Arc<Job> {
        let command = std::iter::once("weflow".to_string())
            .chain(args.iter().cloned())
            .collect();
        let job = Arc::new(Job::new(command, file_out));
        self.list.lock().unwrap().push_back(job.clone());
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

    /// Cancels a queued or running job, waiting (10 s at most) for it to end, or forgets a job that had ended.
    pub(super) async fn cancel_or_forget(&self, job: &Arc<Job>) -> Ended {
        let mut events = job.sender.subscribe();
        if job.status().finished() {
            self.list.lock().unwrap().retain(|j| !Arc::ptr_eq(j, job));
            job.remove_folder();
            return Ended::Forgotten;
        }
        let _ = job.cancel.send(true);
        // the child is killed right away; wait for the job to record it
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

fn cancelled_result() -> Value {
    json!({
        "success": false,
        "error": { "code": "user_interrupt", "message": "the job was cancelled" }
    })
}

/// A job's life: queued until `limit` gives it a place, then running `work`, then ended, or cancelled at any point.
/// `work` ends the job itself; cancelling it is up to `work` too, which hears of it from `cancelled`.
async fn run<'a, W, F>(
    job: &'a Job,
    limit: Arc<Semaphore>,
    mut cancelled: watch::Receiver<bool>,
    work: W,
) where
    W: FnOnce(&'a Job, watch::Receiver<bool>) -> F,
    F: std::future::Future<Output = ()> + 'a,
{
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

/// Runs the job's command in a child process: its stderr lines become the job's events, its stdout the result. A
/// cancel kills the process.
async fn run_child(
    job: &Job,
    exe: PathBuf,
    args: Vec<String>,
    lang: Option<&'static str>,
    mut cancelled: watch::Receiver<bool>,
) {
    let mut child = match super::spawn(&exe, &args, lang).await {
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
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let out = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf).await;
        buf
    });
    // stderr is read in a task of its own and its lines are handed over here, where `job` can be borrowed
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
        let _ = child.kill().await;
        job.finish(Status::Cancelled, None, 499, cancelled_result());
        return;
    };
    let stdout = out.await.unwrap_or_default();
    let stderr_tail = reader.await.unwrap_or_default();
    // killed by a signal: no exit code
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
        let doc = job.to_json(true);
        assert_eq!(doc["status"], "succeeded");
        assert_eq!(doc["result"]["data"], 1);
        assert!(job.to_json(false).get("result").is_none());
        let st = job.state.lock().unwrap();
        let last = st.events.back().unwrap();
        assert!(last.is_result());
        assert_eq!(
            serde_json::from_str::<Value>(&last.data).unwrap()["status"],
            "succeeded"
        );
    }

    /// The end is written once: a second end, a late progress line or a cancel after it change nothing.
    #[test]
    fn the_end_of_a_job_is_written_once() {
        let job = job();
        job.stderr_line(r#"{"type":"progress","current":1}"#);
        assert!(job.finish(Status::Succeeded, Some(0), 200, succeeded(1)));
        let ended = job.to_json(true);
        let events = job.state.lock().unwrap().events.len();
        assert!(!job.finish(Status::Cancelled, None, 499, cancelled_result()));
        assert!(!job.finish(Status::Failed, Some(1), 500, succeeded(2)));
        job.stderr_line(r#"{"type":"progress","current":2}"#);
        job.stderr_line("late text");
        assert!(!job.set_running());
        assert_eq!(job.to_json(true), ended);
        assert_eq!(job.state.lock().unwrap().events.len(), events);
    }

    /// A subscriber gets the `result` event whenever it subscribes, also at the moment the job ends (#396, risk 1):
    /// subscribing over and over while another thread ends the job, each subscription that finds the job ended has
    /// the result among the events kept, and the others get it as a new event.
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
        // no place: the job waits in line
        let limit = Arc::new(Semaphore::new(0));
        let job = jobs.start(
            PathBuf::from("weflow-that-is-not-there"),
            vec!["export".into()],
            None,
            limit,
            None,
        );
        tokio::task::yield_now().await;
        assert_eq!(job.status(), Status::Queued);
        assert!(matches!(
            jobs.cancel_or_forget(&job).await,
            Ended::Cancelled
        ));
        let doc = job.to_json(true);
        assert_eq!(doc["status"], "cancelled");
        assert_eq!(doc["httpStatus"], 499);
        assert_eq!(doc["startedAt"], Value::Null);
        assert!(matches!(
            jobs.cancel_or_forget(&job).await,
            Ended::Forgotten
        ));
        assert!(jobs.get(&job.id).is_none());
        assert_eq!(job.to_json(true), doc);
    }

    #[tokio::test]
    async fn a_command_that_cannot_start_fails_its_job() {
        let jobs = Arc::new(Jobs::new());
        let job = jobs.start(
            PathBuf::from("weflow-that-is-not-there"),
            vec!["export".into()],
            None,
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
        let doc = job.to_json(true);
        assert_eq!(doc["status"], "failed");
        assert_eq!(doc["result"]["error"]["code"], "runtime_error");
    }
}
