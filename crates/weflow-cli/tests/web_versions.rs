//! `weflow web` and the WebUI of `weflow serve` (issue #148), with a registry of the test's own
//! (`WEFLOW_WEB_REGISTRY`): the default version of this weflow downloaded beside the server, a restart without the
//! network, the API alone, and versions installed, picked and pruned.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const TOKEN: &str = "t0k-for-tests";
const REPO: &str = "334456777/weflow-web";
/// The default version of this weflow: web/package.json's.
fn built_in() -> String {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../web/package.json"
    ))
    .unwrap();
    serde_json::from_str::<Value>(&text).unwrap()["version"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A package of a version: the page and its metadata (for any weflow at all, as a CI build may lack its tag).
fn package(version: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let meta = json!({ "name": "weflow-web", "version": version, "apiVersion": "2", "minBackendVersion": "0.0.1" });
    for (name, content) in [
        (
            "index.html",
            format!("<!doctype html><title>WeFlow {version}</title>"),
        ),
        ("weflow-web.json", meta.to_string()),
    ] {
        zip.start_file(name, opts).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// A registry answering tags, manifests and blobs of `versions`, without a token; its address.
fn registry(versions: &[&str]) -> String {
    let packages: HashMap<String, Vec<u8>> = versions
        .iter()
        .map(|v| (v.to_string(), package(v)))
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let packages = packages.clone();
            std::thread::spawn(move || answer(stream, &packages));
        }
    });
    format!("http://{addr}")
}

fn answer(mut stream: TcpStream, packages: &HashMap<String, Vec<u8>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let path = line.split(' ').nth(1).unwrap_or("").to_string();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
            break;
        }
    }
    let sha = |b: &[u8]| -> String {
        Sha256::digest(b)
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect()
    };
    let base = format!("/v2/{REPO}/");
    let rest = path.strip_prefix(&base).unwrap_or("");
    let (status, body): (&str, Vec<u8>) = if rest.starts_with("tags/list") {
        (
            "200 OK",
            json!({ "tags": packages.keys().collect::<Vec<_>>() })
                .to_string()
                .into_bytes(),
        )
    } else if let Some(version) = rest.strip_prefix("manifests/") {
        match packages.get(version) {
            Some(p) => ("200 OK", json!({
                "schemaVersion": 2,
                "artifactType": "application/vnd.weflow.web.v1",
                "layers": [{ "mediaType": "application/vnd.weflow.web.layer.v1+zip", "digest": format!("sha256:{}", sha(p)), "size": p.len() }],
                "annotations": { "io.weflow.web.api-version": "2", "io.weflow.web.min-backend-version": "0.0.1" },
            }).to_string().into_bytes()),
            None => ("404 Not Found", Vec::new()),
        }
    } else if let Some(digest) = rest.strip_prefix("blobs/sha256:") {
        match packages.values().find(|p| sha(p) == digest) {
            Some(p) => ("200 OK", p.clone()),
            None => ("404 Not Found", Vec::new()),
        }
    } else {
        ("404 Not Found", Vec::new())
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
}

fn temp_root(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("web-versions-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn weflow(home: &Path, registry: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_weflow"));
    cmd.env("WEFLOW_HOME", home)
        .env("WEFLOW_LANG", "en")
        .env("WEFLOW_WEB_REGISTRY", registry)
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env_remove("WEFLOW_HTTP_TOKEN")
        .stdin(Stdio::null());
    cmd
}

/// `weflow --json <args>`: the document it prints.
fn cli(home: &Path, registry: &str, args: &[&str]) -> Value {
    let out = weflow(home, registry)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", String::from_utf8_lossy(&out.stderr)));
    assert_eq!(doc["success"], true, "{args:?}: {doc}");
    doc["data"].clone()
}

/// `weflow --json <args>` that fails: its error.
fn cli_error(home: &Path, registry: &str, args: &[&str]) -> Value {
    let out = weflow(home, registry)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(!out.status.success(), "{args:?} succeeded");
    let doc: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", String::from_utf8_lossy(&out.stderr)));
    assert_eq!(doc["success"], false, "{args:?}: {doc}");
    doc["error"].clone()
}

/// A version kept by another weflow, written straight into the store.
fn keep(home: &Path, version: &str, api_version: &str, min_backend_version: &str) {
    let dir = home.join("cache").join("web").join(version);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("index.html"),
        format!("<!doctype html><title>WeFlow {version}</title>"),
    )
    .unwrap();
    let meta = json!({ "name": "weflow-web", "version": version, "apiVersion": api_version, "minBackendVersion": min_backend_version });
    std::fs::write(dir.join("weflow-web.json"), meta.to_string()).unwrap();
}

/// A running `weflow serve` and the events it prints on stderr.
struct Server {
    child: Child,
    events: mpsc::Receiver<Value>,
    started: Value,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn start(home: &Path, registry: &str, extra: &[&str]) -> Self {
        Self::start_with(home, registry, TOKEN, extra)
    }

    fn start_with(home: &Path, registry: &str, token: &str, extra: &[&str]) -> Self {
        let mut child = weflow(home, registry)
            .args(["--json", "serve", "--port", "0", "--api-token", token])
            .args(extra)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let lines = BufReader::new(child.stderr.take().unwrap()).lines();
        let (tx, events) = mpsc::channel();
        std::thread::spawn(move || {
            for line in lines.map_while(Result::ok) {
                if let Ok(event) = serde_json::from_str::<Value>(&line) {
                    let _ = tx.send(event);
                }
            }
        });
        let started = Self::wait_in(&events, "server_started");
        Self {
            child,
            events,
            started,
        }
    }

    fn wait_in(events: &mpsc::Receiver<Value>, kind: &str) -> Value {
        loop {
            let event = events
                .recv_timeout(Duration::from_secs(60))
                .unwrap_or_else(|_| panic!("no {kind} event"));
            if event["type"] == kind {
                return event;
            }
        }
    }

    fn wait(&self, kind: &str) -> Value {
        Self::wait_in(&self.events, kind)
    }

    /// `GET target`: (status, body).
    fn get(&self, target: &str) -> (u16, String) {
        let addr = self.started["url"]
            .as_str()
            .unwrap()
            .trim_start_matches("http://");
        let mut stream = TcpStream::connect(addr).unwrap();
        write!(
            stream,
            "GET {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        let status = raw.split(' ').nth(1).unwrap().parse().unwrap();
        (
            status,
            raw.split_once("\r\n\r\n")
                .map(|(_, b)| b.to_string())
                .unwrap_or_default(),
        )
    }
}

#[test]
fn serve_downloads_the_webui_it_comes_with_and_starts_with_it_offline() {
    let home = temp_root("download");
    let version = built_in();
    let reg = registry(&[&version]);
    let server = Server::start(&home, &reg, &[]);
    assert_eq!(server.started["ui"], false);
    assert_eq!(server.started["uiDownloading"], version.as_str());
    // the API answers while the page downloads
    assert_eq!(server.get("/health").0, 200);
    let ready = server.wait("webui_ready");
    assert_eq!(ready["version"], version.as_str());
    assert!(ready["uiUrl"]
        .as_str()
        .unwrap()
        .ends_with(&format!("/ui/?token={TOKEN}")));
    let (status, body) = server.get("/ui/");
    assert_eq!(status, 200);
    assert!(body.contains(&format!("WeFlow {version}")));
    drop(server);

    // without the registry, the version kept is shown at once
    let server = Server::start(&home, "http://127.0.0.1:9", &[]);
    assert_eq!(server.started["ui"], true);
    assert_eq!(server.started["uiVersion"], version.as_str());
    assert!(server.started["uiDownloading"].is_null());
    assert_eq!(server.get("/ui/").0, 200);
    drop(server);

    // and none with --api-only
    let server = Server::start(&home, &reg, &["--api-only"]);
    assert_eq!(server.started["ui"], false);
    assert!(server.started["uiDownloading"].is_null());
    assert_eq!(server.get("/ui/").0, 404);
}

#[test]
fn without_the_registry_the_api_runs_alone() {
    let home = temp_root("offline");
    let server = Server::start(&home, "http://127.0.0.1:9", &[]);
    assert_eq!(server.started["ui"], false);
    let failed = server.wait("webui_unavailable");
    assert_eq!(failed["kept"], false);
    assert!(failed["error"].as_str().unwrap().contains("registry"));
    assert_eq!(server.get("/health").0, 200);
    assert_eq!(server.get("/ui/").0, 404);
}

#[test]
fn versions_are_installed_picked_and_pruned() {
    let home = temp_root("pick");
    let version = built_in();
    let reg = registry(&["0.0.8", "0.0.9", &version]);

    let remote = cli(&home, &reg, &["web", "list", "--remote"]);
    let listed: Vec<&str> = remote
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["version"].as_str().unwrap())
        .collect();
    assert_eq!(listed.len(), 3);
    assert!(remote
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["suits"] == true && v["downloaded"] == false));

    let info = cli(&home, &reg, &["web", "info"]);
    assert_eq!(info["builtIn"], version.as_str());
    assert_eq!(info["wanted"], version.as_str());
    assert_eq!(info["downloaded"], false);

    cli(&home, &reg, &["web", "install", "0.0.8"]);
    let used = cli(&home, &reg, &["web", "use", "0.0.9"]);
    assert_eq!(used["version"], "0.0.9");
    let info = cli(&home, &reg, &["web", "info"]);
    assert_eq!(info["picked"], "0.0.9");
    assert_eq!(info["inUse"], "0.0.9");
    assert_eq!(info["installed"], json!(["0.0.9", "0.0.8"]));

    // serve shows the version picked; --web-version another for one run
    let server = Server::start(&home, &reg, &[]);
    assert_eq!(server.started["uiVersion"], "0.0.9");
    assert!(server.get("/ui/").1.contains("WeFlow 0.0.9"));
    drop(server);
    let server = Server::start(&home, &reg, &["--web-version", "0.0.8"]);
    assert_eq!(server.started["uiVersion"], "0.0.8");
    drop(server);

    // prune keeps the version picked and the default one of this weflow
    cli(&home, &reg, &["web", "install", &version]);
    let pruned = cli(&home, &reg, &["web", "prune"]);
    assert_eq!(pruned["removed"], json!(["0.0.8"]));
    let mut kept: Vec<String> = serde_json::from_value(pruned["kept"].clone()).unwrap();
    kept.sort();
    let mut want = vec!["0.0.9".to_string(), version.clone()];
    want.sort();
    assert_eq!(kept, want);

    // the versions are a part of the cache
    let parts = cli(&home, &reg, &["cache", "list"]);
    let web = parts
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["part"] == "web")
        .unwrap();
    assert!(web["files"].as_u64().unwrap() >= 4);
    cli(&home, &reg, &["cache", "clear", "--web", "-y"]);
    assert_eq!(cli(&home, &reg, &["web", "list"]), json!([]));
}

#[test]
fn the_page_address_carries_the_token_as_it_is() {
    let home = temp_root("token");
    let version = built_in();
    keep(&home, &version, "2", "0.0.1");
    let token = "abc+def/ghi==&x#y z%";
    let server = Server::start_with(&home, "http://127.0.0.1:9", token, &[]);
    let url = server.started["uiUrl"].as_str().unwrap();
    assert!(
        url.ends_with("/ui/?token=abc%2Bdef%2Fghi%3D%3D%26x%23y%20z%25"),
        "{url}"
    );
}

#[test]
fn versions_kept_by_another_weflow_are_checked_again() {
    let home = temp_root("kept");
    let reg = registry(&["0.0.9"]);
    cli(&home, &reg, &["web", "use", "0.0.9"]);
    // kept by a newer weflow: another API, and a weflow newer than this one
    keep(&home, "0.0.6", "3", "0.0.1");
    keep(&home, "0.0.7", "2", "99.0.0");
    for version in ["0.0.6", "0.0.7"] {
        for args in [["web", "install", version], ["web", "use", version]] {
            let error = cli_error(&home, &reg, &args);
            assert_eq!(error["code"], "incompatible", "{args:?}: {error}");
        }
        let failed = weflow(&home, &reg)
            .args(["--json", "serve", "--port", "0", "--web-version", version])
            .output()
            .unwrap();
        assert!(!failed.status.success(), "serve --web-version {version}");
    }
    // the pick stays as it was
    assert_eq!(cli(&home, &reg, &["web", "info"])["picked"], "0.0.9");

    // picked before this weflow: serve shows the newest version that suits it instead, and downloads nothing
    std::fs::write(
        home.join("cache").join("web").join("current"),
        "0.0.7
",
    )
    .unwrap();
    let server = Server::start(&home, &reg, &[]);
    assert_eq!(server.started["uiVersion"], "0.0.9");
    assert!(server.started["uiDownloading"].is_null());
    assert!(server.get("/ui/").1.contains("WeFlow 0.0.9"));
}

#[test]
fn prune_leaves_the_version_a_server_shows() {
    let home = temp_root("shown");
    let version = built_in();
    let reg = registry(&["0.0.8", &version]);
    cli(&home, &reg, &["web", "install", "0.0.8"]);
    cli(&home, &reg, &["web", "install", &version]);

    let server = Server::start(&home, &reg, &["--web-version", "0.0.8"]);
    assert_eq!(server.started["uiVersion"], "0.0.8");
    let pruned = cli(&home, &reg, &["web", "prune"]);
    assert_eq!(pruned["removed"], json!([]));
    assert_eq!(pruned["shown"], json!(["0.0.8"]));
    let (status, body) = server.get("/ui/");
    assert_eq!(status, 200);
    assert!(body.contains("WeFlow 0.0.8"));
    drop(server);

    let pruned = cli(&home, &reg, &["web", "prune"]);
    assert_eq!(pruned["removed"], json!(["0.0.8"]));
    assert_eq!(pruned["shown"], json!([]));

    // the version shown while the one wanted downloads (here: cannot) is left too
    let home = temp_root("meanwhile");
    cli(&home, &reg, &["web", "install", "0.0.8"]);
    let server = Server::start(&home, "http://127.0.0.1:9", &[]);
    assert_eq!(server.started["uiVersion"], "0.0.8");
    assert_eq!(server.started["uiDownloading"], version.as_str());
    assert_eq!(server.wait("webui_unavailable")["kept"], true);
    let pruned = cli(&home, &reg, &["web", "prune"]);
    assert_eq!(pruned["shown"], json!(["0.0.8"]));
    assert_eq!(server.get("/ui/").0, 200);
}
