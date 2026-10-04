//! The local HTTP API (`electron/services/httpService.ts`): ChatLab-style message queries,
//! Moments, media files and the message push stream.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use axum::Router;
use serde_json::{json, Value};

use crate::api::{self, ApiError, Params};
use crate::push::PushBroker;
use crate::services::{ServiceHub, SnsExportOptions, SnsTimelineQuery};

#[derive(Clone, Debug)]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
    /// `httpApiToken`; with no token every request except `/health` is refused.
    pub token: Option<String>,
    /// `messagePushEnabled`
    pub push_enabled: bool,
}

#[derive(Clone)]
struct AppState {
    hub: Arc<ServiceHub>,
    cfg: Arc<HttpConfig>,
    broker: Arc<PushBroker>,
}

const MAX_BODY: usize = 10 * 1024 * 1024;

pub fn router(hub: ServiceHub, cfg: HttpConfig, broker: Arc<PushBroker>) -> Router {
    Router::new().fallback(dispatch).with_state(AppState {
        hub: Arc::new(hub),
        cfg: Arc::new(cfg),
        broker,
    })
}

fn json_response(status: StatusCode, value: &Value) -> Response {
    let body = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(body))
        .unwrap()
}

fn send_json(value: &Value) -> Response {
    json_response(StatusCode::OK, value)
}

fn send_error(status: u16, message: &str) -> Response {
    let body = serde_json::to_string(&json!({ "error": message })).unwrap();
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(body))
        .unwrap()
}

fn method_not_allowed(allow: &str) -> Response {
    let mut r = send_error(405, &format!("Method Not Allowed. Allowed: {allow}"));
    r.headers_mut()
        .insert(header::ALLOW, HeaderValue::from_str(allow).unwrap());
    r
}

fn from_api(r: Result<Value, ApiError>) -> Response {
    match r {
        Ok(v) => send_json(&v),
        Err(e) => send_error(e.status, &e.message),
    }
}

fn safe_equal(a: &str, b: &str) -> bool {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    if x.len() != y.len() {
        return false;
    }
    x.iter().zip(y).fold(0u8, |acc, (p, q)| acc | (p ^ q)) == 0
}

fn verify_token(cfg: &HttpConfig, headers: &HeaderMap, params: &Params) -> bool {
    let Some(expected) = cfg
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return false;
    };
    if let Some(auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if auth.to_lowercase().starts_with("bearer ") && safe_equal(auth[7..].trim(), expected) {
            return true;
        }
    }
    params
        .get("access_token")
        .is_some_and(|t| safe_equal(t.trim(), expected))
}

fn query_params(uri: &axum::http::Uri) -> Params {
    let mut out = Params::new();
    if let Some(q) = uri.query() {
        for pair in q.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            out.entry(url_decode(k)).or_insert_with(|| url_decode(v));
        }
    }
    out
}

fn url_decode(s: &str) -> String {
    String::from_utf8_lossy(&crate::message::percent_decode_bytes(&s.replace('+', " ")))
        .into_owned()
}

fn cors(headers: &HeaderMap, mut resp: Response) -> Response {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let local = crate::message::rx(r"^https?://(localhost|127\.0\.0\.1)(:\d+)?$").is_match(origin);
    let h = resp.headers_mut();
    if local {
        h.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_str(origin).unwrap(),
        );
        h.insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, Authorization"),
    );
    resp
}

async fn dispatch(State(st): State<AppState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let headers = parts.headers.clone();
    if parts.method == Method::OPTIONS {
        return cors(
            &headers,
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Body::empty())
                .unwrap(),
        );
    }
    let resp = route(&st, &parts.method, &parts.uri, &headers, body).await;
    cors(&headers, resp)
}

async fn route(
    st: &AppState,
    method: &Method,
    uri: &axum::http::Uri,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    let path = uri.path().to_string();
    let mut params = query_params(uri);
    // POST bodies are JSON objects whose keys act like query parameters
    if method == Method::POST {
        if let Ok(bytes) = to_bytes(body, MAX_BODY).await {
            if let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&bytes) {
                for (k, v) in map {
                    let text = match &v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    params.entry(k).or_insert(text);
                }
            }
        }
    }
    let is_health = path == "/health" || path == "/api/v1/health";
    if !is_health && !verify_token(&st.cfg, headers, &params) {
        return send_error(401, "Unauthorized: Invalid or missing access_token");
    }
    let base_url = format!("http://{}:{}", st.cfg.host, st.cfg.port);
    let hub = st.hub.clone();
    let get_only = |m: &Method| m != Method::GET;

    if is_health {
        return send_json(&json!({ "status": "ok" }));
    }
    match path.as_str() {
        "/api/v1/push/messages" => push_stream(st, headers, &params).await,
        "/api/v1/messages" => from_api(hub.api_messages(&params, &base_url).await),
        "/api/v1/sessions" => {
            let hub2 = hub.clone();
            from_api(blocking(move || hub2.api_sessions(&params)).await)
        }
        "/api/v1/contacts" => {
            let hub2 = hub.clone();
            from_api(blocking(move || hub2.api_contacts(&params)).await)
        }
        "/api/v1/group-members" => {
            let hub2 = hub.clone();
            from_api(blocking(move || hub2.api_group_members(&params)).await)
        }
        "/api/v1/sns/timeline" => {
            if get_only(method) {
                return method_not_allowed("GET");
            }
            sns_timeline(&hub, &params, &base_url).await
        }
        "/api/v1/sns/usernames" => {
            if get_only(method) {
                return method_not_allowed("GET");
            }
            match blocking(move || {
                hub.sns_usernames_list()
                    .map_err(|e| ApiError::new(500, e.message))
            })
            .await
            {
                Ok(u) => send_json(&json!({ "success": true, "usernames": u })),
                Err(e) => send_error(e.status, &e.message),
            }
        }
        "/api/v1/sns/export/stats" => {
            if get_only(method) {
                return method_not_allowed("GET");
            }
            let fast = api::parse_bool_param(&params, &["fast"], false);
            match blocking(move || {
                hub.sns_export_stats(fast)
                    .map_err(|e| ApiError::new(500, e.message))
            })
            .await
            {
                Ok(data) => send_json(&json!({ "success": true, "data": data })),
                Err(e) => send_error(e.status, &e.message),
            }
        }
        "/api/v1/sns/media/proxy" => {
            if get_only(method) {
                return method_not_allowed("GET");
            }
            sns_media_proxy(&hub, &params).await
        }
        "/api/v1/sns/export" => {
            if method != Method::POST {
                return method_not_allowed("POST");
            }
            sns_export(&hub, &params).await
        }
        "/api/v1/sns/block-delete/status" => {
            if get_only(method) {
                return method_not_allowed("GET");
            }
            trigger_reply(
                blocking(move || {
                    hub.sns_block_delete_status()
                        .map_err(|e| ApiError::new(500, e.message))
                })
                .await,
            )
        }
        "/api/v1/sns/block-delete/install" => {
            if method != Method::POST {
                return method_not_allowed("POST");
            }
            trigger_reply(
                blocking(move || {
                    hub.sns_block_delete_install()
                        .map_err(|e| ApiError::new(500, e.message))
                })
                .await,
            )
        }
        "/api/v1/sns/block-delete/uninstall" => {
            if method != Method::POST {
                return method_not_allowed("POST");
            }
            trigger_reply(
                blocking(move || {
                    hub.sns_block_delete_uninstall()
                        .map_err(|e| ApiError::new(500, e.message))
                })
                .await,
            )
        }
        p if p.starts_with("/api/v1/sessions/") && p.ends_with("/messages") => {
            let id = url_decode(p.split('/').nth(4).unwrap_or(""));
            if id.is_empty() {
                return send_error(400, "Missing session ID");
            }
            from_api(hub.api_pull_messages(&id, &params, &base_url).await)
        }
        p if p.starts_with("/api/v1/sns/post/") => {
            if method != Method::DELETE {
                return method_not_allowed("DELETE");
            }
            let post_id = url_decode(&p["/api/v1/sns/post/".len()..])
                .trim()
                .to_string();
            if post_id.is_empty() {
                return send_error(400, "Missing required path parameter: postId");
            }
            trigger_reply(
                blocking(move || {
                    hub.sns_delete_post(&post_id)
                        .map(|_| json!({ "success": true }))
                        .map_err(|e| ApiError::new(500, e.message))
                })
                .await,
            )
        }
        p if p.starts_with("/api/v1/media/") => media_file(
            &hub.api_media_dir(),
            &url_decode(&p["/api/v1/media/".len()..]),
        ),
        _ => send_error(404, "Not Found"),
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Err(ApiError::new(500, e.to_string())))
}

fn trigger_reply(r: Result<Value, ApiError>) -> Response {
    match r {
        Ok(v) => send_json(&v),
        Err(e) => send_error(e.status, &e.message),
    }
}

async fn sns_timeline(hub: &Arc<ServiceHub>, params: &Params, base_url: &str) -> Response {
    let limit = api::parse_int_param(params.get("limit").map(String::as_str), 20, 1, 200) as i32;
    let offset = api::parse_int_param(
        params.get("offset").map(String::as_str),
        0,
        0,
        i32::MAX as i64,
    ) as i32;
    let usernames = api::parse_string_list_param(params.get("usernames").map(String::as_str))
        .unwrap_or_default();
    let keyword = params
        .get("keyword")
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    let resolve_media = api::parse_bool_param(params, &["media", "resolveMedia", "meiti"], true);
    let inline = resolve_media && api::parse_bool_param(params, &["inline"], false);
    let replace = resolve_media && api::parse_bool_param(params, &["replace"], true);
    let start = api::parse_time_param(params.get("start").map(String::as_str), false);
    let end = api::parse_time_param(params.get("end").map(String::as_str), true);
    let q = SnsTimelineQuery {
        limit,
        offset,
        usernames,
        keyword,
        start,
        end,
    };
    let hub2 = hub.clone();
    let timeline = match blocking(move || {
        hub2.sns_timeline_query(&q)
            .map_err(|e| ApiError::new(500, e.message))
    })
    .await
    {
        Ok(t) => t,
        Err(e) => return send_error(e.status, &e.message),
    };
    let timeline = if resolve_media && !timeline.is_empty() {
        hub.sns_enrich_timeline_media(timeline, base_url, inline, replace)
            .await
    } else {
        timeline
    };
    send_json(&json!({ "success": true, "count": timeline.len(), "timeline": timeline }))
}

/// `toSnsMediaKey`: integer-looking keys go through `Number()`.
fn sns_media_key(raw: Option<&str>) -> Option<String> {
    let text = raw?.trim();
    if text.is_empty() {
        return None;
    }
    if crate::message::rx(r"^-?\d+$").is_match(text) {
        return text.parse::<f64>().ok().map(|f| {
            if f.abs() < 9.0e15 {
                format!("{}", f as i64)
            } else {
                format!("{f}")
            }
        });
    }
    Some(text.to_string())
}

async fn sns_media_proxy(hub: &Arc<ServiceHub>, params: &Params) -> Response {
    let url = params
        .get("url")
        .map(|u| u.trim().to_string())
        .unwrap_or_default();
    if url.is_empty() {
        return send_error(400, "Missing required parameter: url");
    }
    let key = sns_media_key(params.get("key").map(String::as_str));
    match hub.sns_fetch_media(&url, key.as_deref()).await {
        Err(e) => send_error(502, &e.message),
        Ok(f) => {
            let ct = if f.content_type.is_empty() {
                "application/octet-stream".to_string()
            } else {
                f.content_type.clone()
            };
            if let Some(data) = f.data {
                return Response::builder()
                    .status(200)
                    .header(header::CONTENT_TYPE, ct)
                    .header(header::CONTENT_LENGTH, data.len())
                    .body(Body::from(data))
                    .unwrap();
            }
            match f
                .cache_path
                .as_deref()
                .filter(|p| p.exists())
                .map(std::fs::read)
            {
                Some(Ok(data)) => Response::builder()
                    .status(200)
                    .header(header::CONTENT_TYPE, ct)
                    .header(header::CONTENT_LENGTH, data.len())
                    .body(Body::from(data))
                    .unwrap(),
                _ => send_error(502, "Failed to proxy sns media"),
            }
        }
    }
}

async fn sns_export(hub: &Arc<ServiceHub>, params: &Params) -> Response {
    let output_dir = params
        .get("outputDir")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if output_dir.is_empty() {
        return send_error(400, "Missing required field: outputDir");
    }
    let raw = params
        .get("format")
        .map(|f| f.trim().to_lowercase())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| "json".into());
    let format = if raw == "arkme-json" {
        "arkmejson".to_string()
    } else {
        raw
    };
    if !["json", "html", "arkmejson"].contains(&format.as_str()) {
        return send_error(400, "Invalid format, supported: json/html/arkmejson");
    }
    let flag = |k: &str| {
        params
            .contains_key(k)
            .then(|| api::parse_bool_param(params, &[k], false))
    };
    let opts = SnsExportOptions {
        output_dir: PathBuf::from(output_dir),
        format,
        usernames: api::parse_string_list_param(params.get("usernames").map(String::as_str))
            .unwrap_or_default(),
        keyword: params
            .get("keyword")
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty()),
        export_media: api::parse_bool_param(params, &["exportMedia"], false),
        export_images: flag("exportImages"),
        export_live_photos: flag("exportLivePhotos"),
        export_videos: flag("exportVideos"),
        start: api::parse_time_param(params.get("start").map(String::as_str), false),
        end: api::parse_time_param(params.get("end").map(String::as_str), true),
    };
    match hub.sns_export_timeline(&opts).await {
        Ok(v) => send_json(&v),
        Err(e) => send_error(500, &e.message),
    }
}

fn media_file(base: &Path, relative: &str) -> Response {
    let base = std::fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let mut full = base.clone();
    for comp in relative.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                if !full.pop() {
                    return send_error(403, "Forbidden");
                }
            }
            c => full.push(c),
        }
    }
    if !full.starts_with(&base) {
        return send_error(403, "Forbidden");
    }
    if !full.is_file() {
        return send_error(404, "Media not found");
    }
    let ext = full
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let content_type = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    };
    match std::fs::read(&full) {
        Ok(data) => Response::builder()
            .status(200)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CONTENT_LENGTH, data.len())
            .body(Body::from(data))
            .unwrap(),
        Err(_) => send_error(500, "Failed to read media file"),
    }
}

async fn push_stream(st: &AppState, headers: &HeaderMap, params: &Params) -> Response {
    if !st.cfg.push_enabled {
        return send_error(403, "Message push is disabled");
    }
    let last_id = params
        .get("lastEventId")
        .filter(|v| !v.is_empty())
        .or_else(|| params.get("last_event_id").filter(|v| !v.is_empty()))
        .cloned()
        .or_else(|| {
            headers
                .get("last-event-id")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        })
        .and_then(|v| api::js_parse_int(v.trim()))
        .filter(|n| *n > 0)
        .unwrap_or(0) as u64;
    let mut rx = st.broker.subscribe();
    let replay = st.broker.replay_since(last_id);
    let stream_url = format!(
        "http://{}:{}/api/v1/push/messages",
        st.cfg.host, st.cfg.port
    );
    let stream = async_stream::stream! {
        let ready = format!("event: ready\ndata: {}\n\n", serde_json::to_string(&json!({ "success": true, "stream": stream_url })).unwrap());
        yield Ok::<_, Infallible>(axum::body::Bytes::from(ready));
        let mut highest = 0u64;
        for (id, body) in replay {
            highest = highest.max(id);
            yield Ok(axum::body::Bytes::from(body.as_bytes().to_vec()));
        }
        let mut heartbeat = tokio::time::interval(Duration::from_secs(25));
        heartbeat.tick().await;
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Ok((id, body)) => {
                        if id <= highest { continue; }
                        highest = id;
                        yield Ok(axum::body::Bytes::from(body.as_bytes().to_vec()));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                },
                _ = heartbeat.tick() => yield Ok(axum::body::Bytes::from_static(b": ping\n\n")),
            }
        }
    };
    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache, no-transform")
        .header(header::CONNECTION, "keep-alive")
        .header("X-Accel-Buffering", "no")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// Serves the API on an already-bound listener until the future is dropped.
pub async fn serve(
    hub: ServiceHub,
    cfg: HttpConfig,
    broker: Arc<PushBroker>,
    listener: tokio::net::TcpListener,
) -> std::io::Result<()> {
    axum::serve(listener, router(hub, cfg, broker)).await
}
