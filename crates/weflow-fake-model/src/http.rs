//! 假服务够用的 HTTP/1.1：每个连接一个请求，请求体按 `Content-Length` 读取，回答后关闭连接。
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;

/// 读取的请求体的上限，超过时拒绝。
const LARGEST_BODY: usize = 16 << 20;

/// 假服务收到的一个请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// HTTP 方法。
    pub method: String,
    /// 不含查询参数的路径。
    pub path: String,
    /// 不含 `?` 的查询参数；没有时为空。
    pub query: String,
    /// 请求头，名称为小写。
    pub headers: Vec<(String, String)>,
    /// 请求体，按 UTF-8 读取，无效的字节被替换。
    pub body: String,
}

impl Request {
    /// 第一个名为 `name`（小写）的请求头。
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// 按 JSON 解析的请求体；不是 JSON 时为 `None`。
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&self.body).ok()
    }
}

/// 要写出的回答。
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Response {
    /// 状态码为 `status`、内容为 `body` 的 JSON 回答。
    pub fn json(status: u16, body: &serde_json::Value) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.to_string(),
        }
    }

    /// 状态码为 `status`、内容为 `body` 的 HTML 回答，模拟网关返回的错误页。
    pub fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "text/html; charset=utf-8".into())],
            body: body.to_string(),
        }
    }

    /// 加上一个回答头。
    pub fn with_header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }
}

/// 从 `stream` 读取一个请求；连接在请求到来前关闭时为 `None`。
pub fn read_request(stream: &TcpStream) -> io::Result<Option<Request>> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad request line",
        ));
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let length: usize = headers
        .iter()
        .find(|(n, _)| n == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    if length > LARGEST_BODY {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "body too large"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(Request {
        method: method.to_string(),
        path: path.to_string(),
        query: query.to_string(),
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }))
}

/// 把 `response` 写到 `stream`，并告诉客户端随后关闭连接。
pub fn write_response(mut stream: &TcpStream, response: &Response) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\ncontent-length: {}\r\nconnection: close\r\n",
        response.status,
        reason(response.status),
        response.body.len()
    );
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(response.body.as_bytes())?;
    stream.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        529 => "Overloaded",
        _ => "Status",
    }
}
