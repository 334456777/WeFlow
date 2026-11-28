//! 路由外围的 HTTP 处理：访问令牌、CORS、只回答指向本机的请求、压缩，以及服务自己的回答。

use axum::body::Body;
use axum::http::uri::Authority;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::bad_request;
use super::rules::Verb;

/// 按 `Accept-Encoding` 用 gzip 或 br 压缩（#155）：一页消息或联系人大多是重复的列名和 XML。
/// 已经压缩过的或流式发送的内容不压缩：图片、语音、视频、`return=file` 的文件、事件流，以及文件的一部分（`206`）。
pub(super) fn compression(
) -> tower_http::compression::CompressionLayer<impl tower_http::compression::Predicate> {
    use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};
    let whole =
        |status: StatusCode, _: axum::http::Version, _: &HeaderMap, _: &axum::http::Extensions| {
            status != StatusCode::PARTIAL_CONTENT
        };
    tower_http::compression::CompressionLayer::new().compress_when(
        DefaultPredicate::new()
            .and(NotForContentType::const_new("audio/"))
            .and(NotForContentType::const_new("video/"))
            .and(NotForContentType::const_new("multipart/"))
            .and(NotForContentType::const_new("application/octet-stream"))
            .and(whole),
    )
}

/// 状态码为 `status`、内容为 `body` 的 JSON 回答。
pub(super) fn json_body(status: StatusCode, body: impl Into<Body>) -> Response {
    let content_type = HeaderValue::from_static("application/json; charset=utf-8");
    let body: Body = body.into();
    (status, [(header::CONTENT_TYPE, content_type)], body).into_response()
}

/// 跳转到这个服务上的路径 `to`，`to` 可以带着请求的查询参数；`to` 不能作为头的值时回答 `400`。
pub(super) fn redirect(status: StatusCode, to: &str) -> Response {
    match HeaderValue::from_str(to) {
        Ok(location) => (status, [(header::LOCATION, location)]).into_response(),
        Err(_) => bad_request("the address cannot be redirected"),
    }
}

/// 构建器生成的回答；交给它的某个头不是有效的头的值时回答 `500`。
pub(super) fn built(resp: Result<Response, axum::http::Error>) -> Response {
    resp.unwrap_or_else(|err| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "runtime_error",
            &format!("cannot build the answer: {err}"),
        )
    })
}

/// 服务自己回答的错误：格式与失败的 `--json` 命令相同，并且与命令的错误一样用服务端的语言（#1090）；
/// 错误码同时写在 `X-WeFlow-Error-Code` 头里。
pub(super) fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    let message = weflow_core::locale::localize(message.to_owned());
    let body = json!({ "success": false, "error": { "code": code, "message": message } });
    let mut resp = json_body(status, body.to_string());
    resp.headers_mut().insert(
        "x-weflow-error-code",
        HeaderValue::from_str(code).unwrap_or(HeaderValue::from_static("error")),
    );
    resp
}

/// `405 Method Not Allowed`，`Allow` 头列出 `allowed` 中的方法。
pub(super) fn method_not_allowed(allowed: &[Verb]) -> Response {
    let allow = allowed
        .iter()
        .map(|v| v.method().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let mut resp = error_response(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        &format!("method not allowed; use {allow}"),
    );
    #[expect(clippy::expect_used, reason = "用逗号连接的方法名是有效的头的值")]
    let allow = HeaderValue::from_str(&allow).expect("method names are visible ASCII");
    resp.headers_mut().insert(header::ALLOW, allow);
    resp
}

/// 用恒定时间比较 `a` 和 `b`：耗时不随第一个不同的字节的位置变化，不能借此逐个字节猜出令牌。
fn safe_equal(a: &str, b: &str) -> bool {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    x.len() == y.len() && x.iter().zip(y).fold(0u8, |acc, (p, q)| acc | (p ^ q)) == 0
}

/// 请求是否在 `Authorization: Bearer` 头或 `access_token` 参数中给出了服务的 `token`。
/// `token` 为空时谁也不放行：空的参数会与它相等（#808）。
pub(super) fn authorized(token: &str, headers: &HeaderMap, query: &[(String, String)]) -> bool {
    if token.is_empty() {
        return false;
    }
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.get(..7)
                .filter(|p| p.eq_ignore_ascii_case("bearer "))
                .map(|_| v[7..].trim())
        });
    bearer.is_some_and(|t| safe_equal(t, token))
        || query
            .iter()
            .any(|(k, v)| k == "access_token" && safe_equal(v.trim(), token))
}

/// 解码 URL 中的一段：`+` 是空格，`%XX` 按字节解码，不是 UTF-8 的字节换成替换字符。
pub(super) fn url_decode(s: &str) -> String {
    String::from_utf8_lossy(&weflow_core::message::emoji::percent_decode_bytes(
        &s.replace('+', " "),
    ))
    .into_owned()
}

/// 把查询字符串拆成解码后的键值对，按出现的顺序排列；没有 `=` 的键的值为空。
pub(super) fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    query
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (url_decode(k), url_decode(v))
        })
        .collect()
}

/// 允许本机提供的页面（如 WebUI 的开发服务器）调用 API。
pub(super) fn cors(headers: &HeaderMap, mut resp: Response) -> Response {
    // 按请求给出的原样送回 origin
    let Some((value, origin)) = headers
        .get(header::ORIGIN)
        .and_then(|v| Some((v, v.to_str().ok()?)))
    else {
        return resp;
    };
    let host = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    let local = host.is_some_and(|h| {
        let name = match h.rsplit_once(':') {
            Some((name, port)) if !name.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
                name
            }
            _ => h,
        };
        matches!(name, "localhost" | "127.0.0.1" | "[::1]")
    });
    let h = resp.headers_mut();
    if local {
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value.clone());
        h.insert(header::VARY, HeaderValue::from_static("Origin"));
        // 让页面在自己的性能条目中读到 `Server-Timing` 的各阶段
        h.insert("timing-allow-origin", value.clone());
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Content-Type, Authorization, Prefer, Last-Event-ID"),
        );
        h.insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static(
                "X-WeFlow-Exit-Code, X-WeFlow-Error-Code, Location, Retry-After, Server-Timing",
            ),
        );
    }
    resp
}

/// 请求是否指向本机：`localhost` 或回环地址，端口不限。
/// 别的网站把自己的域名解析到 127.0.0.1（DNS 重绑定）后，它的页面发出的请求带着这个域名，因而被拒绝。
pub(super) fn local_host(uri: &Uri, headers: &HeaderMap) -> bool {
    let authority = uri.authority().cloned().or_else(|| {
        headers
            .get(header::HOST)?
            .to_str()
            .ok()?
            .parse::<Authority>()
            .ok()
    });
    let Some(authority) = authority else {
        return false;
    };
    let host = authority.host().trim_end_matches('.');
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// 没有配置访问令牌时这次运行使用的随机令牌：16 个随机字节写成 32 个十六进制字符。
pub fn generate_token() -> String {
    let bytes: [u8; 16] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_checked_from_the_header_or_the_query() {
        let mut headers = HeaderMap::new();
        assert!(!authorized("t0k", &headers, &[]));
        assert!(authorized(
            "t0k",
            &headers,
            &[("access_token".into(), "t0k".into())]
        ));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer t0k"),
        );
        assert!(authorized("t0k", &headers, &[]));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer t0kx"),
        );
        assert!(!authorized("t0k", &headers, &[]));
        assert_eq!(generate_token().len(), 32);
        assert_ne!(generate_token(), generate_token());
    }

    #[test]
    fn an_empty_token_lets_no_one_in() {
        let mut headers = HeaderMap::new();
        let empty = [("access_token".to_string(), String::new())];
        assert!(!authorized("", &headers, &empty));
        assert!(!authorized("", &headers, &[]));
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer "));
        assert!(!authorized("", &headers, &[]));
    }

    #[test]
    fn only_this_machine_is_answered_by_name() {
        let host = |v: &'static str| {
            let mut h = HeaderMap::new();
            h.insert(header::HOST, HeaderValue::from_static(v));
            local_host(&Uri::from_static("/chat/sessions"), &h)
        };
        for ok in [
            "localhost",
            "LOCALHOST:18108",
            "localhost.:18108",
            "127.0.0.1",
            "127.0.0.1:18108",
            "127.1.2.3:80",
            "[::1]",
            "[::1]:18108",
        ] {
            assert!(host(ok), "{ok}");
        }
        for bad in [
            "evil.example",
            "evil.example:18108",
            "localhost.evil.example",
            "127.0.0.1.nip.io",
            "192.168.1.2:18108",
            "[::2]:18108",
            "",
        ] {
            assert!(!host(bad), "{bad}");
        }
        assert!(!local_host(&Uri::from_static("/"), &HeaderMap::new()));
        // HTTP/2 和绝对形式（absolute-form）的请求把主机名放在 URI 里
        assert!(local_host(
            &Uri::from_static("http://localhost:18108/health"),
            &HeaderMap::new()
        ));
        assert!(!local_host(
            &Uri::from_static("http://evil.example/health"),
            &HeaderMap::new()
        ));
    }
}
