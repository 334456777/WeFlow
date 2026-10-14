//! The HTTP API of `weflow serve` (issue #131): every CLI command over HTTP.
//!
//! The path is the command path (`GET /config/list` is `weflow config list`), query or JSON-body keys are the
//! command's long option names, and the HTTP method says what kind of side effect the command has (see
//! [`METHODS`]). Each request runs the command in a child `weflow` process, so the answer is exactly what the
//! terminal gets: by default the `--json` document as it is, with the HTTP status taken from its error code and
//! the exit code. A request may also run the command in the background ([`jobs`]) or get back the file it wrote
//! ([`files`]).

mod files;
mod in_process;
mod jobs;
mod openapi;
mod push;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::uri::Authority;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::Response;
use axum::Router;
use clap::{ArgAction, Command, CommandFactory};
use serde_json::{json, Value};
use tokio::sync::Semaphore;

pub const DEFAULT_PORT: u16 = 57108;

/// The version of the API's rules (routes, methods, parameters, answers, jobs), for clients such as the WebUI to
/// check. Raised only by changes that break existing clients; new commands and options do not raise it.
pub const API_VERSION: &str = "1";

const MAX_BODY: usize = 1 << 20;

/// Commands that are not reachable over HTTP.
const HIDDEN: &[&str] = &["serve"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verb {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Verb {
    fn method(self) -> Method {
        match self {
            Get => Method::GET,
            Post => Method::POST,
            Put => Method::PUT,
            Patch => Method::PATCH,
            Delete => Method::DELETE,
        }
    }
}

use Verb::*;

/// How the HTTP method of a command is chosen.
#[derive(Clone, Copy, Debug)]
enum Rule {
    Fixed(Verb),
    /// `with` when the argument is given, `without` otherwise (`analytics excluded --set`).
    Arg {
        arg: &'static str,
        without: Verb,
        with: Verb,
    },
    /// Chosen by the value of an argument (`sns block-delete <action>`).
    Value {
        arg: &'static str,
        values: &'static [(&'static str, Verb)],
    },
}

/// The HTTP method of every command: `GET` reads (it may fill caches that can be rebuilt), `POST` runs a task or
/// writes files, `PUT` changes a setting, `DELETE` removes one. Commands that would write WeChat's databases keep
/// the method their action implies and are refused by the command itself (`unsupported`, HTTP 501).
/// `tests::every_command_has_a_method` keeps this table in step with the command tree.
const METHODS: &[(&str, Rule)] = &[
    ("config path", Rule::Fixed(Get)),
    ("config list", Rule::Fixed(Get)),
    ("config get", Rule::Fixed(Get)),
    ("config set", Rule::Fixed(Put)),
    ("config unset", Rule::Fixed(Delete)),
    ("config clear", Rule::Fixed(Delete)),
    ("config import", Rule::Fixed(Post)),
    ("db detect", Rule::Fixed(Get)),
    ("db scan", Rule::Fixed(Get)),
    ("db test", Rule::Fixed(Get)),
    ("db open", Rule::Fixed(Get)),
    ("db wxid", Rule::Fixed(Get)),
    ("key db", Rule::Fixed(Post)),
    ("key image", Rule::Fixed(Post)),
    ("key scan-image", Rule::Fixed(Post)),
    ("chat sessions", Rule::Fixed(Get)),
    ("chat messages", Rule::Fixed(Get)),
    ("chat latest", Rule::Fixed(Get)),
    ("chat search", Rule::Fixed(Get)),
    ("chat contacts", Rule::Fixed(Get)),
    ("chat contact", Rule::Fixed(Get)),
    ("chat update-message", Rule::Fixed(Patch)),
    ("chat delete-message", Rule::Fixed(Delete)),
    ("chat anti-revoke sessions", Rule::Fixed(Get)),
    ("chat anti-revoke check", Rule::Fixed(Get)),
    ("chat anti-revoke install", Rule::Fixed(Put)),
    ("chat anti-revoke uninstall", Rule::Fixed(Delete)),
    ("chat message", Rule::Fixed(Get)),
    ("chat dates", Rule::Fixed(Get)),
    ("chat date-counts", Rule::Fixed(Get)),
    ("chat counts", Rule::Fixed(Get)),
    ("chat statuses", Rule::Fixed(Get)),
    ("chat detail", Rule::Fixed(Get)),
    ("chat mark-read", Rule::Fixed(Post)),
    ("chat tab-counts", Rule::Fixed(Get)),
    ("chat export-stats", Rule::Fixed(Get)),
    (
        "chat group-hint",
        Rule::Arg {
            arg: "set",
            without: Get,
            with: Put,
        },
    ),
    ("chat resources", Rule::Fixed(Get)),
    ("chat images", Rule::Fixed(Get)),
    ("chat voice-messages", Rule::Fixed(Get)),
    ("chat media-stream", Rule::Fixed(Get)),
    ("chat transfer-names", Rule::Fixed(Get)),
    ("chat voice", Rule::Fixed(Post)),
    ("chat voice-data", Rule::Fixed(Post)),
    ("chat image-data", Rule::Fixed(Post)),
    ("chat voice-cache", Rule::Fixed(Get)),
    ("chat voice-preload", Rule::Fixed(Post)),
    ("chat emoji", Rule::Fixed(Post)),
    ("export sessions", Rule::Fixed(Post)),
    ("export contacts", Rule::Fixed(Post)),
    ("export footprint", Rule::Fixed(Post)),
    ("export media", Rule::Fixed(Post)),
    ("export messages", Rule::Fixed(Post)),
    ("analytics overall", Rule::Fixed(Get)),
    ("analytics rankings", Rule::Fixed(Get)),
    ("analytics time", Rule::Fixed(Get)),
    (
        "analytics excluded",
        Rule::Arg {
            arg: "set",
            without: Get,
            with: Put,
        },
    ),
    ("analytics exclude-candidates", Rule::Fixed(Get)),
    ("group list", Rule::Fixed(Get)),
    ("group members", Rule::Fixed(Get)),
    ("group ranking", Rule::Fixed(Get)),
    ("group hours", Rule::Fixed(Get)),
    ("group media", Rule::Fixed(Get)),
    ("group member", Rule::Fixed(Get)),
    ("group member-messages", Rule::Fixed(Get)),
    ("group export-member-messages", Rule::Fixed(Post)),
    ("group export-members", Rule::Fixed(Post)),
    ("report annual years", Rule::Fixed(Get)),
    ("report annual generate", Rule::Fixed(Post)),
    ("report dual generate", Rule::Fixed(Post)),
    ("sns timeline", Rule::Fixed(Get)),
    ("sns users", Rule::Fixed(Get)),
    ("sns debug-resource", Rule::Fixed(Get)),
    ("sns stats", Rule::Fixed(Get)),
    ("sns post-counts", Rule::Fixed(Get)),
    ("sns export", Rule::Fixed(Post)),
    ("sns media", Rule::Fixed(Post)),
    ("sns download-emoji", Rule::Fixed(Post)),
    ("sns download-image", Rule::Fixed(Post)),
    (
        "sns block-delete",
        Rule::Value {
            arg: "action",
            values: &[("check", Get), ("install", Put), ("uninstall", Delete)],
        },
    ),
    ("sns delete", Rule::Fixed(Delete)),
    ("biz accounts", Rule::Fixed(Get)),
    ("biz messages", Rule::Fixed(Get)),
    ("biz pay-records", Rule::Fixed(Get)),
    ("image decrypt", Rule::Fixed(Post)),
    ("image resolve-cache", Rule::Fixed(Get)),
    ("image resolve-batch", Rule::Fixed(Post)),
    ("image auto-download start", Rule::Fixed(Post)),
    ("image auto-download status", Rule::Fixed(Get)),
    ("video info", Rule::Fixed(Get)),
    ("video parse-md5", Rule::Fixed(Get)),
    ("insight test", Rule::Fixed(Post)),
    ("insight trigger", Rule::Fixed(Post)),
    ("insight records", Rule::Fixed(Get)),
    ("insight get", Rule::Fixed(Get)),
    ("insight mark-read", Rule::Fixed(Put)),
    ("insight clear", Rule::Fixed(Delete)),
    ("insight today-stats", Rule::Fixed(Get)),
    ("insight scan", Rule::Fixed(Post)),
    ("insight footprint", Rule::Fixed(Get)),
    ("insight footprint-summary", Rule::Fixed(Post)),
    ("runtime info", Rule::Fixed(Get)),
    ("runtime manifest", Rule::Fixed(Get)),
    ("backup create", Rule::Fixed(Post)),
    ("backup inspect", Rule::Fixed(Get)),
    ("backup restore", Rule::Fixed(Post)),
    ("cache list", Rule::Fixed(Get)),
    ("cache clear", Rule::Fixed(Delete)),
    ("cache clear-account", Rule::Fixed(Delete)),
    ("ffmpeg install", Rule::Fixed(Post)),
    ("ffmpeg path", Rule::Fixed(Get)),
    ("ffmpeg set baseurl", Rule::Fixed(Put)),
    ("ffmpeg unset baseurl", Rule::Fixed(Delete)),
    ("lang", Rule::Fixed(Put)),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// `--force`: given as `true` / `false`.
    Flag,
    /// One value.
    Single,
    /// Any number of values (repeated query keys or a JSON array).
    Multi,
    /// Like `Multi`, but may also be given with no value at all (`analytics excluded --set`).
    MultiOrBare,
}

#[derive(Clone, Debug)]
struct Param {
    /// The long option name, or the argument id in kebab case for positional arguments.
    name: String,
    kind: Kind,
    /// Position of a positional argument.
    index: Option<usize>,
}

#[derive(Clone, Debug)]
struct Endpoint {
    path: Vec<String>,
    rule: Rule,
    params: Vec<Param>,
}

/// Every leaf command of the CLI with its method rule and parameters.
struct Endpoints(BTreeMap<String, Endpoint>);

impl Endpoints {
    fn new() -> Self {
        let mut cmd = super::Cli::command();
        cmd.build();
        let mut out = BTreeMap::new();
        collect(&cmd, &mut Vec::new(), &mut out);
        Self(out)
    }

    fn get(&self, path: &[String]) -> Option<&Endpoint> {
        self.0.get(&path.join(" "))
    }
}

fn collect(cmd: &Command, path: &mut Vec<String>, out: &mut BTreeMap<String, Endpoint>) {
    let subs: Vec<&Command> = cmd
        .get_subcommands()
        .filter(|c| c.get_name() != "help")
        .collect();
    if subs.is_empty() {
        let key = path.join(" ");
        if HIDDEN.contains(&key.as_str()) {
            return;
        }
        // a command missing from METHODS is left out here and reported by the tests
        let Some(rule) = METHODS.iter().find(|(p, _)| *p == key).map(|(_, r)| *r) else {
            return;
        };
        out.insert(
            key,
            Endpoint {
                path: path.clone(),
                rule,
                params: params_of(cmd),
            },
        );
        return;
    }
    for sub in subs {
        path.push(sub.get_name().to_string());
        collect(sub, path, out);
        path.pop();
    }
}

fn params_of(cmd: &Command) -> Vec<Param> {
    api_args(cmd).map(param_of).collect()
}

/// The arguments of a command that a request may set.
fn api_args(cmd: &Command) -> impl Iterator<Item = &clap::Arg> {
    cmd.get_arguments().filter(|a| {
        let id = a.get_id().as_str();
        !a.is_global_set() && !a.is_hide_set() && id != "help" && id != "version"
    })
}

fn param_of(a: &clap::Arg) -> Param {
    let name = a
        .get_long()
        .map(str::to_string)
        .unwrap_or_else(|| a.get_id().as_str().replace('_', "-"));
    let kind = match a.get_action() {
        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count => Kind::Flag,
        _ => {
            let range = a
                .get_num_args()
                .unwrap_or(clap::builder::ValueRange::SINGLE);
            if range.min_values() == 0 && range.takes_values() {
                Kind::MultiOrBare
            } else if range.max_values() > 1 || matches!(a.get_action(), ArgAction::Append) {
                Kind::Multi
            } else {
                Kind::Single
            }
        }
    };
    Param {
        name,
        kind,
        index: a.get_index(),
    }
}

/// Request parameters by name: query strings, overridden by the keys of a JSON body.
type Params = BTreeMap<String, Vec<Value>>;

fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn flag_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_i64().map(|n| n != 0),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "" | "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// The parameter a request key names: the long option name, with `_` accepted for `-`.
fn find_param<'a>(endpoint: &'a Endpoint, key: &str) -> Option<&'a Param> {
    let key = key.replace('_', "-");
    endpoint.params.iter().find(|p| p.name == key)
}

impl Endpoint {
    fn command_name(&self) -> String {
        self.path.join(" ")
    }

    /// The verb a request with these parameters must use, or `None` when it cannot be told yet (the command
    /// then reports the missing argument itself).
    fn verb(&self, params: &Params) -> Option<Verb> {
        match self.rule {
            Rule::Fixed(v) => Some(v),
            Rule::Arg { arg, without, with } => {
                let given = params.iter().any(|(k, values)| {
                    find_param(self, k).is_some_and(|p| p.name == arg)
                        && !values.iter().all(|v| v == &Value::Bool(false))
                });
                Some(if given { with } else { without })
            }
            Rule::Value { arg, values } => {
                let given = params
                    .iter()
                    .find(|(k, _)| find_param(self, k).is_some_and(|p| p.name == arg))
                    .and_then(|(_, v)| v.first())
                    .map(text_of)?;
                values
                    .iter()
                    .find(|(name, _)| *name == given)
                    .map(|(_, v)| *v)
            }
        }
    }

    fn verbs(&self) -> Vec<Verb> {
        match self.rule {
            Rule::Fixed(v) => vec![v],
            Rule::Arg { without, with, .. } => vec![without, with],
            Rule::Value { values, .. } => {
                let mut out: Vec<Verb> = Vec::new();
                for (_, v) in values {
                    if !out.contains(v) {
                        out.push(*v);
                    }
                }
                out
            }
        }
    }

    /// The command line (after `weflow` and the global options) for these parameters. Options are written as
    /// `--name=value` and positional arguments after `--`, so values starting with `-` stay values. The command
    /// itself checks the result, exactly as on a terminal.
    fn argv(&self, params: &Params) -> Result<Vec<String>, String> {
        let mut options = Vec::new();
        let mut positionals: Vec<(usize, Vec<String>)> = Vec::new();
        for (key, values) in params {
            let Some(param) = find_param(self, key) else {
                let known: Vec<&str> = self.params.iter().map(|p| p.name.as_str()).collect();
                return Err(if known.is_empty() {
                    format!(
                        "unknown parameter `{key}`: `{}` takes no parameters",
                        self.command_name()
                    )
                } else {
                    format!(
                        "unknown parameter `{key}` for `{}`; expected one of: {}",
                        self.command_name(),
                        known.join(", ")
                    )
                });
            };
            let texts = || -> Vec<String> {
                values
                    .iter()
                    .flat_map(|v| match v {
                        Value::Array(items) if param.kind != Kind::Single => {
                            items.iter().map(text_of).collect()
                        }
                        other => vec![text_of(other)],
                    })
                    .collect()
            };
            if let Some(index) = param.index {
                positionals.push((index, texts()));
                continue;
            }
            match param.kind {
                Kind::Flag => {
                    for v in values {
                        match flag_value(v) {
                            Some(true) => options.push(format!("--{}", param.name)),
                            Some(false) => {}
                            None => {
                                return Err(format!(
                                    "parameter `{}` is a switch: pass true or false",
                                    param.name
                                ))
                            }
                        }
                    }
                }
                Kind::MultiOrBare if texts().iter().all(String::is_empty) => {
                    options.push(format!("--{}", param.name));
                }
                _ => {
                    for text in texts() {
                        options.push(format!("--{}={text}", param.name));
                    }
                }
            }
        }
        let mut argv = self.path.clone();
        argv.extend(options);
        if !positionals.is_empty() {
            positionals.sort_by_key(|(index, _)| *index);
            argv.push("--".into());
            argv.extend(positionals.into_iter().flat_map(|(_, v)| v));
        }
        Ok(argv)
    }
}

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
    /// Images and voices of the `/api/` media routes decoded at once; more wait in line (see [`MediaLimit`]).
    pub max_media: usize,
    /// Answer only requests whose `Host` names this machine (see [`local_host`]); set when listening on a
    /// loopback address.
    pub check_host: bool,
    /// Print each request on stderr (`--log-requests`): method, path, status and time, never the query.
    pub log_requests: bool,
}

struct AppState {
    cfg: ApiConfig,
    endpoints: Endpoints,
    jobs: Arc<jobs::Jobs>,
    tasks: Arc<Semaphore>,
    reads: Arc<Semaphore>,
    media: MediaLimit,
    /// `/openapi.json` in English and in Chinese, built when first asked for.
    openapi: [OnceLock<String>; 2],
    /// The hub of the `/api/` routes.
    hub: Arc<in_process::Hub>,
    /// `/api/push/messages`.
    push: push::Push,
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

/// Media requests waiting for a place at once, beyond those decoded.
const MEDIA_QUEUE: usize = 256;
/// How long a media request waits for a place before it answers `429`.
const MEDIA_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The limit of the `/api/` media routes. A screen of thumbnails asks for its images at once and each takes a few
/// milliseconds, so a request over the limit waits for a place instead of answering `429`, unless the line is full
/// or the wait too long.
struct MediaLimit {
    /// Requests decoded and waiting.
    admitted: Arc<Semaphore>,
    decoding: Arc<Semaphore>,
}

impl MediaLimit {
    fn new(max: usize) -> Self {
        let max = max.max(1);
        Self {
            admitted: Arc::new(Semaphore::new(max + MEDIA_QUEUE)),
            decoding: Arc::new(Semaphore::new(max)),
        }
    }

    /// A place to decode in, after waiting at most `wait`; `None` when the line is full or the wait ran out.
    async fn enter(
        &self,
        wait: std::time::Duration,
    ) -> Option<(
        tokio::sync::OwnedSemaphorePermit,
        tokio::sync::OwnedSemaphorePermit,
    )> {
        let admitted = self.admitted.clone().try_acquire_owned().ok()?;
        let decoding = tokio::time::timeout(wait, self.decoding.clone().acquire_owned())
            .await
            .ok()?
            .ok()?;
        Some((admitted, decoding))
    }
}

pub fn router(cfg: ApiConfig) -> Router {
    let tasks = Arc::new(Semaphore::new(cfg.max_tasks.max(1)));
    let reads = Arc::new(Semaphore::new(cfg.max_reads.max(1)));
    let media = MediaLimit::new(cfg.max_media);
    let hub = Arc::new(in_process::Hub::default());
    hub.warm();
    Router::new()
        .fallback(dispatch)
        .layer(compression())
        .with_state(Arc::new(AppState {
            cfg,
            endpoints: Endpoints::new(),
            jobs: Arc::new(jobs::Jobs::new()),
            tasks,
            reads,
            media,
            openapi: Default::default(),
            hub,
            push: Default::default(),
        }))
}

/// gzip or br, as `Accept-Encoding` asks (issue #155): a page of messages or the contacts are mostly repeated
/// column names and XML. Not for what is compressed already or streamed: images, voices, videos, the files of
/// `return=file`, the event streams, and a part of a file (`206`).
fn compression(
) -> tower_http::compression::CompressionLayer<impl tower_http::compression::Predicate> {
    use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};
    let whole =
        |status: StatusCode, _: axum::http::Version, _: &HeaderMap, _: &axum::http::Extensions| {
            status != StatusCode::PARTIAL_CONTENT
        };
    tower_http::compression::CompressionLayer::new().compress_when(
        DefaultPredicate::new()
            .and(NotForContentType::const_new("audio/"))
            .and(NotForContentType::const_new("video/"))
            .and(NotForContentType::const_new("multipart/"))
            .and(NotForContentType::const_new("application/octet-stream"))
            .and(whole),
    )
}

/// The default of [`ApiConfig::max_reads`] and [`ApiConfig::max_media`]: one per CPU.
pub fn default_max_reads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get())
}

/// Serves the API on an already-bound listener until the future is dropped.
pub async fn serve(cfg: ApiConfig, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, router(cfg)).await
}

fn json_body(status: StatusCode, body: impl Into<Body>) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(body.into())
        .unwrap()
}

/// An error answered by the server itself, in the shape of a failed `--json` command.
fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    let body = json!({ "success": false, "error": { "code": code, "message": message } });
    let mut resp = json_body(status, body.to_string());
    resp.headers_mut().insert(
        "x-weflow-error-code",
        HeaderValue::from_str(code).unwrap_or(HeaderValue::from_static("error")),
    );
    resp
}

fn method_not_allowed(allowed: &[Verb]) -> Response {
    let allow = allowed
        .iter()
        .map(|v| v.method().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let mut resp = error_response(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        &format!("method not allowed; use {allow}"),
    );
    resp.headers_mut()
        .insert(header::ALLOW, HeaderValue::from_str(&allow).unwrap());
    resp
}

fn safe_equal(a: &str, b: &str) -> bool {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    x.len() == y.len() && x.iter().zip(y).fold(0u8, |acc, (p, q)| acc | (p ^ q)) == 0
}

fn authorized(token: &str, headers: &HeaderMap, query: &[(String, String)]) -> bool {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.get(..7)
                .filter(|p| p.eq_ignore_ascii_case("bearer "))
                .map(|_| v[7..].trim())
        });
    bearer.is_some_and(|t| safe_equal(t, token))
        || query
            .iter()
            .any(|(k, v)| k == "access_token" && safe_equal(v.trim(), token))
}

fn url_decode(s: &str) -> String {
    String::from_utf8_lossy(&weflow_core::message::percent_decode_bytes(
        &s.replace('+', " "),
    ))
    .into_owned()
}

fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    query
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (url_decode(k), url_decode(v))
        })
        .collect()
}

/// Allows pages served from this machine (the WebUI's dev server, for one) to call the API.
fn cors(headers: &HeaderMap, mut resp: Response) -> Response {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let host = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    let local = host.is_some_and(|h| {
        let name = match h.rsplit_once(':') {
            Some((name, port)) if !name.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
                name
            }
            _ => h,
        };
        matches!(name, "localhost" | "127.0.0.1" | "[::1]")
    });
    let h = resp.headers_mut();
    if local {
        h.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_str(origin).unwrap(),
        );
        h.insert(header::VARY, HeaderValue::from_static("Origin"));
        // the stages of `Server-Timing` in the page's own performance entries
        h.insert(
            "timing-allow-origin",
            HeaderValue::from_str(origin).unwrap(),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static(
                "Content-Type, Authorization, Accept-Language, Prefer, Last-Event-ID",
            ),
        );
        h.insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static(
                "X-WeFlow-Exit-Code, X-WeFlow-Error-Code, Location, Retry-After, Server-Timing",
            ),
        );
    }
    resp
}

/// Whether the request names this machine: `localhost` or a loopback address, with any port. A page of another
/// site whose name it made resolve to 127.0.0.1 (DNS rebinding) sends that name and is refused.
fn local_host(uri: &Uri, headers: &HeaderMap) -> bool {
    let authority = uri.authority().cloned().or_else(|| {
        headers
            .get(header::HOST)?
            .to_str()
            .ok()?
            .parse::<Authority>()
            .ok()
    });
    let Some(authority) = authority else {
        return false;
    };
    let host = authority.host().trim_end_matches('.');
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
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
        Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .unwrap()
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

/// `429 Too Many Requests` of a media route: its line of waiting requests is full, or the wait ran out.
fn busy_media() -> Response {
    busy_with("too many images and voices are asked for; retry later", "1")
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
struct FileOut {
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
        let body = json!({
            "status": "ok",
            "apiVersion": API_VERSION,
            "version": env!("WEFLOW_VERSION"),
        });
        return json_body(StatusCode::OK, body.to_string());
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
        let zh = match request_lang(&parts.headers) {
            Some(lang) => lang == "zh",
            None => weflow_core::locale::current() == weflow_core::locale::Lang::Zh,
        };
        let doc = st.openapi[zh as usize].get_or_init(|| openapi::document(zh).to_string());
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
        return st.push.stream(st.hub.clone(), last_id);
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
        return jobs::route(&st.jobs, &parts.method, &segments, last_event_id).await;
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
        if background {
            return bad_request(
                "return=file is answered right away; send it without `Prefer: respond-async`",
            );
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

    if media {
        if params
            .keys()
            .any(|k| find_param(endpoint, k).is_some_and(|p| p.name == "out"))
        {
            return bad_request(
                "an /api/ media route answers with the file itself: leave out `out`",
            );
        }
        // satisfies the command line; nothing is written there
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
    let lang = request_lang(&parts.headers);
    let limit = st.limit(verb);
    if background {
        if std != Std::Json {
            return bad_request(
                "std=out and std=err are answered right away; send them without `Prefer: respond-async`",
            );
        }
        let job = st.jobs.start(st.cfg.exe.clone(), full, lang, limit.clone());
        return jobs::accepted(&job);
    }
    let mut timing = Timing::default();
    let (_permit, _decoding) = if media {
        let waited = std::time::Instant::now();
        let Some((admitted, decoding)) = st.media.enter(MEDIA_WAIT).await else {
            return busy_media();
        };
        timing.stage("wait", Some("in line for a media place"), waited);
        (admitted, Some(decoding))
    } else {
        let Ok(permit) = limit.clone().try_acquire_owned() else {
            return busy(verb == Some(Post));
        };
        (permit, None)
    };
    let ran = std::time::Instant::now();
    if media {
        let done = in_process::media(&st.hub, &argv).await;
        timing.stage("cmd", Some("in process"), ran);
        match done {
            Some(Ok(file)) => return timing.on(media_response(&parts.headers, file)),
            Some(Err((doc, exit))) => return timing.on(in_process_document(&doc, exit, None)),
            // the child process reports the command line error; with `out` set to `-` and the command line
            // refused, it writes nothing
            None => {}
        }
    } else if in_process {
        if let Some((doc, exit)) = in_process::run(&st.hub, &argv).await {
            timing.stage("cmd", Some("in process"), ran);
            return timing.on(in_process_document(&doc, exit, fields.as_deref()));
        }
    }
    if let Some(dir) = file_out.as_ref().and_then(|f| f.dir.as_ref()) {
        if let Err(err) = std::fs::create_dir_all(dir) {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "runtime_error",
                &format!("failed to create {}: {err}", dir.display()),
            );
        }
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
    resp.headers_mut().insert(
        "x-weflow-exit-code",
        HeaderValue::from_str(&exit.to_string()).unwrap(),
    );
    timing.on(resp)
}

/// The `--json` document of a command run in this process, answered as the child process prints it.
fn in_process_document(doc: &Value, exit: u8, fields: Option<&[String]>) -> Response {
    let body = match fields {
        Some(fields) => json_line(&only_fields(doc.clone(), fields)),
        None => json_line(doc),
    };
    let mut resp = document_response(doc, status_for(doc, exit.into()), body);
    resp.headers_mut()
        .insert("x-weflow-exit-code", HeaderValue::from(i32::from(exit)));
    resp
}

/// A `--json` document as the command prints it: on one line.
fn json_line(doc: &Value) -> String {
    format!("{}\n", serde_json::to_string(doc).unwrap())
}

/// The names of `?fields=`: repeated, comma-separated, or a JSON array.
fn field_names(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .flat_map(|v| match v {
            Value::Array(items) => items.iter().map(text_of).collect(),
            other => vec![text_of(other)],
        })
        .flat_map(|v| {
            v.split(',')
                .map(|f| f.trim().to_string())
                .collect::<Vec<_>>()
        })
        .filter(|f| !f.is_empty())
        .collect()
}

/// `?fields=` (issue #155): of a successful document, only the named keys of each row: the objects of `data` when
/// it is a list, or of each list in `data` when it is an object. Anything else is left as it is.
fn only_fields(mut doc: Value, fields: &[String]) -> Value {
    if doc["success"] != true {
        return doc;
    }
    let keep = |row: &mut Value| {
        if let Value::Object(row) = row {
            row.retain(|k, _| fields.iter().any(|f| f == k));
        }
    };
    match doc.get_mut("data") {
        Some(Value::Array(rows)) => rows.iter_mut().for_each(keep),
        Some(Value::Object(data)) => {
            for value in data.values_mut() {
                if let Value::Array(rows) = value {
                    rows.iter_mut().for_each(keep);
                }
            }
        }
        _ => {}
    }
    doc
}

/// The file of an `/api/` media route. Its `ETag` lets a page ask again with `If-None-Match` and get `304`; the
/// file of a message rarely changes (a thumbnail replaced by the full image), so it may be kept for an hour.
fn media_response(headers: &HeaderMap, file: in_process::MediaFile) -> Response {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file.bytes.hash(&mut hasher);
    let etag = format!("\"{:016x}\"", hasher.finish());
    let unchanged = headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|tag| {
            let tag = tag.trim();
            tag == "*" || tag.trim_start_matches("W/") == etag
        });
    let builder = Response::builder()
        .header(header::ETAG, &etag)
        .header(header::CACHE_CONTROL, "private, max-age=3600")
        .header(header::ACCEPT_RANGES, "bytes");
    if unchanged {
        return builder
            .status(StatusCode::NOT_MODIFIED)
            .body(Body::empty())
            .unwrap();
    }
    let len = file.bytes.len();
    // `If-Range` asks for the part only while the file is the one it names
    let same = headers
        .get(header::IF_RANGE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|tag| tag.trim() == etag);
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .filter(|_| same)
        .and_then(|v| byte_range(v, len));
    let builder = builder.header(header::CONTENT_TYPE, file.mime);
    match range {
        None => builder
            .status(StatusCode::OK)
            .body(Body::from(file.bytes))
            .unwrap(),
        Some(Ok((first, last))) => {
            let mut bytes = file.bytes;
            bytes.truncate(last + 1);
            bytes.drain(..first);
            builder
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, format!("bytes {first}-{last}/{len}"))
                .body(Body::from(bytes))
                .unwrap()
        }
        Some(Err(())) => builder
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{len}"))
            .body(Body::empty())
            .unwrap(),
    }
}

/// The one range of a `Range: bytes=…` header for a file of `len` bytes, as (first, last) byte, clamped to the file;
/// `Err` when it starts past the end. `None` (the whole file) for a header that names several ranges or that this
/// does not read, as RFC 9110 allows.
fn byte_range(header: &str, len: usize) -> Option<Result<(usize, usize), ()>> {
    let spec = header.trim().strip_prefix("bytes=")?.trim();
    if spec.contains(',') {
        return None;
    }
    let (from, to) = spec.split_once('-')?;
    let (from, to) = (from.trim(), to.trim());
    if from.is_empty() {
        // the last `to` bytes
        let n: usize = to.parse().ok()?;
        if n == 0 {
            return Some(Err(()));
        }
        return Some(Ok((len.saturating_sub(n), len.checked_sub(1)?)));
    }
    let first: usize = from.parse().ok()?;
    let last = match to {
        "" => len.saturating_sub(1),
        to => to.parse::<usize>().ok()?.min(len.saturating_sub(1)),
    };
    if first >= len {
        return Some(Err(()));
    }
    (first <= last).then_some(Ok((first, last)))
}

/// A command's `--json` document, with its error code in a header.
fn document_response(doc: &Value, status: StatusCode, body: impl Into<Body>) -> Response {
    let mut resp = json_body(status, body);
    if let Some(code) = doc["error"]["code"].as_str().filter(|c| !c.is_empty()) {
        if let Ok(v) = HeaderValue::from_str(code) {
            resp.headers_mut().insert("x-weflow-error-code", v);
        }
    }
    resp
}

/// `zh` or `en` from `Accept-Language`; `None` leaves the command to its usual language detection.
fn request_lang(headers: &HeaderMap) -> Option<&'static str> {
    let value = headers.get(header::ACCEPT_LANGUAGE)?.to_str().ok()?.trim();
    let first = value.split([',', ';']).next()?.trim();
    if first.is_empty() || first == "*" {
        return None;
    }
    Some(if first.to_ascii_lowercase().starts_with("zh") {
        "zh"
    } else {
        "en"
    })
}

/// `weflow <args>` with no stdin and both outputs captured.
fn command(exe: &std::path::Path, args: &[String], lang: Option<&str>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // the command is killed if its request or job goes away before it ends
        .kill_on_drop(true);
    if let Some(lang) = lang {
        cmd.env("WEFLOW_LANG", lang);
    }
    cmd
}

/// Set for a command whose stdin carries the database keys the server derived (see [`spawn`]).
pub const KEYS_ON_STDIN: &str = "WEFLOW_KEYS_ON_STDIN";

/// [`command`], started, with the database keys this server derived written to its stdin
/// ([`weflow_core::page_keys::export_keys`]): opening a database derives its key first (about 0.1 s), and the
/// command does not have to do it again. The keys go through the pipe only, never to a file or the environment.
pub(super) async fn spawn(
    exe: &std::path::Path,
    args: &[String],
    lang: Option<&str>,
) -> std::io::Result<tokio::process::Child> {
    let keys = weflow_core::page_keys::export_keys();
    let mut cmd = command(exe, args, lang);
    if !keys.is_empty() {
        cmd.stdin(std::process::Stdio::piped())
            .env(KEYS_ON_STDIN, "1");
    }
    let mut child = cmd.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // a command that ends without reading them did not need them; closing the pipe ends what it reads
        let _ = tokio::io::AsyncWriteExt::write_all(&mut stdin, &keys).await;
    }
    Ok(child)
}

async fn run(
    exe: &std::path::Path,
    args: &[String],
    lang: Option<&str>,
) -> std::io::Result<std::process::Output> {
    spawn(exe, args, lang).await?.wait_with_output().await
}

/// The HTTP status of a `--json` document: by its error code first, then by the exit code.
fn status_for(doc: &Value, exit: i32) -> StatusCode {
    if exit == 0 && doc["success"] == Value::Bool(true) {
        return StatusCode::OK;
    }
    let error = &doc["error"];
    match error["code"].as_str().unwrap_or("") {
        "invalid_arguments" => StatusCode::BAD_REQUEST,
        "config_error" => StatusCode::CONFLICT,
        "unsupported" => StatusCode::NOT_IMPLEMENTED,
        "native_error" => StatusCode::INTERNAL_SERVER_ERROR,
        "runtime_error" if error["details"]["failureKind"] == "not_found" => StatusCode::NOT_FOUND,
        "runtime_error" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => status_for_exit(exit),
    }
}

fn status_for_exit(exit: i32) -> StatusCode {
    match exit {
        0 => StatusCode::OK,
        2 => StatusCode::BAD_REQUEST,
        3 => StatusCode::CONFLICT,
        // the client closed the request (nginx's 499)
        130 => StatusCode::from_u16(499).unwrap(),
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// A random access token for a run without a configured one.
pub fn generate_token() -> String {
    let bytes: [u8; 16] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Query / body keys the server reads itself; no command argument may use them.
    const RESERVED_PARAMS: &[&str] = &["std", "return", "access_token", "fields"];

    /// First path segments that are not commands (the WebUI, later endpoints).
    const RESERVED_PATHS: &[&str] = &["api", "assets", "health", "jobs", "openapi.json", "ui"];

    fn leaves(cmd: &Command, path: &mut Vec<String>, out: &mut Vec<String>) {
        let subs: Vec<&Command> = cmd
            .get_subcommands()
            .filter(|c| c.get_name() != "help")
            .collect();
        if subs.is_empty() {
            out.push(path.join(" "));
        }
        for sub in subs {
            path.push(sub.get_name().to_string());
            leaves(sub, path, out);
            path.pop();
        }
    }

    fn all_leaves() -> Vec<String> {
        let mut cmd = super::super::Cli::command();
        cmd.build();
        let mut out = Vec::new();
        leaves(&cmd, &mut Vec::new(), &mut out);
        out
    }

    #[test]
    fn every_command_has_a_method() {
        let leaves = all_leaves();
        let missing: Vec<&String> = leaves
            .iter()
            .filter(|l| !HIDDEN.contains(&l.as_str()) && !METHODS.iter().any(|(p, _)| p == l))
            .collect();
        assert!(
            missing.is_empty(),
            "commands without an HTTP method: {missing:?}"
        );
        let stale: Vec<&str> = METHODS
            .iter()
            .map(|(p, _)| *p)
            .filter(|p| !leaves.iter().any(|l| l == p))
            .collect();
        assert!(
            stale.is_empty(),
            "METHODS lists unknown commands: {stale:?}"
        );
        let mut seen = std::collections::HashSet::new();
        for (p, _) in METHODS {
            assert!(seen.insert(*p), "{p} is listed twice");
        }
    }

    #[test]
    fn file_commands_write_to_out() {
        let endpoints = Endpoints::new();
        for (name, _, _) in files::FILE_COMMANDS {
            let ep = endpoints
                .0
                .get(*name)
                .unwrap_or_else(|| panic!("{name} is not a command"));
            // `--out`, or the positional `out` of the group exports
            assert!(
                ep.params.iter().any(|p| p.name == "out"),
                "{name} has no out"
            );
        }
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

    #[test]
    fn rules_name_real_arguments() {
        let endpoints = Endpoints::new();
        for (path, rule) in METHODS {
            let arg = match rule {
                Rule::Fixed(_) => continue,
                Rule::Arg { arg, .. } | Rule::Value { arg, .. } => arg,
            };
            let ep = endpoints.0.get(*path).unwrap();
            assert!(
                ep.params.iter().any(|p| p.name == *arg),
                "{path} has no argument {arg}"
            );
        }
    }

    #[test]
    fn reserved_names_stay_free() {
        let mut cmd = super::super::Cli::command();
        cmd.build();
        for sub in cmd.get_subcommands() {
            assert!(
                !RESERVED_PATHS.contains(&sub.get_name()),
                "command `{}` takes a reserved path",
                sub.get_name()
            );
        }
        for ep in Endpoints::new().0.values() {
            for p in &ep.params {
                assert!(
                    !RESERVED_PARAMS.contains(&p.name.as_str())
                        && !RESERVED_PARAMS.contains(&p.name.replace('-', "_").as_str()),
                    "{} uses the reserved parameter {}",
                    ep.command_name(),
                    p.name
                );
            }
        }
    }

    fn params(pairs: &[(&str, Value)]) -> Params {
        let mut out = Params::new();
        for (k, v) in pairs {
            out.entry(k.to_string()).or_default().push(v.clone());
        }
        out
    }

    fn argv(path: &str, pairs: &[(&str, Value)]) -> Result<Vec<String>, String> {
        let endpoints = Endpoints::new();
        endpoints.0.get(path).unwrap().argv(&params(pairs))
    }

    #[test]
    fn builds_the_command_line() {
        assert_eq!(argv("config list", &[]).unwrap(), ["config", "list"]);
        assert_eq!(
            argv(
                "chat latest",
                &[("session-id", json!("-a b")), ("limit", json!(5))]
            )
            .unwrap(),
            ["chat", "latest", "--limit=5", "--", "-a b"]
        );
        // snake case is accepted for kebab case
        assert_eq!(
            argv("chat latest", &[("session_id", json!("x"))]).unwrap(),
            ["chat", "latest", "--", "x"]
        );
        // repeated positional values, from repeated keys or an array
        assert_eq!(
            argv(
                "chat counts",
                &[("sessions", json!("a")), ("sessions", json!("b"))]
            )
            .unwrap(),
            ["chat", "counts", "--", "a", "b"]
        );
        assert_eq!(
            argv("chat counts", &[("sessions", json!(["a", "微信"]))]).unwrap(),
            ["chat", "counts", "--", "a", "微信"]
        );
        // switches
        assert_eq!(
            argv(
                "chat detail",
                &[
                    ("session-id", json!("s")),
                    ("fast", json!(true)),
                    ("extra", json!("false"))
                ]
            )
            .unwrap(),
            ["chat", "detail", "--fast", "--", "s"]
        );
        assert!(argv("chat detail", &[("fast", json!("maybe"))]).is_err());
        // an option that takes any number of values, none included
        assert_eq!(
            argv("analytics excluded", &[("set", json!(""))]).unwrap(),
            ["analytics", "excluded", "--set"]
        );
        assert_eq!(
            argv("analytics excluded", &[("set", json!(["a", "b"]))]).unwrap(),
            ["analytics", "excluded", "--set=a", "--set=b"]
        );
        // a JSON value for a single-valued argument is passed as JSON text
        assert_eq!(
            argv(
                "chat voice-preload",
                &[
                    ("session-id", json!("s")),
                    ("messages-json", json!([{"localId": 1}]))
                ]
            )
            .unwrap(),
            ["chat", "voice-preload", "--", "s", r#"[{"localId":1}]"#]
        );
        let err = argv("config list", &[("bogus", json!(1))]).unwrap_err();
        assert!(err.contains("bogus"), "{err}");
    }

    #[test]
    fn methods_follow_the_arguments() {
        let endpoints = Endpoints::new();
        let ep = |p: &str| endpoints.0.get(p).unwrap();
        assert_eq!(ep("config list").verb(&Params::new()), Some(Get));
        assert_eq!(ep("analytics excluded").verb(&Params::new()), Some(Get));
        assert_eq!(
            ep("analytics excluded").verb(&params(&[("set", json!(["a"]))])),
            Some(Put)
        );
        assert_eq!(
            ep("sns block-delete").verb(&params(&[("action", json!("uninstall"))])),
            Some(Delete)
        );
        assert_eq!(ep("sns block-delete").verb(&Params::new()), None);
        assert_eq!(ep("sns block-delete").verbs(), [Get, Put, Delete]);
    }

    #[test]
    fn status_follows_the_error_code_then_the_exit_code() {
        let fail = |code: &str, details: Value| json!({ "success": false, "error": { "code": code, "message": "m", "details": details } });
        assert_eq!(
            status_for(&json!({ "success": true, "data": 1 }), 0),
            StatusCode::OK
        );
        assert_eq!(
            status_for(&fail("invalid_arguments", Value::Null), 2),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status_for(&fail("config_error", Value::Null), 3),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status_for(&fail("unsupported", Value::Null), 4),
            StatusCode::NOT_IMPLEMENTED
        );
        assert_eq!(
            status_for(&fail("native_error", Value::Null), 4),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status_for(
                &fail("runtime_error", json!({ "failureKind": "not_found" })),
                1
            ),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status_for(&fail("runtime_error", Value::Null), 1),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status_for(&fail("user_interrupt", Value::Null), 130).as_u16(),
            499
        );
        assert_eq!(
            status_for(&fail("something_new", Value::Null), 3),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn tokens_are_checked_from_the_header_or_the_query() {
        let mut headers = HeaderMap::new();
        assert!(!authorized("t0k", &headers, &[]));
        assert!(authorized(
            "t0k",
            &headers,
            &[("access_token".into(), "t0k".into())]
        ));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer t0k"),
        );
        assert!(authorized("t0k", &headers, &[]));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer t0kx"),
        );
        assert!(!authorized("t0k", &headers, &[]));
        assert_eq!(generate_token().len(), 32);
        assert_ne!(generate_token(), generate_token());
    }

    #[test]
    fn only_this_machine_is_answered_by_name() {
        let host = |v: &'static str| {
            let mut h = HeaderMap::new();
            h.insert(header::HOST, HeaderValue::from_static(v));
            local_host(&Uri::from_static("/chat/sessions"), &h)
        };
        for ok in [
            "localhost",
            "LOCALHOST:57108",
            "localhost.:57108",
            "127.0.0.1",
            "127.0.0.1:57108",
            "127.1.2.3:80",
            "[::1]",
            "[::1]:57108",
        ] {
            assert!(host(ok), "{ok}");
        }
        for bad in [
            "evil.example",
            "evil.example:57108",
            "localhost.evil.example",
            "127.0.0.1.nip.io",
            "192.168.1.2:57108",
            "[::2]:57108",
            "",
        ] {
            assert!(!host(bad), "{bad}");
        }
        assert!(!local_host(&Uri::from_static("/"), &HeaderMap::new()));
        // HTTP/2 and absolute-form requests carry the name in the URI
        assert!(local_host(
            &Uri::from_static("http://localhost:57108/health"),
            &HeaderMap::new()
        ));
        assert!(!local_host(
            &Uri::from_static("http://evil.example/health"),
            &HeaderMap::new()
        ));
    }

    #[test]
    fn language_comes_from_accept_language() {
        let lang = |v: &'static str| {
            let mut h = HeaderMap::new();
            h.insert(header::ACCEPT_LANGUAGE, HeaderValue::from_static(v));
            request_lang(&h)
        };
        assert_eq!(lang("zh-CN,zh;q=0.9,en;q=0.8"), Some("zh"));
        assert_eq!(lang("en-US"), Some("en"));
        assert_eq!(lang("*"), None);
        assert_eq!(request_lang(&HeaderMap::new()), None);
    }

    #[test]
    fn byte_ranges_are_read_as_rfc_9110_has_them() {
        let r = |h: &str| byte_range(h, 100);
        assert_eq!(r("bytes=0-9"), Some(Ok((0, 9))));
        assert_eq!(r("bytes=90-"), Some(Ok((90, 99))));
        assert_eq!(r("bytes=90-500"), Some(Ok((90, 99))), "clamped to the file");
        assert_eq!(r("bytes=-10"), Some(Ok((90, 99))), "the last ten");
        assert_eq!(r("bytes=-500"), Some(Ok((0, 99))));
        assert_eq!(r(" bytes= 5 - 6 "), Some(Ok((5, 6))));
        // past the end: 416
        assert_eq!(r("bytes=100-"), Some(Err(())));
        assert_eq!(r("bytes=-0"), Some(Err(())));
        // the whole file: several ranges, other units, nonsense, a range that ends before it starts
        for h in [
            "bytes=0-1,5-6",
            "items=0-1",
            "bytes=a-b",
            "bytes=5",
            "bytes=9-3",
            "",
        ] {
            assert_eq!(r(h), None, "{h}");
        }
        assert_eq!(byte_range("bytes=-5", 0), None);
    }

    #[tokio::test]
    async fn media_wait_in_line_until_it_is_full_or_too_long() {
        use std::time::Duration;
        let limit = MediaLimit::new(1);
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
        let line: Vec<_> = (0..MEDIA_QUEUE)
            .map(|_| limit.admitted.clone().try_acquire_owned().unwrap())
            .collect();
        assert!(limit.enter(Duration::from_secs(10)).await.is_none());
        drop((line, decoding));
        assert!(limit.enter(Duration::ZERO).await.is_some());
    }
}
