//! `/ui/` 下的 WebUI（#146）：构建好的 WebUI（`web/dist`）的文件，只读，不需要令牌（令牌在页面的地址里，
//! 页面在每个 API 请求中带上它），但同样经过 `Host` 检查。
//!
//! - `/ui` 跳转到 `/ui/`，`/ui/` 是页面的 `index.html`。
//! - 页面自己的路径（没有扩展名，如 `/ui/chat`）不是文件时同样回答 `index.html`，由页面自己路由；
//!   缺少的带扩展名的文件（`/ui/assets/x.js`）回答 `404`，不回答页面。`/ui/` 以外的路径不会退回到页面：
//!   未知的 API 路径仍是 `404`。
//! - 只提供文件夹里的文件：`..` 之类的路径回答 `404`，不跟随指向文件夹外的链接。
//! - 构建出的资源（`/ui/assets/`，按内容命名）永久缓存；其他文件每次都重新请求，新版本因此立即生效。
//!   页面带 `Referrer-Policy: no-referrer` 发送：页面删掉地址里的令牌之前，令牌一直在地址里。页面还带
//!   `Content-Security-Policy`（#818，见 `content_security_policy`），且不能放进框架（frame）。

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};

mod pending;

pub use pending::Pending;

/// `/ui/` 显示的内容：页面，或者没有页面的原因。
#[derive(Clone, Debug)]
pub enum Ui {
    /// 构建好的 WebUI。
    Page(UiDir),
    /// 要显示的版本在服务启动后下载期间，或者下载失败后，没有页面（#946）。
    Pending(Pending),
    /// 完全没有页面：`--api-only`，或者没有可显示的版本。
    Absent,
}

impl Ui {
    /// 显示的页面；没有时为 `None`。
    pub fn page(&self) -> Option<&UiDir> {
        match self {
            Self::Page(dir) => Some(dir),
            Self::Pending(_) | Self::Absent => None,
        }
    }
}

/// `/ui/…` 的回答（`rest` 与 [`serve`] 的相同）：页面的文件；没有页面时说明原因。
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

/// 存放构建好的 WebUI 的文件夹。
#[derive(Clone, Debug)]
pub struct UiDir {
    /// 解析过的路径（跟随了链接），用来判断一个文件是否在它里面。
    root: PathBuf,
    /// 它是 `weflow web` 保存的版本时，这个版本号。
    version: Option<String>,
    /// 服务显示这个版本期间，防止 `weflow web prune` 删掉它。
    _hold: Option<std::sync::Arc<std::fs::File>>,
}

impl UiDir {
    /// `dir` 中有页面（`index.html`）时打开它。
    pub fn open(dir: &Path) -> Option<Self> {
        let root = std::fs::canonicalize(dir).ok()?;
        root.join("index.html").is_file().then_some(Self {
            root,
            version: None,
            _hold: None,
        })
    }

    /// 打开 `weflow web` 保存的版本 `version`，在这个值或它的克隆存在期间一直占用这个版本（见 `WebStore::hold`）。
    pub fn open_version(store: &weflow_core::webui::WebStore, version: &str) -> Option<Self> {
        let hold = store.hold(version).ok()?;
        Self::open(&store.dir(version)).map(|ui| Self {
            version: Some(version.to_string()),
            _hold: Some(std::sync::Arc::new(hold)),
            ..ui
        })
    }

    /// 它是 `weflow web` 保存的版本时的版本号。
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// 文件夹内的路径 `segments` 指向的文件；不是文件或在文件夹之外时为 `None`。
    fn file(&self, segments: &[String]) -> Option<PathBuf> {
        let mut path = self.root.clone();
        path.extend(segments);
        let real = std::fs::canonicalize(&path).ok()?;
        (real.starts_with(&self.root) && real.is_file()).then_some(real)
    }
}

/// 对 `rest`（路径中 `/ui` 之后的部分：`""`、`"/"`、`"/assets/index.js"` 等）的回答；
/// 从 `/ui` 跳转到 `/ui/` 时保留 `query`。
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
            // 页面自己的路径：没有扩展名，也不在构建出的资源中
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

/// 路径解码后的各段；某一段会离开文件夹，或者指向的不是其中的文件时（`..`、`.`、盘符、反斜杠、NUL）为 `None`。
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

/// 发送 `file`，`head` 为真时只发送头；缓存方式和页面的安全头见模块文档。
async fn send(ui: &UiDir, file: &Path, head: bool) -> Response {
    let Ok(bytes) = tokio::fs::read(file).await else {
        return not_found();
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

/// 页面的 `Content-Security-Policy`（#818）。
///
/// 页面持有命令 API 的令牌，混进页面的他人脚本可以运行任何命令：脚本只能来自文件夹，
/// 另外只允许页面自己按哈希列出的内联脚本（`web/index.html` 中在首次绘制前设置主题的脚本），
/// 页面也只能与这个服务通信。图片还可以来自微信的服务器（头像、表情、链接封面；`http:` 和 `https:` 一样常见），
/// 样式可以内联（渲染器的 `style` 属性）：既然图片可以来自任何地方，样式能发出去的东西不会比图片多。
/// 其他网站不能把页面放进框架。
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
    // 关键字和 base64 总能组成有效的头；万一无效，就只运行文件夹里的脚本
    HeaderValue::try_from(policy)
        .unwrap_or_else(|_| HeaderValue::from_static("default-src 'self'; frame-ancestors 'none'"))
}

/// `html` 中每个不带 `src` 的 `<script>` 的文字，与浏览器计算哈希时的一样：换行都换成 `\n`，
/// HTML 解析器最先做的就是这一步。
fn inline_scripts(html: &[u8]) -> Vec<Vec<u8>> {
    const OPEN: &[u8] = b"<script";
    const CLOSE: &[u8] = b"</script";
    let mut scripts = Vec::new();
    let mut rest = html;
    while let Some(start) = find_ignoring_case(rest, OPEN) {
        let after = &rest[start + OPEN.len()..];
        // `<scripts>` 之类的是别的元素
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

/// 开始标签的属性（标签名与 `>` 之间的部分）中是否有 `src`：属性名从空白或 `/` 之后开始，到空白、`/` 或 `=` 结束。
/// 带 `src` 的脚本运行它指向的文件，从不运行自己的文字，地址为空时也一样。
fn has_src(attrs: &[u8]) -> bool {
    let starts =
        |at: usize| at > 0 && (attrs[at - 1] == b'/' || attrs[at - 1].is_ascii_whitespace());
    let ends =
        |b: Option<&u8>| b.is_none_or(|&b| b == b'/' || b == b'=' || b.is_ascii_whitespace());
    attrs.windows(3).enumerate().any(|(at, window)| {
        window.eq_ignore_ascii_case(b"src") && starts(at) && ends(attrs.get(at + 3))
    })
}

/// 把 `\r\n` 和单独的 `\r` 换成 `\n`。
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
        // `alert(1)` 的哈希，与浏览器拒绝运行这段脚本时报告的相同
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
