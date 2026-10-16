//! WebUI versions (issue #148) against a registry on this machine that answers as GHCR does: an anonymous token
//! after a `401`, tags, manifests and blobs.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde_json::json;
use sha2::{Digest, Sha256};
use weflow_core::webui::{self, Backend, Registry, WebStore, ARTIFACT_TYPE, LAYER_TYPE};

const REPO: &str = "334456777/weflow-web";
const BACKEND: Backend = Backend {
    api_version: "1",
    version: "5.0.0-rc.5",
};

/// One version as published: its package, the size and digest its manifest gives, and its annotations.
#[derive(Clone)]
struct Version {
    package: Vec<u8>,
    size: u64,
    digest: String,
    annotations: serde_json::Value,
}

#[derive(Default)]
struct Registry_ {
    versions: Mutex<HashMap<String, Version>>,
    tokens: AtomicUsize,
    blobs: AtomicUsize,
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A package: the page, an asset and the metadata, plus `extra` entries (name, content).
fn package(version: &str, api: &str, min: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let meta = json!({ "name": "weflow-web", "version": version, "apiVersion": api, "minBackendVersion": min });
    for (name, content) in [
        (
            "index.html",
            "<!doctype html><title>WeFlow</title>".to_string(),
        ),
        ("assets/index-a1.js", "console.log(1)".to_string()),
        ("weflow-web.json", meta.to_string()),
    ] {
        zip.start_file(name, opts).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    for (name, content) in extra {
        zip.start_file(*name, opts).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn publish(reg: &Registry_, version: &str, package: Vec<u8>, api: &str, min: &str) {
    let v = Version {
        size: package.len() as u64,
        digest: format!("sha256:{}", sha(&package)),
        annotations: json!({ webui::ANNOTATION_API: api, webui::ANNOTATION_MIN_BACKEND: min }),
        package,
    };
    reg.versions.lock().unwrap().insert(version.to_string(), v);
}

fn authorized(headers: &HeaderMap) -> bool {
    headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer t0k")
}

fn challenge(addr: &str) -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(
            "www-authenticate",
            format!("Bearer realm=\"http://{addr}/token\",service=\"test\",scope=\"repository:{REPO}:pull\""),
        )
        .body(Body::empty())
        .unwrap()
}

async fn serve(reg: Arc<Registry_>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let base = format!("/v2/{REPO}");
    let a1 = addr.clone();
    let a2 = addr.clone();
    let a3 = addr.clone();
    let app = Router::new()
        .route(
            "/token",
            get(|State(reg): State<Arc<Registry_>>| async move {
                reg.tokens.fetch_add(1, Ordering::SeqCst);
                axum::Json(json!({ "token": "t0k" }))
            }),
        )
        .route(
            &format!("{base}/tags/list"),
            get(move |State(reg): State<Arc<Registry_>>, headers: HeaderMap| async move {
                if !authorized(&headers) {
                    return challenge(&a1);
                }
                let mut tags: Vec<String> = reg.versions.lock().unwrap().keys().cloned().collect();
                tags.push("latest".into());
                Response::new(Body::from(json!({ "name": REPO, "tags": tags }).to_string()))
            }),
        )
        .route(
            &format!("{base}/manifests/{{tag}}"),
            get(move |State(reg): State<Arc<Registry_>>, Path(tag): Path<String>, headers: HeaderMap| async move {
                if !authorized(&headers) {
                    return challenge(&a2);
                }
                let Some(v) = reg.versions.lock().unwrap().get(&tag).cloned() else {
                    return Response::builder().status(404).body(Body::empty()).unwrap();
                };
                let manifest = json!({
                    "schemaVersion": 2,
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "artifactType": ARTIFACT_TYPE,
                    "config": { "mediaType": "application/vnd.oci.empty.v1+json", "digest": "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a", "size": 2 },
                    "layers": [{ "mediaType": LAYER_TYPE, "digest": v.digest, "size": v.size }],
                    "annotations": v.annotations,
                });
                Response::new(Body::from(manifest.to_string()))
            }),
        )
        .route(
            &format!("{base}/blobs/{{digest}}"),
            get(move |State(reg): State<Arc<Registry_>>, Path(digest): Path<String>, headers: HeaderMap| async move {
                if !authorized(&headers) {
                    return challenge(&a3);
                }
                reg.blobs.fetch_add(1, Ordering::SeqCst);
                let found = reg.versions.lock().unwrap().values().find(|v| v.digest == digest).cloned();
                match found {
                    Some(v) => Response::new(Body::from(v.package)),
                    None => Response::builder().status(404).body(Body::empty()).unwrap(),
                }
            }),
        )
        .with_state(reg);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn temp_store(tag: &str) -> WebStore {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("webui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    WebStore::new(dir)
}

/// Names in the store's folder (the versions, `current`, and any download left behind).
fn entries(store: &WebStore) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(store.root())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

const QUIET: &(dyn Fn(u64, u64) + Send + Sync) = &|_, _| {};

#[tokio::test]
async fn a_version_is_downloaded_checked_and_kept() {
    let reg = Arc::new(Registry_::default());
    publish(
        &reg,
        "0.1.0",
        package("0.1.0", "1", "5.0.0-rc.5", &[]),
        "1",
        "5.0.0-rc.5",
    );
    publish(
        &reg,
        "0.2.0-rc.1",
        package("0.2.0-rc.1", "1", "5.0.0-rc.5", &[]),
        "1",
        "5.0.0-rc.5",
    );
    let base = serve(reg.clone()).await;
    let registry = Registry::new(&base, REPO).unwrap();
    let store = temp_store("install");

    // tags: versions only, newest first; one anonymous token for all the requests
    assert_eq!(registry.versions().await.unwrap(), ["0.2.0-rc.1", "0.1.0"]);
    let meta = webui::install(&store, &registry, "v0.1.0", BACKEND, QUIET)
        .await
        .unwrap();
    assert_eq!(meta.version, "0.1.0");
    assert_eq!(reg.tokens.load(Ordering::SeqCst), 1);
    assert!(store.dir("0.1.0").join("index.html").is_file());
    assert_eq!(
        std::fs::read_to_string(store.dir("0.1.0").join("assets/index-a1.js")).unwrap(),
        "console.log(1)"
    );
    assert_eq!(store.meta("0.1.0").unwrap(), meta);
    assert_eq!(entries(&store), ["0.1.0"]);

    // kept: no second download
    webui::install(&store, &registry, "0.1.0", BACKEND, QUIET)
        .await
        .unwrap();
    assert_eq!(reg.blobs.load(Ordering::SeqCst), 1);

    // picked, listed, removed
    webui::install(&store, &registry, "0.2.0-rc.1", BACKEND, QUIET)
        .await
        .unwrap();
    store.choose("0.2.0-rc.1").unwrap();
    assert_eq!(store.chosen().as_deref(), Some("0.2.0-rc.1"));
    let listed: Vec<String> = store.installed().into_iter().map(|m| m.version).collect();
    assert_eq!(listed, ["0.2.0-rc.1", "0.1.0"]);
    assert!(store.remove("0.2.0-rc.1").unwrap());
    assert_eq!(store.chosen(), None);
    assert_eq!(entries(&store), ["0.1.0"]);

    // not published
    let err = webui::install(&store, &registry, "9.9.9", BACKEND, QUIET)
        .await
        .unwrap_err();
    assert_eq!(err.code, "not_found");
    let err = webui::install(&store, &registry, "../x", BACKEND, QUIET)
        .await
        .unwrap_err();
    assert_eq!(err.code, "invalid_arguments");
}

#[tokio::test]
async fn damaged_and_incompatible_versions_change_nothing() {
    let reg = Arc::new(Registry_::default());
    publish(
        &reg,
        "0.1.0",
        package("0.1.0", "1", "5.0.0-rc.5", &[]),
        "1",
        "5.0.0-rc.5",
    );
    // the manifest's digest does not match the package
    publish(
        &reg,
        "0.2.0",
        package("0.2.0", "1", "5.0.0-rc.5", &[]),
        "1",
        "5.0.0-rc.5",
    );
    reg.versions
        .lock()
        .unwrap()
        .get_mut("0.2.0")
        .unwrap()
        .digest = format!("sha256:{}", sha(b"something else"));
    // a package cut short of what its manifest says
    let mut cut = package("0.2.1", "1", "5.0.0-rc.5", &[]);
    let full = cut.len() as u64;
    publish(&reg, "0.2.1", cut.clone(), "1", "5.0.0-rc.5");
    cut.truncate(cut.len() / 2);
    {
        let mut versions = reg.versions.lock().unwrap();
        let v = versions.get_mut("0.2.1").unwrap();
        v.package = cut;
        v.size = full;
    }
    // for another API, said in the manifest: refused before the download
    publish(
        &reg,
        "1.0.0",
        package("1.0.0", "2", "5.0.0-rc.5", &[]),
        "2",
        "5.0.0-rc.5",
    );
    // for a newer weflow, said only inside the package
    publish(
        &reg,
        "0.3.0",
        package("0.3.0", "1", "6.0.0", &[]),
        "1",
        "5.0.0-rc.5",
    );
    // holding another version
    publish(
        &reg,
        "0.4.0",
        package("0.3.9", "1", "5.0.0-rc.5", &[]),
        "1",
        "5.0.0-rc.5",
    );
    let base = serve(reg.clone()).await;
    let registry = Registry::new(&base, REPO).unwrap();
    let store = temp_store("damaged");
    webui::install(&store, &registry, "0.1.0", BACKEND, QUIET)
        .await
        .unwrap();
    store.choose("0.1.0").unwrap();

    for (version, code, text) in [
        ("0.2.0", "runtime_error", "damaged"),
        ("0.2.1", "runtime_error", "damaged"),
        ("1.0.0", "incompatible", "API 2"),
        ("0.3.0", "incompatible", "6.0.0 or newer"),
        ("0.4.0", "runtime_error", "0.3.9"),
    ] {
        let blobs = reg.blobs.load(Ordering::SeqCst);
        let err = webui::install(&store, &registry, version, BACKEND, QUIET)
            .await
            .unwrap_err();
        assert_eq!(err.code, code, "{version}: {}", err.message);
        assert!(err.message.contains(text), "{version}: {}", err.message);
        if version == "1.0.0" {
            assert_eq!(
                reg.blobs.load(Ordering::SeqCst),
                blobs,
                "downloaded although refused"
            );
        }
        // what was there stays, and nothing is left of the attempt
        assert_eq!(entries(&store), ["0.1.0", "current"], "{version}");
        assert_eq!(store.chosen().as_deref(), Some("0.1.0"));
    }
}

#[tokio::test]
async fn packages_reaching_out_of_their_folder_are_refused() {
    let reg = Arc::new(Registry_::default());
    publish(
        &reg,
        "0.1.0",
        package("0.1.0", "1", "5.0.0-rc.5", &[("../escaped.txt", "x")]),
        "1",
        "5.0.0-rc.5",
    );
    publish(
        &reg,
        "0.1.1",
        package("0.1.1", "1", "5.0.0-rc.5", &[("/abs.txt", "x")]),
        "1",
        "5.0.0-rc.5",
    );
    let mut linked = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    linked
        .add_symlink("index.html", "../../secret", opts)
        .unwrap();
    linked.start_file("weflow-web.json", opts).unwrap();
    linked
        .write_all(json!({ "name": "weflow-web", "version": "0.1.2", "apiVersion": "1", "minBackendVersion": "5.0.0" }).to_string().as_bytes())
        .unwrap();
    publish(
        &reg,
        "0.1.2",
        linked.finish().unwrap().into_inner(),
        "1",
        "5.0.0-rc.5",
    );
    let base = serve(reg.clone()).await;
    let registry = Registry::new(&base, REPO).unwrap();
    let store = temp_store("escape");
    for version in ["0.1.0", "0.1.1", "0.1.2"] {
        let err = webui::install(&store, &registry, version, BACKEND, QUIET)
            .await
            .unwrap_err();
        assert!(
            err.message.contains("refused"),
            "{version}: {}",
            err.message
        );
    }
    assert!(entries(&store).is_empty());
    assert!(!store.root().parent().unwrap().join("escaped.txt").exists());
}

#[tokio::test]
async fn two_installs_at_once_keep_one_copy() {
    let reg = Arc::new(Registry_::default());
    publish(
        &reg,
        "0.1.0",
        package("0.1.0", "1", "5.0.0-rc.5", &[]),
        "1",
        "5.0.0-rc.5",
    );
    let base = serve(reg.clone()).await;
    let store = temp_store("race");
    let (a, b) = (
        Registry::new(&base, REPO).unwrap(),
        Registry::new(&base, REPO).unwrap(),
    );
    let (x, y) = tokio::join!(
        webui::install(&store, &a, "0.1.0", BACKEND, QUIET),
        webui::install(&store, &b, "0.1.0", BACKEND, QUIET),
    );
    assert_eq!(x.unwrap().version, "0.1.0");
    assert_eq!(y.unwrap().version, "0.1.0");
    assert_eq!(entries(&store), ["0.1.0"]);
}
