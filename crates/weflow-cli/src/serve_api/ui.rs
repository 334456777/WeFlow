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
//!   holds the token until it removes it. It also carries a `Content-Security-Policy` (#818, see
//!   `content_security_policy`) and may not be framed.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};

mod pending;

pub use pending::Pending;

/// What `/ui/` shows: a page, or why there is none.
#[derive(Clone, Debug)]
pub enum Ui {
    /// A built WebUI.
    Page(UiDir),
    /// No page while the version wanted is downloaded beside the server, or after that failed (issue #946).
    Pending(Pending),
    /// No page at all: `--api-only`, or no version to show.
    Absent,
}

impl Ui {
    /// The page shown, if any.
    pub fn page(&self) -> Option<&UiDir> {
        match self {
            Self::Page(dir) => Some(dir),
            Self::Pending(_) | Self::Absent => None,
        }
    }
}

/// The answer of `/ui/…` (`rest` as for [`serve`]): the page's files, or when there is none, why.
pub async fn answer(ui: &Ui, method: &Method, rest: &str, query: Option<&str>) -> Response {
    match ui {
        Ui::Page(dir) => serve(dir, method, rest, query).await,
        Ui::Pending(pending) => pending::answer(pending, method),
        Ui::Absent => super::error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "no WebUI is served; start serve with --ui-dir <a built WebUI>",
        ),
    }
}

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
        return super::redirect(StatusCode::PERMANENT_REDIRECT, &to);
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
        let seg = String::from_utf8(weflow_core::message::emoji::percent_decode_bytes(raw)).ok()?;
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
    let headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static(content_type(&ext)),
        ),
        (header::CONTENT_LENGTH, HeaderValue::from(bytes.len())),
        (
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ),
        (
            header::CACHE_CONTROL,
            HeaderValue::from_static(if built {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            }),
        ),
    ];
    let policy = (ext == "html").then(|| content_security_policy(&bytes));
    let body = if head {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    let mut resp = (StatusCode::OK, headers, body).into_response();
    if let Some(policy) = policy {
        let page = resp.headers_mut();
        page.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
        page.insert(header::CONTENT_SECURITY_POLICY, policy);
        page.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    resp
}

/// The `Content-Security-Policy` of a page (#818). The page holds the token of the command API, so a script of someone
/// else's in it could run any command: scripts come only from the folder, besides the page's own inline ones named by
/// their hash (the theme set before the first paint, `web/index.html`), and the page talks to this server alone.
/// Pictures also come from WeChat's servers (avatars, stickers, link covers; over `http:` as often as `https:`), and
/// styles may be inline (the renderer's `style` attributes): with pictures from anywhere, a style could send out no
/// more than a picture could. No other site may show the page in a frame.
fn content_security_policy(html: &[u8]) -> HeaderValue {
    use base64::Engine;
    use sha2::{Digest, Sha256};

    let scripts: String = inline_scripts(html)
        .iter()
        .map(|script| {
            let digest = Sha256::digest(script);
            let hash = base64::engine::general_purpose::STANDARD.encode(digest);
            format!(" 'sha256-{hash}'")
        })
        .collect();
    let policy = format!(
        "default-src 'self'; script-src 'self'{scripts}; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob: http: https:; media-src 'self' data: blob:; connect-src 'self'; \
         object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"
    );
    // keywords and base64 are always a valid header; were they not, no script but the folder's would run
    HeaderValue::try_from(policy)
        .unwrap_or_else(|_| HeaderValue::from_static("default-src 'self'; frame-ancestors 'none'"))
}

/// The text of each `<script>` of `html` without a `src`, as the browser hashes it: with its line breaks made `\n`,
/// as the HTML parser makes them before anything else.
fn inline_scripts(html: &[u8]) -> Vec<Vec<u8>> {
    const OPEN: &[u8] = b"<script";
    const CLOSE: &[u8] = b"</script";
    let mut scripts = Vec::new();
    let mut rest = html;
    while let Some(start) = find_ignoring_case(rest, OPEN) {
        let after = &rest[start + OPEN.len()..];
        // `<scripts>` or the like is another element
        if !after
            .first()
            .is_some_and(|&b| b == b'>' || b == b'/' || b.is_ascii_whitespace())
        {
            rest = after;
            continue;
        }
        let Some(tag_end) = after.iter().position(|&b| b == b'>') else {
            break;
        };
        let body = &after[tag_end + 1..];
        let Some(end) = find_ignoring_case(body, CLOSE) else {
            break;
        };
        if !has_src(&after[..tag_end]) {
            scripts.push(normalize_newlines(&body[..end]));
        }
        rest = &body[end..];
    }
    scripts
}

fn find_ignoring_case(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

/// Whether the attributes of a start tag (what lies between its name and `>`) have a `src`: a name starts after a
/// space or `/` and ends at one or at `=`. A script with one runs the file it names, never its text, even when the
/// address is empty.
fn has_src(attrs: &[u8]) -> bool {
    let starts =
        |at: usize| at > 0 && (attrs[at - 1] == b'/' || attrs[at - 1].is_ascii_whitespace());
    let ends =
        |b: Option<&u8>| b.is_none_or(|&b| b == b'/' || b == b'=' || b.is_ascii_whitespace());
    attrs.windows(3).enumerate().any(|(at, window)| {
        window.eq_ignore_ascii_case(b"src") && starts(at) && ends(attrs.get(at + 3))
    })
}

/// `\r\n` and a lone `\r` made `\n`.
fn normalize_newlines(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.iter().peekable();
    while let Some(&b) = bytes.next() {
        if b == b'\r' {
            bytes.next_if_eq(&&b'\n');
            out.push(b'\n');
        } else {
            out.push(b);
        }
    }
    out
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

    #[test]
    fn only_inline_scripts_are_hashed() {
        let html = b"<!doctype html><head><script>\r\n  theme();\r\n</script>\
            <script type=\"module\" crossorigin src=\"/ui/assets/index-a1.js\"></script>\
            <SCRIPT data-x=\"1\">b()\r</SCRIPT><scripts>no</scripts><script src></script>\
            <script data-src=\"x\">c()</script></head>";
        let scripts = inline_scripts(html);
        assert_eq!(
            scripts,
            [&b"\n  theme();\n"[..], b"b()\n", b"c()"],
            "{:?}",
            scripts
                .iter()
                .map(|s| String::from_utf8_lossy(s))
                .collect::<Vec<_>>()
        );
        assert!(has_src(b" src=\"/x.js\""));
        assert!(has_src(b" type=module SRC = '/x.js'"));
        assert!(has_src(b"/src"));
        assert!(!has_src(b" data-src=\"x\""));
        assert!(!has_src(b" srcset=\"x\""));
    }

    #[test]
    fn the_policy_names_the_inline_scripts_by_their_hash() {
        // the hash of `alert(1)`, as a browser reports it for a script it refuses
        let policy = content_security_policy(b"<script>alert(1)</script>");
        let policy = policy.to_str().unwrap();
        assert!(
            policy.contains(
                "script-src 'self' 'sha256-bhHHL3z2vDgxUt0W3dWQOrprscmda2Y5pLsLg4GF+pI=';"
            ),
            "{policy}"
        );
        assert!(policy.contains("connect-src 'self'"), "{policy}");
        assert!(policy.contains("frame-ancestors 'none'"), "{policy}");
        let none = content_security_policy(b"<script src=\"/x.js\"></script>");
        assert!(none.to_str().unwrap().contains("script-src 'self';"));
    }
}
