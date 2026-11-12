//! HTTP around the routes: the access token, CORS, answering only requests that name this machine, compression
//! and the server's own answers.

use axum::body::Body;
use axum::http::uri::Authority;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::bad_request;
use super::rules::Verb;

/// gzip or br, as `Accept-Encoding` asks (issue #155): a page of messages or the contacts are mostly repeated
/// column names and XML. Not for what is compressed already or streamed: images, voices, videos, the files of
/// `return=file`, the event streams, and a part of a file (`206`).
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

pub(super) fn json_body(status: StatusCode, body: impl Into<Body>) -> Response {
    let content_type = HeaderValue::from_static("application/json; charset=utf-8");
    let body: Body = body.into();
    (status, [(header::CONTENT_TYPE, content_type)], body).into_response()
}

/// A redirect to `to`, a path on this server, which may carry the query of the request.
pub(super) fn redirect(status: StatusCode, to: &str) -> Response {
    match HeaderValue::from_str(to) {
        Ok(location) => (status, [(header::LOCATION, location)]).into_response(),
        Err(_) => bad_request("the address cannot be redirected"),
    }
}

/// The answer a builder made; `500` when one of the headers given to it is no valid header value.
pub(super) fn built(resp: Result<Response, axum::http::Error>) -> Response {
    resp.unwrap_or_else(|err| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "runtime_error",
            &format!("cannot build the answer: {err}"),
        )
    })
}

/// An error answered by the server itself, in the shape of a failed `--json` command.
pub(super) fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    let body = json!({ "success": false, "error": { "code": code, "message": message } });
    let mut resp = json_body(status, body.to_string());
    resp.headers_mut().insert(
        "x-weflow-error-code",
        HeaderValue::from_str(code).unwrap_or(HeaderValue::from_static("error")),
    );
    resp
}

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
    #[expect(
        clippy::expect_used,
        reason = "method names joined by commas are a valid header value"
    )]
    let allow = HeaderValue::from_str(&allow).expect("method names are visible ASCII");
    resp.headers_mut().insert(header::ALLOW, allow);
    resp
}

fn safe_equal(a: &str, b: &str) -> bool {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    x.len() == y.len() && x.iter().zip(y).fold(0u8, |acc, (p, q)| acc | (p ^ q)) == 0
}

pub(super) fn authorized(token: &str, headers: &HeaderMap, query: &[(String, String)]) -> bool {
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

pub(super) fn url_decode(s: &str) -> String {
    String::from_utf8_lossy(&weflow_core::message::emoji::percent_decode_bytes(
        &s.replace('+', " "),
    ))
    .into_owned()
}

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

/// Allows pages served from this machine (the WebUI's dev server, for one) to call the API.
pub(super) fn cors(headers: &HeaderMap, mut resp: Response) -> Response {
    // the origin is sent back as the request gave it
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
        // the stages of `Server-Timing` in the page's own performance entries
        h.insert("timing-allow-origin", value.clone());
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
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

/// Whether the request names this machine: `localhost` or a loopback address, with any port. A page of another
/// site whose name it made resolve to 127.0.0.1 (DNS rebinding) sends that name and is refused.
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

/// A random access token for a run without a configured one.
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
        // HTTP/2 and absolute-form requests carry the name in the URI
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
