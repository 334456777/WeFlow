//! A fake model API on 127.0.0.1, for weflow's tests and the WebUI's dev server: OpenAI's Chat Completions
//! (`POST /v1/chat/completions`) and Anthropic's Messages (`POST /v1/messages`), each with `GET /v1/models`, built from
//! the two services' documents. It checks a request as they do (the key, the version header, the required fields),
//! answers in their shapes and with their errors, and keeps every request for a test to look at. Its models
//! ([`MODELS`]) each answer one way: an answer, reasoning first, a refusal, a rate limit, an overload that passes,
//! a used-up quota, a server error.
//!
//! ```no_run
//! let fake = weflow_fake_model::FakeModel::start()?;
//! // ai_model_api_base_url = fake.openai_base_url(), ai_model_api_key = weflow_fake_model::KEY, ai_model_api_model = "fake-chat"
//! # Ok::<(), std::io::Error>(())
//! ```
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

mod anthropic;
mod http;
mod openai;
mod scenario;

pub use http::Request;
pub use scenario::{Behaviour, FakeModelInfo, MODELS, REASONING_TOKENS, SLOW_ANSWER};

/// The key the fake takes unless it is started with another.
pub const KEY: &str = "sk-fake-weflow";

/// What the connections share: the key, the counters and the requests received.
pub(crate) struct Shared {
    key: String,
    served: AtomicU64,
    flaky: AtomicU64,
    requests: Mutex<Vec<Request>>,
    stopped: AtomicBool,
}

impl Shared {
    /// The number of the next answer, from 1: in ids and in the choice of an answer.
    pub(crate) fn next(&self) -> u64 {
        self.served.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Whether this request of a [`Behaviour::Flaky`] model fails: the first, the third and so on.
    pub(crate) fn flaky_fails(&self) -> bool {
        self.flaky.fetch_add(1, Ordering::Relaxed).is_multiple_of(2)
    }
}

/// A running fake; it stops taking connections when dropped.
pub struct FakeModel {
    addr: SocketAddr,
    shared: Arc<Shared>,
}

/// Where it listens.
impl std::fmt::Debug for FakeModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeModel")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl FakeModel {
    /// Start on a free port of 127.0.0.1, taking [`KEY`].
    ///
    /// # Errors
    ///
    /// The port cannot be bound.
    pub fn start() -> std::io::Result<Self> {
        Self::start_on("127.0.0.1:0", KEY)
    }

    /// Start on `addr`, taking `key`.
    ///
    /// # Errors
    ///
    /// `addr` cannot be bound.
    pub fn start_on(addr: &str, key: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            key: key.to_string(),
            served: AtomicU64::new(0),
            flaky: AtomicU64::new(0),
            requests: Mutex::new(Vec::new()),
            stopped: AtomicBool::new(false),
        });
        let accepting = Arc::clone(&shared);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if accepting.stopped.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let shared = Arc::clone(&accepting);
                std::thread::spawn(move || serve(&stream, &shared));
            }
        });
        Ok(Self { addr, shared })
    }

    /// The address it listens on, with the port the system gave when it was asked for port 0.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The base URL of the OpenAI side, `http://127.0.0.1:<port>/v1`, as OpenAI's own ends in `/v1`.
    pub fn openai_base_url(&self) -> String {
        format!("http://{}/v1", self.addr)
    }

    /// The base URL of the Anthropic side, `http://127.0.0.1:<port>`, as Anthropic's own has no path.
    pub fn anthropic_base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The requests received so far, the first first.
    pub fn requests(&self) -> Vec<Request> {
        self.shared
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for FakeModel {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Relaxed);
        // wake the accepting thread so that it sees the flag
        let _ = TcpStream::connect(self.addr);
    }
}

fn serve(stream: &TcpStream, shared: &Shared) {
    let Ok(Some(request)) = http::read_request(stream) else {
        return;
    };
    shared
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(request.clone());
    let response = route(&request, shared);
    let _ = http::write_response(stream, &response);
}

fn route(request: &Request, shared: &Shared) -> http::Response {
    let anthropic = request.header("anthropic-version").is_some();
    match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/v1/chat/completions") => openai::chat(request, shared),
        ("POST", "/v1/messages") => anthropic::messages(request, shared),
        ("GET", "/v1/models") if anthropic => anthropic::models(request, shared),
        ("GET", "/v1/models") => openai::models(request, shared),
        (_, "/v1/chat/completions" | "/v1/messages" | "/v1/models") => http::Response::json(
            405,
            &serde_json::json!({ "error": { "message": "method not allowed", "type": "invalid_request_error" } }),
        ),
        _ => http::Response::json(
            404,
            &serde_json::json!({ "error": { "message": format!("no route for {} {}", request.method, request.path), "type": "invalid_request_error", "code": "unknown_url" } }),
        ),
    }
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
