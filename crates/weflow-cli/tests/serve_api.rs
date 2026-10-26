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
        // no registry: `serve` would download its default WebUI from GHCR, and `/` would turn into a
        // redirect once it had (a test that wants a registry sets its own)
        .env("WEFLOW_WEB_REGISTRY", "http://127.0.0.1:9")
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
    assert_eq!(r.json()["apiVersion"], "2");
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
    // in the server's language, whatever the request asks for (#433)
    assert_eq!(get("zh-CN"), en);
    assert_eq!(server.call("POST", "/openapi.json", "").status, 405);
    let zh_root = temp_root("openapi-zh");
    let zh_server = start_env(&zh_root, &["--api-token", TOKEN], &[("WEFLOW_LANG", "zh")]);
    let zh = zh_server
        .request(
            "GET",
            "/openapi.json",
            &[("Authorization", &auth), ("Accept-Language", "en")],
            "",
        )
        .json();
    let summary = |d: &Value| d["paths"]["/chat/sessions"]["get"]["summary"].clone();
    assert_ne!(summary(&en), summary(&zh));
}

/// Every answer is in the server's language, from its process or a child's, whatever the request's
/// `Accept-Language` (#433).
#[test]
fn answers_are_in_the_server_s_language() {
    let root = temp_root("language");
    let home = account_home(&root);
    let server = start_env(&home, &["--api-token", TOKEN], &[("WEFLOW_LANG", "zh")]);
    let auth = format!("Bearer {TOKEN}");
    let chinese = |text: &str| text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    let mut seen = Vec::new();
    // a usage error, and an error of the read itself
    for target in ["/chat/messages", "/chat/message?session-id=wxid_bob"] {
        for route in [target.to_string(), format!("/api{target}")] {
            for lang in ["en", "zh-CN"] {
                let r = server.request(
                    "GET",
                    &route,
                    &[("Authorization", &auth), ("Accept-Language", lang)],
                    "",
                );
                assert_eq!(r.status, 400, "{route}: {}", r.body);
                let message = r.json()["error"]["message"].as_str().unwrap().to_string();
                assert!(chinese(&message), "{route} {lang}: {message}");
                seen.push((target, message));
            }
        }
    }
    // one message for each, with or without `/api/`
    seen.dedup();
    assert_eq!(seen.len(), 2, "{seen:#?}");
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
fn contacts_come_parsed_and_export_by_kind() {
    let root = temp_root("contacts");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    // the list the contacts page reads, in process, as the CLI gives it
    let r = server.call("GET", "/api/chat/contacts?parsed=true", "");
    let (_, stdout, _) = run_cli(&home, &["--json", "chat", "contacts", "--parsed"]);
    assert_eq!((r.status, &r.body), (200, &stdout));
    let kinds: Vec<String> = r.json()["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["type"].as_str().unwrap().to_string())
        .collect();
    assert!(
        kinds.iter().any(|k| k == "group") && kinds.iter().any(|k| k == "friend"),
        "{kinds:?}"
    );

    let out = root.join("groups.json");
    let (exit, _, stderr) = run_cli(
        &home,
        &[
            "--json",
            "export",
            "contacts",
            "--parsed",
            "--kind",
            "group",
            "--out",
            &out.to_string_lossy(),
        ],
    );
    assert_eq!(exit, 0, "{stderr}");
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let written = written.as_array().unwrap();
    assert_eq!(
        written.len(),
        kinds.iter().filter(|k| *k == "group").count()
    );
    assert!(written
        .iter()
        .all(|c| c["type"] == "group" && c.get("extra_buffer").is_none()));

    // a kind asks for the parsed list
    let (exit, _, _) = run_cli(
        &home,
        &[
            "export",
            "contacts",
            "--kind",
            "group",
            "--out",
            &out.to_string_lossy(),
        ],
    );
    assert_eq!(exit, 2);
}

#[test]
fn clearing_an_account_can_be_looked_at_first() {
    let root = temp_root("clear-dry");
    let home = account_home(&root);
    let kept = home.join("cache").join("Images").join("wxid_me_ab12");
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::write(kept.join("a.jpg"), b"jpeg").unwrap();
    let server = start(&home, &["--api-token", TOKEN]);

    // what would go, and nothing goes
    let r = server.call("DELETE", "/cache/clear-account?dry-run=true", "");
    assert_eq!(r.status, 200, "{}", r.body);
    let plan = &r.json()["data"];
    assert_eq!(plan["dryRun"], true);
    let paths: Vec<PathBuf> = plan["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| PathBuf::from(p["path"].as_str().unwrap()))
        .collect();
    assert!(
        paths.iter().any(|p| p.ends_with("wxid_me_ab12")),
        "{paths:?}"
    );
    assert!(plan["bytes"].as_u64().unwrap() >= 4);
    assert_eq!(plan["profileKeys"][1], "wxid");
    assert!(kept.join("a.jpg").exists());
    let (_, wxid, _) = run_cli(&home, &["config", "get", "wxid"]);
    assert_eq!(wxid.trim(), "wxid_me_ab12");

    // and only with `yes`, without `dry-run`
    let r = server.call("DELETE", "/cache/clear-account?dry-run=true&yes=true", "");
    assert_eq!(r.status, 400, "{}", r.body);
}

#[test]
fn refused_commands_are_marked_and_refused() {
    let root = temp_root("refused");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let doc = server.call("GET", "/openapi.json", "").json();
    let mut marked = Vec::new();
    for (path, ops) in doc["paths"].as_object().unwrap() {
        for (method, op) in ops.as_object().unwrap() {
            if op.get("x-weflow-refused") != Some(&Value::Bool(true)) {
                continue;
            }
            marked.push(format!("{} {path}", method.to_uppercase()));
            // its required parameters and its lists (the conversations of `anti-revoke install`, which refuses
            // each one), made up, in the query: they are taken from there for every method
            let mut query = Vec::new();
            let mut take = |name: &str, schema: &Value| {
                let value = match (schema.get("enum"), schema["type"].as_str()) {
                    (Some(values), _) => values[0].as_str().unwrap().to_string(),
                    (_, Some("integer")) => "1".to_string(),
                    _ => "x".to_string(),
                };
                query.push(format!("{name}={value}"));
            };
            for p in op["parameters"].as_array().unwrap() {
                if p["required"] == true || p["schema"]["type"] == "array" {
                    take(p["name"].as_str().unwrap(), &p["schema"]);
                }
            }
            let body = &op["requestBody"]["content"]["application/json"]["schema"];
            let required = body["required"].as_array().cloned().unwrap_or_default();
            for (name, schema) in body["properties"].as_object().into_iter().flatten() {
                if required.contains(&Value::from(name.as_str())) || schema["type"] == "array" {
                    take(name, schema);
                }
            }
            let r = server.call(
                &method.to_uppercase(),
                &format!("{path}?{}", query.join("&")),
                "",
            );
            assert_eq!(r.status, 501, "{method} {path}: {}", r.body);
            assert_eq!(r.json()["error"]["code"], "unsupported");
        }
    }
    marked.sort();
    assert_eq!(
        marked,
        [
            "DELETE /chat/anti-revoke/uninstall",
            "DELETE /chat/delete-message",
            "DELETE /sns/block-delete",
            "DELETE /sns/delete",
            "PATCH /chat/update-message",
            "PUT /chat/anti-revoke/install",
            "PUT /sns/block-delete",
        ]
    );
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
        "/chat/resources",
        "/chat/resources?session=wxid_bob&type=image&limit=1",
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
        // #413: the other reads the WebUI asks for
        "/chat/statuses?usernames=wxid_bob&usernames=room1%40chatroom",
        "/chat/statuses",
        "/chat/tab-counts",
        "/group/list",
        "/group/ranking?chatroom-id=room1%40chatroom",
        "/group/ranking?chatroom-id=room1%40chatroom&limit=1&start=2020-01-01&end=2099-12-31",
        "/group/hours?chatroom-id=room1%40chatroom",
        "/group/media?chatroom-id=room1%40chatroom",
        "/group/member?chatroom-id=room1%40chatroom&username=wxid_bob",
        "/group/member-messages?chatroom-id=room1%40chatroom&username=wxid_bob&limit=1",
        "/group/member-messages?chatroom-id=room1%40chatroom&username=wxid_bob&cursor=1",
        // the aggregate the child process keeps in its file, then read again
        "/analytics/overall",
        "/analytics/overall?force=true",
        "/analytics/rankings",
        "/analytics/rankings?limit=1&start=2020-01-01",
        "/analytics/time",
        "/analytics/excluded",
        "/analytics/exclude-candidates",
        "/biz/accounts",
        "/biz/messages?username=wxid_bob&limit=2",
        "/biz/pay-records",
        "/report/annual/years",
        "/cache/list",
        "/config/get",
        "/config/get?key=wxid",
        "/config/get?key=config_path",
        "/config/profiles",
        "/runtime/info",
        "/db/detect",
        "/db/test",
        "/db/wxid",
    ] {
        let r = same(target);
        assert_eq!(r.status, 200, "{target}: {}", r.body);
    }
    // an insight record that is not there: the command's own failure
    assert_eq!(same("/insight/get?id=none").status, 500);
    // a cursor that is not one fails the same way
    assert_eq!(same("/chat/resources?before=1.2.3").status, 400);
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
    // the footprint says how long it took
    for target in [
        "/insight/footprint",
        "/insight/footprint?start=2020-01-01&end=2099-12-31",
    ] {
        let timed = |path: &str| {
            let r = server.call("GET", path, "");
            assert_eq!(r.status, 200, "{path}: {}", r.body);
            let mut doc = r.json();
            assert!(doc["data"]["diagnostics"]["elapsed_ms"].take().is_number());
            doc
        };
        assert_eq!(timed(&format!("/api{target}")), timed(target), "{target}");
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

/// `key db` run as the server runs it ends by itself once its stdin ends, with `user_interrupt`, as on Ctrl+C: a
/// hook it had set would be out of WeChat first (#287). It is ended here while it waits for WeChat, before any hook.
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn key_db_ends_by_itself_when_its_stdin_ends() {
    let root = temp_root("key-db-stdin");
    let mut child = weflow(&root)
        .env("WEFLOW_CANCEL_ON_STDIN", "1")
        .args(["--json", "key", "db", "--timeout", "60"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // waiting for WeChat to quit or to start, whichever this machine needs
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let phase = loop {
        let line = lines
            .next()
            .expect("key db ended before it waited")
            .unwrap();
        if let Ok(event) = serde_json::from_str::<Value>(&line) {
            if event["type"] == "key_phase" {
                break event["phase"].clone();
            }
        }
    };
    assert!(phase == "quit_wechat" || phase == "open_wechat", "{phase}");
    let ended = std::time::Instant::now();
    drop(child.stdin.take());
    std::thread::spawn(move || lines.for_each(drop));
    let status = child.wait().unwrap();
    assert!(ended.elapsed() < std::time::Duration::from_secs(5));
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    let doc: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["error"]["code"], "user_interrupt", "{out}");
    assert_eq!(status.code(), Some(130));
}

/// An `/api/` request with parameters the command line refuses gets clap's error from the server's process, the one
/// the route without `/api/` answers from a child process, without starting one (#432).
#[test]
fn usage_errors_are_answered_in_process() {
    let root = temp_root("usage-errors");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    for target in [
        "/chat/messages",
        "/chat/messages?session-id=wxid_bob&limit=many",
        "/chat/messages?session-id=wxid_bob&limit=1&limit=2",
        "/sns/timeline?limit=x",
        "/group/members",
    ] {
        let own = server.call("GET", &format!("/api{target}"), "");
        let child = server.call("GET", target, "");
        assert_eq!(
            (own.status, &own.body),
            (child.status, &child.body),
            "{target}"
        );
        assert_eq!(own.status, 400, "{target}: {}", own.body);
        assert_eq!(own.json()["error"]["code"], "invalid_arguments");
        for h in ["x-weflow-exit-code", "x-weflow-error-code"] {
            assert_eq!(own.headers.get(h), child.headers.get(h), "{target} {h}");
        }
        let timing = &own.headers["server-timing"];
        assert!(
            timing.contains("cmd;desc=\"refused in process\""),
            "{target}: {timing}"
        );
    }
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

    // the files WeChat keeps of it (#194): the thumbnail and the image are here, the HD original is not yet
    let info = format!("/api/chat/image-info?{query}");
    let r = server.call("GET", &info, "");
    assert_eq!(r.status, 200, "{}", r.body);
    let v = r.json()["data"].clone();
    assert_eq!(
        (v["thumb"].clone(), v["normal"].clone(), v["hd"].clone()),
        (json!(true), json!(true), json!(false))
    );
    assert_eq!(
        server.call("GET", &info, "").body,
        server
            .call("GET", &format!("/chat/image-info?{query}"), "")
            .body
    );
    let hd_url = format!("/api/chat/image-data?{query}&variant=hd");
    assert_eq!(server.call("GET", &hd_url, "").status, 404);
    let normal_url = format!("/api/chat/image-data?{query}&variant=normal");
    assert_eq!(server.call("GET", &normal_url, "").bytes, jpeg);
    // once WeChat has downloaded it: `hd` is the original, and `normal` still the image opened
    let mut original = vec![0xFF, 0xD8, 0xFF, 0xE0];
    original.extend((0..6000u32).map(|i| (i as u8).wrapping_mul(13) | 1));
    std::fs::write(
        img_dir.join("aabbccddeeff00112233445566778899_h.dat"),
        original.iter().map(|b| b ^ 0x5a).collect::<Vec<u8>>(),
    )
    .unwrap();
    assert_eq!(server.call("GET", &info, "").json()["data"]["hd"], true);
    let r = server.call("GET", &hd_url, "");
    assert_eq!((r.status, r.bytes == original), (200, true));
    assert_eq!(server.call("GET", &normal_url, "").bytes, jpeg);
    assert_eq!(
        server
            .call("GET", &format!("{normal_url}&thumb=true"), "")
            .status,
        400
    );
    std::fs::remove_file(img_dir.join("aabbccddeeff00112233445566778899_h.dat")).unwrap();

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
        let name = weflow_native::native_db::hex(&Md5::digest(url.as_bytes())) + ext;
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

#[test]
fn wechat_pay_records_come_in_pages_that_say_whether_more_follow() {
    use weflow_native::fixture::{MsgSpec, T0};
    let root = temp_root("pay");
    let notice = |title: &str, des: &str, shop: &str| {
        format!(
            "<msg><appmsg><title>{title}</title><des>{des}</des><mmreader><template_header>\
             <display_name>{shop}</display_name><pub_time>{T0}</pub_time></template_header></mmreader></appmsg></msg>"
        )
    };
    const PAY: &str = "gh_3dfda90e39d6";
    let home = account_home_with(&root, |f| {
        f.message_shard(
            0,
            &[(
                PAY,
                vec![
                    MsgSpec::text(
                        1,
                        PAY,
                        T0,
                        &notice("微信支付凭证", "付款金额¥28.00\n支付方式零钱", "Cafe"),
                    )
                    .of_type(49),
                    // not a notice: left out, but still read
                    MsgSpec::text(
                        2,
                        PAY,
                        T0 + 60,
                        "<msg><appmsg><type>5</type></appmsg></msg>",
                    )
                    .of_type(49),
                    MsgSpec::text(3, PAY, T0 + 120, &notice("退款到账", "退款金额¥5.00", ""))
                        .of_type(49),
                ],
            )],
        );
    });
    let server = start(&home, &["--api-token", TOKEN]);

    let page = |offset: i32| {
        let r = server.call(
            "GET",
            &format!("/biz/pay-records?limit=2&offset={offset}"),
            "",
        );
        assert_eq!(r.status, 200, "{}", r.body);
        r.json()["data"].clone()
    };
    // newest first: the refund, then the message that is no notice
    let first = page(0);
    assert_eq!(first["hasMore"], true);
    let records = first["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["title"], "退款到账");
    // no merchant named: WeChat Pay itself, in the server's language
    assert!(!records[0]["merchantName"].as_str().unwrap().is_empty());
    let second = page(2);
    assert_eq!(second["hasMore"], false);
    let records = second["records"].as_array().unwrap();
    assert_eq!(records[0]["merchantName"], "Cafe");
    assert_eq!(records[0]["description"], "付款金额¥28.00\n支付方式零钱");
    assert_eq!(records[0]["timestamp"], T0);
    // all of them at once
    let all = server.call("GET", "/biz/pay-records?limit=0", "").json();
    assert_eq!(all["data"]["records"].as_array().unwrap().len(), 2);
    assert_eq!(all["data"]["hasMore"], false);
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

#[test]
fn jobs_keep_the_file_they_wrote_for_the_browser() {
    let root = temp_root("job-files");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");
    let async_call = |target: &str| {
        let r = server.request(
            "POST",
            target,
            &[("Authorization", &auth), ("Prefer", "respond-async")],
            "",
        );
        assert_eq!(r.status, 202, "{target}: {}", r.body);
        r.json()["data"]["id"].as_str().unwrap().to_string()
    };
    let finished = |id: &str| {
        let events = sse_events(&server.call("GET", &format!("/jobs/{id}/events"), "").body);
        events.last().unwrap().1.clone()
    };

    // the server picks the folder, as for a direct request, and keeps the file once the job has succeeded
    let id = async_call("/export/sessions?return=file");
    let done = finished(&id);
    assert_eq!(done["status"], "succeeded", "{done}");
    assert_eq!(done["file"]["name"], "sessions.json");
    let path = PathBuf::from(done["result"]["data"]["out"].as_str().unwrap());
    assert!(
        path.starts_with(home.join("cache").join("serve-files")),
        "{path:?}"
    );
    let listed = server.call("GET", "/jobs", "").json();
    assert_eq!(
        listed["data"][0]["file"]["size"],
        std::fs::metadata(&path).unwrap().len()
    );

    let r = server.call("GET", &format!("/jobs/{id}/file"), "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.headers["content-type"].starts_with("application/json"));
    assert!(r.headers["content-disposition"].starts_with("attachment; filename=\"sessions.json\""));
    assert_eq!(r.bytes, std::fs::read(&path).unwrap());
    assert_eq!(
        server.call("POST", &format!("/jobs/{id}/file"), "").status,
        405
    );

    // forgetting the job removes the folder the server chose
    assert_eq!(
        server.call("DELETE", &format!("/jobs/{id}"), "").status,
        200
    );
    assert!(!path.parent().unwrap().exists());

    // several formats packed with --zip (#342): the job's file is the zip, the formats' files inside it
    let id = async_call(
        "/export/messages?return=file&session-id=wxid_bob&format=html&format=txt&zip=true",
    );
    let done = finished(&id);
    assert_eq!(done["status"], "succeeded", "{done}");
    assert_eq!(done["file"]["name"], "wxid_bob.zip", "{done}");
    let r = server.call("GET", &format!("/jobs/{id}/file"), "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(
        r.headers["content-type"].starts_with("application/zip"),
        "{:?}",
        r.headers
    );
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(r.bytes.clone())).unwrap();
    let mut names: Vec<String> = archive.file_names().map(str::to_string).collect();
    names.sort();
    assert_eq!(names, ["wxid_bob.html", "wxid_bob.txt"]);
    let mut text = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("wxid_bob.txt").unwrap(), &mut text)
        .unwrap();
    assert!(text.contains("see you"), "{text}");
    let zipped = PathBuf::from(done["result"]["data"]["out"].as_str().unwrap());
    assert!(
        !zipped.with_file_name(".wxid_bob.zip.parts").exists(),
        "the folder the zip was made from is taken away"
    );
    assert_eq!(done["result"]["data"]["files"][1]["out"], "wxid_bob.txt");
    assert_eq!(
        server.call("DELETE", &format!("/jobs/{id}"), "").status,
        200
    );

    // a job without return=file, or one that failed, has no file
    let plain = async_call("/export/sessions");
    assert_eq!(finished(&plain)["file"], Value::Null);
    assert_eq!(
        server
            .call("GET", &format!("/jobs/{plain}/file"), "")
            .status,
        404
    );
    let failed = async_call("/export/messages?return=file&session-id=x&format=nope");
    assert_eq!(finished(&failed)["status"], "failed");
    assert_eq!(
        server
            .call("GET", &format!("/jobs/{failed}/file"), "")
            .status,
        404
    );
    // its folder is gone
    let left: Vec<_> = std::fs::read_dir(home.join("cache").join("serve-files"))
        .map(|d| d.flatten().collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
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

/// `export footprint` (#315): JSON as it is, CSV and Excel as the timeline; the formats for messages refused before
/// anything is read.
#[test]
fn the_footprint_is_exported_as_json_or_as_a_timeline() {
    let root = temp_root("footprint");
    let home = account_home(&root);
    let export = |format: &str| {
        let out = root.join("out").join(format);
        let (code, stdout, stderr) = run_cli(
            &home,
            &[
                "--json",
                "export",
                "footprint",
                "--format",
                format,
                "--out",
                out.to_str().unwrap(),
            ],
        );
        (
            code,
            serde_json::from_str::<Value>(&stdout).unwrap_or(Value::Null),
            stderr,
        )
    };

    // CSV: a row for each of the four segments with Bob, by the name shown for him (his remark)
    let (code, v, stderr) = export("csv");
    assert_eq!(code, 0, "{stderr}");
    let csv = std::fs::read_to_string(v["data"]["out"].as_str().unwrap()).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines[0],
        "Time,Kind,Conversation,Sender,Received,Sent,Replied,Content,Conversation ID"
    );
    assert_eq!(lines.len(), 5, "{csv}");
    assert!(
        lines[1..]
            .iter()
            .all(|l| l.contains(",private chat,Bobby,,") && l.ends_with(",wxid_bob")),
        "{csv}"
    );
    assert!(lines[1].contains(",1,1,yes,"), "{csv}");

    // Excel: the same table in a workbook
    let (code, v, stderr) = export("excel");
    assert_eq!(code, 0, "{stderr}");
    let xlsx = std::fs::read(v["data"]["out"].as_str().unwrap()).unwrap();
    assert!(xlsx.starts_with(b"PK"), "an xlsx is a zip");

    // JSON as before
    let (code, v, _) = export("json");
    assert_eq!(code, 0);
    let json: Value =
        serde_json::from_str(&std::fs::read_to_string(v["data"]["out"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(json["private_segments"].as_array().map(Vec::len), Some(4));

    // a format for messages: refused, nothing written
    let (code, v, _) = export("html");
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "invalid_arguments");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("json, csv or excel"),
        "{v}"
    );
    assert!(!root.join("out").join("html").exists());
}

// The regression matrix of the refactoring in #391 (#403): behaviour the shared business entry point (#397) and the
// job and HTTP layers (#398) must keep; the scenarios and the baseline are in #403.

/// A failed read answers the same through `/api/` as through the child process: status, body and error headers, for
/// the ways a request goes wrong before, while and after its command runs.
#[test]
fn reads_fail_the_same_in_process_and_in_a_child() {
    let root = temp_root("errors");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let same = |method: &str, target: &str, headers: &[(&str, &str)]| {
        let child = server.request(method, target, headers, "");
        let own = server.request(method, &format!("/api{target}"), headers, "");
        assert_eq!(
            (own.status, &own.body),
            (child.status, &child.body),
            "{method} {target}"
        );
        for h in ["x-weflow-exit-code", "x-weflow-error-code", "allow"] {
            assert_eq!(
                own.headers.get(h),
                child.headers.get(h),
                "{method} {target} {h}"
            );
        }
        own
    };
    let auth = format!("Bearer {TOKEN}");
    let with_token: &[(&str, &str)] = &[("Authorization", &auth)];
    for target in [
        // the command line is refused: a required argument missing, a value of the wrong type, arguments that
        // exclude each other
        "/chat/messages",
        "/chat/messages?session-id=wxid_bob&limit=many",
        "/chat/messages?session-id=wxid_bob&offset=1&before=1",
        // the server refuses a name the command does not have, and a switch that is neither true nor false
        "/chat/messages?session-id=wxid_bob&bogus=1",
        "/chat/sessions?parsed=maybe",
        // the command itself refuses
        "/sns/timeline?limit=x",
        "/chat/resources?before=1.2.3",
    ] {
        let r = same("GET", target, with_token);
        assert_eq!(r.status, 400, "{target}: {}", r.body);
        assert_eq!(r.json()["success"], false, "{target}");
        assert_eq!(r.json()["error"]["code"], "invalid_arguments", "{target}");
    }
    // nothing found is a success with nothing in it
    for target in [
        "/chat/messages?session-id=wxid_nobody",
        "/chat/search?keyword=qzxwvk",
        "/chat/date-counts?session-id=wxid_nobody",
    ] {
        let r = same("GET", target, with_token);
        assert_eq!(r.status, 200, "{target}: {}", r.body);
    }
    // the method, and the token: refused before any command runs
    let r = same("POST", "/chat/sessions", with_token);
    assert_eq!((r.status, r.headers["allow"].as_str()), (405, "GET"));
    for headers in [&[][..], &[("Authorization", "Bearer wrong")][..]] {
        let r = same("GET", "/chat/sessions", headers);
        assert_eq!(r.status, 401);
        assert_eq!(r.json()["error"]["code"], "unauthorized");
        let r = server.request(
            "GET",
            "/api/chat/image-data?session-id=wxid_bob&msg-id=1",
            headers,
            "",
        );
        assert_eq!(r.status, 401);
    }
    // what `/api/` does not take
    for target in [
        "/api/chat/sessions?std=in",
        "/api/chat/sessions?return=file",
    ] {
        assert_eq!(server.call("GET", target, "").status, 400, "{target}");
    }
    let r = server.request(
        "GET",
        "/api/chat/sessions",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    assert_eq!(r.status, 400);
    let r = server.call("GET", "/api/config/path", "");
    assert_eq!(r.status, 404);
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("/config/path"));
}

/// Another profile, another account: the reads of both routes answer from the account the config names now, right
/// after it changed, also when the names of the profiles are as long as each other (the hub tells a changed config by
/// the time and size of its file).
#[test]
fn reads_follow_the_profile_right_after_it_changes() {
    use weflow_native::fixture::{MsgSpec, SessionSpec, T0};
    let root = temp_root("profiles");
    let (home, first) = account(&root);
    // the second account: Carol only
    let other = Fixture::standard(&root.join("data-b"));
    other.session_db(&[SessionSpec {
        username: "wxid_carol",
        summary: "from b",
        last_timestamp: T0,
        unread: 0,
        last_msg_type: 1,
    }]);
    other.message_shard(
        0,
        &[(
            "wxid_carol",
            vec![MsgSpec::text(1, "wxid_carol", T0, "only in b")],
        )],
    );
    let profile = |name: &str, account: &Fixture| {
        for (key, value) in [
            ("current_profile", name.to_string()),
            ("db_path", account.account_dir.to_string_lossy().to_string()),
            ("wxid", "wxid_me_ab12".to_string()),
            ("decrypt_key", account.key_hex()),
        ] {
            let (exit, _, stderr) = run_cli(&home, &["config", "set", key, &value]);
            assert_eq!(exit, 0, "{stderr}");
        }
    };
    profile("pb", &other);
    // the first account again, under a name as long as the second's
    profile("pa", &first);
    let server = start(&home, &["--api-token", TOKEN]);
    let sessions = |route: &str| -> Vec<String> {
        let r = server.call("GET", &format!("{route}/chat/sessions"), "");
        assert_eq!(r.status, 200, "{}", r.body);
        r.json()["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["username"].as_str().unwrap().to_string())
            .collect()
    };
    let start_list = sessions("/api");
    assert!(start_list.iter().any(|s| s == "wxid_bob"), "{start_list:?}");
    assert_eq!(sessions(""), start_list);
    for round in 0..4 {
        let (name, in_b) = if round % 2 == 0 {
            ("pb", true)
        } else {
            ("pa", false)
        };
        let r = server.call(
            "PUT",
            "/config/set",
            &json!({ "key": "current_profile", "value": name }).to_string(),
        );
        assert_eq!(r.status, 200, "{}", r.body);
        // at once, without waiting for anything
        let own = sessions("/api");
        assert_eq!(
            own.iter().any(|s| s == "wxid_carol"),
            in_b,
            "round {round} ({name}): {own:?}"
        );
        assert_eq!(own.iter().any(|s| s == "wxid_bob"), !in_b, "round {round}");
        assert_eq!(sessions(""), own, "round {round}");
        let r = server.call("GET", "/api/chat/messages?session-id=wxid_carol", "");
        let found = r.json()["data"].as_array().map_or(0, Vec::len);
        assert_eq!(found > 0, in_b, "round {round}: {}", r.body);
    }

    // another config file chosen (#413): its profile at once, and the config reads say so
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("config.json")).unwrap()).unwrap();
    let mut moved = config.clone();
    moved["current_profile"] = json!("pb");
    let other = root.join("other").join("config.json");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    std::fs::write(&other, moved.to_string()).unwrap();
    let path = other.to_string_lossy().to_string();
    let r = server.call(
        "PUT",
        "/config/set",
        &json!({ "key": "config_path", "value": path }).to_string(),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let own = sessions("/api");
    assert!(own.iter().any(|s| s == "wxid_carol"), "{own:?}");
    assert_eq!(sessions(""), own);
    for route in ["/api", ""] {
        let r = server.call("GET", &format!("{route}/config/get?key=config_path"), "");
        assert_eq!(r.json()["data"], path.as_str(), "{route}");
    }
    let r = server.call(
        "DELETE",
        "/config/unset",
        &json!({ "key": "config_path" }).to_string(),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let own = sessions("/api");
    assert!(own.iter().any(|s| s == "wxid_bob"), "{own:?}");
    assert_eq!(sessions(""), own);
}

/// Cancelling a job twice: the first time cancels it, the second forgets it; neither changes how it ended.
#[test]
fn a_job_is_cancelled_once_and_then_forgotten() {
    use std::time::Duration;
    let root = temp_root("cancel-twice");
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
    let r = server.request(
        "POST",
        "/insight/test",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    assert_eq!(r.status, 202, "{}", r.body);
    let id = r.json()["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(ai.recv_timeout(Duration::from_secs(30)), Ok("open"));

    let r = server.call("DELETE", &format!("/jobs/{id}"), "");
    assert_eq!(r.status, 200);
    let cancelled = r.json()["data"].clone();
    assert_eq!(cancelled["status"], "cancelled", "{cancelled}");
    assert_eq!(ai.recv_timeout(Duration::from_secs(5)), Ok("closed"));
    // its events end with the result, once
    let events = sse_events(&server.call("GET", &format!("/jobs/{id}/events"), "").body);
    let results: Vec<&Value> = events
        .iter()
        .filter(|(n, _)| n == "result")
        .map(|(_, d)| d)
        .collect();
    assert_eq!(results.len(), 1, "{events:?}");
    assert_eq!(results[0]["status"], "cancelled");
    assert_eq!(events.last().unwrap().0, "result");

    let r = server.call("DELETE", &format!("/jobs/{id}"), "");
    assert_eq!(r.status, 200);
    let forgotten = r.json()["data"].clone();
    assert_eq!(
        (
            &forgotten["status"],
            &forgotten["finishedAt"],
            &forgotten["result"]
        ),
        (
            &cancelled["status"],
            &cancelled["finishedAt"],
            &cancelled["result"]
        )
    );
    assert_eq!(server.call("GET", &format!("/jobs/{id}"), "").status, 404);
    assert_eq!(
        server.call("DELETE", &format!("/jobs/{id}"), "").status,
        404
    );
}

/// The contract the WebUI is built against (#399).
fn contract() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../web/contract/openapi.json"
    ))
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// The errors of `value` against `schema`, whose `$ref`s point into the contract's components, as `path: error`.
fn misfits(contract: &Value, schema: &Value, value: &Value) -> Vec<String> {
    let mut root = schema.clone();
    root["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    root["components"] = contract["components"].clone();
    let validator = jsonschema::validator_for(&root).unwrap();
    validator
        .iter_errors(value)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect()
}

/// The answers of the routes the contract describes fit it as the server sends them, from its own process and from a
/// child, over an account whose messages lie in three databases with keys that tie; pages walked from their cursors
/// list each message once; failures fit `Failure` and jobs `Job` (#400).
#[test]
fn answers_fit_the_contract() {
    use weflow_native::fixture::{MsgSpec, DAY, T0};
    let root = temp_root("contract");
    let home = account_home_with(&root, |f| {
        f.message_shard(
            2,
            &[(
                "wxid_bob",
                vec![
                    // the very time of the second database's message 5
                    MsgSpec::text(6, "wxid_bob", T0 + 4 * DAY, "same time"),
                    MsgSpec::text(
                        7,
                        "wxid_me",
                        T0 + 5 * DAY,
                        "<msg><appmsg><type>6</type><title>plan.pdf</title><appattach><totallen>2048</totallen>\
                         <fileext>pdf</fileext></appattach></appmsg></msg>",
                    )
                    .of_type(49),
                    // a row that does not say who sent it
                    MsgSpec::text(8, "", T0 + 5 * DAY + 1, "\"Bob\" recalled a message")
                        .of_type(10000),
                ],
            )],
        );
    });
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    let component = |name: &str| json!({ "$ref": format!("#/components/schemas/{name}") });
    let list_of = |name: &str| json!({ "type": "array", "items": component(name) });
    let answer_schema = |route: &str, status: &str| {
        let schema = contract["paths"][route]["get"]["responses"][status]["content"]
            ["application/json"]["schema"]
            .clone();
        assert!(
            schema.is_object(),
            "{route} {status} is not in the contract"
        );
        schema
    };
    let check = |what: &str, schema: &Value, value: &Value| {
        let errors = misfits(&contract, schema, value);
        assert!(errors.is_empty(), "{what}: {errors:#?}\n{value}");
    };
    // the `data` of `target`, in the process and from a child, both fitting the route's answer and, where the route
    // answers one of several shapes, `shape`: a parsed message fits `Row` too
    let read = |target: &str, shape: Option<Value>| -> Value {
        let route = target.split('?').next().unwrap();
        let ok = answer_schema(route, "200");
        let own = server.call("GET", target, "");
        let child = server.call("GET", target.strip_prefix("/api").unwrap(), "");
        for r in [&own, &child] {
            assert_eq!(r.status, 200, "{target}: {}", r.body);
            check(target, &ok, &r.json());
            if let Some(shape) = &shape {
                check(target, shape, &r.json()["data"]);
            }
        }
        own.json()["data"].clone()
    };

    let sessions = read("/api/chat/sessions", None);
    assert!(sessions.as_array().unwrap().len() >= 3, "{sessions}");
    // the reads typed since (#430), each answering the shape its route says
    for target in [
        "/api/chat/statuses?usernames=wxid_bob&usernames=room1%40chatroom",
        "/api/chat/counts?sessions=wxid_bob&sessions=room1%40chatroom",
        "/api/chat/tab-counts",
        "/api/chat/dates?session-id=wxid_bob",
        "/api/chat/date-counts?session-id=wxid_bob",
        "/api/chat/detail?session-id=wxid_bob&fast=true",
        "/api/chat/detail?session-id=wxid_bob",
        "/api/chat/detail?session-id=wxid_bob&extra=true",
        &format!(
            "/api/chat/image-info?session-id=wxid_bob&msg-id=3&create-time={}",
            T0 + DAY
        ),
    ] {
        let data = read(target, None);
        assert!(
            data.as_object().is_some_and(|o| !o.is_empty()),
            "{target}: {data}"
        );
    }
    for target in [
        "/api/chat/messages?session-id=wxid_bob&limit=100",
        "/api/chat/messages?session-id=room1%40chatroom",
        "/api/chat/latest?session-id=wxid_bob&limit=2",
        "/api/chat/search?keyword=hello",
        "/api/chat/contacts",
    ] {
        let rows = read(target, Some(list_of("Row")));
        assert!(!rows.as_array().unwrap().is_empty(), "{target}");
    }
    for target in [
        "/api/chat/messages?session-id=room1%40chatroom&parsed=true",
        "/api/chat/search?keyword=hello&parsed=true",
    ] {
        let messages = read(target, Some(list_of("ParsedMessage")));
        assert!(!messages.as_array().unwrap().is_empty(), "{target}");
        // a key names the database, not where it is on this machine (#443)
        for m in messages.as_array().unwrap() {
            let key = m["messageKey"].as_str().unwrap();
            assert!(
                key.contains("message_") && !key.contains("%2F") && !key.contains("%5C"),
                "{key}"
            );
        }
    }
    let contacts = read("/api/chat/contacts?parsed=true", Some(list_of("Contact")));
    assert!(contacts.as_array().unwrap().len() >= 5, "{contacts}");
    read(
        "/api/chat/message?session-id=wxid_bob&local-id=2",
        Some(component("MessageLookup")),
    );
    let message = read(
        "/api/chat/message?session-id=wxid_bob&local-id=8&parsed=true",
        Some(component("ParsedMessage")),
    );
    assert_eq!(message["senderUsername"], Value::Null, "{message}");

    // pages of 2, older and older from the cursor of each page's last message, list every message once and in the
    // order of the whole list; so do pages of newer ones from the first message of each
    let parsed = Some(list_of("ParsedMessage"));
    let page = |from: &str| {
        read(
            &format!("/api/chat/messages?session-id=wxid_bob&parsed=true&limit=2{from}"),
            parsed.clone(),
        )
    };
    let cursors = |page: &Value| -> Vec<String> {
        page.as_array()
            .unwrap()
            .iter()
            .map(|m| m["_cursor"].as_str().unwrap().to_string())
            .collect()
    };
    let all = cursors(&read(
        "/api/chat/messages?session-id=wxid_bob&parsed=true&limit=100",
        parsed.clone(),
    ));
    assert_eq!(all.len(), 8, "{all:?}");
    let mut seen = std::collections::HashSet::new();
    assert!(
        all.iter().all(|c| seen.insert(c)),
        "a cursor twice: {all:?}"
    );
    let mut walked = cursors(&page(""));
    loop {
        let next = cursors(&page(&format!(
            "&before={}",
            percent(walked.last().unwrap())
        )));
        if next.is_empty() {
            break;
        }
        walked.extend(next);
    }
    assert_eq!(walked, all);
    let mut forward = vec![all.last().unwrap().clone()];
    loop {
        let next = cursors(&page(&format!("&after={}", percent(&forward[0]))));
        if next.is_empty() {
            break;
        }
        forward.splice(0..0, next);
    }
    assert_eq!(forward, all);

    // the resources of every conversation, a page of one at a time, until a page says no more follow
    let resources = Some(component("ResourcePage"));
    let whole = read("/api/chat/resources?limit=100", resources.clone());
    let keys = |page: &Value| -> Vec<String> {
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["_cursor"].as_str().unwrap().to_string())
            .collect()
    };
    assert!(keys(&whole).len() >= 2, "{whole}");
    assert_eq!(whole["nextCursor"], Value::Null);
    let mut listed = Vec::new();
    let mut before = String::new();
    loop {
        let page = read(
            &format!("/api/chat/resources?limit=1{before}"),
            resources.clone(),
        );
        listed.extend(keys(&page));
        if page["hasMore"] == false {
            assert_eq!(page["nextCursor"], Value::Null);
            break;
        }
        assert_eq!(
            page["nextCursor"].as_str(),
            keys(&page).last().map(String::as_str)
        );
        before = format!("&before={}", percent(page["nextCursor"].as_str().unwrap()));
    }
    assert_eq!(listed, keys(&whole));

    // failures: a missing argument, a cursor of nothing, an image not on this computer (whose route without `/api/`
    // writes a file, by POST)
    let failure = answer_schema("/api/chat/messages", "default");
    let image = format!(
        "/api/chat/image-data?session-id=wxid_bob&msg-id=3&create-time={}",
        T0 + DAY
    );
    for (target, status) in [
        ("/api/chat/messages", 400),
        ("/api/chat/messages?session-id=wxid_bob&before=1.2.3", 400),
        ("/chat/messages", 400),
        ("/chat/messages?session-id=wxid_bob&before=1.2.3", 400),
        (image.as_str(), 404),
    ] {
        let r = server.call("GET", target, "");
        assert_eq!(r.status, status, "{target}: {}", r.body);
        check(target, &failure, &r.json());
    }
    let r = server.call("GET", &image, "");
    assert_eq!(r.json()["error"]["details"]["failureKind"], "not_found");

    // a job, while it runs and once it has ended, and the list of jobs
    let auth = format!("Bearer {TOKEN}");
    let r = server.request(
        "GET",
        "/chat/sessions",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    assert_eq!(r.status, 202, "{}", r.body);
    let job_answer = answer_schema("/jobs/{id}", "200");
    check("the job started", &job_answer, &r.json());
    let id = r.json()["data"]["id"].as_str().unwrap().to_string();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let r = server.call("GET", &format!("/jobs/{id}"), "");
        check("the job", &job_answer, &r.json());
        if r.json()["data"]["status"] == "succeeded" {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{}", r.body);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let jobs = server.call("GET", "/jobs", "").json();
    check("the jobs", &list_of("Job"), &jobs["data"]);
}
