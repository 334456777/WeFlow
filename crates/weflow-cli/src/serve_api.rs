//! The HTTP API of `weflow serve` (issue #131): every CLI command over HTTP.
//!
//! The path is the command path (`GET /config/list` is `weflow config list`), query or JSON-body keys are the
//! command's long option names, and the HTTP method says what kind of side effect the command has (see
//! [`METHODS`](rules::METHODS)). Each request runs the command in a child `weflow` process, so the answer is
//! exactly what the terminal gets: by default the `--json` document as it is, with the HTTP status taken from its error code and
//! the exit code. A request may also run the command in the background ([`jobs`]) or get back the file it wrote
//! ([`files`]).

mod answer;
pub(crate) mod child;
mod endpoints;
mod files;
pub(crate) mod http;
mod in_process;
mod job_routes;
mod jobs;
mod openapi;
mod push;
mod rules;
mod status;
pub use status::Farewell;
pub mod ui;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde_json::{json, Value};
use tokio::sync::Semaphore;
use weflow_core::sync::RwLockExt;

use answer::{
    document_response, failed_in_process, field_names, in_process_document, json_line,
    media_response, only_fields,
};
use child::{run, server_lang, status_for, status_for_exit};
use endpoints::{find_param, text_of, Endpoints, Params};
use http::{
    authorized, compression, cors, error_response, generate_token, json_body, local_host,
    method_not_allowed, query_pairs, redirect, url_decode,
};
use rules::Verb;

// Below 49152: Windows reserves blocks of its dynamic range (49152-65535) for Hyper-V, WSL 2 and Docker,
// and a reserved port cannot be bound (#188)
pub const DEFAULT_PORT: u16 = 18108;

pub use weflow_core::contract::API_VERSION;

const MAX_BODY: usize = 1 << 20;

use rules::Verb::*;

pub struct ApiConfig {
    /// The `weflow` executable the commands run in.
    pub exe: PathBuf,
    pub token: String,
    /// Where `?return=file` puts the files of requests that name no `--out` (one folder per request).
    pub files_dir: PathBuf,
    /// `POST` commands (exports, keys, backups…) that may run at once.
    pub max_tasks: usize,
    /// Other commands that may run at once.
    pub max_reads: usize,
    /// Images and voices of the `/api/` media routes decoded at once; more wait in line (see [`Line`]).
    pub max_media: usize,
    /// Answer only requests whose `Host` names this machine (see [`local_host`]); set when listening on a
    /// loopback address.
    pub check_host: bool,
    /// Print each request on stderr (`--log-requests`): method, path, status and time, never the query.
    pub log_requests: bool,
    /// The WebUI served at `/ui/` (issue #146), if any; set later when its version is downloaded beside the server.
    pub ui: Arc<std::sync::RwLock<Option<ui::UiDir>>>,
    /// The versions kept by `weflow web` that `ui` comes from, unless `--ui-dir`, `--web-version` or `--api-only`
    /// chose otherwise: once `web use` or `web update` answers through the API, the version it chose is shown.
    pub web_store: Option<weflow_core::webui::WebStore>,
    /// The configuration the commands read: what `data` of `/health` follows.
    pub config_path: PathBuf,
    /// Ends the event streams with `bye` when the server stops.
    pub farewell: Farewell,
    /// Whether the insight engine runs beside (`--insight`), as `/health` says.
    pub insight: bool,
}

struct AppState {
    cfg: ApiConfig,
    endpoints: Endpoints,
    jobs: Arc<jobs::Jobs>,
    tasks: Arc<Semaphore>,
    reads: Arc<Semaphore>,
    /// The `/api/` reads, which wait for a place of `reads`.
    api_reads: Line,
    media: Line,
    /// `/openapi.json` in the server's language, built when first asked for.
    openapi: OnceLock<String>,
    /// The hub of the `/api/` routes.
    hub: Arc<in_process::Hub>,
    /// `/api/push/messages`.
    push: push::Push,
    /// What `/health` and the event streams say of the server.
    status: status::Status,
}

impl AppState {
    /// The limit a command with this method runs under.
    fn limit(&self, verb: Option<Verb>) -> &Arc<Semaphore> {
        if verb == Some(Post) {
            &self.tasks
        } else {
            &self.reads
        }
    }
}

/// Requests of a [`Line`] waiting at once for a place, beyond those running.
const QUEUE: usize = 256;
/// How long a request of a [`Line`] waits for a place before it answers `429`.
const WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// A limit whose requests over it wait for a place instead of answering `429`, unless the line is full or the
/// wait too long (issue #157): the `/api/` reads and media routes, which a page asks for several at once and
/// each of which takes a few milliseconds. The routes without `/api/` start a process each, and answer `429`
/// right away.
struct Line {
    /// Requests running and waiting.
    admitted: Arc<Semaphore>,
    running: Arc<Semaphore>,
}

impl Line {
    /// A line for the places of `running`, of which there are `max`.
    fn new(running: Arc<Semaphore>, max: usize) -> Self {
        Self {
            admitted: Arc::new(Semaphore::new(max.max(1) + QUEUE)),
            running,
        }
    }

    /// A place to run in, after waiting at most `wait`; `None` when the line is full or the wait ran out.
    async fn enter(
        &self,
        wait: std::time::Duration,
    ) -> Option<(
        tokio::sync::OwnedSemaphorePermit,
        tokio::sync::OwnedSemaphorePermit,
    )> {
        let admitted = self.admitted.clone().try_acquire_owned().ok()?;
        let running = tokio::time::timeout(wait, self.running.clone().acquire_owned())
            .await
            .ok()?
            .ok()?;
        Some((admitted, running))
    }
}

fn router(cfg: ApiConfig) -> Router {
    let tasks = Arc::new(Semaphore::new(cfg.max_tasks.max(1)));
    let reads = Arc::new(Semaphore::new(cfg.max_reads.max(1)));
    let api_reads = Line::new(reads.clone(), cfg.max_reads);
    let media = Line::new(
        Arc::new(Semaphore::new(cfg.max_media.max(1))),
        cfg.max_media,
    );
    let hub = Arc::new(in_process::Hub::default());
    hub.warm();
    let server = status::Status::new(
        cfg.config_path.clone(),
        &status::ServerInfo {
            max_tasks: cfg.max_tasks,
            max_reads: cfg.max_reads,
            max_media: cfg.max_media,
            insight: cfg.insight,
        },
    );
    Router::new()
        .fallback(dispatch)
        .layer(compression())
        .with_state(Arc::new(AppState {
            cfg,
            endpoints: Endpoints::new(),
            jobs: Arc::new(jobs::Jobs::new()),
            tasks,
            reads,
            api_reads,
            media,
            openapi: Default::default(),
            hub,
            push: Default::default(),
            status: server,
        }))
}

/// The default of [`ApiConfig::max_reads`] and [`ApiConfig::max_media`]: one per CPU.
pub fn default_max_reads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get())
}

/// Serves the API on an already-bound listener until the future is dropped.
pub async fn serve(cfg: ApiConfig, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    // a read run in process (`analytics overall`) reports progress, which is no request's: the server draws no bar
    weflow_core::output::set_progress_mode(weflow_core::output::ProgressMode::Off);
    axum::serve(listener, router(cfg)).await
}

async fn dispatch(State(st): State<Arc<AppState>>, req: Request) -> Response {
    let start = std::time::Instant::now();
    let headers = req.headers().clone();
    let (method, path) = (req.method().clone(), req.uri().path().to_string());
    let mut resp = if st.cfg.check_host && !local_host(req.uri(), &headers) {
        error_response(
            StatusCode::FORBIDDEN,
            "invalid_host",
            "this server only answers requests to localhost, 127.0.0.1 or [::1]; to reach it by another name, \
             start it with --host",
        )
    } else if req.method() == Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else {
        route(&st, req).await
    };
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut timing = resp
        .extensions_mut()
        .remove::<Timing>()
        .unwrap_or_default()
        .0;
    timing.push(format!("total;dur={ms:.1}"));
    if let Ok(v) = HeaderValue::from_str(&timing.join(", ")) {
        resp.headers_mut().insert("server-timing", v);
    }
    if st.cfg.log_requests {
        weflow_core::output::event(json!({
            "type": "request",
            "method": method.as_str(),
            "path": path,
            "status": resp.status().as_u16(),
            "ms": (ms * 10.0).round() / 10.0,
        }));
    }
    cors(&headers, resp)
}

/// What a request spent its time on besides the total, for `Server-Timing` (issue #158): kept in the answer's
/// extensions by [`route`], written out by [`dispatch`].
#[derive(Clone, Default)]
struct Timing(Vec<String>);

impl Timing {
    /// The time since `since`, as `name` (and what it was, `desc`).
    fn stage(&mut self, name: &str, desc: Option<&str>, since: std::time::Instant) {
        let ms = since.elapsed().as_secs_f64() * 1000.0;
        self.0.push(match desc {
            Some(desc) => format!("{name};desc=\"{desc}\";dur={ms:.1}"),
            None => format!("{name};dur={ms:.1}"),
        });
    }

    fn on(self, mut resp: Response) -> Response {
        resp.extensions_mut().insert(self);
        resp
    }
}

/// What a request asks for besides the command (`?std=`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Std {
    /// The `--json` document.
    Json,
    /// The equivalent command line, without running it.
    In,
    /// The human-readable stdout.
    Out,
    /// The human-readable run's stderr.
    Err,
}

fn bad_request(message: &str) -> Response {
    error_response(StatusCode::BAD_REQUEST, "invalid_arguments", message)
}

/// `429 Too Many Requests`: as many commands of this kind as allowed are running.
fn busy(task: bool) -> Response {
    busy_with(
        if task {
            "another task is running; retry later, or send `Prefer: respond-async` to queue it"
        } else {
            "too many commands are running; retry later, or send `Prefer: respond-async` to queue it"
        },
        if task { "5" } else { "1" },
    )
}

/// `429 Too Many Requests` of an `/api/` route: its line of waiting requests is full, or the wait ran out.
fn busy_line(media: bool) -> Response {
    busy_with(
        if media {
            "too many images, voices and videos are asked for; retry later"
        } else {
            "too many reads are asked for; retry later"
        },
        "1",
    )
}

fn busy_with(message: &str, retry_after: &'static str) -> Response {
    let mut resp = error_response(StatusCode::TOO_MANY_REQUESTS, "busy", message);
    resp.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static(retry_after));
    resp
}

/// `Prefer: respond-async`: run the command as a job.
fn prefers_async(headers: &HeaderMap) -> bool {
    headers
        .get_all("prefer")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|p| {
            p.split(';')
                .next()
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("respond-async"))
        })
}

/// Where `?return=file` finds the written file.
pub(super) struct FileOut {
    /// The `--out` passed to the command.
    out: String,
    /// The result field holding the written path.
    field: Option<&'static str>,
    /// The request's own folder, when the server chose `--out`.
    dir: Option<PathBuf>,
}

async fn route(st: &AppState, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let path = parts.uri.path();
    if path == "/health" {
        return json_body(StatusCode::OK, st.status.health().to_string());
    }
    if path == "/health/events" {
        if parts.method != Method::GET {
            return method_not_allowed(&[Get]);
        }
        return status::events(st.status.clone(), &st.cfg.farewell);
    }
    // the WebUI's files need no token: the page brings it in its address
    if let Some(rest) = path
        .strip_prefix("/ui")
        .filter(|r| r.is_empty() || r.starts_with('/'))
    {
        let shown = st.cfg.ui.read_or_panic().clone();
        return match &shown {
            Some(dir) => ui::serve(dir, &parts.method, rest, parts.uri.query()).await,
            None => error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "no WebUI is served; start serve with --ui-dir <a built WebUI>",
            ),
        };
    }
    if path == "/" && st.cfg.ui.read_or_panic().is_some() && parts.method == Method::GET {
        let to = format!(
            "/ui/{}",
            parts
                .uri
                .query()
                .map(|q| format!("?{q}"))
                .unwrap_or_default()
        );
        return redirect(StatusCode::FOUND, &to);
    }
    let query = query_pairs(parts.uri.query());
    if !authorized(&st.cfg.token, &parts.headers, &query) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid or missing access token",
        );
    }
    if path == "/openapi.json" {
        if parts.method != Method::GET {
            return method_not_allowed(&[Get]);
        }
        // in the server's language, as every answer (#433)
        let doc = st.openapi.get_or_init(|| {
            openapi::document(weflow_core::locale::current() == weflow_core::locale::Lang::Zh)
                .to_string()
        });
        return json_body(StatusCode::OK, doc.clone());
    }
    if path == "/api/push/messages" {
        if parts.method != Method::GET {
            return method_not_allowed(&[Get]);
        }
        // EventSource sends the header when it reconnects; a page that opens a new one passes the query
        let last_id = parts
            .headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .or_else(|| {
                query
                    .iter()
                    .find(|(k, _)| k == "last-event-id")
                    .map(|(_, v)| v.clone())
            })
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        return st
            .push
            .stream(st.hub.clone(), last_id, st.status.clone(), &st.cfg.farewell);
    }
    let segments: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(url_decode)
        .collect();
    if segments.first().is_some_and(|s| s == "jobs") {
        let last_event_id = parts
            .headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok());
        return job_routes::route(&st.jobs, &parts.method, &segments, last_event_id).await;
    }
    // `/api/<command path>`: the same command, run in this process (in_process)
    let in_process = segments.first().is_some_and(|s| s == "api");
    let Some(endpoint) = st.endpoints.get(&segments[in_process as usize..]) else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "no such command");
    };
    let media = in_process && in_process::MEDIA.contains(&endpoint.command_name().as_str());
    if in_process && !media && !in_process::COMMANDS.contains(&endpoint.command_name().as_str()) {
        return error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &format!(
                "`{}` does not run in process; use /{}",
                endpoint.command_name(),
                endpoint.path.join("/")
            ),
        );
    }

    let mut params = Params::new();
    for (k, v) in query {
        params.entry(k).or_default().push(Value::String(v));
    }
    if parts.method != Method::GET && parts.method != Method::HEAD {
        let bytes = match to_bytes(body, MAX_BODY).await {
            Ok(b) => b,
            Err(_) => {
                return error_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "invalid_arguments",
                    "request body too large",
                )
            }
        };
        if !bytes.iter().all(u8::is_ascii_whitespace) {
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(Value::Object(map)) => {
                    for (k, v) in map {
                        if v.is_null() {
                            params.remove(&k);
                        } else {
                            params.insert(k, vec![v]);
                        }
                    }
                }
                _ => return bad_request("the request body must be a JSON object"),
            }
        }
    }
    params.remove("access_token");
    let std = match params.remove("std").and_then(|v| v.into_iter().next()) {
        None => Std::Json,
        Some(v) => match text_of(&v).as_str() {
            "in" => Std::In,
            "out" => Std::Out,
            "err" => Std::Err,
            other => {
                return bad_request(&format!(
                    "unknown std value `{other}`; expected in, out or err"
                ))
            }
        },
    };
    let want_file = match params.remove("return").and_then(|v| v.into_iter().next()) {
        None => false,
        Some(v) => match text_of(&v).as_str() {
            "json" => false,
            "file" => true,
            other => {
                return bad_request(&format!(
                    "unknown return value `{other}`; expected json or file"
                ))
            }
        },
    };
    let fields = params.remove("fields").map(|values| field_names(&values));
    let background = prefers_async(&parts.headers);
    if fields.is_some() && (std != Std::Json || want_file || background || media) {
        return bad_request(
            "fields picks the columns of a --json document answered right away: not with std, return=file, \
             `Prefer: respond-async` or a media route",
        );
    }
    if in_process && (std != Std::Json || want_file || background) {
        return bad_request(
            "/api/ answers the --json document right away: std, return=file and `Prefer: respond-async` are for \
             the routes without /api/",
        );
    }

    // a media route reads, whatever the command does without /api/
    let verb = if media {
        Some(Get)
    } else {
        endpoint.verb(&params)
    };
    match verb {
        Some(verb) if verb.method() != parts.method => return method_not_allowed(&[verb]),
        None if !endpoint.verbs().iter().any(|v| v.method() == parts.method) => {
            return method_not_allowed(&endpoint.verbs())
        }
        _ => {}
    }

    let command = endpoint.command_name();
    let mut file_out = None;
    if want_file {
        if std != Std::Json {
            return bad_request("return=file cannot be combined with std");
        }
        let Some((default_out, field)) = files::file_command(&command) else {
            return bad_request(&format!(
                "`{command}` does not write a single file, so return=file is not available for it"
            ));
        };
        let given = params
            .iter()
            .find(|(k, _)| find_param(endpoint, k).is_some_and(|p| p.name == "out"))
            .and_then(|(_, v)| v.first())
            .map(text_of);
        file_out = Some(match given {
            Some(out) => FileOut {
                out,
                field,
                dir: None,
            },
            None => {
                let dir = st.cfg.files_dir.join(&generate_token()[..16]);
                let out = if default_out.is_empty() {
                    dir.clone()
                } else {
                    dir.join(default_out)
                };
                let out = out.to_string_lossy().into_owned();
                params.insert("out".into(), vec![Value::String(out.clone())]);
                FileOut {
                    out,
                    field,
                    dir: Some(dir),
                }
            }
        });
    }

    if media
        && params
            .keys()
            .any(|k| find_param(endpoint, k).is_some_and(|p| p.name == "out"))
    {
        return bad_request("an /api/ media route answers with the file itself: leave out `out`");
    }
    if let Err(message) = endpoint.check(&params) {
        return bad_request(&message);
    }
    let mut timing = Timing::default();
    // `/api/`: the typed read of the request, run in this process (#397). A request it does not take goes to a child
    // process, which answers it as the terminal does, in the place the request waited for.
    let mut place = None;
    if in_process {
        let line = if media { &st.media } else { &st.api_reads };
        let waited = std::time::Instant::now();
        let Some(entered) = line.enter(WAIT).await else {
            return busy_line(media);
        };
        timing.stage("wait", Some("in line for a place"), waited);
        let given = in_process::Given::new(endpoint, &params);
        let ran = std::time::Instant::now();
        let hub = st.hub.clone();
        if media {
            if let Ok(asked) = given.media(&command) {
                let done =
                    in_process::held(entered, async move { in_process::media(&hub, asked).await })
                        .await;
                timing.stage("cmd", Some("in process"), ran);
                return timing.on(match done {
                    Ok(Ok(file)) => media_response(&parts.headers, file),
                    Ok(Err((doc, exit))) => in_process_document(&doc, exit, None),
                    Err(err) => failed_in_process(&err),
                });
            }
        } else if let Ok(read) = given.read(&command) {
            let done =
                in_process::held(entered, async move { in_process::run(&hub, read).await }).await;
            timing.stage("cmd", Some("in process"), ran);
            return timing.on(match done {
                Ok((doc, exit)) => in_process_document(&doc, exit, fields.as_deref()),
                Err(err) => failed_in_process(&err),
            });
        }
        place = Some(entered);
    }
    if media {
        // satisfies the command line of the child process; nothing is written there
        params.insert("out".into(), vec![Value::String("-".into())]);
    }
    let argv = match endpoint.argv(&params) {
        Ok(a) => a,
        Err(message) => return bad_request(&message),
    };
    let mut full = vec!["--json".to_string()];
    if background {
        full.push("--progress".into());
    }
    full.extend(argv.iter().cloned());
    if std == Std::In {
        let shown: Vec<String> = std::iter::once("weflow".to_string()).chain(full).collect();
        return json_body(
            StatusCode::OK,
            json!({ "argv": shown, "stdin": Value::Null }).to_string(),
        );
    }
    // an `/api/` request its typed read did not take and clap refuses too: answered here with clap's error, as the
    // child process would print it, without starting one (#432)
    if place.is_some() && !background {
        let ran = std::time::Instant::now();
        // the program's name as the child's would be, which its usage line shows (`weflow.exe` on Windows)
        let program = std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "weflow".into());
        let line: Vec<String> = std::iter::once(program)
            .chain(full.iter().cloned())
            .collect();
        if let Err(err) = crate::i18n::try_parse_from::<crate::cli::Cli>(&line) {
            if let Some((doc, exit)) = crate::i18n::json_failure(&err) {
                timing.stage("cmd", Some("refused in process"), ran);
                return timing.on(in_process_document(
                    &doc,
                    u8::try_from(exit).unwrap_or(1),
                    None,
                ));
            }
        }
    }
    // the child answers in the server's language, as a read in process does, whatever the request's
    // `Accept-Language` and whatever `weflow lang` was set to since the server started (#433)
    let lang = server_lang();
    let limit = st.limit(verb);
    // the folder the server chose for the file, made once the command is sure to run
    let make_folder =
        |file_out: &Option<FileOut>| match file_out.as_ref().and_then(|f| f.dir.as_ref()) {
            Some(dir) => std::fs::create_dir_all(dir).err().map(|err| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "runtime_error",
                    &format!("failed to create {}: {err}", dir.display()),
                )
            }),
            None => None,
        };
    if background {
        if std != Std::Json {
            return bad_request(
                "std=out and std=err are answered right away; send them without `Prefer: respond-async`",
            );
        }
        if let Some(failed) = make_folder(&file_out) {
            return failed;
        }
        let job = st
            .jobs
            .start(st.cfg.exe.clone(), full, lang, limit.clone(), file_out);
        return job_routes::accepted(&job);
    }
    let (_permit, _running) = match place {
        Some((admitted, running)) => (admitted, Some(running)),
        None => {
            let Ok(permit) = limit.clone().try_acquire_owned() else {
                return busy(verb == Some(Post));
            };
            (permit, None)
        }
    };
    if let Some(failed) = make_folder(&file_out) {
        return failed;
    }
    let args = if std == Std::Json { &full } else { &argv };
    let ran = std::time::Instant::now();
    let output = match run(&st.cfg.exe, args, lang).await {
        Ok(o) => o,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "runtime_error",
                &format!("failed to run weflow: {err}"),
            )
        }
    };
    timing.stage("cmd", Some("child process"), ran);
    // killed by a signal: no exit code
    let exit = output.status.code().unwrap_or(-1);
    let mut resp = match std {
        Std::Json => match serde_json::from_slice::<Value>(&output.stdout) {
            Ok(doc) => {
                let status = status_for(&doc, exit);
                if status == StatusCode::OK && matches!(command.as_str(), "web use" | "web update")
                {
                    show_chosen(&st.cfg, &doc);
                }
                match file_out {
                    Some(file) if status == StatusCode::OK => {
                        let mut path = files::written_file(&doc, file.field, &file.out);
                        if file.dir.is_some() {
                            path = files::add_extension(&path);
                        }
                        match files::multipart(&path).await {
                            Ok(resp) => resp,
                            Err(message) => error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "runtime_error",
                                &message,
                            ),
                        }
                    }
                    _ => match &fields {
                        Some(fields) => document_response(
                            &doc,
                            status,
                            json_line(&only_fields(doc.clone(), fields)),
                        ),
                        None => document_response(&doc, status, output.stdout),
                    },
                }
            }
            Err(_) => {
                let body = json!({
                    "success": false,
                    "error": {
                        "code": "runtime_error",
                        "message": "the command did not print a JSON document",
                        "details": {
                            "exitCode": exit,
                            "stderr": String::from_utf8_lossy(&output.stderr),
                        }
                    }
                });
                json_body(StatusCode::INTERNAL_SERVER_ERROR, body.to_string())
            }
        },
        Std::Out | Std::Err => {
            let (key, bytes) = if std == Std::Out {
                ("stdout", &output.stdout)
            } else {
                ("stderr", &output.stderr)
            };
            let body = json!({ "exitCode": exit, key: String::from_utf8_lossy(bytes) });
            json_body(status_for_exit(exit), body.to_string())
        }
        Std::In => unreachable!(),
    };
    resp.headers_mut()
        .insert("x-weflow-exit-code", HeaderValue::from(exit));
    timing.on(resp)
}

/// Shows at `/ui/` the version `web use` or `web update` chose (`data.version` of its answer), when the page follows
/// the versions kept by `weflow web`; the page reloads to get it.
fn show_chosen(cfg: &ApiConfig, doc: &Value) {
    let Some(store) = &cfg.web_store else {
        return;
    };
    let Some(version) = doc.pointer("/data/version").and_then(Value::as_str) else {
        return;
    };
    if let Some(dir) = ui::UiDir::open_version(store, version) {
        *cfg.ui.write_or_panic() = Some(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn async_is_asked_for_with_prefer() {
        let prefer = |v: &'static str| {
            let mut h = HeaderMap::new();
            h.insert("prefer", HeaderValue::from_static(v));
            prefers_async(&h)
        };
        assert!(prefer("respond-async"));
        assert!(prefer("return=minimal, Respond-Async; wait=10"));
        assert!(!prefer("respond-asynchronously"));
        assert!(!prefers_async(&HeaderMap::new()));
    }

    pub(super) fn params(pairs: &[(&str, Value)]) -> Params {
        let mut out = Params::new();
        for (k, v) in pairs {
            out.entry(k.to_string()).or_default().push(v.clone());
        }
        out
    }

    #[tokio::test]
    async fn requests_wait_in_line_until_it_is_full_or_too_long() {
        use std::time::Duration;
        let limit = Line::new(Arc::new(Semaphore::new(1)), 1);
        let first = limit.enter(Duration::ZERO).await.expect("a free place");
        // the place is taken: a request waits, and gets it once it is given back
        let (waited, _) = tokio::join!(limit.enter(Duration::from_secs(10)), async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(first);
        });
        let decoding = waited.expect("the place given back");
        // a wait that runs out
        assert!(limit.enter(Duration::from_millis(20)).await.is_none());
        // a full line: no wait at all
        let line: Vec<_> = (0..QUEUE)
            .map(|_| limit.admitted.clone().try_acquire_owned().unwrap())
            .collect();
        assert!(limit.enter(Duration::from_secs(10)).await.is_none());
        drop((line, decoding));
        assert!(limit.enter(Duration::ZERO).await.is_some());
    }

    /// A read whose client went away keeps its place until it ends (#396, risk 2), so the reads running at once stay
    /// within the limit.
    #[tokio::test]
    async fn a_read_keeps_its_place_after_its_client_is_gone() {
        use std::time::Duration;
        let limit = Line::new(Arc::new(Semaphore::new(1)), 1);
        let place = limit.enter(Duration::ZERO).await.expect("a free place");
        let (end, ended) = tokio::sync::oneshot::channel::<()>();
        // the request, which its client gives up on while the read runs
        let request = tokio::spawn(in_process::held(place, async move {
            let _ = ended.await;
        }));
        tokio::task::yield_now().await;
        request.abort();
        let _ = request.await;
        assert!(
            limit.enter(Duration::from_millis(50)).await.is_none(),
            "the place was given back while the read ran"
        );
        end.send(()).unwrap();
        assert!(limit.enter(Duration::from_secs(10)).await.is_some());
    }
}
