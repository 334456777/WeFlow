//! `/jobs`, `/jobs/{id}`, `/jobs/{id}/events` and `/jobs/{id}/file`: the jobs of [`super::jobs`] over HTTP, as JSON,
//! server-sent events and the file a job wrote. Only how they are shown is here; what a job does is in `jobs`.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{header, Method, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use super::jobs::{Event, Job, Jobs, Subscription};
use super::{error_response, json_body, method_not_allowed, Verb};

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
        format!("/jobs/{}", job.id)
            .parse()
            .expect("a job id is a valid header value"),
    );
    resp
}

/// `/jobs` and what is under it; `segments` are the path's, starting with `jobs`.
pub(super) async fn route(
    jobs: &Arc<Jobs>,
    method: &Method,
    segments: &[String],
    last_event_id: Option<u64>,
) -> Response {
    match segments {
        [_] if method == Method::GET => {
            let list: Vec<Value> = jobs.all().iter().map(|j| j.to_json(false)).collect();
            success(StatusCode::OK, Value::Array(list))
        }
        [_] => method_not_allowed(&[Verb::Get]),
        [_, id] => {
            let Some(job) = jobs.get(id) else {
                return not_found();
            };
            if method == Method::GET {
                success(StatusCode::OK, job.to_json(true))
            } else if method == Method::DELETE {
                // cancelled, or forgotten when it had ended: either way the job as it ended
                let _ = jobs.cancel_or_forget(&job).await;
                success(StatusCode::OK, job.to_json(true))
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

fn sse(event: &Event) -> Bytes {
    Bytes::from(format!(
        "id: {}\nevent: {}\ndata: {}\n\n",
        event.seq, event.name, event.data
    ))
}

/// `GET /jobs/{id}/events`: the events kept so far (after `Last-Event-ID`), then the new ones until `result`.
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
            let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(25));
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
                            // fall back on the events kept in the job
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
        .expect("the event stream's headers are valid")
}
