//! Commands run in the background (`Prefer: respond-async`): `/jobs`, their progress as server-sent events, and
//! cancelling them. Jobs live in memory only; the most recent finished ones are kept for [`KEEP_FINISHED`] lookups.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::http::{header, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt};
use tokio::sync::{broadcast, watch, Semaphore};

use super::{error_response, json_body, status_for};

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

    fn finished(self) -> bool {
        matches!(self, Status::Succeeded | Status::Failed | Status::Cancelled)
    }
}

/// One server-sent event: its sequence number (the SSE `id`), name and JSON data.
#[derive(Clone, Debug)]
struct Event {
    seq: u64,
    name: String,
    data: String,
}

impl Event {
    fn to_sse(&self) -> Bytes {
        Bytes::from(format!(
            "id: {}\nevent: {}\ndata: {}\n\n",
            self.seq, self.name, self.data
        ))
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
    id: String,
    command: Vec<String>,
    created: u64,
    state: Mutex<State>,
    sender: broadcast::Sender<Event>,
    cancel: watch::Sender<bool>,
    /// Where the command writes its file, for a job started with `?return=file`.
    file_out: Option<super::FileOut>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Job {
    /// The job as JSON; `full` adds the command's result.
    fn to_json(&self, full: bool) -> Value {
        let st = self.state.lock().unwrap();
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

    fn status(&self) -> Status {
        self.state.lock().unwrap().status
    }

    /// Records an event and sends it to the subscribers.
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

    fn set_running(&self) {
        let mut st = self.state.lock().unwrap();
        st.status = Status::Running;
        st.started = Some(now_ms());
        self.emit(
            &mut st,
            "status",
            json!({ "type": "status", "status": "running" }).to_string(),
        );
    }

    /// One line the command wrote on stderr: its NDJSON events keep their `type` as the event name, any other
    /// text becomes a `stderr` event.
    fn stderr_line(&self, line: &str) {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            return;
        }
        let mut st = self.state.lock().unwrap();
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

    fn finish(&self, status: Status, exit_code: Option<i32>, http_status: u16, result: Value) {
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
        if file.is_none() {
            self.remove_folder();
        }
        let mut st = self.state.lock().unwrap();
        st.file = file;
        st.status = status;
        st.finished = Some(now_ms());
        st.exit_code = exit_code;
        st.http_status = Some(http_status);
        st.result = Some(result);
        drop(st);
        let doc = self.to_json(true).to_string();
        let mut st = self.state.lock().unwrap();
        self.emit(&mut st, "result", doc);
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

impl Jobs {
    pub(super) fn new() -> Self {
        Self {
            list: Mutex::new(VecDeque::new()),
        }
    }

    fn get(&self, id: &str) -> Option<Arc<Job>> {
        self.list
            .lock()
            .unwrap()
            .iter()
            .find(|j| j.id == id)
            .cloned()
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

    /// Queues `args` (after `weflow`) and runs them once `limit` lets it; returns the new job. With `file_out`
    /// (`?return=file`), the job keeps the file the command wrote for `GET /jobs/{id}/file`.
    pub(super) fn start(
        self: &Arc<Self>,
        exe: PathBuf,
        args: Vec<String>,
        lang: Option<&'static str>,
        limit: Arc<Semaphore>,
        file_out: Option<super::FileOut>,
    ) -> Arc<Job> {
        let (sender, _) = broadcast::channel(256);
        let (cancel, cancelled) = watch::channel(false);
        let job = Arc::new(Job {
            id: super::generate_token()[..16].to_string(),
            command: std::iter::once("weflow".to_string())
                .chain(args.iter().cloned())
                .collect(),
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
        });
        self.list.lock().unwrap().push_back(job.clone());
        let jobs = self.clone();
        let running = job.clone();
        tokio::spawn(async move {
            run(&running, &exe, &args, lang, limit, cancelled).await;
            jobs.prune();
        });
        job
    }
}

fn cancelled_result() -> Value {
    json!({
        "success": false,
        "error": { "code": "user_interrupt", "message": "the job was cancelled" }
    })
}

async fn run(
    job: &Job,
    exe: &std::path::Path,
    args: &[String],
    lang: Option<&str>,
    limit: Arc<Semaphore>,
    mut cancelled: watch::Receiver<bool>,
) {
    let _permit = tokio::select! {
        permit = limit.acquire_owned() => permit.expect("the semaphore is never closed"),
        _ = cancelled.wait_for(|c| *c) => {
            job.finish(Status::Cancelled, None, 499, cancelled_result());
            return;
        }
    };
    job.set_running();
    let mut child = match super::spawn(exe, args, lang).await {
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
            let state = if http == StatusCode::OK {
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

fn not_found() -> Response {
    error_response(StatusCode::NOT_FOUND, "not_found", "no such job")
}

fn success(status: StatusCode, data: Value) -> Response {
    json_body(status, json!({ "success": true, "data": data }).to_string())
}

/// The `202 Accepted` answer to a request that started `job`.
pub(super) fn accepted(job: &Job) -> Response {
    let mut resp = success(StatusCode::ACCEPTED, job.to_json(false));
    resp.headers_mut().insert(
        header::LOCATION,
        format!("/jobs/{}", job.id).parse().unwrap(),
    );
    resp
}

/// `/jobs`, `/jobs/{id}` and `/jobs/{id}/events`.
pub(super) async fn route(
    jobs: &Arc<Jobs>,
    method: &axum::http::Method,
    segments: &[String],
    last_event_id: Option<u64>,
) -> Response {
    use axum::http::Method;
    match segments {
        [_] if method == Method::GET => {
            let list: Vec<Value> = jobs
                .list
                .lock()
                .unwrap()
                .iter()
                .map(|j| j.to_json(false))
                .collect();
            success(StatusCode::OK, Value::Array(list))
        }
        [_] => super::method_not_allowed(&[super::Verb::Get]),
        [_, id] => {
            let Some(job) = jobs.get(id) else {
                return not_found();
            };
            if method == Method::GET {
                success(StatusCode::OK, job.to_json(true))
            } else if method == Method::DELETE {
                cancel_or_forget(jobs, &job).await
            } else {
                super::method_not_allowed(&[super::Verb::Get, super::Verb::Delete])
            }
        }
        [_, id, events] if events == "events" => {
            let Some(job) = jobs.get(id) else {
                return not_found();
            };
            if method != Method::GET {
                return super::method_not_allowed(&[super::Verb::Get]);
            }
            event_stream(job, last_event_id.unwrap_or(0))
        }
        [_, id, file] if file == "file" => {
            let Some(job) = jobs.get(id) else {
                return not_found();
            };
            if method != Method::GET {
                return super::method_not_allowed(&[super::Verb::Get]);
            }
            let path = job.state.lock().unwrap().file.clone();
            match path {
                Some(path) => match super::files::attachment(&path).await {
                    Ok(resp) => resp,
                    Err(message) => {
                        error_response(StatusCode::NOT_FOUND, "not_found", &message)
                    }
                },
                None => error_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "the job has no file: it was not started with return=file, or has not succeeded",
                ),
            }
        }
        _ => error_response(StatusCode::NOT_FOUND, "not_found", "no such job"),
    }
}

/// `DELETE /jobs/{id}`: cancels a queued or running job (and waits for it to stop), or forgets a finished one.
async fn cancel_or_forget(jobs: &Jobs, job: &Arc<Job>) -> Response {
    if job.status().finished() {
        jobs.list.lock().unwrap().retain(|j| !Arc::ptr_eq(j, job));
        job.remove_folder();
        return success(StatusCode::OK, job.to_json(true));
    }
    let mut rx = job.sender.subscribe();
    let _ = job.cancel.send(true);
    // the child is killed right away; wait for the job to record it
    let wait = async {
        while !job.status().finished() {
            match rx.recv().await {
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), wait).await;
    success(StatusCode::OK, job.to_json(true))
}

/// `GET /jobs/{id}/events`: the events kept so far (after `Last-Event-ID`), then the new ones until `result`.
fn event_stream(job: Arc<Job>, after: u64) -> Response {
    let (replay, mut rx, done) = {
        let st = job.state.lock().unwrap();
        let replay: Vec<Event> = st
            .events
            .iter()
            .filter(|e| e.seq > after)
            .cloned()
            .collect();
        (replay, job.sender.subscribe(), st.status.finished())
    };
    let stream = async_stream::stream! {
        let mut highest = after;
        for event in replay {
            highest = event.seq;
            yield Ok::<_, std::convert::Infallible>(event.to_sse());
        }
        if !done {
            let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(25));
            heartbeat.tick().await;
            loop {
                tokio::select! {
                    msg = rx.recv() => match msg {
                        Ok(event) => {
                            if event.seq <= highest {
                                continue;
                            }
                            highest = event.seq;
                            let last = event.name == "result";
                            yield Ok(event.to_sse());
                            if last {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            // fall back on the events kept in the job
                            let missed: Vec<Event> = job
                                .state
                                .lock()
                                .unwrap()
                                .events
                                .iter()
                                .filter(|e| e.seq > highest)
                                .cloned()
                                .collect();
                            let mut last = false;
                            for event in missed {
                                highest = event.seq;
                                last = event.name == "result";
                                yield Ok(event.to_sse());
                            }
                            if last {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    },
                    _ = heartbeat.tick() => yield Ok(Bytes::from_static(b": ping\n\n")),
                }
            }
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache, no-transform")
        .header("X-Accel-Buffering", "no")
        .body(Body::from_stream(stream))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> Job {
        let (sender, _) = broadcast::channel(4);
        let (cancel, _) = watch::channel(false);
        Job {
            id: "j".into(),
            command: vec!["weflow".into()],
            created: 0,
            state: Mutex::new(State {
                status: Status::Running,
                started: Some(1),
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
            file_out: None,
        }
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
        assert_eq!(names, ["progress", "stderr", "message"]);
        assert_eq!(st.progress.as_ref().unwrap()["current"], 1);
        assert_eq!(
            serde_json::from_str::<Value>(&st.events[1].data).unwrap()["text"],
            "warning: something"
        );
        assert_eq!(
            st.events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [1, 2, 3]
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
        assert_eq!(st.events.front().unwrap().seq, 6);
    }

    #[test]
    fn a_finished_job_reports_its_result() {
        let job = job();
        job.finish(
            Status::Succeeded,
            Some(0),
            200,
            json!({ "success": true, "data": 1 }),
        );
        let doc = job.to_json(true);
        assert_eq!(doc["status"], "succeeded");
        assert_eq!(doc["result"]["data"], 1);
        assert!(job.to_json(false).get("result").is_none());
        let st = job.state.lock().unwrap();
        let last = st.events.back().unwrap();
        assert_eq!(last.name, "result");
        assert_eq!(
            serde_json::from_str::<Value>(&last.data).unwrap()["status"],
            "succeeded"
        );
    }
}
