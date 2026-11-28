//! `/jobs`、`/jobs/{id}`、`/jobs/{id}/events` 和 `/jobs/{id}/file`：通过 HTTP 提供 [`super::jobs`] 的作业，
//! 形式为 JSON、服务器发送事件和作业写出的文件。这里只管作业怎样呈现，作业做什么在 `jobs` 里。

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use super::http::{error_response, json_body, method_not_allowed};
use super::jobs::{Event, Job, Jobs, Subscription};
use super::rules::Verb;

fn not_found() -> Response {
    error_response(StatusCode::NOT_FOUND, "not_found", "no such job")
}

fn success(status: StatusCode, data: Value) -> Response {
    json_body(status, json!({ "success": true, "data": data }).to_string())
}

/// 启动了 `job` 的请求的 `202 Accepted` 回答，`Location` 指向这个作业。
pub(super) fn accepted(job: &Job) -> Response {
    let mut resp = success(StatusCode::ACCEPTED, job.to_json());
    #[expect(clippy::expect_used, reason = "作业 ID 由十六进制数字组成")]
    let location = HeaderValue::from_str(&format!("/jobs/{}", job.id))
        .expect("a job id is a valid header value");
    resp.headers_mut().insert(header::LOCATION, location);
    resp
}

/// `/jobs` 及其下的路由；`segments` 是路径的各段，以 `jobs` 开头。
pub(super) async fn route(
    jobs: &Arc<Jobs>,
    method: &Method,
    segments: &[String],
    last_event_id: Option<u64>,
) -> Response {
    match segments {
        [_] if method == Method::GET => {
            let list: Vec<Value> = jobs.all().iter().map(|j| j.to_json()).collect();
            success(StatusCode::OK, Value::Array(list))
        }
        [_] => method_not_allowed(&[Verb::Get]),
        [_, id] => {
            let Some(job) = jobs.get(id) else {
                return not_found();
            };
            if method == Method::GET {
                success(StatusCode::OK, job.to_json_with_result())
            } else if method == Method::DELETE {
                // 作业还在运行时取消它，已经结束时忘掉它：两种情况都回答作业结束时的样子
                let _ = jobs.cancel_or_forget(&job).await;
                success(StatusCode::OK, job.to_json_with_result())
            } else {
                method_not_allowed(&[Verb::Get, Verb::Delete])
            }
        }
        [_, id, events] if events == "events" => {
            let Some(job) = jobs.get(id) else {
                return not_found();
            };
            if method != Method::GET {
                return method_not_allowed(&[Verb::Get]);
            }
            event_stream(job, last_event_id.unwrap_or(0))
        }
        [_, id, file] if file == "file" => {
            let Some(job) = jobs.get(id) else {
                return not_found();
            };
            if method != Method::GET {
                return method_not_allowed(&[Verb::Get]);
            }
            match job.file() {
                Some(path) => match super::files::attachment(&path).await {
                    Ok(resp) => resp,
                    Err(message) => error_response(StatusCode::NOT_FOUND, "not_found", &message),
                },
                None => error_response(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "the job has no file: it was not started with return=file, or has not succeeded",
                ),
            }
        }
        _ => not_found(),
    }
}

/// 作业的事件流没有事件时发送的内容：与其他事件流一样是一个事件（见 `status`），但不带 id，
/// 所以不改变 `Last-Event-ID`。
const HEARTBEAT: &[u8] = b"event: heartbeat\ndata: {}\n\n";

/// 把作业的事件写成服务器发送事件，序号作为 id。
fn sse(event: &Event) -> Bytes {
    Bytes::from(format!(
        "id: {}\nevent: {}\ndata: {}\n\n",
        event.seq, event.name, event.data
    ))
}

/// `GET /jobs/{id}/events`：先发送已经保存的事件（`Last-Event-ID` 之后的），再发送新的事件，直到 `result`；
/// 没有事件时发送 `heartbeat`。
fn event_stream(job: Arc<Job>, after: u64) -> Response {
    let Subscription {
        replay,
        mut live,
        ended,
    } = job.subscribe(after);
    let stream = async_stream::stream! {
        let mut highest = after;
        for event in replay {
            highest = event.seq;
            yield Ok::<_, std::convert::Infallible>(sse(&event));
        }
        if !ended {
            let mut heartbeat = tokio::time::interval(super::status::heartbeat());
            heartbeat.tick().await;
            loop {
                tokio::select! {
                    msg = live.recv() => match msg {
                        Ok(event) => {
                            if event.seq <= highest {
                                continue;
                            }
                            highest = event.seq;
                            yield Ok(sse(&event));
                            if event.is_result() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            // 改从作业保存的事件中补发
                            let mut last = false;
                            for event in job.kept_after(highest) {
                                highest = event.seq;
                                last = event.is_result();
                                yield Ok(sse(&event));
                            }
                            if last {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    },
                    _ = heartbeat.tick() => yield Ok(Bytes::from_static(HEARTBEAT)),
                }
            }
        }
    };
    super::status::event_stream(Body::from_stream(stream))
}
