//! The WebUI under `/ui/` (issue #146): the files of a built WebUI (`web/dist`), read-only, without the token (the
//! page brings it in its address and sends it with each API request), and still behind the `Host` check.
//!
//! - `/ui` goes to `/ui/`, and `/ui/` is the page's `index.html`.
//! - A path of the page's own (no extension: `/ui/chat`) that is not a file gets `index.html` too, so the page can
//!   route it; a missing file with an extension (`/ui/assets/x.js`) answers `404`, never the page. Nothing outside
//!   `/ui/` falls back: an unknown API path stays a `404`.
//! - Only files inside the folder are served: `..` and the like answer `404`, and a link leading out of the folder
//!   is not followed.
//! - The built assets (`/ui/assets/`, named by their content) are cached for good; everything else is asked again
//!   each time, so a new version shows at once. The page is sent with `Referrer-Policy: no-referrer`: its address
//!   holds the token until it removes it.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::Response;

/// A folder holding a built WebUI.
#[derive(Clone, Debug)]
pub struct UiDir {
    /// Resolved (links followed), to tell what lies inside it.
    root: PathBuf,
    /// The version kept by `weflow web` it is, if it is one.
    version: Option<String>,
    /// Keeps that version from `weflow web prune` while the server shows it.
    _hold: Option<std::sync::Arc<std::fs::File>>,
}

impl UiDir {
    /// The folder, when it holds a page (`index.html`).
    pub fn open(dir: &Path) -> Option<Self> {
        let root = std::fs::canonicalize(dir).ok()?;
        root.join("index.html").is_file().then_some(Self {
            root,
            version: None,
            _hold: None,
        })
    }

    /// A version kept by `weflow web`, held (see `WebStore::hold`) for as long as this or a clone of it lives.
    pub fn open_version(store: &weflow_core::webui::WebStore, version: &str) -> Option<Self> {
        let hold = store.hold(version).ok()?;
        Self::open(&store.dir(version)).map(|ui| Self {
            version: Some(version.to_string()),
            _hold: Some(std::sync::Arc::new(hold)),
            ..ui
        })
    }

    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// The file a path inside the folder names, when it is one and lies inside the folder.
    fn file(&self, segments: &[String]) -> Option<PathBuf> {
        let mut path = self.root.clone();
        path.extend(segments);
        let real = std::fs::canonicalize(&path).ok()?;
        (real.starts_with(&self.root) && real.is_file()).then_some(real)
    }
}

/// The answer to `rest`, what follows `/ui` in the path (`""`, `"/"`, `"/assets/index.js"`, …); `query` is kept
/// when going from `/ui` to `/ui/`.
pub async fn serve(ui: &UiDir, method: &Method, rest: &str, query: Option<&str>) -> Response {
    if rest.is_empty() {
        let to = format!("/ui/{}", query.map(|q| format!("?{q}")).unwrap_or_default());
        return Response::builder()
            .status(StatusCode::PERMANENT_REDIRECT)
            .header(header::LOCATION, to)
            .body(Body::empty())
            .unwrap();
    }
    if method != Method::GET && method != Method::HEAD {
        let mut resp = super::error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "the WebUI's files are read-only; use GET",
        );
        resp.headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return resp;
    }
    let Some(segments) = segments(rest) else {
        return not_found();
    };
    let file = if segments.is_empty() {
        ui.file(&["index.html".into()])
    } else {
        ui.file(&segments).or_else(|| {
            // a path of the page's own: no extension, and not among the built assets
            let last = segments.last().map(String::as_str).unwrap_or("");
            (!last.contains('.') && segments[0] != "assets")
                .then(|| ui.file(&["index.html".into()]))
                .flatten()
        })
    };
    match file {
        Some(file) => send(ui, &file, method == Method::HEAD).await,
        None => not_found(),
    }
}

/// The decoded segments of a path, or `None` when one would leave the folder or name something else than a file in
/// it (`..`, `.`, a drive, a backslash, a NUL).
fn segments(rest: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for raw in rest.split('/').filter(|s| !s.is_empty()) {
        let seg = String::from_utf8(weflow_core::message::percent_decode_bytes(raw)).ok()?;
        if seg == "."
            || seg == ".."
            || seg.contains(['/', '\\', ':', '\0'])
            || seg.chars().any(char::is_control)
        {
            return None;
        }
        out.push(seg);
    }
    Some(out)
}

fn not_found() -> Response {
    super::error_response(
        StatusCode::NOT_FOUND,
        "not_found",
        "no such file in the WebUI",
    )
}

async fn send(ui: &UiDir, file: &Path, head: bool) -> Response {
    let bytes = match tokio::fs::read(file).await {
        Ok(bytes) => bytes,
        Err(_) => return not_found(),
    };
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let built = file.starts_with(ui.root.join("assets"));
    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type(&ext))
        .header(header::CONTENT_LENGTH, bytes.len())
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(
            header::CACHE_CONTROL,
            if built {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            },
        );
    if ext == "html" {
        resp = resp.header(header::REFERRER_POLICY, "no-referrer");
    }
    resp.body(if head {
        Body::empty()
    } else {
        Body::from(bytes)
    })
    .unwrap()
}

fn content_type(ext: &str) -> &'static str {
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_the_folder() {
        assert_eq!(segments("/").unwrap(), Vec::<String>::new());
        assert_eq!(
            segments("/assets/index-a1.js").unwrap(),
            ["assets", "index-a1.js"]
        );
        assert_eq!(segments("/chat/%E8%81%8A").unwrap(), ["chat", "聊"]);
        for bad in [
            "/../config.json",
            "/%2e%2e/config.json",
            "/assets/..%2f..%2fx",
            "/a%5cb",
            "/C:/Windows",
            "/a%00b",
            "/./index.html",
        ] {
            assert!(segments(bad).is_none(), "{bad}");
        }
        assert_eq!(content_type("js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("exe"), "application/octet-stream");
    }
}
