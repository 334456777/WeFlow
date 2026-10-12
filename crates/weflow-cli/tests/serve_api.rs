//! End-to-end tests of the command API (`weflow serve`): a real server process, real HTTP requests, and the
//! same commands run directly for comparison, over a synthetic encrypted account.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::{json, Value};
use weflow_native::fixture::Fixture;

const TOKEN: &str = "t0k-for-tests";

fn weflow(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_weflow"));
    cmd.env("WEFLOW_HOME", home)
        .env("WEFLOW_LANG", "en")
        // the fake AI server of `jobs_are_cancelled_and_limited` is on this machine
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env_remove("WEFLOW_HTTP_TOKEN")
        .stdin(Stdio::null());
    cmd
}

/// `weflow <args>` run directly: (exit code, stdout, stderr).
fn run_cli(home: &Path, args: &[&str]) -> (i32, String, String) {
    let out = weflow(home).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

struct Server {
    child: Child,
    addr: String,
    home: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn temp_root(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("serve-api-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Starts `weflow serve` on a free port and waits for its `server_started` event.
fn start(home: &Path, extra: &[&str]) -> Server {
    let mut child = weflow(home)
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
    std::thread::spawn(move || for _ in lines {});
    let addr = started["url"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://")
        .to_string();
    Server {
        child,
        addr,
        home: home.to_path_buf(),
    }
}

struct Reply {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
    bytes: Vec<u8>,
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
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

impl Server {
    fn request(&self, method: &str, target: &str, headers: &[(&str, &str)], body: &str) -> Reply {
        let mut stream = TcpStream::connect(&self.addr).unwrap();
        let mut req = format!(
            "{method} {target} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n",
            self.addr,
            body.len()
        );
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

    fn call(&self, method: &str, target: &str, body: &str) -> Reply {
        let auth = format!("Bearer {TOKEN}");
        self.request(method, target, &[("Authorization", &auth)], body)
    }
}

#[test]
fn health_needs_no_token_and_everything_else_does() {
    let root = temp_root("auth");
    let server = start(&root, &["--api-token", TOKEN]);
    let r = server.request("GET", "/health", &[], "");
    assert_eq!((r.status, r.json()), (200, json!({ "status": "ok" })));

    let r = server.request("GET", "/config/path", &[], "");
    assert_eq!(r.status, 401);
    assert_eq!(r.json()["error"]["code"], "unauthorized");
    let r = server.request(
        "GET",
        "/config/path",
        &[("Authorization", "Bearer nope")],
        "",
    );
    assert_eq!(r.status, 401);
    let r = server.request(
        "GET",
        &format!("/config/path?access_token={TOKEN}"),
        &[],
        "",
    );
    assert_eq!(r.status, 200);
}

#[test]
fn a_token_is_made_up_when_none_is_configured() {
    let root = temp_root("gen-token");
    let mut child = weflow(&root)
        .args(["--json", "serve", "--port", "0"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let event: Value = loop {
        let line = lines.next().unwrap().unwrap();
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            break v;
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(event["tokenConfigured"], false);
    assert_eq!(event["api"], true);
    assert_eq!(event["generatedToken"].as_str().unwrap().len(), 32);
}

#[test]
fn commands_answer_like_the_terminal() {
    let root = temp_root("config");
    let server = start(&root, &["--api-token", TOKEN]);
    let home = &server.home;

    // the default answer is the `--json` document itself
    let r = server.call("GET", "/config/path", "");
    let (exit, stdout, _) = run_cli(home, &["--json", "config", "path"]);
    assert_eq!((r.status, exit), (200, 0));
    assert_eq!(r.body, stdout);
    assert_eq!(r.headers["x-weflow-exit-code"], "0");

    // a setting goes in with PUT and comes back with GET
    let r = server.call(
        "PUT",
        "/config/set",
        r#"{"key": "no_progress", "value": "true"}"#,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let r = server.call("GET", "/config/get?key=no_progress", "");
    assert_eq!(r.status, 200);
    let (_, stdout, _) = run_cli(home, &["--json", "config", "get", "no_progress"]);
    assert_eq!(r.body, stdout);

    // `std=in` shows the command without running it
    let r = server.call("GET", "/config/get?key=no_progress&std=in", "");
    assert_eq!(
        r.json(),
        json!({ "argv": ["weflow", "--json", "config", "get", "--", "no_progress"], "stdin": null })
    );

    // `std=out` / `std=err` carry the human-readable run
    let r = server.call("GET", "/config/get?key=no_progress&std=out", "");
    let (_, stdout, stderr) = run_cli(home, &["config", "get", "no_progress"]);
    assert_eq!(r.json(), json!({ "exitCode": 0, "stdout": stdout }));
    let r = server.call("GET", "/config/get?key=no_progress&std=err", "");
    assert_eq!(r.json(), json!({ "exitCode": 0, "stderr": stderr }));
    let r = server.call("GET", "/config/path?std=all", "");
    assert_eq!(r.status, 400);

    // the command checks its arguments as on a terminal: exit code 2 is 400
    let r = server.call("GET", "/config/get?key=a&key=b", "");
    let (exit, stdout, _) = run_cli(home, &["--json", "config", "get", "--", "a", "b"]);
    assert_eq!((r.status, exit), (400, 2));
    assert_eq!(r.body, stdout);
    assert_eq!(r.headers["x-weflow-error-code"], "invalid_arguments");
    // the server refuses names the command does not have
    let r = server.call("GET", "/config/path?bogus=1", "");
    assert_eq!(r.status, 400);
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("bogus"));

    // the method follows the command
    let r = server.call("POST", "/config/path", "");
    assert_eq!((r.status, r.headers["allow"].as_str()), (405, "GET"));
    let r = server.call("PUT", "/analytics/excluded", r#"{"set": "x"}"#);
    assert_ne!(r.status, 405);
    let r = server.call("PUT", "/analytics/excluded", "");
    assert_eq!((r.status, r.headers["allow"].as_str()), (405, "GET"));

    // unknown paths and `serve` itself are not commands
    for path in ["/nope", "/serve", "/config", "/"] {
        assert_eq!(server.call("GET", path, "").status, 404, "{path}");
    }
    let r = server.call("POST", "/config/import", "[1]");
    assert_eq!(r.status, 400);
}

/// A WeFlow home under `root` set up for a synthetic account.
fn account_home(root: &Path) -> PathBuf {
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
    home
}

#[test]
fn account_commands_and_read_only_refusals() {
    let root = temp_root("account");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);

    let r = server.call("GET", "/chat/sessions", "");
    let (_, stdout, _) = run_cli(&home, &["--json", "chat", "sessions"]);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.body, stdout);
    assert!(!r.json()["data"].as_array().unwrap().is_empty());

    // writing WeChat's databases is refused: 501 with the `unsupported` code
    let r = server.call(
        "DELETE",
        "/chat/delete-message",
        r#"{"session-id": "wxid_friend", "local-id": 1, "create-time": 1}"#,
    );
    assert_eq!(r.status, 501, "{}", r.body);
    assert_eq!(r.json()["error"]["code"], "unsupported");
    assert_eq!(r.headers["x-weflow-exit-code"], "4");
    let r = server.call("DELETE", "/sns/block-delete?action=uninstall", "");
    assert_eq!(r.status, 501, "{}", r.body);
    let r = server.call("GET", "/sns/block-delete?action=uninstall", "");
    assert_eq!((r.status, r.headers["allow"].as_str()), (405, "DELETE"));
}

#[test]
fn a_missing_account_is_a_conflict() {
    let root = temp_root("no-account");
    let server = start(&root, &["--api-token", TOKEN]);
    let r = server.call("GET", "/chat/sessions", "");
    assert_eq!(r.status, 409, "{}", r.body);
    assert_eq!(r.json()["error"]["code"], "config_error");
}

#[test]
fn desktop_mode_serves_the_desktop_api() {
    let root = temp_root("desktop");
    let server = start(&root, &["--desktop", "--api-token", TOKEN]);
    let r = server.call("GET", "/api/v1/health", "");
    assert_eq!((r.status, r.json()), (200, json!({ "status": "ok" })));
    // the command API is not there
    assert_eq!(server.call("GET", "/config/path", "").status, 404);

    let (exit, stdout, _) = run_cli(&root, &["--json", "serve", "--message-push"]);
    assert_eq!(exit, 2);
    assert!(stdout.contains("--desktop"), "{stdout}");
    // the old name of --desktop is gone
    let (exit, _, _) = run_cli(&root, &["--json", "serve", "--http"]);
    assert_eq!(exit, 2);
    // the limits are for the command API only
    let (exit, stdout, _) = run_cli(&root, &["--json", "serve", "--desktop", "--max-tasks", "2"]);
    assert_eq!(exit, 2);
    assert!(stdout.contains("--max-tasks"), "{stdout}");
    let (exit, _, _) = run_cli(&root, &["--json", "serve", "--max-reads", "0"]);
    assert_eq!(exit, 2);
}

/// The events of a server-sent event stream: (name, data).
fn sse_events(body: &str) -> Vec<(String, Value)> {
    body.split("\n\n")
        .filter_map(|block| {
            let mut name = None;
            let mut data = None;
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("event: ") {
                    name = Some(v.to_string());
                } else if let Some(v) = line.strip_prefix("data: ") {
                    data = Some(serde_json::from_str(v).unwrap());
                }
            }
            Some((name?, data?))
        })
        .collect()
}

/// The parts of a `multipart/form-data` answer: (headers, content).
fn form_parts(reply: &Reply) -> Vec<(String, Vec<u8>)> {
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

#[test]
fn jobs_run_in_the_background() {
    let root = temp_root("jobs");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");
    let out = root.join("backup.zip");
    let body = json!({ "out": out }).to_string();

    let r = server.request(
        "POST",
        "/backup/create",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        &body,
    );
    assert_eq!(r.status, 202, "{}", r.body);
    let job = r.json()["data"].clone();
    let id = job["id"].as_str().unwrap().to_string();
    assert_eq!(r.headers["location"], format!("/jobs/{id}"));
    assert!(matches!(job["status"].as_str(), Some("queued" | "running")));
    assert_eq!(
        job["command"].as_array().unwrap()[..3],
        json!(["weflow", "--json", "--progress"])
            .as_array()
            .unwrap()[..]
    );

    // the events: status, progress from `--progress`, and the result at the end
    let r = server.call("GET", &format!("/jobs/{id}/events"), "");
    assert_eq!(r.status, 200);
    assert!(r.headers["content-type"].starts_with("text/event-stream"));
    let events = sse_events(&r.body);
    let names: Vec<&str> = events.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names.first(), Some(&"status"), "{names:?}");
    assert!(names.contains(&"progress"), "{names:?}");
    let (last, result) = events.last().unwrap();
    assert_eq!(last, "result");
    assert_eq!(result["status"], "succeeded", "{result}");
    assert_eq!(result["result"]["success"], true);
    assert!(out.is_file());

    // the job keeps its result; a late subscriber still gets every event
    let r = server.call("GET", &format!("/jobs/{id}"), "");
    assert_eq!(r.status, 200);
    let doc = r.json()["data"].clone();
    assert_eq!(
        (
            doc["status"].as_str(),
            doc["exitCode"].as_i64(),
            doc["httpStatus"].as_u64()
        ),
        (Some("succeeded"), Some(0), Some(200))
    );
    assert_eq!(doc["result"], result["result"]);
    let again = sse_events(&server.call("GET", &format!("/jobs/{id}/events"), "").body);
    assert_eq!(again.len(), events.len());
    // after `Last-Event-ID` only the newer ones
    let r = server.request(
        "GET",
        &format!("/jobs/{id}/events"),
        &[("Authorization", &auth), ("Last-Event-ID", "1")],
        "",
    );
    assert_eq!(sse_events(&r.body).len(), events.len() - 1);

    let list = server.call("GET", "/jobs", "").json();
    assert_eq!(list["data"][0]["id"], id.as_str());
    assert!(list["data"][0].get("result").is_none());

    // a failing command is a failed job with the command's error
    let r = server.request(
        "GET",
        "/config/get?key=a&key=b",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    let failed = r.json()["data"]["id"].as_str().unwrap().to_string();
    let events = sse_events(
        &server
            .call("GET", &format!("/jobs/{failed}/events"), "")
            .body,
    );
    let (_, result) = events.last().unwrap();
    assert_eq!(result["status"], "failed", "{result}");
    assert_eq!(result["result"]["success"], false);

    // DELETE forgets a finished job
    assert_eq!(
        server.call("DELETE", &format!("/jobs/{id}"), "").status,
        200
    );
    assert_eq!(server.call("GET", &format!("/jobs/{id}"), "").status, 404);
    assert_eq!(server.call("GET", "/jobs/nope/events", "").status, 404);
    let r = server.call("POST", "/jobs", "");
    assert_eq!((r.status, r.headers["allow"].as_str()), (405, "GET"));
    // the human-readable answers are not jobs
    let r = server.request(
        "GET",
        "/config/path?std=out",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    assert_eq!(r.status, 400);
}

#[test]
fn files_come_back_with_the_result() {
    let root = temp_root("files");
    let home = account_home(&root);
    // files of an earlier run are cleared when the server starts
    let stale = home.join("cache").join("serve-files").join("old");
    std::fs::create_dir_all(&stale).unwrap();
    let server = start(&home, &["--api-token", TOKEN]);
    assert!(!stale.exists());

    // no --out: the server picks the folder, the command names the file
    let r = server.call("POST", "/export/sessions?return=file", "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.headers["content-type"].starts_with("multipart/form-data; boundary="));
    assert_eq!(
        r.headers["content-length"].parse::<usize>().unwrap(),
        r.bytes.len()
    );
    let parts = form_parts(&r);
    assert_eq!(parts.len(), 2);
    assert!(parts[0].0.contains("name=\"result\""), "{}", parts[0].0);
    let result: Value = serde_json::from_slice(&parts[0].1).unwrap();
    assert_eq!(result["success"], true);
    let path = PathBuf::from(result["data"]["path"].as_str().unwrap());
    assert_eq!(path.file_name().unwrap(), "sessions.json");
    assert!(
        path.starts_with(home.join("cache").join("serve-files")),
        "{path:?}"
    );
    assert!(
        parts[1].0.contains("filename=\"sessions.json\""),
        "{}",
        parts[1].0
    );
    assert!(
        parts[1].0.contains("Content-Type: application/json"),
        "{}",
        parts[1].0
    );
    assert_eq!(parts[1].1, std::fs::read(&path).unwrap());

    // with --out the file is written there and sent as well
    let out = root.join("mine.zip");
    let r = server.call(
        "POST",
        "/backup/create?return=file",
        &json!({ "out": out }).to_string(),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let parts = form_parts(&r);
    let result: Value = serde_json::from_slice(&parts[0].1).unwrap();
    assert_eq!(
        std::fs::canonicalize(result["data"]["path"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(&out).unwrap()
    );
    assert!(
        parts[1].0.contains("Content-Type: application/zip"),
        "{}",
        parts[1].0
    );
    assert_eq!(parts[1].1, std::fs::read(&out).unwrap());

    // the folder is part of `cache list` (`--api`)
    let (_, stdout, _) = run_cli(&home, &["--json", "cache", "list"]);
    let list: Value = serde_json::from_str(&stdout).unwrap();
    let api = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["part"] == "api")
        .unwrap()
        .clone();
    assert!(api["paths"].to_string().contains("serve-files"), "{api}");
    assert!(api["files"].as_u64().unwrap() >= 1);

    // a failure is the usual JSON error
    let r = server.call(
        "POST",
        "/export/messages?return=file",
        r#"{"session-id": "x", "format": "nope"}"#,
    );
    assert_eq!(r.status, 400, "{}", r.body);
    assert_eq!(r.json()["success"], false);
    // only commands that write one file
    for (method, target) in [
        ("GET", "/chat/sessions?return=file"),
        ("POST", "/export/media?return=file"),
        ("POST", "/export/sessions?return=file&std=out"),
        ("POST", "/export/sessions?return=zip"),
    ] {
        let r = server.call(method, target, "");
        assert_eq!(r.status, 400, "{target}: {}", r.body);
    }
}

/// An AI endpoint that accepts connections and never answers; reports when each one opens and closes.
fn silent_ai_server() -> (String, std::sync::mpsc::Receiver<&'static str>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let tx = tx.clone();
            let _ = tx.send("open");
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while matches!(stream.read(&mut buf), Ok(n) if n > 0) {}
                let _ = tx.send("closed");
            });
        }
    });
    (url, rx)
}

#[test]
fn jobs_are_cancelled_and_limited() {
    use std::time::Duration;
    let root = temp_root("cancel");
    let (url, ai) = silent_ai_server();
    for (key, value) in [
        ("ai_model_api_base_url", url.as_str()),
        ("ai_model_api_key", "k"),
    ] {
        let (exit, _, stderr) = run_cli(&root, &["config", "set", key, value]);
        assert_eq!(exit, 0, "{stderr}");
    }
    let server = start(&root, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");
    let queue = |target: &str| {
        let r = server.request(
            "POST",
            target,
            &[("Authorization", &auth), ("Prefer", "respond-async")],
            "",
        );
        assert_eq!(r.status, 202, "{}", r.body);
        r.json()["data"]["id"].as_str().unwrap().to_string()
    };
    // `insight test` waits for the AI server (15 s at most)
    let first = queue("/insight/test");
    assert_eq!(ai.recv_timeout(Duration::from_secs(30)), Ok("open"));

    // one task at a time: another one is refused right away, or queued
    let r = server.call("POST", "/insight/test", "");
    assert_eq!(r.status, 429, "{}", r.body);
    assert_eq!(r.headers["retry-after"], "5");
    assert_eq!(r.json()["error"]["code"], "busy");
    let second = queue("/insight/test");
    let r = server.call("GET", &format!("/jobs/{second}"), "");
    assert_eq!(r.json()["data"]["status"], "queued");
    // reads are not held up by tasks
    assert_eq!(server.call("GET", "/config/path", "").status, 200);

    // cancelling kills the command: its connection to the AI server goes away
    let r = server.call("DELETE", &format!("/jobs/{first}"), "");
    assert_eq!(r.status, 200);
    let doc = r.json()["data"].clone();
    assert_eq!(doc["status"], "cancelled", "{doc}");
    assert_eq!(doc["httpStatus"], 499);
    assert_eq!(doc["result"]["error"]["code"], "user_interrupt");
    assert_eq!(ai.recv_timeout(Duration::from_secs(5)), Ok("closed"));
    // and the queued job starts
    assert_eq!(ai.recv_timeout(Duration::from_secs(30)), Ok("open"));
    let r = server.call("DELETE", &format!("/jobs/{second}"), "");
    assert_eq!(r.json()["data"]["status"], "cancelled");
    assert_eq!(ai.recv_timeout(Duration::from_secs(5)), Ok("closed"));

    // a client that goes away takes its command with it
    let mut stream = TcpStream::connect(&server.addr).unwrap();
    write!(
        stream,
        "POST /insight/test HTTP/1.1\r\nHost: {}\r\nAuthorization: {auth}\r\nContent-Length: 0\r\n\r\n",
        server.addr
    )
    .unwrap();
    assert_eq!(ai.recv_timeout(Duration::from_secs(30)), Ok("open"));
    drop(stream);
    assert_eq!(ai.recv_timeout(Duration::from_secs(5)), Ok("closed"));
}
