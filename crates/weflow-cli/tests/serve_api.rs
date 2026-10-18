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
    /// The lines the server printed on stderr after it started.
    stderr: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
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
    start_env(home, extra, &[])
}

fn start_env(home: &Path, extra: &[&str], env: &[(&str, &str)]) -> Server {
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
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["status"], "ok");
    assert_eq!(r.json()["apiVersion"], "1");
    // how it runs, for the settings (#214)
    assert_eq!(r.json()["limits"]["maxTasks"], 1);
    assert!(r.json()["limits"]["maxReads"].as_u64().unwrap() >= 1);
    assert_eq!(r.json()["insight"], false);
    drop(server);
    let server = start(
        &root,
        &[
            "--api-token",
            TOKEN,
            "--max-tasks",
            "2",
            "--max-reads",
            "3",
            "--max-media",
            "4",
        ],
    );
    let limits = server.request("GET", "/health", &[], "").json()["limits"].clone();
    assert_eq!(
        limits,
        json!({ "maxTasks": 2, "maxReads": 3, "maxMedia": 4 })
    );

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
fn health_data_changes_with_the_configuration_and_each_run() {
    let root = temp_root("data");
    let server = start(&root, &["--api-token", TOKEN]);
    let data = || {
        server.request("GET", "/health", &[], "").json()["data"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let first = data();
    assert_eq!(first.len(), 16);
    assert_eq!(data(), first);
    // another account set up while the server runs
    let (code, _, err) = run_cli(&root, &["config", "set", "myWxid", "wxid_synthetic_b"]);
    assert_eq!(code, 0, "{err}");
    let second = data();
    assert_ne!(second, first);
    assert_eq!(data(), second);
    drop(server);
    // the same configuration, another run
    let server = start(&root, &["--api-token", TOKEN]);
    let third = server.request("GET", "/health", &[], "").json()["data"].clone();
    assert_ne!(third, json!(second));
}

#[test]
fn the_api_describes_itself() {
    let root = temp_root("openapi");
    let server = start(&root, &["--api-token", TOKEN]);
    assert_eq!(server.request("GET", "/openapi.json", &[], "").status, 401);
    let auth = format!("Bearer {TOKEN}");
    let get = |lang: &str| {
        let r = server.request(
            "GET",
            "/openapi.json",
            &[("Authorization", &auth), ("Accept-Language", lang)],
            "",
        );
        assert_eq!(r.status, 200);
        r.json()
    };
    let en = get("en");
    assert_eq!(en["openapi"], "3.1.0");
    let health = server.request("GET", "/health", &[], "").json();
    assert_eq!(en["info"]["x-weflow-api-version"], health["apiVersion"]);
    assert_eq!(en["info"]["version"], health["version"]);
    for path in ["/chat/sessions", "/backup/create", "/jobs/{id}/events"] {
        assert!(en["paths"][path].is_object(), "{path} is missing");
    }
    assert!(en["paths"]["/serve"].is_null());
    let zh = get("zh-CN");
    let summary = |d: &Value| d["paths"]["/chat/sessions"]["get"]["summary"].clone();
    assert_ne!(summary(&en), summary(&zh));
    assert_eq!(server.call("POST", "/openapi.json", "").status, 405);
}

#[test]
fn only_requests_to_this_machine_are_answered() {
    let root = temp_root("host");
    let server = start(&root, &["--api-token", TOKEN]);
    let port = server.addr.rsplit_once(':').unwrap().1.to_string();
    let auth = format!("Bearer {TOKEN}");
    for host in ["evil.example", "evil.example:80", "localhost.evil.example"] {
        let host = host.replace(":80", &format!(":{port}"));
        for target in ["/health", "/config/path"] {
            let r = server.request(
                "GET",
                target,
                &[("Host", &host), ("Authorization", &auth)],
                "",
            );
            assert_eq!(r.status, 403, "{host} {target}");
            assert_eq!(r.json()["error"]["code"], "invalid_host");
        }
    }
    for host in [format!("localhost:{port}"), format!("127.0.0.1:{port}")] {
        let r = server.request(
            "GET",
            "/config/path",
            &[("Host", &host), ("Authorization", &auth)],
            "",
        );
        assert_eq!(r.status, 200, "{host}");
    }
}

/// A built WebUI's folder: its page, an asset, and a secret beside the folder that must stay out of reach.
fn webui(root: &Path) -> PathBuf {
    let dist = root.join("dist");
    std::fs::create_dir_all(dist.join("assets")).unwrap();
    std::fs::write(
        dist.join("index.html"),
        "<!doctype html><title>WeFlow</title>",
    )
    .unwrap();
    std::fs::write(dist.join("assets/index-a1b2.js"), "console.log(1)").unwrap();
    std::fs::write(dist.join("weflow-web.json"), "{}").unwrap();
    std::fs::write(root.join("secret.txt"), "not for the page").unwrap();
    dist
}

#[test]
fn the_webui_is_served_under_ui() {
    let root = temp_root("webui");
    let dist = webui(&root);
    let server = start(
        &root,
        &["--api-token", TOKEN, "--ui-dir", dist.to_str().unwrap()],
    );
    let get = |target: &str| server.request("GET", target, &[], "");

    // the page and its files need no token
    let page = get("/ui/");
    assert_eq!(page.status, 200);
    assert!(page.body.contains("<title>WeFlow"));
    assert_eq!(page.headers["content-type"], "text/html; charset=utf-8");
    assert_eq!(page.headers["cache-control"], "no-cache");
    assert_eq!(page.headers["referrer-policy"], "no-referrer");
    assert_eq!(page.headers["x-content-type-options"], "nosniff");
    let script = get("/ui/assets/index-a1b2.js");
    assert_eq!(script.status, 200);
    assert_eq!(script.body, "console.log(1)");
    assert_eq!(
        script.headers["content-type"],
        "text/javascript; charset=utf-8"
    );
    assert!(script.headers["cache-control"].contains("immutable"));
    assert_eq!(
        get("/ui/weflow-web.json").headers["cache-control"],
        "no-cache"
    );

    // the page's own paths get the page; missing files and API paths do not
    assert!(get("/ui/chat/wxid_a").body.contains("<title>WeFlow"));
    assert_eq!(get("/ui/assets/missing.js").status, 404);
    assert_eq!(get("/ui/assets/chunk").status, 404);
    assert_eq!(get("/ui/missing.png").status, 404);
    let api = server.call("GET", "/chat/no-such-command", "");
    assert_eq!(api.status, 404);
    assert_eq!(api.json()["success"], false);

    // nothing outside the folder
    for target in [
        "/ui/../secret.txt",
        "/ui/%2e%2e/secret.txt",
        "/ui/assets/..%2f..%2fsecret.txt",
        "/ui/..%5csecret.txt",
    ] {
        let r = get(target);
        assert!(!r.body.contains("not for the page"), "{target}");
        assert_ne!(r.status, 200, "{target}");
    }

    // into the page, keeping the token in the address
    let r = get("/ui?token=abc");
    assert_eq!(r.status, 308);
    assert_eq!(r.headers["location"], "/ui/?token=abc");
    let r = get("/?token=abc");
    assert_eq!(r.status, 302);
    assert_eq!(r.headers["location"], "/ui/?token=abc");
    // read-only, and still only for this machine
    assert_eq!(server.call("POST", "/ui/", "").status, 405);
    let r = server.request("GET", "/ui/", &[("Host", "evil.example")], "");
    assert_eq!(r.status, 403);
}

#[test]
fn a_link_out_of_the_webui_folder_is_not_followed() {
    let root = temp_root("webui-link");
    let dist = webui(&root);
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(root.join("secret.txt"), dist.join("leak.txt"));
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(root.join("secret.txt"), dist.join("leak.txt"));
    if linked.is_err() {
        // Windows without the right to make links
        return;
    }
    let server = start(
        &root,
        &["--api-token", TOKEN, "--ui-dir", dist.to_str().unwrap()],
    );
    let r = server.request("GET", "/ui/leak.txt", &[], "");
    assert_eq!(r.status, 404);
    assert!(!r.body.contains("not for the page"));
}

#[test]
fn without_a_webui_the_api_runs_alone() {
    let root = temp_root("no-webui");
    let mut child = weflow(&root)
        .args(["--json", "serve", "--port", "0", "--api-token", TOKEN])
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
    assert_eq!(event["ui"], false);
    assert!(event["uiUrl"].is_null());

    let server = start(&root, &["--api-token", TOKEN]);
    let r = server.request("GET", "/ui/", &[], "");
    assert_eq!(r.status, 404);
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--ui-dir"));
    // `/` is not taken over
    assert_eq!(server.request("GET", "/", &[], "").status, 401);

    // a folder without a page is refused at the start
    let (code, _, err) = run_cli(
        &root,
        &["serve", "--port", "0", "--ui-dir", root.to_str().unwrap()],
    );
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("index.html"), "{err}");
}

#[test]
fn the_startup_message_gives_the_webui_address_with_the_token() {
    let root = temp_root("webui-url");
    let dist = webui(&root);
    let mut child = weflow(&root)
        .args([
            "--json",
            "serve",
            "--port",
            "0",
            "--api-token",
            TOKEN,
            "--ui-dir",
        ])
        .arg(&dist)
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
    assert_eq!(event["ui"], true);
    let url = event["uiUrl"].as_str().unwrap();
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    assert!(url.ends_with(&format!("/ui/?token={TOKEN}")), "{url}");
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
    account_home_with(root, |_| {})
}

/// [`account_home`] with more done to the account first.
fn account_home_with(root: &Path, more: impl FnOnce(&Fixture)) -> PathBuf {
    let (home, fixture) = account(root);
    more(&fixture);
    home
}

/// [`account_home`] and the account, to change it later.
fn account(root: &Path) -> (PathBuf, Fixture) {
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
fn commands_answer_with_the_keys_the_server_hands_over() {
    let root = temp_root("keys");
    let home = account_home(&root);
    let (exit, terminal, stderr) = run_cli(&home, &["--json", "chat", "sessions"]);
    assert_eq!(exit, 0, "{stderr}");

    // once the server has opened the databases of a chat page, the commands it runs get their keys
    let server = start(&home, &["--api-token", TOKEN]);
    assert_eq!(server.call("GET", "/api/chat/sessions", "").status, 200);
    let r = server.call("GET", "/chat/sessions", "");
    assert_eq!((r.status, &r.body), (200, &terminal));

    // keys that do not open the files, a cut-off record or none at all: the keys are derived again
    for given in [vec![0u8; 80 * 3], vec![1u8; 7], Vec::new()] {
        let mut child = weflow(&home)
            .args(["--json", "chat", "sessions"])
            .env("WEFLOW_KEYS_ON_STDIN", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&given).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), terminal);
    }
}

#[test]
fn reads_run_in_process_with_the_same_answers() {
    let root = temp_root("in-process");
    let home = account_home_with(&root, |f| {
        use weflow_native::fixture::{SnsPostSpec, T0};
        let post = |tid: i64, user: &'static str, desc: &'static str| SnsPostSpec {
            tid,
            user,
            create_time: T0 + tid * 60,
            desc,
            kind: 1,
            media: tid as usize % 3,
            likes: &[("wxid_me_ab12", "Me")],
            extra: "",
        };
        f.sns_db(&[
            post(1, "wxid_bob", "first"),
            post(2, "wxid_me_ab12", "mine"),
            post(3, "wxid_bob", "third"),
        ]);
    });
    let server = start(&home, &["--api-token", TOKEN]);
    let same = |target: &str| {
        let child = server.call("GET", target, "");
        let own = server.call("GET", &format!("/api{target}"), "");
        assert_eq!(
            (own.status, &own.body),
            (child.status, &child.body),
            "{target}"
        );
        for h in ["x-weflow-exit-code", "x-weflow-error-code"] {
            assert_eq!(own.headers.get(h), child.headers.get(h), "{target} {h}");
        }
        own
    };
    for target in [
        "/chat/sessions",
        "/chat/sessions?limit=1",
        "/chat/messages?session-id=wxid_bob&limit=2&offset=1",
        "/chat/messages?session-id=room1%40chatroom",
        "/chat/latest?session-id=wxid_bob&limit=1",
        "/chat/message?session-id=wxid_bob&local-id=2",
        "/chat/message?session-id=wxid_bob&local-id=2&parsed=true",
        "/chat/contact?username=wxid_bob",
        "/chat/contacts",
        "/chat/counts?sessions=wxid_bob&sessions=room1%40chatroom",
        "/chat/search?keyword=hello",
        "/chat/dates?session-id=wxid_bob",
        "/chat/date-counts?session-id=wxid_bob",
        "/chat/detail?session-id=wxid_bob&fast=true",
        // the rest of the details, and the timeline with its media, from the child process
        "/chat/detail?session-id=wxid_bob",
        "/sns/timeline",
        "/sns/timeline?limit=1&offset=1&user=wxid_bob",
        "/sns/timeline?keyword=thi",
        "/sns/users",
        "/sns/stats",
        "/sns/stats?fast=true",
        "/sns/post-counts",
        "/sns/post-counts?prefer-cache=true",
        "/sns/post-counts?user=wxid_bob",
    ] {
        let r = same(target);
        assert_eq!(r.status, 200, "{target}: {}", r.body);
    }
    // A media list lookup has exactly the parsed shape and cursor of the conversation page.
    let parsed = same("/chat/messages?session-id=wxid_bob&parsed=true").json();
    let lookup = same("/chat/message?session-id=wxid_bob&local-id=2&parsed=true").json();
    assert_eq!(
        parsed["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["localId"] == 2),
        Some(&lookup["data"])
    );
    // a page from a message's cursor: the one before Bob's message 4 (in message_1.db) goes on in message_0.db
    {
        use weflow_native::fixture::{DAY, T0};
        let latest = same("/chat/messages?session-id=wxid_bob&limit=2").json();
        let cursor = format!("{}.{}.4.1", (T0 + 3 * DAY) * 1000, T0 + 3 * DAY);
        assert_eq!(latest["data"][1]["_cursor"], cursor.as_str());
        let r = same(&format!(
            "/chat/messages?session-id=wxid_bob&limit=2&before={cursor}"
        ));
        assert_eq!(r.status, 200, "{}", r.body);
        let ids: Vec<i64> = r.json()["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["local_id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, [3, 2]);
        assert_eq!(
            same(&format!(
                "/chat/messages?session-id=wxid_bob&before={cursor}&offset=1"
            ))
            .status,
            400
        );
    }
    // the member panel carries the time it was made, and the open hub keeps it for a while
    let target = "/group/members?chatroom-id=room1%40chatroom";
    let panel = |path: &str| {
        let r = server.call("GET", path, "");
        assert_eq!(r.status, 200, "{path}: {}", r.body);
        let mut doc = r.json();
        let cached = doc["data"]["fromCache"].take();
        doc["data"]["updatedAt"].take();
        (doc, cached)
    };
    let (child, _) = panel(target);
    let (own, _) = panel(&format!("/api{target}"));
    assert_eq!(own, child);
    assert_eq!(panel(&format!("/api{target}")).1, true);
    assert_eq!(panel(&format!("/api{target}&refresh=true")).1, false);
    // failures answer the same too: from the command, and from parsing (left to the child process)
    assert_eq!(same("/chat/message?session-id=wxid_bob").status, 400);
    assert_eq!(same("/chat/messages").status, 400);
    assert_eq!(
        same("/chat/messages?limit=many&session-id=wxid_bob").status,
        400
    );

    // a changed config is read again
    let r = server.call(
        "PUT",
        "/config/set",
        r#"{"key": "decrypt_key", "value": "00"}"#,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_ne!(same("/chat/sessions").status, 200);

    // only the listed reads, as plain --json
    let r = server.call("POST", "/api/export/sessions", "");
    assert_eq!(r.status, 404);
    assert_eq!(server.call("GET", "/api/config/list", "").status, 404);
    assert_eq!(server.call("POST", "/api/chat/sessions", "").status, 405);
    for target in ["/api/chat/sessions?std=out", "/api/chat/sessions?std=in"] {
        assert_eq!(server.call("GET", target, "").status, 400, "{target}");
    }
    let auth = format!("Bearer {TOKEN}");
    let r = server.request(
        "GET",
        "/api/chat/sessions",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    assert_eq!(r.status, 400);
}

#[test]
fn answers_are_compressed_when_asked_and_cut_to_fields() {
    let root = temp_root("reply-size");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");

    // gzip or br as the client takes them, the same answer once decoded; nothing without `Accept-Encoding`
    for target in ["/chat/contacts", "/api/chat/contacts"] {
        let plain = server.call("GET", target, "");
        assert_eq!(plain.status, 200, "{}", plain.body);
        assert_eq!(plain.headers.get("content-encoding"), None);
        let ask = |encoding: &str| {
            server.request(
                "GET",
                target,
                &[("Authorization", &auth), ("Accept-Encoding", encoding)],
                "",
            )
        };
        let gz = ask("gzip");
        assert_eq!(gz.headers["content-encoding"], "gzip", "{target}");
        let mut unzipped = Vec::new();
        flate2::read::GzDecoder::new(gz.bytes.as_slice())
            .read_to_end(&mut unzipped)
            .unwrap();
        assert_eq!(unzipped, plain.bytes, "{target}");
        let br = ask("br;q=1, gzip;q=0.5");
        assert_eq!(br.headers["content-encoding"], "br", "{target}");
        let mut unbrotlied = Vec::new();
        brotli_decompressor::Decompressor::new(br.bytes.as_slice(), 4096)
            .read_to_end(&mut unbrotlied)
            .unwrap();
        assert_eq!(unbrotlied, plain.bytes, "{target}");
        assert_eq!(ask("identity").headers.get("content-encoding"), None);
    }

    // `fields`: only those keys of each row, the same from both routes, however the names are given
    let rows = |target: &str| {
        let child = server.call("GET", target, "");
        let own = server.call("GET", &format!("/api{target}"), "");
        assert_eq!(
            (own.status, &own.body),
            (child.status, &child.body),
            "{target}"
        );
        assert_eq!(child.status, 200, "{target}: {}", child.body);
        child.json()["data"].clone()
    };
    let page = rows("/chat/messages?session-id=wxid_bob&fields=local_id,create_time");
    let full = rows("/chat/messages?session-id=wxid_bob");
    assert!(full[0].as_object().unwrap().len() > 2);
    let cut: Vec<Value> = full
        .as_array()
        .unwrap()
        .iter()
        .map(|m| json!({ "local_id": m["local_id"], "create_time": m["create_time"] }))
        .collect();
    assert_eq!(page, Value::Array(cut));
    assert_eq!(
        rows("/chat/messages?session-id=wxid_bob&fields=local_id&fields=+create_time,,"),
        page
    );
    let contacts = rows("/chat/contacts?fields=username,nosuchfield");
    assert!(contacts.as_array().unwrap().iter().all(|c| c
        .as_object()
        .unwrap()
        .keys()
        .eq(["username"])));
    // a failure as it is
    let failed = server.call("GET", "/chat/messages?fields=local_id", "");
    assert_eq!(
        (failed.status, &failed.body),
        (400, &server.call("GET", "/chat/messages", "").body)
    );
    for target in [
        "/chat/sessions?fields=username&std=out",
        "/chat/sessions?fields=username&return=file",
    ] {
        assert_eq!(server.call("GET", target, "").status, 400, "{target}");
    }
    let r = server.request(
        "GET",
        "/chat/sessions?fields=username",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    assert_eq!(r.status, 400);
}

#[test]
fn answers_tell_where_the_time_went() {
    let root = temp_root("timing");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN, "--log-requests"]);
    // `Server-Timing`: what ran the command and how long it took, then the total
    let stages = |target: &str| {
        let r = server.call("GET", target, "");
        assert_eq!(r.status, 200, "{target}: {}", r.body);
        let timing = r.headers["server-timing"].clone();
        timing
            .split(", ")
            .map(|s| {
                let (name, rest) = s.split_once(';').unwrap();
                let ms: f64 = rest.rsplit_once("dur=").unwrap().1.parse().unwrap();
                assert!(ms >= 0.0, "{timing}");
                let desc = rest
                    .split_once("desc=\"")
                    .map(|(_, d)| d.split('"').next().unwrap().to_string());
                (name.to_string(), desc)
            })
            .collect::<Vec<_>>()
    };
    let cmd = |desc: &str| ("cmd".to_string(), Some(desc.to_string()));
    let total = ("total".to_string(), None);
    assert_eq!(
        stages("/api/chat/sessions?limit=1"),
        [
            ("wait".to_string(), Some("in line for a place".to_string())),
            cmd("in process"),
            total.clone()
        ]
    );
    assert_eq!(
        stages("/chat/sessions?limit=1"),
        [cmd("child process"), total.clone()]
    );
    assert_eq!(stages("/health"), [total]);
    let local = server.request("GET", "/health", &[("Origin", "http://localhost:5173")], "");
    assert_eq!(
        local.headers["timing-allow-origin"],
        "http://localhost:5173"
    );
    assert!(local.headers["access-control-expose-headers"].contains("Server-Timing"));

    // `--log-requests`: one line each, with the path and without the query, which may hold ids
    let r = server.call("GET", "/chat/messages?session-id=wxid_bob&limit=1", "");
    assert_eq!(r.status, 200);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let logged = loop {
        let lines = server.stderr.lock().unwrap().clone();
        let found: Vec<Value> = lines
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|e| e["type"] == "request")
            .collect();
        if found.len() >= 5 || std::time::Instant::now() > deadline {
            break (found, lines);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let (events, lines) = logged;
    assert!(lines.iter().all(|l| !l.contains("wxid_bob")), "{lines:?}");
    let last = events.last().unwrap_or_else(|| panic!("{lines:?}"));
    assert_eq!(last["method"], "GET");
    assert_eq!(last["path"], "/chat/messages");
    assert_eq!(last["status"], 200);
    assert!(last["ms"].as_f64().unwrap() >= 0.0);
    assert_eq!(events.len(), 5, "{lines:?}");
}

#[test]
fn media_come_back_as_files_from_the_process() {
    use weflow_native::fixture::{VoiceSpec, DAY, T0};
    let root = temp_root("media");
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
    jpeg.extend((0..3000u32).map(|i| (i as u8).wrapping_mul(7) | 1));
    let voice_at = T0 + 500;
    let mut img_dir = PathBuf::new();
    let home = account_home_with(&root, |f| {
        // the image of `wxid_bob`'s message 3 (an old-style .dat: the whole file XOR-ed) and a real SILK voice
        let img = f
            .account_dir
            .join("msg/attach")
            .join(weflow_core::image::session_dir_for_storage(
                "wxid_bob",
                |s| s.to_string(),
            ))
            .join(weflow_core::image::year_month_from_create_time(Some(
                T0 + DAY,
            )))
            .join("Img");
        std::fs::create_dir_all(&img).unwrap();
        std::fs::write(
            img.join("aabbccddeeff00112233445566778899.dat"),
            jpeg.iter().map(|b| b ^ 0x5a).collect::<Vec<u8>>(),
        )
        .unwrap();
        img_dir = img;
        f.media_db(
            0,
            &[VoiceSpec {
                chat: "wxid_bob",
                create_time: voice_at,
                local_id: 6,
                svr_id: 9_000_000_000_001,
                data: weflow_silk::testenc::encode_tone(24000),
                index: "0",
            }],
        );
    });
    let (exit, _, stderr) = run_cli(&home, &["config", "set", "image_xor_key", "90"]);
    assert_eq!(exit, 0, "{stderr}");
    let server = start(
        &home,
        &["--api-token", TOKEN, "--max-media", "1", "--max-reads", "1"],
    );
    let auth = format!("Bearer {TOKEN}");

    // the image: its bytes, the same as the child process writes, with an ETag that answers 304
    let query = format!("session-id=wxid_bob&msg-id=3&create-time={}", T0 + DAY);
    let r = server.call("GET", &format!("/api/chat/image-data?{query}"), "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.headers["content-type"], "image/jpeg");
    assert_eq!(r.bytes, jpeg);
    assert!(r.headers["cache-control"].contains("max-age"));
    // the time in line for a place to decode, then the decoding
    let timing = &r.headers["server-timing"];
    assert!(
        timing.starts_with("wait;") && timing.contains(", cmd;desc=\"in process\";"),
        "{timing}"
    );
    let etag = r.headers["etag"].clone();
    let child = server.call(
        "POST",
        "/chat/image-data?return=file",
        &format!(
            r#"{{"session-id": "wxid_bob", "msg-id": "3", "create-time": {}}}"#,
            T0 + DAY
        ),
    );
    assert_eq!(child.status, 200, "{}", child.body);
    assert_eq!(form_parts(&child)[1].1, jpeg);
    let again = server.request(
        "GET",
        &format!("/api/chat/image-data?{query}"),
        &[("Authorization", &auth), ("If-None-Match", &etag)],
        "",
    );
    assert_eq!(again.status, 304);
    assert!(again.bytes.is_empty());
    assert_eq!(again.headers["etag"], etag);
    let other = server.request(
        "GET",
        &format!("/api/chat/image-data?{query}"),
        &[("Authorization", &auth), ("If-None-Match", "\"0\"")],
        "",
    );
    assert_eq!(other.status, 200);
    // the file as it is, even to a client that takes compressed answers: an image is compressed already
    let plain = server.request(
        "GET",
        &format!("/api/chat/image-data?{query}"),
        &[("Authorization", &auth), ("Accept-Encoding", "gzip, br")],
        "",
    );
    assert_eq!(plain.headers.get("content-encoding"), None);
    assert_eq!(plain.bytes, jpeg);

    // a part of it, as a player or a resumed download asks: 206, or 416 past the end, or the whole file when it
    // changed since (`If-Range`)
    let part = |range: &str, if_range: &str| {
        let mut headers = vec![
            ("Authorization", auth.as_str()),
            ("Range", range),
            ("Accept-Encoding", "gzip"),
        ];
        if !if_range.is_empty() {
            headers.push(("If-Range", if_range));
        }
        server.request(
            "GET",
            &format!("/api/chat/image-data?{query}"),
            &headers,
            "",
        )
    };
    let len = jpeg.len();
    let r = part("bytes=0-9", "");
    assert_eq!(r.status, 206);
    assert_eq!(r.bytes, jpeg[..10]);
    assert_eq!(r.headers["content-range"], format!("bytes 0-9/{len}"));
    assert_eq!(r.headers["accept-ranges"], "bytes");
    assert_eq!(r.headers.get("content-encoding"), None);
    let r = part("bytes=-4", &etag);
    assert_eq!((r.status, r.bytes.as_slice()), (206, &jpeg[len - 4..]));
    let r = part("bytes=0-9", "\"0\"");
    assert_eq!((r.status, r.bytes.len()), (200, len));
    let r = part(&format!("bytes={len}-"), "");
    assert_eq!(r.status, 416);
    assert_eq!(r.headers["content-range"], format!("bytes */{len}"));

    // a thumbnail for a list: WeChat's own `_t.dat` when there is one, else the image
    let thumb = format!("/api/chat/image-data?{query}&thumb=true");
    assert_eq!(server.call("GET", &thumb, "").bytes, jpeg);
    let small: Vec<u8> = [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3].to_vec();
    std::fs::write(
        img_dir.join("aabbccddeeff00112233445566778899_t.dat"),
        small.iter().map(|b| b ^ 0x5a).collect::<Vec<u8>>(),
    )
    .unwrap();
    let r = server.call("GET", &thumb, "");
    assert_eq!(
        (r.status, r.headers["content-type"].as_str()),
        (200, "image/jpeg")
    );
    assert_eq!(r.bytes, small);
    let child = server.call(
        "POST",
        "/chat/image-data?return=file",
        &format!(
            r#"{{"session-id": "wxid_bob", "msg-id": "3", "create-time": {}, "thumb": true}}"#,
            T0 + DAY
        ),
    );
    assert_eq!(form_parts(&child)[1].1, small);
    // and the image itself is still the image
    assert_eq!(
        server
            .call("GET", &format!("/api/chat/image-data?{query}"), "")
            .bytes,
        jpeg
    );

    // a screen of images asked for at once, one decoded at a time: they wait in line instead of answering 429, and
    // take no place of the other reads
    let statuses: Vec<u16> = std::thread::scope(|s| {
        let all: Vec<_> = (0..20)
            .map(|_| s.spawn(|| server.call("GET", &format!("/api/chat/image-data?{query}"), "")))
            .collect();
        assert_eq!(server.call("GET", "/api/chat/sessions", "").status, 200);
        all.into_iter().map(|h| h.join().unwrap().status).collect()
    });
    assert!(statuses.iter().all(|&s| s == 200), "{statuses:?}");
    // so do the reads of /api/, one at a time here; a child process route is refused at once instead
    let at_once = |target: &str, n: usize| -> Vec<u16> {
        std::thread::scope(|s| {
            let all: Vec<_> = (0..n)
                .map(|_| s.spawn(|| server.call("GET", target, "").status))
                .collect();
            all.into_iter().map(|h| h.join().unwrap()).collect()
        })
    };
    let statuses = at_once("/api/chat/sessions?limit=1", 12);
    assert!(statuses.iter().all(|&s| s == 200), "{statuses:?}");
    let statuses = at_once("/chat/sessions?limit=1", 8);
    assert!(statuses.contains(&429), "{statuses:?}");

    // the voice, decoded to WAV: the same as the child process prints in base64
    let query = format!(
        "session-id=wxid_bob&msg-id=6&create-time={voice_at}&server-id=9000000000001&sender=wxid_bob"
    );
    let r = server.call("GET", &format!("/api/chat/voice-data?{query}"), "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.headers["content-type"], "audio/wav");
    assert_eq!(&r.bytes[..4], b"RIFF");
    let wav = server.request(
        "GET",
        &format!("/api/chat/voice-data?{query}"),
        &[("Authorization", &auth), ("Accept-Encoding", "gzip")],
        "",
    );
    assert_eq!(wav.headers.get("content-encoding"), None);
    assert_eq!(wav.bytes, r.bytes);
    let child = server.call(
        "POST",
        "/chat/voice-data",
        &format!(
            r#"{{"session-id": "wxid_bob", "msg-id": "6", "create-time": {voice_at}, "server-id": "9000000000001", "sender": "wxid_bob"}}"#
        ),
    );
    assert_eq!(child.status, 200, "{}", child.body);
    let printed = child.json()["data"]
        .as_object()
        .unwrap()
        .values()
        .find_map(|v| v.as_str().filter(|s| s.len() > 100).map(String::from))
        .expect("the WAV in base64");
    use base64::Engine;
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(printed)
            .unwrap(),
        r.bytes
    );

    // failures are the command's --json document, as from the child process
    let failed = |api: &str, child_body: &str| {
        let own = server.call("GET", api, "");
        let child = server.call("POST", &api["/api".len()..], child_body);
        assert_ne!(own.status, 200, "{api}");
        assert_eq!(
            (own.status, own.json()["error"]["code"].clone()),
            (child.status, child.json()["error"]["code"].clone()),
            "{api}: {} / {}",
            own.body,
            child.body
        );
    };
    // a text message has no image
    failed(
        "/api/chat/image-data?session-id=wxid_bob&msg-id=1",
        r#"{"session-id": "wxid_bob", "msg-id": "1", "out": "unused.jpg"}"#,
    );
    failed(
        "/api/chat/voice-data?session-id=wxid_bob&msg-id=99",
        r#"{"session-id": "wxid_bob", "msg-id": "99"}"#,
    );
    // a command line that does not parse is reported by the child process, which writes nothing
    let r = server.call("GET", "/api/chat/image-data?session-id=wxid_bob", "");
    assert_eq!(r.status, 400, "{}", r.body);
    assert_eq!(r.json()["error"]["code"], "invalid_arguments");
    assert!(!Path::new("-").exists());

    // only GET, and the file is the answer
    let r = server.call("POST", &format!("/api/chat/image-data?{query}"), "");
    assert_eq!(r.status, 405);
    assert_eq!(r.headers["allow"], "GET");
    let r = server.call(
        "GET",
        "/api/chat/image-data?session-id=wxid_bob&msg-id=3&out=x.jpg",
        "",
    );
    assert_eq!(r.status, 400, "{}", r.body);
}

/// An open `GET /api/push/messages` and what it sent so far.
#[test]
fn videos_are_sent_in_the_parts_asked_for() {
    use weflow_native::fixture::{MsgSpec, DAY, T0};
    let root = temp_root("video");
    let md5 = "0123456789abcdef0123456789abcdef";
    let at = T0 + 4 * DAY + 60;
    // larger than one chunk read, so a range crosses chunks
    let video: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    let (cover, thumb) = (b"\xFF\xD8\xFFcover".to_vec(), b"\xFF\xD8\xFFthumb".to_vec());
    let mut folder = PathBuf::new();
    let home = account_home_with(&root, |f| {
        let xml = |md5: &str| {
            format!("<msg><videomsg md5=\"{md5}\" length=\"700000\" playlength=\"3\" /></msg>")
        };
        f.message_shard(
            1,
            &[(
                "wxid_bob",
                vec![
                    MsgSpec::text(4, "wxid_me", T0 + 3 * DAY, "later"),
                    MsgSpec::text(5, "wxid_bob", T0 + 4 * DAY, "see you"),
                    MsgSpec::text(7, "wxid_bob", at, &xml(md5)).of_type(43),
                    // WeChat has not downloaded this one
                    MsgSpec::text(
                        8,
                        "wxid_bob",
                        at + 60,
                        &xml("ffffffffffffffffffffffffffffffff"),
                    )
                    .of_type(43),
                ],
            )],
        );
        folder = f
            .account_dir
            .join("msg/video")
            .join(weflow_core::image::year_month_from_create_time(Some(at)));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("{md5}.mp4")), &video).unwrap();
        std::fs::write(folder.join(format!("{md5}.jpg")), &cover).unwrap();
        std::fs::write(folder.join(format!("{md5}_thumb.jpg")), &thumb).unwrap();
    });
    let server = start(&home, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");
    let query = format!("session-id=wxid_bob&msg-id=7&create-time={at}");
    let get = |extra: &[(&str, &str)], q: &str| {
        let mut headers = vec![("Authorization", auth.as_str())];
        headers.extend_from_slice(extra);
        server.request("GET", &format!("/api/chat/video-data?{q}"), &headers, "")
    };

    // the whole video, then parts of it, as a player asks while it plays and seeks
    let r = get(&[], &query);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.headers["content-type"], "video/mp4");
    assert_eq!(r.headers["content-length"], video.len().to_string());
    assert_eq!(r.headers["accept-ranges"], "bytes");
    assert_eq!(r.bytes, video);
    let etag = r.headers["etag"].clone();
    let len = video.len();
    for (range, first, last) in [
        ("bytes=0-1", 0, 1),
        ("bytes=200000-600000", 200_000, 600_000),
        ("bytes=650000-", 650_000, len - 1),
        ("bytes=-10", len - 10, len - 1),
    ] {
        let r = get(&[("Range", range)], &query);
        assert_eq!(r.status, 206, "{range}");
        assert_eq!(
            r.headers["content-range"],
            format!("bytes {first}-{last}/{len}")
        );
        assert_eq!(r.headers["content-length"], (last + 1 - first).to_string());
        assert_eq!(r.bytes, video[first..=last], "{range}");
    }
    assert_eq!(
        get(&[("Range", &format!("bytes={len}-"))], &query).status,
        416
    );
    assert_eq!(get(&[("If-None-Match", &etag)], &query).status, 304);
    // a changed file is another file
    let r = get(&[("Range", "bytes=0-1"), ("If-Range", &etag)], &query);
    assert_eq!(r.status, 206);
    std::fs::write(folder.join(format!("{md5}.mp4")), &video[..1000]).unwrap();
    let r = get(&[("Range", "bytes=0-1"), ("If-Range", &etag)], &query);
    assert_eq!((r.status, r.bytes.len()), (200, 1000));
    std::fs::write(folder.join(format!("{md5}.mp4")), &video).unwrap();

    // its thumbnail, else its cover
    let r = get(&[], &format!("{query}&thumb=true"));
    assert_eq!(
        (r.status, r.headers["content-type"].as_str()),
        (200, "image/jpeg")
    );
    assert_eq!(r.bytes, thumb);
    std::fs::remove_file(folder.join(format!("{md5}_thumb.jpg"))).unwrap();
    assert_eq!(get(&[], &format!("{query}&thumb=true")).bytes, cover);

    // through the child process: the video written to a file
    let child = server.call(
        "POST",
        "/chat/video-data?return=file",
        &format!(r#"{{"session-id": "wxid_bob", "msg-id": "7", "create-time": {at}}}"#),
    );
    assert_eq!(child.status, 200, "{}", child.body);
    let parts = form_parts(&child);
    assert_eq!(parts[1].1, video);
    let result: Value = serde_json::from_slice(&parts[0].1).unwrap();
    assert_eq!(result["success"], true, "{result}");

    // not on this computer: 404; not a video: 400
    let r = get(
        &[],
        &format!("session-id=wxid_bob&msg-id=8&create-time={}", at + 60),
    );
    assert_eq!(r.status, 404, "{}", r.body);
    assert_eq!(r.json()["error"]["details"]["failureKind"], "not_found");
    let r = get(
        &[],
        &format!("session-id=wxid_bob&msg-id=5&create-time={}", T0 + 4 * DAY),
    );
    assert_eq!(r.status, 400, "{}", r.body);
}

#[test]
fn moments_media_come_from_the_cache_and_only_from_wechat() {
    use md5::{Digest, Md5};
    // what `sns media` fetched before is kept in the cache, named after the md5 of its address: the route reads it
    // from there, without the network (#202)
    let root = temp_root("sns-media");
    let home = account_home(&root);
    let (exit, listed, _) = run_cli(&home, &["--json", "cache", "list"]);
    assert_eq!(exit, 0);
    let listed: Value = serde_json::from_str(&listed).unwrap();
    let folder = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["part"] == "sns")
        .and_then(|part| part["paths"][0].as_str())
        .map(PathBuf::from)
        .expect("the Moments cache folder");
    std::fs::create_dir_all(&folder).unwrap();
    let cached = |url: &str, ext: &str, bytes: &[u8]| {
        let name = format!("{:x}{ext}", Md5::digest(url.as_bytes()));
        std::fs::write(folder.join(name), bytes).unwrap();
    };
    let image_url = "https://shmmsns.qpic.cn/mmsns/abc/0?token=T&idx=1";
    let image = b"\xFF\xD8\xFF\xE0moments".to_vec();
    cached(image_url, ".jpg", &image);
    let video_url = "https://szzjwxsns.video.qq.com/102/20202/snsvideodownload?m=x&token=T&idx=1";
    let video: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
    cached(video_url, ".mp4", &video);

    let server = start(&home, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");
    let get = |url: &str, extra: &[(&str, &str)]| {
        let mut headers = vec![("Authorization", auth.as_str())];
        headers.extend_from_slice(extra);
        let q = format!("url={}&key=123", percent(url));
        server.request("GET", &format!("/api/sns/media?{q}"), &headers, "")
    };

    let r = get(image_url, &[]);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.headers["content-type"], "image/jpeg");
    assert_eq!(r.bytes, image);
    assert_eq!(
        get(image_url, &[("If-None-Match", &r.headers["etag"])]).status,
        304
    );
    // a video is sent from its file, in the parts a player asks for
    let r = get(video_url, &[("Range", "bytes=1000-1999")]);
    assert_eq!(r.status, 206, "{}", r.body);
    assert_eq!(r.headers["content-type"], "video/mp4");
    assert_eq!(
        r.headers["content-range"],
        format!("bytes 1000-1999/{}", video.len())
    );
    assert_eq!(r.bytes, video[1000..2000]);

    // only https addresses of WeChat's media servers: the route does not fetch whatever it is given
    for url in [
        "http://shmmsns.qpic.cn/mmsns/abc/0",
        "https://127.0.0.1/secret",
        "https://qpic.cn.example.com/a",
        "file:///etc/hosts",
    ] {
        let r = get(url, &[]);
        assert_eq!(r.status, 400, "{url}: {}", r.body);
        assert_eq!(r.json()["error"]["code"], "invalid_arguments", "{url}");
    }
    // and it needs the token, as every /api/ route does
    assert_eq!(
        server
            .request(
                "GET",
                "/api/sns/media?url=https%3A%2F%2Fa.qpic.cn%2Fx",
                &[],
                ""
            )
            .status,
        401
    );
}

/// `text` as it goes in a query string.
fn percent(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

struct EventStream {
    stream: TcpStream,
    text: String,
}

impl EventStream {
    fn open(server: &Server, headers: &[(&str, &str)]) -> Self {
        let auth = format!("Bearer {TOKEN}");
        Self::open_at(
            server,
            "/api/push/messages",
            &[&[("Authorization", auth.as_str())], headers].concat(),
        )
    }

    fn open_at(server: &Server, path: &str, headers: &[(&str, &str)]) -> Self {
        let mut stream = TcpStream::connect(&server.addr).unwrap();
        let mut req = format!("GET {path} HTTP/1.1\r\nHost: {}\r\n", server.addr);
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes()).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        Self {
            stream,
            text: String::new(),
        }
    }

    /// Reads until `needle` arrives (true) or `secs` pass (false).
    fn wait_for(&mut self, needle: &str, secs: u64) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        let mut buf = [0u8; 4096];
        while !self.text.contains(needle) && std::time::Instant::now() < deadline {
            match self.stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.text.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(_) => {}
            }
        }
        self.text.contains(needle)
    }
}

#[test]
fn health_events_say_hello_then_beat_without_a_token() {
    let root = temp_root("health-events");
    let server = start_env(
        &root,
        &["--api-token", TOKEN],
        &[("WEFLOW_SSE_HEARTBEAT_MS", "200")],
    );
    let health = server.request("GET", "/health", &[], "").json();
    let mut events = EventStream::open_at(&server, "/health/events", &[]);
    assert!(events.wait_for("event: heartbeat", 10), "{}", events.text);
    assert!(events.text.starts_with("HTTP/1.1 200"), "{}", events.text);
    assert!(events.text.contains("text/event-stream"), "{}", events.text);
    // hello first, with what /health says; then heartbeats with the data
    let hello = events.text.find("event: hello").unwrap();
    assert!(hello < events.text.find("event: heartbeat").unwrap());
    for key in ["apiVersion", "version", "data"] {
        let said = format!("\"{key}\":{}", health[key]);
        assert!(events.text.contains(&said), "{key}: {}", events.text);
    }
    assert!(events.text.contains(&format!(
        "event: heartbeat\ndata: {{\"data\":{}}}",
        health["data"]
    )));
    let beats = events.text.matches("event: heartbeat").count();
    events.wait_for("not in the stream", 1);
    assert!(events.text.matches("event: heartbeat").count() >= beats + 2);

    assert_eq!(
        server.request("POST", "/health/events", &[], "").status,
        405
    );
    // the Host check holds here too
    let mut stream = TcpStream::connect(&server.addr).unwrap();
    stream
        .write_all(
            b"GET /health/events HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
}

#[test]
fn official_accounts_and_their_pushes_as_the_webui_reads_them() {
    use weflow_native::fixture::{MsgSpec, T0};
    let root = temp_root("biz");
    let push = |title: &str, more: &str| {
        format!(
            "<msg><appmsg><title>{title}</title><des>about {title}</des><url>https://mp.example/{title}</url>\
             <thumburl>https://mp.example/{title}.jpg</thumburl><content_list>{more}</content_list></appmsg></msg>"
        )
    };
    let second = "<item><title>second</title><url>https://mp.example/second</url><cover>https://mp.example/2.jpg</cover><digest>two</digest></item>";
    let home = account_home_with(&root, |f| {
        f.message_shard(
            0,
            &[(
                "gh_news",
                vec![
                    MsgSpec::text(1, "gh_news", T0, &push("first", "")).of_type(49),
                    MsgSpec::text(2, "gh_news", T0 + 3600, &push("lead", second)).of_type(49),
                ],
            )],
        );
    });
    let server = start(&home, &["--api-token", TOKEN]);

    let accounts = server.call("GET", "/biz/accounts", "").json();
    let accounts = accounts["data"]["accounts"].as_array().unwrap().clone();
    let news = accounts
        .iter()
        .find(|a| a["username"] == "gh_news")
        .unwrap_or_else(|| panic!("{accounts:?}"));
    assert_eq!(news["nick_name"], "News");

    let r = server.call(
        "GET",
        "/biz/messages?username=gh_news&limit=20&offset=0",
        "",
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let pushes = r.json()["data"]["messages"].as_array().unwrap().clone();
    // newest first, the lead article and the others of the push
    let titles: Vec<&str> = pushes
        .iter()
        .map(|p| p["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["lead", "first"]);
    assert_eq!(pushes[0]["url"], "https://mp.example/lead");
    assert_eq!(pushes[0]["cover"], "https://mp.example/lead.jpg");
    assert_eq!(pushes[0]["description"], "about lead");
    assert_eq!(pushes[0]["contentList"][0]["title"], "second");
    assert_eq!(pushes[0]["contentList"][0]["summary"], "two");
    assert!(pushes[0]["createTime"].as_i64().unwrap() > pushes[1]["createTime"].as_i64().unwrap());
    let r = server.call(
        "GET",
        "/biz/messages?username=gh_news&limit=20&offset=1",
        "",
    );
    assert_eq!(r.json()["data"]["messages"].as_array().unwrap().len(), 1);
}

// Ctrl+C as a signal; Windows has no way to send one to another process alone
#[cfg(unix)]
#[test]
fn event_streams_say_bye_when_the_server_stops() {
    let root = temp_root("bye");
    let server = start(&root, &["--api-token", TOKEN]);
    let mut events = EventStream::open_at(&server, "/health/events", &[]);
    assert!(events.wait_for("event: hello", 10), "{}", events.text);
    let sent = Command::new("kill")
        .args(["-INT", &server.child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    assert!(events.wait_for("event: bye", 5), "{}", events.text);
    assert!(
        events.text.contains("\"reason\":\"stopped\""),
        "{}",
        events.text
    );
}

#[test]
fn new_messages_are_pushed_as_events() {
    use weflow_native::fixture::{MsgSpec, SessionSpec, DAY, T0};
    let root = temp_root("push");
    let (home, fixture) = account(&root);
    let server = start(&home, &["--api-token", TOKEN]);

    // as a browser asks: the events stream as they come, not compressed
    let mut first = EventStream::open(&server, &[("Accept-Encoding", "gzip, deflate, br")]);
    assert!(first.wait_for("event: ready", 10), "{}", first.text);
    assert!(
        !first.text.to_ascii_lowercase().contains("content-encoding"),
        "{}",
        first.text
    );
    assert!(first.text.starts_with("HTTP/1.1 200"), "{}", first.text);
    assert!(first.text.contains("text/event-stream"), "{}", first.text);
    // the engine takes its baseline within a second of the first listener
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Bob writes again: a newer conversation time and a new message in the second message database
    fixture.message_shard(
        1,
        &[(
            "wxid_bob",
            vec![
                MsgSpec::text(4, "wxid_me", T0 + 3 * DAY, "later, compressed").compressed(),
                MsgSpec::text(5, "wxid_bob", T0 + 4 * DAY, "see you"),
                MsgSpec::text(6, "wxid_bob", T0 + 5 * DAY, "brand new"),
            ],
        )],
    );
    fixture.session_db(&[
        SessionSpec {
            username: "wxid_bob",
            summary: "brand new",
            last_timestamp: T0 + 5 * DAY,
            unread: 3,
            last_msg_type: 1,
        },
        SessionSpec {
            username: "room1@chatroom",
            summary: "ok",
            last_timestamp: T0 + 2 * DAY,
            unread: 0,
            last_msg_type: 1,
        },
        SessionSpec {
            username: "gh_news",
            summary: "daily",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 49,
        },
    ]);
    assert!(first.wait_for("brand new", 10), "{}", first.text);
    assert!(first.text.contains("event: message.new"), "{}", first.text);
    assert!(
        first.text.contains(r#""sessionId":"wxid_bob""#),
        "{}",
        first.text
    );
    let id: u64 = first
        .text
        .lines()
        .find_map(|l| l.strip_prefix("id: "))
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    // a page that comes later gets the kept events; one that reconnects after them does not get them again
    let mut later = EventStream::open(&server, &[]);
    assert!(later.wait_for("brand new", 5), "{}", later.text);
    let last = id.to_string();
    let mut again = EventStream::open(&server, &[("Last-Event-ID", &last)]);
    assert!(again.wait_for("event: ready", 5), "{}", again.text);
    assert!(!again.wait_for("brand new", 2), "{}", again.text);
    // one that comes back with the id of a run before (from 1 on, say) misses none of this run's: they start at
    // the time the server started
    assert!(id > 1_700_000_000_000, "{id}");
    let mut restarted = EventStream::open(&server, &[("Last-Event-ID", "57")]);
    assert!(restarted.wait_for("brand new", 5), "{}", restarted.text);
    // nothing new: each message is pushed once
    assert!(
        !first.wait_for(&format!("id: {}", id + 1), 2),
        "{}",
        first.text
    );

    let r = server.call("POST", "/api/push/messages", "");
    assert_eq!(r.status, 405);
    assert_eq!(
        server.request("GET", "/api/push/messages", &[], "").status,
        401
    );
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
