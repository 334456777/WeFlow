//! `weflow serve` 的命令 API（#131）：通过 HTTP 调用每一条命令。
//!
//! 路径就是命令路径（`GET /config/list` 即 `weflow config list`），查询参数和 JSON 请求体的键是命令的长选项名，
//! HTTP 方法表示命令有哪种副作用（见 [`METHODS`](rules::METHODS)）。
//!
//! 不带 `/api/` 的路由在子进程中运行 `weflow`，回答与终端得到的完全一样：默认是原样的 `--json` 文档，
//! HTTP 状态由其中的错误码和退出码决定。`/api/` 下的路由在服务进程内执行读取（[`in_process`]），
//! 类型化读取不接受的请求仍交给子进程。请求也可以让命令在后台运行（[`jobs`]），或者取回命令写出的文件（[`files`]）。

mod answer;
mod child;
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

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::request::Parts;
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
use endpoints::{find_param, text_of, Endpoint, Endpoints, Params};
use http::{
    authorized, compression, cors, error_response, generate_token, json_body, local_host,
    method_not_allowed, query_pairs, redirect, url_decode,
};
use rules::Verb;

// 低于 49152：Windows 把动态端口范围（49152-65535）中的一些段保留给 Hyper-V、WSL 2 和 Docker，
// 被保留的端口无法绑定（#188）
pub const DEFAULT_PORT: u16 = 18108;

pub use weflow_core::contract::API_VERSION;

/// 请求体的上限（1 MiB），超过时回答 `413`。
const MAX_BODY: usize = 1 << 20;

use rules::Verb::*;

/// 命令 API 的设置，由 `serve` 命令按它的参数填写。
pub struct ApiConfig {
    /// 运行命令的 `weflow` 可执行文件。
    pub exe: PathBuf,
    /// 访问令牌：`/health`、`/health/events` 和 `/ui/` 以外的请求都要带上它（见 [`authorized`]）。
    pub token: String,
    /// 请求没有给 `--out` 时，`?return=file` 把命令写出的文件放在这里，每个请求一个文件夹。
    pub files_dir: PathBuf,
    /// 可以同时运行的任务（导出、取钥、备份等 `POST` 命令）数。
    pub max_tasks: usize,
    /// 可以同时运行的其他命令数。
    pub max_reads: usize,
    /// `/api/` 的媒体路由（图片、语音、视频和朋友圈的媒体）可以同时运行的请求数，超出的排队等待（见 [`Line`]）。
    pub max_media: usize,
    /// 只回答 `Host` 指向本机的请求（见 [`local_host`]）；监听回环地址时设置。
    pub check_host: bool,
    /// 在 stderr 输出每个请求（`--log-requests`）：方法、路径、状态码和耗时，不输出查询参数。
    pub log_requests: bool,
    /// `/ui/` 提供的 WebUI（#146），或者没有网页的原因；
    /// 要显示的版本在服务启动后才下载时，下载完成或失败后再更新它（#946）。
    pub ui: Arc<std::sync::RwLock<ui::Ui>>,
    /// `weflow web` 保存的各个版本，`ui` 取自其中；用了 `--ui-dir`、`--web-version` 或 `--api-only` 时为 `None`。
    /// 有它时，通过命令 API 运行的 `web use` 或 `web update` 成功后，`/ui/` 改为显示选定的版本。
    pub web_store: Option<weflow_core::webui::WebStore>,
    /// 服务启动时命令读取的配置文件；找不到当前使用的配置文件时，`/health` 的 `data` 按它计算（见
    /// [`status::Status::data`]）。
    pub config_path: PathBuf,
    /// 服务停止时用 `bye` 结束事件流。
    pub farewell: Farewell,
    /// 与服务一起运行的见解引擎（`--insight`）的 `ServiceHub`，没有运行时为 `None`：`/health` 报告它是否在运行，
    /// `insight today-stats` 读取它记下的次数（#1264）。
    pub insight: Option<weflow_core::services::ServiceHub>,
    /// 服务运行的原图自动下载钩子（`serve_image_auto_download`），由 `GET /image/auto-download/status` 报告；
    /// 没有运行时为 `None`。
    pub image_auto_download: Option<weflow_core::image_download::ImageAutoDownload>,
}

/// 所有请求共用的服务状态。
struct AppState {
    cfg: ApiConfig,
    endpoints: Endpoints,
    jobs: Arc<jobs::Jobs>,
    /// 任务的空位，共 `max_tasks` 个。
    tasks: Arc<Semaphore>,
    /// 其他命令的空位，共 `max_reads` 个。
    reads: Arc<Semaphore>,
    /// `/api/` 的读取，排队等待 `reads` 的空位。
    api_reads: Line,
    /// `/api/` 的媒体路由，排队等待自己的 `max_media` 个空位。
    media: Line,
    /// 用服务端的语言写的 `/openapi.json`，第一次请求时生成。
    openapi: OnceLock<String>,
    /// `/api/` 路由共用的 `Hub`。
    hub: Arc<in_process::Hub>,
    /// `/api/push/messages` 的消息推送。
    push: push::Push,
    /// `/health` 和事件流报告的服务状态。
    status: status::Status,
}

impl AppState {
    /// 这种方法的命令受哪个上限约束：`POST` 受 `tasks`，其他方法受 `reads`。
    fn limit(&self, verb: Verb) -> &Arc<Semaphore> {
        if verb == Post {
            &self.tasks
        } else {
            &self.reads
        }
    }
}

/// 一个 [`Line`] 中除了正在运行的请求，最多还能有多少个请求同时排队等待空位。
const QUEUE: usize = 256;
/// [`Line`] 中的请求等待空位的最长时间，超过后回答 `429`。
const WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// 超出上限的请求排队等待空位的限流，队伍已满或等待太久时才回答 `429`（#157）。
///
/// 用于 `/api/` 的读取和媒体路由：网页会同时请求好几个，每个只需几毫秒。不带 `/api/` 的路由每个请求启动一个进程，
/// 超出上限时立即回答 `429`。
struct Line {
    /// 正在运行和排队的请求共用的名额，共 `max` 加 [`QUEUE`] 个。
    admitted: Arc<Semaphore>,
    /// 运行的空位；`/api/` 的读取与不带 `/api/` 的路由共用 `reads` 的空位。
    running: Arc<Semaphore>,
}

impl Line {
    /// 排队等待 `running` 的空位的队伍，`running` 共有 `max` 个空位。
    fn new(running: Arc<Semaphore>, max: usize) -> Self {
        Self {
            admitted: Arc::new(Semaphore::new(max.max(1) + QUEUE)),
            running,
        }
    }

    /// 最多等待 `wait`，取得一个运行的空位；队伍已满或等待超时时为 `None`。
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
            insight: cfg.insight.is_some(),
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

/// [`ApiConfig::max_reads`] 和 [`ApiConfig::max_media`] 的默认值：每个 CPU 一个，取不到 CPU 数时为 4。
pub fn default_max_reads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get())
}

/// 在已绑定的 `listener` 上提供命令 API，直到这个 future 被丢弃。
///
/// # Errors
///
/// 只传递 axum 的服务循环返回的 I/O 错误。
pub async fn serve(cfg: ApiConfig, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    // 进程内执行的读取（如 `analytics overall`）会报告进度，而进度不属于任何请求：服务不画进度条
    weflow_core::output::set_progress_mode(weflow_core::output::ProgressMode::Off);
    axum::serve(listener, router(cfg)).await
}

/// 所有请求的入口：检查 `Host`、回答 `OPTIONS`，其余交给 [`route`]；再加上 `Server-Timing` 和 CORS 的头，
/// 按 `--log-requests` 记下请求。
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

/// 请求在总耗时之外各阶段的耗时，用于 `Server-Timing`（#158）：[`command_route`] 把它放进回答的 extensions，
/// [`dispatch`] 写出。
#[derive(Clone, Default)]
struct Timing(Vec<String>);

impl Timing {
    /// 记下从 `since` 到现在的耗时，阶段名为 `name`，`desc` 是阶段的说明。
    fn stage(&mut self, name: &str, desc: Option<&str>, since: std::time::Instant) {
        let ms = since.elapsed().as_secs_f64() * 1000.0;
        self.0.push(match desc {
            Some(desc) => format!("{name};desc=\"{desc}\";dur={ms:.1}"),
            None => format!("{name};dur={ms:.1}"),
        });
    }

    /// 把这些耗时放进 `resp` 的 extensions，由 [`dispatch`] 写出。
    fn on(self, mut resp: Response) -> Response {
        resp.extensions_mut().insert(self);
        resp
    }
}

/// 请求要的输出（`?std=`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Std {
    /// `--json` 文档。
    Json,
    /// 等价的命令行，不运行命令。
    In,
    /// 不带 `--json` 运行时供人阅读的 stdout。
    Out,
    /// 不带 `--json` 运行时的 stderr。
    Err,
}

fn bad_request(message: &str) -> Response {
    error_response(StatusCode::BAD_REQUEST, "invalid_arguments", message)
}

/// `429 Too Many Requests`：这类命令同时运行的数量已达上限。
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

/// `/api/` 路由的 `429 Too Many Requests`：排队的请求已满，或者等待超时。
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

/// 请求是否带 `Prefer: respond-async`，即要求把命令作为作业运行。
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

/// `?return=file` 到哪里找命令写出的文件。
pub(super) struct FileOut {
    /// 传给命令的 `--out`。
    out: String,
    /// 结果中保存写出的路径的字段。
    field: Option<&'static str>,
    /// 由服务端选定 `--out` 时，请求自己的文件夹。
    dir: Option<PathBuf>,
}

impl FileOut {
    /// `?return=file` 到哪里找 `command` 写出的文件：请求给了 `out` 时就是它；否则是 `files_dir` 下请求自己的文件夹，
    /// 这时把它（命令有默认的文件名时是其中的这个文件）作为 `out` 加进 `params`。
    /// 命令不只写出一个文件时，`Err` 是 `400` 的回答。
    fn new(
        files_dir: &Path,
        endpoint: &Endpoint,
        command: &str,
        params: &mut Params,
    ) -> Result<Self, Refused> {
        let Some((default_out, field)) = files::file_command(command) else {
            return Err(bad_request(&format!(
                "`{command}` does not write a single file, so return=file is not available for it"
            ))
            .into());
        };
        if let Some(why) = files::not_one_file(command, endpoint, params) {
            return Err(bad_request(why).into());
        }
        let given = params
            .iter()
            .find(|(k, _)| find_param(endpoint, k).is_some_and(|p| p.name == "out"))
            .and_then(|(_, v)| v.first())
            .map(text_of);
        Ok(match given {
            Some(out) => Self {
                out,
                field,
                dir: None,
            },
            None => {
                let dir = files_dir.join(&generate_token()[..16]);
                let out = if default_out.is_empty() {
                    dir.clone()
                } else {
                    dir.join(default_out)
                };
                let out = out.to_string_lossy().into_owned();
                params.insert("out".into(), vec![Value::String(out.clone())]);
                Self {
                    out,
                    field,
                    dir: Some(dir),
                }
            }
        })
    }
}

/// 服务端为直接回答（不是作业）的请求的文件选定的文件夹，丢弃时删除（#815）：
/// 发送文件的回答在文件发完或客户端断开后丢弃它，命令失败或请求没有继续时立即丢弃。
pub(super) struct RequestFolder(PathBuf);

impl Drop for RequestFolder {
    fn drop(&mut self) {
        if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
            return;
        }
        // 别处还没结束的读取可能会让文件多打开一会儿，而 Windows 不能删除打开着的文件：稍后在另一个线程里重试
        let dir = std::mem::take(&mut self.0);
        std::thread::spawn(move || {
            for _ in 0..20 {
                std::thread::sleep(std::time::Duration::from_millis(250));
                if std::fs::remove_dir_all(&dir).is_ok() || !dir.exists() {
                    return;
                }
            }
        });
    }
}

/// 命令确定会运行后，创建服务端为 `?return=file` 的文件选定的文件夹。
fn make_folder(file_out: Option<&FileOut>) -> Result<(), Refused> {
    let Some(dir) = file_out.and_then(|f| f.dir.as_ref()) else {
        return Ok(());
    };
    std::fs::create_dir_all(dir).map_err(|err| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "runtime_error",
            &format!("failed to create {}: {err}", dir.display()),
        )
        .into()
    })
}

/// 命令运行之前就拒绝请求时的回答；装箱是因为 [`Response`] 作为 `Err` 太大。
type Refused = Box<Response>;

/// 请求在命令自己的参数之外要求的东西。
struct Asked {
    /// `?std=` 选择的输出。
    std: Std,
    /// `?return=file`：回答命令写出的文件，而不是它的 `--json` 文档。
    want_file: bool,
    /// `?fields=`：只回答 `--json` 文档中的这些列。
    fields: Option<Vec<String>>,
    /// `Prefer: respond-async`：把命令作为作业运行。
    background: bool,
}

impl Asked {
    /// 从 `params` 中取出请求自己的参数，只留下命令的参数；`access_token` 也一并去掉。
    fn take(params: &mut Params, headers: &HeaderMap) -> Result<Self, Refused> {
        params.remove("access_token");
        let std = match params.remove("std").and_then(|v| v.into_iter().next()) {
            None => Std::Json,
            Some(v) => match text_of(&v).as_str() {
                "in" => Std::In,
                "out" => Std::Out,
                "err" => Std::Err,
                other => {
                    return Err(bad_request(&format!(
                        "unknown std value `{other}`; expected in, out or err"
                    ))
                    .into())
                }
            },
        };
        let want_file = match params.remove("return").and_then(|v| v.into_iter().next()) {
            None => false,
            Some(v) => match text_of(&v).as_str() {
                "json" => false,
                "file" => true,
                other => {
                    return Err(bad_request(&format!(
                        "unknown return value `{other}`; expected json or file"
                    ))
                    .into())
                }
            },
        };
        let fields = params.remove("fields").map(|values| field_names(&values));
        Ok(Self {
            std,
            want_file,
            fields,
            background: prefers_async(headers),
        })
    }
}

/// 不需要令牌的路由交给 [`open_route`]；其余的检查令牌后按路径分派：`/openapi.json`、`/api/push/messages`、
/// `/jobs/…` 和命令的路由。
async fn route(st: &AppState, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    if let Some(resp) = open_route(st, &parts).await {
        return resp;
    }
    let query = query_pairs(parts.uri.query());
    if !authorized(&st.cfg.token, &parts.headers, &query) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid or missing access token",
        );
    }
    match parts.uri.path() {
        "/openapi.json" => openapi_route(st, &parts.method),
        "/api/push/messages" => push_route(st, &parts, &query),
        path => {
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
                job_routes::route(&st.jobs, &parts.method, &segments, last_event_id).await
            } else {
                command_route(st, parts, body, query, &segments)
                    .await
                    .unwrap_or_else(|refused| *refused)
            }
        }
    }
}

/// 不需要令牌的路由：服务的状态和 WebUI；其他路径为 `None`。
async fn open_route(st: &AppState, parts: &Parts) -> Option<Response> {
    let path = parts.uri.path();
    if path == "/health" {
        return Some(json_body(StatusCode::OK, st.status.health().to_string()));
    }
    if path == "/health/events" {
        if parts.method != Method::GET {
            return Some(method_not_allowed(&[Get]));
        }
        return Some(status::events(st.status.clone(), &st.cfg.farewell));
    }
    // WebUI 的文件不需要令牌：令牌在打开页面的地址里，由页面带给 API
    if let Some(rest) = path
        .strip_prefix("/ui")
        .filter(|r| r.is_empty() || r.starts_with('/'))
    {
        let shown = st.cfg.ui.read_or_panic().clone();
        return Some(ui::answer(&shown, &parts.method, rest, parts.uri.query()).await);
    }
    if path == "/" && st.cfg.ui.read_or_panic().page().is_some() && parts.method == Method::GET {
        let to = format!(
            "/ui/{}",
            parts
                .uri
                .query()
                .map(|q| format!("?{q}"))
                .unwrap_or_default()
        );
        return Some(redirect(StatusCode::FOUND, &to));
    }
    None
}

/// `/openapi.json`，与其他回答一样用服务端的语言（#433）。
fn openapi_route(st: &AppState, method: &Method) -> Response {
    if *method != Method::GET {
        return method_not_allowed(&[Get]);
    }
    let doc = st.openapi.get_or_init(|| {
        openapi::document(weflow_core::locale::current() == weflow_core::locale::Lang::Zh)
            .to_string()
    });
    json_body(StatusCode::OK, doc.clone())
}

/// `/api/push/messages`：新消息的事件流。
fn push_route(st: &AppState, parts: &Parts, query: &[(String, String)]) -> Response {
    if parts.method != Method::GET {
        return method_not_allowed(&[Get]);
    }
    // EventSource 重连时自己发送这个头；页面新建一个 EventSource 时用同名的查询参数传入
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
    st.push
        .stream(st.hub.clone(), last_id, st.status.clone(), &st.cfg.farewell)
}

/// 命令的路由，路径就是命令路径：在子进程中运行，`/api/` 下的在服务进程内运行。
/// `Err` 是命令运行之前就拒绝请求时的回答。
async fn command_route(
    st: &AppState,
    parts: Parts,
    body: Body,
    query: Vec<(String, String)>,
    segments: &[String],
) -> Result<Response, Refused> {
    // `/api/<命令路径>`：同一条命令，在服务进程内执行（`in_process`）
    let in_process = segments.first().is_some_and(|s| s == "api");
    let Some(endpoint) = st.endpoints.get(&segments[usize::from(in_process)..]) else {
        return Err(error_response(StatusCode::NOT_FOUND, "not_found", "no such command").into());
    };
    let media = in_process && in_process::MEDIA.contains(&endpoint.command_name().as_str());
    if in_process && !media && !in_process::COMMANDS.contains(&endpoint.command_name().as_str()) {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &format!(
                "`{}` does not run in process; use /{}",
                endpoint.command_name(),
                endpoint.path.join("/")
            ),
        )
        .into());
    }

    let mut params = request_params(query, &parts.method, body).await?;
    let asked = Asked::take(&mut params, &parts.headers)?;
    if asked.fields.is_some()
        && (asked.std != Std::Json || asked.want_file || asked.background || media)
    {
        return Err(bad_request(
            "fields picks the columns of a --json document answered right away: not with std, return=file, \
             `Prefer: respond-async` or a media route",
        ).into());
    }
    if in_process && (asked.std != Std::Json || asked.want_file || asked.background) {
        return Err(bad_request(
            "/api/ answers the --json document right away: std, return=file and `Prefer: respond-async` are for \
             the routes without /api/",
        ).into());
    }

    // 媒体路由只读取（`GET`），不管这条命令不带 `/api/` 时用什么方法
    let verb = if media { Get } else { endpoint.verb(&params) };
    if verb.method() != parts.method {
        return Err(method_not_allowed(&[verb]).into());
    }

    let command = endpoint.command_name();
    let file_out = if asked.want_file {
        if asked.std != Std::Json {
            return Err(bad_request("return=file cannot be combined with std").into());
        }
        Some(FileOut::new(
            &st.cfg.files_dir,
            endpoint,
            &command,
            &mut params,
        )?)
    } else {
        None
    };

    if media
        && params
            .keys()
            .any(|k| find_param(endpoint, k).is_some_and(|p| p.name == "out"))
    {
        return Err(bad_request(
            "an /api/ media route answers with the file itself: leave out `out`",
        )
        .into());
    }
    endpoint
        .check(&params)
        .map_err(|message| bad_request(&message))?;
    let mut timing = Timing::default();
    // `/api/`：把请求读成类型化读取，在服务进程内执行（#397）。类型化读取不接受的请求交给子进程，像终端一样回答，
    // 占用的是请求排队得到的空位。
    let mut place = None;
    if in_process {
        let line = if media { &st.media } else { &st.api_reads };
        let waited = std::time::Instant::now();
        let Some(entered) = line.enter(WAIT).await else {
            return Err(busy_line(media).into());
        };
        timing.stage("wait", Some("in line for a place"), waited);
        let given = in_process::Given::new(endpoint, &params);
        let ran = std::time::Instant::now();
        let done = if media {
            media_in_process(&st.hub, entered, &given, &command, &parts.headers).await
        } else {
            read_in_process(&st.hub, entered, &given, &command, asked.fields.as_deref()).await
        };
        match done {
            ControlFlow::Break(answer) => {
                timing.stage("cmd", Some("in process"), ran);
                return Ok(timing.on(answer));
            }
            ControlFlow::Continue(entered) => place = Some(entered),
        }
    }
    if media {
        // 补全下面交给 clap 检查的命令行，clap 会拒绝类型化读取没有接受的内容；这条命令行只用于检查，
        // 不交给子进程运行，否则会写出一个名为 `-` 的文件
        params.insert("out".into(), vec![Value::String("-".into())]);
    }
    let argv = endpoint
        .argv(&params)
        .map_err(|message| bad_request(&message))?;
    let mut full = vec!["--json".to_string()];
    if asked.background {
        full.push("--progress".into());
    }
    full.extend(argv.iter().cloned());
    if asked.std == Std::In {
        let shown: Vec<String> = std::iter::once("weflow".to_string()).chain(full).collect();
        return Ok(json_body(
            StatusCode::OK,
            json!({ "argv": shown, "stdin": Value::Null }).to_string(),
        ));
    }
    // 服务进程自己持有的状态：子进程只能用它自己的状态回答，其中没有这些（#1264）
    if rules::OWN_STATE.contains(&command.as_str()) && asked.std == Std::Json && !asked.background {
        let hook = st.cfg.image_auto_download.as_ref();
        if let Some((doc, exit)) = own_state(&command, hook, st.cfg.insight.as_ref()) {
            return Ok(in_process_document(&doc, exit, asked.fields.as_deref()));
        }
    }
    // 类型化读取不接受、clap 也拒绝的 `/api/` 请求：在这里直接回答 clap 的错误，与子进程打印的相同，
    // 不必启动子进程（#432）
    if place.is_some() && !asked.background {
        let ran = std::time::Instant::now();
        if let Some(refused) = refused_by_clap(&full) {
            timing.stage("cmd", Some("refused in process"), ran);
            return Ok(timing.on(refused));
        }
        if media {
            return Err(bad_request(
                "this /api/ media route does not take the request as it is: give each parameter once, as the command \
                 takes it",
            )
            .into());
        }
    }
    // 子进程与进程内读取一样用服务端的语言回答，不管请求的 `Accept-Language`，也不管服务启动后 `weflow lang`
    // 改成了什么（#433）
    let lang = server_lang();
    let limit = st.limit(verb);
    if asked.background {
        if asked.std != Std::Json {
            return Err(bad_request(
                "std=out and std=err are answered right away; send them without `Prefer: respond-async`",
            ).into());
        }
        make_folder(file_out.as_ref())?;
        let job = st
            .jobs
            .start(st.cfg.exe.clone(), full, lang, limit.clone(), file_out);
        return Ok(job_routes::accepted(&job));
    }
    let (_permit, _running) = match place {
        Some((admitted, running)) => (admitted, Some(running)),
        None => {
            let Ok(permit) = limit.clone().try_acquire_owned() else {
                return Err(busy(verb == Post).into());
            };
            (permit, None)
        }
    };
    make_folder(file_out.as_ref())?;
    let folder = file_out
        .as_ref()
        .and_then(|f| f.dir.clone())
        .map(RequestFolder);
    let args = if asked.std == Std::Json { &full } else { &argv };
    let ran = std::time::Instant::now();
    let output = run(&st.cfg.exe, args, lang).await.map_err(|err| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "runtime_error",
            &format!("failed to run weflow: {err}"),
        )
    })?;
    timing.stage("cmd", Some("child process"), ran);
    Ok(timing.on(child_answer(st, &command, output, asked, file_out, folder).await))
}

/// 命令请求的参数：查询参数，再用 JSON 请求体的键覆盖（值为 `null` 时去掉这个参数）。
async fn request_params(
    query: Vec<(String, String)>,
    method: &Method,
    body: Body,
) -> Result<Params, Refused> {
    let mut params = Params::new();
    for (k, v) in query {
        params.entry(k).or_default().push(Value::String(v));
    }
    if *method == Method::GET || *method == Method::HEAD {
        return Ok(params);
    }
    let bytes = to_bytes(body, MAX_BODY).await.map_err(|_| {
        error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "invalid_arguments",
            "request body too large",
        )
    })?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(params);
    }
    let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&bytes) else {
        return Err(bad_request("the request body must be a JSON object").into());
    };
    for (k, v) in map {
        if v.is_null() {
            params.remove(&k);
        } else {
            params.insert(k, vec![v]);
        }
    }
    Ok(params)
}

/// `/api/` 媒体路由的回答，占着 `place` 在服务进程内读取；类型化读取不接受请求时交还 `place`。
async fn media_in_process(
    hub: &Arc<in_process::Hub>,
    place: in_process::Place,
    given: &in_process::Given,
    command: &str,
    headers: &HeaderMap,
) -> ControlFlow<Response, in_process::Place> {
    let Ok(asked) = given.media(command) else {
        return ControlFlow::Continue(place);
    };
    let hub = hub.clone();
    let done = in_process::held(place, async move { in_process::media(&hub, asked).await }).await;
    ControlFlow::Break(match done {
        Ok(Ok(file)) => media_response(headers, file),
        Ok(Err((doc, exit))) => in_process_document(&doc, exit, None),
        Err(err) => failed_in_process(&err),
    })
}

/// `/api/` 读取的回答，占着 `place` 在服务进程内执行；类型化读取不接受请求时交还 `place`。
async fn read_in_process(
    hub: &Arc<in_process::Hub>,
    place: in_process::Place,
    given: &in_process::Given,
    command: &str,
    fields: Option<&[String]>,
) -> ControlFlow<Response, in_process::Place> {
    let Ok(read) = given.read(command) else {
        return ControlFlow::Continue(place);
    };
    let hub = hub.clone();
    let done = in_process::held(place, async move { in_process::run(&hub, read).await }).await;
    ControlFlow::Break(match done {
        Ok((doc, exit)) => in_process_document(&doc, exit, fields),
        Err(err) => failed_in_process(&err),
    })
}

/// clap 拒绝命令行 `full`（不含程序名）时的错误，与子进程打印的相同；clap 接受时为 `None`。
fn refused_by_clap(full: &[String]) -> Option<Response> {
    // 程序名与子进程的相同，用法行里会显示它（Windows 上是 `weflow.exe`）
    let program = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "weflow".into());
    let line: Vec<String> = std::iter::once(program)
        .chain(full.iter().cloned())
        .collect();
    let err = crate::i18n::try_parse_from::<crate::cli::Cli>(&line).err()?;
    let (doc, exit) = crate::i18n::json_failure(&err)?;
    Some(in_process_document(
        &doc,
        u8::try_from(exit).unwrap_or(1),
        None,
    ))
}

/// `command` 在子进程中运行、输出 `output` 之后的回答。
async fn child_answer(
    st: &AppState,
    command: &str,
    output: std::process::Output,
    asked: Asked,
    file_out: Option<FileOut>,
    folder: Option<RequestFolder>,
) -> Response {
    // 被信号终止时没有退出码
    let exit = output.status.code().unwrap_or(-1);
    let mut resp = match asked.std {
        Std::Json => match serde_json::from_slice::<Value>(&output.stdout) {
            Ok(doc) => {
                let status = status_for(&doc, exit);
                if status == StatusCode::OK && matches!(command, "web use" | "web update") {
                    show_chosen(&st.cfg, &doc);
                }
                match file_out {
                    Some(file) if status == StatusCode::OK => {
                        let mut path = files::written_file(&doc, file.field, &file.out);
                        if file.dir.is_some() {
                            path = files::add_extension(&path);
                        }
                        match files::multipart(&path, folder).await {
                            Ok(resp) => resp,
                            Err(message) => error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "runtime_error",
                                &message,
                            ),
                        }
                    }
                    _ => match &asked.fields {
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
            let (key, bytes) = if asked.std == Std::Out {
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
    resp
}

/// `/ui/` 跟随 `weflow web` 保存的版本时，改为显示 `web use` 或 `web update` 选定的版本（回答的 `data.version`）；
/// 网页重新加载后显示新的版本。
fn show_chosen(cfg: &ApiConfig, doc: &Value) {
    let Some(store) = &cfg.web_store else {
        return;
    };
    let Some(version) = doc.pointer("/data/version").and_then(Value::as_str) else {
        return;
    };
    if let Some(dir) = ui::UiDir::open_version(store, version) {
        *cfg.ui.write_or_panic() = ui::Ui::Page(dir);
    }
}

/// [`rules::OWN_STATE`] 中的命令在这个服务上的 `--json` 文档，服务运行的原图自动下载钩子是 `hook`，见解引擎是
/// `engine`；其他命令为 `None`。
fn own_state(
    command: &str,
    hook: Option<&weflow_core::image_download::ImageAutoDownload>,
    engine: Option<&weflow_core::services::ServiceHub>,
) -> Option<(Value, u8)> {
    match command {
        "image auto-download status" => Some(auto_download_status(hook)),
        "insight today-stats" => Some(today_stats(engine)),
        _ => None,
    }
}

/// 这个服务的 `insight today-stats` 的 `--json` 文档：它运行的见解引擎记下的次数，没有运行见解引擎时为空。
fn today_stats(engine: Option<&weflow_core::services::ServiceHub>) -> (Value, u8) {
    let stats = engine.map(weflow_core::services::ServiceHub::insight_today_stats);
    crate::reads::json_document(Ok(json!(stats.unwrap_or_default())))
}

/// 这个服务的 `image auto-download status` 的 `--json` 文档：它运行的钩子的状态，没有运行钩子时是未挂钩的状态。
fn auto_download_status(
    hook: Option<&weflow_core::image_download::ImageAutoDownload>,
) -> (Value, u8) {
    let state = hook.map(weflow_core::image_download::ImageAutoDownload::status);
    let status = weflow_core::contract::AutoDownloadStatus {
        is_hooked: state.as_ref().is_some_and(|s| s["isHooked"] == true),
        pid: state
            .as_ref()
            .and_then(|s| s["pid"].as_u64())
            .and_then(|pid| u32::try_from(pid).ok()),
        supported: weflow_core::image_download::supported(),
    };
    crate::reads::json_document(Ok(json!(status)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`rules::OWN_STATE`] 中的每条命令都由 [`own_state`] 回答，其他命令不由它回答：漏掉的命令会在子进程中运行，
    /// 而子进程没有服务的状态（#1264）。
    #[test]
    fn own_state_answers_the_commands_declared() {
        for command in rules::OWN_STATE {
            let (doc, exit) = own_state(command, None, None).unwrap_or_else(|| panic!("{command}"));
            assert_eq!(
                (exit, &doc["success"]),
                (0, &json!(true)),
                "{command}: {doc}"
            );
        }
        assert_eq!(own_state("insight records", None, None), None);
        // 没有运行见解引擎：没有次数
        assert_eq!(today_stats(None).0["data"], json!([]));
    }

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

    /// 由键值对组成的请求参数，同一个键可以出现多次。
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
        // 空位已被占用：请求等待，空位交还后得到它
        let (waited, ()) = tokio::join!(limit.enter(Duration::from_secs(10)), async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(first);
        });
        let decoding = waited.expect("the place given back");
        // 等待超时
        assert!(limit.enter(Duration::from_millis(20)).await.is_none());
        // 队伍已满：完全不等待
        let line: Vec<_> = (0..QUEUE)
            .map(|_| limit.admitted.clone().try_acquire_owned().unwrap())
            .collect();
        assert!(limit.enter(Duration::from_secs(10)).await.is_none());
        drop((line, decoding));
        assert!(limit.enter(Duration::ZERO).await.is_some());
    }

    /// 客户端断开的读取一直占着空位，直到读取结束（#396 的风险 2），同时运行的读取因此不会超过上限。
    #[tokio::test]
    async fn a_read_keeps_its_place_after_its_client_is_gone() {
        use std::time::Duration;
        let limit = Line::new(Arc::new(Semaphore::new(1)), 1);
        let place = limit.enter(Duration::ZERO).await.expect("a free place");
        let (end, ended) = tokio::sync::oneshot::channel::<()>();
        // 请求：读取运行期间，客户端放弃了它
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
