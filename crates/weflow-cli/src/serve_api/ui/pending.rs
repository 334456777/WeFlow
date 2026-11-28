//! 还没有页面时 `/ui/` 的回答（#946）：要显示的版本在服务启动后正在下载，或者下载失败了。
//! 浏览器收到一个用服务端的语言写的简短页面，下载期间它会自动刷新；
//! 程序按 `X-WeFlow-Error-Code`（`webui_downloading`、`webui_unavailable`）区分这两种情况。

use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use weflow_core::locale::tr;

/// `/ui/` 暂时没有页面的原因。
#[derive(Clone, Debug)]
pub enum Pending {
    /// 正在下载这个版本，下载完成后显示。
    Downloading { version: String },
    /// 这个版本下载失败，以及失败的原因（用服务端的语言）。
    Failed { version: String, error: String },
}

/// 下载网页期间，页面隔几秒重新请求一次：下载通常只要几秒。
const RETRY_SECONDS: u32 = 3;

/// 还没有页面时 `/ui/` 的回答：`503` 和一个简短的页面，`X-WeFlow-Error-Code` 说明原因；
/// 下载期间带 `Retry-After`，页面每隔 [`RETRY_SECONDS`] 秒自动刷新。
pub(super) fn answer(pending: &Pending, method: &Method) -> Response {
    if method != Method::GET && method != Method::HEAD {
        let mut resp = super::super::error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "the WebUI's files are read-only; use GET",
        );
        resp.headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return resp;
    }
    let zh = weflow_core::locale::current() == weflow_core::locale::Lang::Zh;
    let (code, title, text, retry) = match pending {
        Pending::Downloading { version } => {
            (
                "webui_downloading",
                tr("Downloading the WebUI", "正在下载网页"),
                if zh {
                    format!("正在下载网页 {version}，下载完成后本页会自动显示网页。")
                } else {
                    format!("WebUI {version} is being downloaded; this page shows it once it is in place.")
                },
                true,
            )
        }
        Pending::Failed { version, error } => (
            "webui_unavailable",
            tr("The WebUI is not available", "网页不可用"),
            if zh {
                format!(
                    "网页 {version} 下载失败：{error}。请确认这台机器能访问 ghcr.io（或 WEFLOW_WEB_REGISTRY 指定的仓库）后重启 weflow serve；也可以用 --ui-dir 指定构建好的网页。命令 API 照常可用。"
                )
            } else {
                format!(
                    "WebUI {version} could not be downloaded: {error}. Check that this machine can reach ghcr.io (or the registry WEFLOW_WEB_REGISTRY names), then restart weflow serve; or start it with --ui-dir and a built WebUI. The command API runs as usual."
                )
            },
            false,
        ),
    };
    let lang = if zh { "zh-CN" } else { "en" };
    let refresh = if retry {
        format!("<meta http-equiv=\"refresh\" content=\"{RETRY_SECONDS}\">")
    } else {
        String::new()
    };
    let body = format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{refresh}\
         <title>WeFlow · {title}</title></head>\
         <body style=\"font-family: system-ui, sans-serif; max-width: 40rem; margin: 4rem auto; padding: 0 1rem; line-height: 1.6\">\
         <h1 style=\"font-size: 1.25rem\">{title}</h1><p>{text}</p></body></html>",
        title = escape(title),
        text = escape(&text),
    );
    let mut resp = (StatusCode::SERVICE_UNAVAILABLE, body).into_response();
    let headers = resp.headers_mut();
    for (name, value) in [
        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
        (header::CACHE_CONTROL, "no-store"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        // 地址里有令牌
        (header::REFERRER_POLICY, "no-referrer"),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'",
        ),
        (header::X_FRAME_OPTIONS, "DENY"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    headers.insert("x-weflow-error-code", HeaderValue::from_static(code));
    if retry {
        headers.insert(header::RETRY_AFTER, HeaderValue::from(RETRY_SECONDS));
    }
    resp
}

/// 把 `text` 转义成 HTML 的文字。
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reason_of_a_failed_download_is_text_not_markup() {
        assert_eq!(
            escape("<script>alert('x')</script> & \"y\""),
            "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &quot;y&quot;"
        );
    }
}
