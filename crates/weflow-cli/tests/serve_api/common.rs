//! What the tests share: running `weflow` and `weflow serve`, requests and their replies, the synthetic accounts
//! and the contract the answers are held against.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::{json, Value};
use weflow_native::fixture::{Fixture, TestDir};

pub(crate) const TOKEN: &str = "t0k-for-tests";

pub(crate) fn weflow(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_weflow"));
    cmd.env("WEFLOW_HOME", home)
        .env("WEFLOW_LANG", "en")
        // the fake AI server of `jobs_are_cancelled_and_limited` is on this machine
        .env("NO_PROXY", "127.0.0.1,localhost")
        // no registry: `serve` would download its default WebUI from GHCR, and `/` would turn into a
        // redirect once it had (a test that wants a registry sets its own)
        .env("WEFLOW_WEB_REGISTRY", "http://127.0.0.1:9")
        .env_remove("WEFLOW_HTTP_TOKEN")
        .stdin(Stdio::null());
    cmd
}

/// `weflow <args>` run directly: (exit code, stdout, stderr).
pub(crate) fn run_cli(home: &Path, args: &[&str]) -> (i32, String, String) {
    let out = weflow(home).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

pub(crate) struct Server {
    pub(crate) child: Child,
    pub(crate) addr: String,
    pub(crate) home: PathBuf,
    /// The lines the server printed on stderr after it started.
    pub(crate) stderr: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn temp_root(tag: &str) -> TestDir {
    TestDir::new_in(
        Path::new(env!("CARGO_TARGET_TMPDIR")),
        &format!("serve-api-{tag}"),
    )
}

/// Starts `weflow serve` on a free port and waits for its `server_started` event.
pub(crate) fn start(home: &Path, extra: &[&str]) -> Server {
    start_env(home, extra, &[])
}

pub(crate) fn start_env(home: &Path, extra: &[&str], env: &[(&str, &str)]) -> Server {
    let mut child = weflow(home)
        .envs(env.iter().copied())
        .args(["--json", "serve", "--port", "0"])
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let started = loop {
        let line = lines.next().expect("serve exited before starting").unwrap();
        if let Ok(event) = serde_json::from_str::<Value>(&line) {
            if event["type"] == "server_started" {
                break event;
            }
        }
    };
    // keep draining stderr so the server never blocks on a full pipe
    let stderr = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let printed = stderr.clone();
    std::thread::spawn(move || {
        for line in lines.map_while(Result::ok) {
            printed.lock().unwrap().push(line);
        }
    });
    let addr = started["url"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://")
        .to_string();
    Server {
        child,
        addr,
        home: home.to_path_buf(),
        stderr,
    }
}

pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: HashMap<String, String>,
    pub(crate) body: String,
    pub(crate) bytes: Vec<u8>,
}

/// The body of a `Transfer-Encoding: chunked` answer.
fn dechunk(mut raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(end) = raw.windows(2).position(|w| w == b"\r\n") else {
            return out;
        };
        let size =
            usize::from_str_radix(std::str::from_utf8(&raw[..end]).unwrap().trim(), 16).unwrap();
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&raw[end + 2..end + 2 + size]);
        raw = &raw[end + 2 + size + 2..];
    }
}

impl Reply {
    pub(crate) fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

impl Server {
    pub(crate) fn request(
        &self,
        method: &str,
        target: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Reply {
        let mut stream = TcpStream::connect(&self.addr).unwrap();
        let mut req = format!(
            "{method} {target} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n",
            body.len()
        );
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
            req.push_str(&format!("Host: {}\r\n", self.addr));
        }
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        req.push_str(body);
        stream.write_all(req.as_bytes()).unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8(raw[..split].to_vec()).unwrap();
        let mut head_lines = head.lines();
        let status = head_lines
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers: HashMap<String, String> = head_lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let mut bytes = raw[split + 4..].to_vec();
        if headers
            .get("transfer-encoding")
            .is_some_and(|v| v == "chunked")
        {
            bytes = dechunk(&bytes);
        }
        Reply {
            status,
            headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
            bytes,
        }
    }

    pub(crate) fn call(&self, method: &str, target: &str, body: &str) -> Reply {
        let auth = format!("Bearer {TOKEN}");
        self.request(method, target, &[("Authorization", &auth)], body)
    }
}

/// A WeFlow home under `root` set up for a synthetic account.
pub(crate) fn account_home(root: &Path) -> PathBuf {
    account_home_with(root, |_| {})
}

/// [`account_home`] with more done to the account first.
pub(crate) fn account_home_with(root: &Path, more: impl FnOnce(&Fixture)) -> PathBuf {
    let (home, fixture) = account(root);
    more(&fixture);
    home
}

/// [`account_home`] and the account, to change it later.
pub(crate) fn account(root: &Path) -> (PathBuf, Fixture) {
    let fixture = Fixture::standard(&root.join("data"));
    let home = root.join("home");
    for (key, value) in [
        ("db_path", fixture.account_dir.to_string_lossy().to_string()),
        ("wxid", "wxid_me_ab12".to_string()),
        ("decrypt_key", fixture.key_hex()),
    ] {
        let (exit, _, stderr) = run_cli(&home, &["config", "set", key, &value]);
        assert_eq!(exit, 0, "{stderr}");
    }
    (home, fixture)
}

/// `text` as it goes in a query string.
pub(crate) fn percent(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The parts of a `multipart/form-data` answer: (headers, content).
pub(crate) fn form_parts(reply: &Reply) -> Vec<(String, Vec<u8>)> {
    let boundary = reply.headers["content-type"]
        .split("boundary=")
        .nth(1)
        .unwrap()
        .to_string();
    let delimiter = format!("--{boundary}");
    let raw = &reply.bytes;
    let mut starts = Vec::new();
    let mut i = 0;
    while let Some(at) = raw[i..]
        .windows(delimiter.len())
        .position(|w| w == delimiter.as_bytes())
    {
        starts.push(i + at);
        i += at + delimiter.len();
    }
    starts
        .windows(2)
        .map(|w| {
            // skip the delimiter's line break; drop the one before the next delimiter
            let part = &raw[w[0] + delimiter.len() + 2..w[1] - 2];
            let split = part.windows(4).position(|x| x == b"\r\n\r\n").unwrap();
            (
                String::from_utf8(part[..split].to_vec()).unwrap(),
                part[split + 4..].to_vec(),
            )
        })
        .collect()
}

/// The contract the WebUI is built against (#399).
pub(crate) fn contract() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../web/contract/openapi.json"
    ))
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// The errors of `value` against `schema`, whose `$ref`s point into the contract's components, as `path: error`.
pub(crate) fn misfits(contract: &Value, schema: &Value, value: &Value) -> Vec<String> {
    let mut root = schema.clone();
    root["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    root["components"] = contract["components"].clone();
    let validator = jsonschema::validator_for(&root).unwrap();
    validator
        .iter_errors(value)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect()
}
