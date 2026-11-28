//! 监听 127.0.0.1 的假模型服务，供 weflow 的测试和 WebUI 的开发服务器使用：按两家服务的接口文档实现 OpenAI 的
//! Chat Completions（`POST /v1/chat/completions`）和 Anthropic 的 Messages（`POST /v1/messages`），两种格式各有
//! `GET /v1/models`。它像真实的服务一样检查请求（密钥、版本头、必填字段），按它们的格式回答、返回它们的错误，
//! 并保存收到的每个请求供测试查看。每个模型（[`MODELS`]）只有一种回答方式：正常回答、先推理再回答、拒绝回答、
//! 限流、过一次就好的过载、额度用完、服务器错误。
//!
//! ```no_run
//! let fake = weflow_fake_model::FakeModel::start()?;
//! // 让 weflow 使用它的配置：
//! // ai_model_api_base_url = fake.openai_base_url()
//! // ai_model_api_key = weflow_fake_model::KEY
//! // ai_model_api_model = "fake-chat"
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

/// 默认接受的密钥；用 [`FakeModel::start_on`] 启动时可以另给一个。
pub const KEY: &str = "sk-fake-weflow";

/// 各个连接共用的状态：密钥、计数器和收到的请求。
pub(crate) struct Shared {
    key: String,
    served: AtomicU64,
    flaky: AtomicU64,
    requests: Mutex<Vec<Request>>,
    stopped: AtomicBool,
}

impl Shared {
    /// 下一个回答的序号，从 1 开始，用在 ID 里，也用来挑选回答。
    pub(crate) fn next(&self) -> u64 {
        self.served.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// [`Behaviour::Flaky`] 模型的这次请求是否失败：第 1、3、5……次失败。
    pub(crate) fn flaky_fails(&self) -> bool {
        self.flaky.fetch_add(1, Ordering::Relaxed).is_multiple_of(2)
    }
}

/// 运行中的假服务，丢弃时停止接受连接。
pub struct FakeModel {
    addr: SocketAddr,
    shared: Arc<Shared>,
}

/// 只显示监听的地址。
impl std::fmt::Debug for FakeModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeModel")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl FakeModel {
    /// 在 127.0.0.1 的空闲端口上启动，接受 [`KEY`]。
    ///
    /// # Errors
    ///
    /// 无法绑定端口。
    pub fn start() -> std::io::Result<Self> {
        Self::start_on("127.0.0.1:0", KEY)
    }

    /// 在 `addr` 上启动，接受 `key`。
    ///
    /// # Errors
    ///
    /// 无法绑定 `addr`。
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

    /// 监听的地址；启动时给的是端口 0 时，这里是系统分配的端口。
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// OpenAI 格式的基础地址 `http://127.0.0.1:<port>/v1`，与 OpenAI 官方的基础地址一样以 `/v1` 结尾。
    pub fn openai_base_url(&self) -> String {
        format!("http://{}/v1", self.addr)
    }

    /// Anthropic 格式的基础地址 `http://127.0.0.1:<port>`，与 Anthropic 官方的基础地址一样不带路径。
    pub fn anthropic_base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// 到目前为止收到的请求，按收到的先后排列。
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
        // 连一次自己，唤醒等待连接的线程，让它看到停止的标志
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
    // 两种格式的 `GET /v1/models` 路径相同，按 Anthropic 要求必带的 `anthropic-version` 头区分
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
