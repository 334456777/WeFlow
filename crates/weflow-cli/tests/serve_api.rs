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
        let text = String::from_utf8(raw).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        let mut head_lines = head.lines();
        let status = head_lines
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = head_lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        Reply {
            status,
            headers,
            body: body.to_string(),
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

#[test]
fn account_commands_and_read_only_refusals() {
    let root = temp_root("account");
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
}
