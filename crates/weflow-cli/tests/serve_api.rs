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
    let home = account_home(&root);
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
        "/chat/contact?username=wxid_bob",
        "/chat/contacts",
        "/chat/counts?sessions=wxid_bob&sessions=room1%40chatroom",
        "/chat/search?keyword=hello",
    ] {
        let r = same(target);
        assert_eq!(r.status, 200, "{target}: {}", r.body);
    }
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

    // a part of it, as a player or a resumed download asks: 206, or 416 past the end, or the whole file when it
    // changed since (`If-Range`)
    let part = |range: &str, if_range: &str| {
        let mut headers = vec![("Authorization", auth.as_str()), ("Range", range)];
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

    // the voice, decoded to WAV: the same as the child process prints in base64
    let query = format!(
        "session-id=wxid_bob&msg-id=6&create-time={voice_at}&server-id=9000000000001&sender=wxid_bob"
    );
    let r = server.call("GET", &format!("/api/chat/voice-data?{query}"), "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.headers["content-type"], "audio/wav");
    assert_eq!(&r.bytes[..4], b"RIFF");
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
struct EventStream {
    stream: TcpStream,
    text: String,
}

impl EventStream {
    fn open(server: &Server, headers: &[(&str, &str)]) -> Self {
        let mut stream = TcpStream::connect(&server.addr).unwrap();
        let mut req = format!(
            "GET /api/push/messages HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {TOKEN}\r\n",
            server.addr
        );
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
fn new_messages_are_pushed_as_events() {
    use weflow_native::fixture::{MsgSpec, SessionSpec, DAY, T0};
    let root = temp_root("push");
    let (home, fixture) = account(&root);
    let server = start(&home, &["--api-token", TOKEN]);

    let mut first = EventStream::open(&server, &[]);
    assert!(first.wait_for("event: ready", 10), "{}", first.text);
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
