//! The server process and the requests the benchmark makes of it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Stdio};
use std::time::Instant;

use anyhow::{anyhow, ensure, Context, Result};
use serde_json::Value;

use super::account::weflow_command;
use super::TOKEN;

pub(crate) struct Server {
    pub(crate) child: Child,
    pub(crate) addr: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts `weflow serve` on a free port; returns it with the milliseconds it took to listen.
pub(crate) fn start(weflow: &Path, home: &Path, extra: &[String]) -> Result<(Server, f64)> {
    let begin = Instant::now();
    let mut child = weflow_command(weflow, home)
        .args(["--json", "serve", "--port", "0", "--api-token", TOKEN])
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot start {}", weflow.display()))?;
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let started = loop {
        let line = lines
            .next()
            .ok_or_else(|| anyhow!("weflow serve exited before it started listening"))??;
        if let Ok(event) = serde_json::from_str::<Value>(&line) {
            if event["type"] == "server_started" {
                break event;
            }
        }
    };
    let ms = begin.elapsed().as_secs_f64() * 1000.0;
    // keep reading stderr so the server never blocks on a full pipe; nothing of it is printed
    std::thread::spawn(move || for _ in lines {});
    let addr = started["url"]
        .as_str()
        .ok_or_else(|| anyhow!("server_started without a url"))?
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    Ok((Server { child, addr }, ms))
}

pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
    pub(crate) ms: f64,
    /// The size of the whole file, of a `206` answer.
    pub(crate) content_range_total: Option<usize>,
}

impl Reply {
    pub(crate) fn data(&self) -> Result<Value> {
        ensure!(self.status == 200, "status {}", self.status);
        let doc: Value = serde_json::from_slice(&self.body)?;
        Ok(doc["data"].clone())
    }
}

fn dechunk(mut raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(end) = raw.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(&raw[..end])
            .ok()
            .and_then(|s| usize::from_str_radix(s.trim(), 16).ok())
            .unwrap_or(0);
        if size == 0 || raw.len() < end + 2 + size {
            break;
        }
        out.extend_from_slice(&raw[end + 2..end + 2 + size]);
        raw = &raw[(end + 4 + size).min(raw.len())..];
    }
    out
}

pub(crate) fn request(addr: &str, method: &str, target: &str, body: &str) -> Result<Reply> {
    request_with(addr, method, target, body, "")
}

/// [`request`] with more header lines, each ending in CRLF; the body comes back as sent (compressed or not).
pub(crate) fn request_with(
    addr: &str,
    method: &str,
    target: &str,
    body: &str,
    headers: &str,
) -> Result<Reply> {
    let begin = Instant::now();
    let mut stream = TcpStream::connect(addr)?;
    let mut req = format!(
        "{method} {target} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}",
        body.len()
    );
    if !body.is_empty() {
        req.push_str("Content-Type: application/json\r\n");
    }
    req.push_str("\r\n");
    req.push_str(body);
    stream.write_all(req.as_bytes())?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let ms = begin.elapsed().as_secs_f64() * 1000.0;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("no HTTP head in the answer to {method}"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut body = raw[split + 4..].to_vec();
    if head.contains("\r\ntransfer-encoding: chunked") {
        body = dechunk(&body);
    }
    let content_range_total = head
        .lines()
        .find_map(|l| l.strip_prefix("content-range: bytes "))
        .and_then(|r| r.rsplit_once('/'))
        .and_then(|(_, total)| total.trim().parse().ok());
    Ok(Reply {
        status,
        body,
        ms,
        content_range_total,
    })
}

pub(crate) fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The list in a command's `data`: the value itself or its first array field.
pub(crate) fn rows(data: &Value) -> Vec<Value> {
    match data {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o
            .values()
            .find_map(|v| v.as_array().cloned())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

pub(crate) fn field<'a>(row: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|n| row.get(*n))
}

pub(crate) fn number(v: Option<&Value>) -> Option<i64> {
    v.and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
}
