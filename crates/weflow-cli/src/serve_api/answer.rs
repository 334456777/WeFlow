//! The answers: a command's `--json` document and its status, `?fields=`, and the media files of `/api/` routes
//! with their byte ranges.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::Value;

use super::child::status_for;
use super::endpoints::text_of;
use super::http::{built, error_response, json_body};
use super::in_process;

/// A read run in this process that did not end (it panicked).
pub(super) fn failed_in_process(err: &tokio::task::JoinError) -> Response {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "runtime_error",
        &format!("the command failed: {err}"),
    )
}

/// The `--json` document of a command run in this process, answered as the child process prints it.
pub(super) fn in_process_document(doc: &Value, exit: u8, fields: Option<&[String]>) -> Response {
    let body = match fields {
        Some(fields) => json_line(&only_fields(doc.clone(), fields)),
        None => json_line(doc),
    };
    let mut resp = document_response(doc, status_for(doc, exit.into()), body);
    resp.headers_mut()
        .insert("x-weflow-exit-code", HeaderValue::from(i32::from(exit)));
    resp
}

/// A `--json` document as the command prints it: on one line.
pub(super) fn json_line(doc: &Value) -> String {
    format!("{doc}\n")
}

/// The names of `?fields=`: repeated, comma-separated, or a JSON array.
pub(super) fn field_names(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .flat_map(|v| match v {
            Value::Array(items) => items.iter().map(text_of).collect(),
            other => vec![text_of(other)],
        })
        .flat_map(|v| {
            v.split(',')
                .map(|f| f.trim().to_string())
                .collect::<Vec<_>>()
        })
        .filter(|f| !f.is_empty())
        .collect()
}

/// `?fields=` (issue #155): of a successful document, only the named keys of each row: the objects of `data` when
/// it is a list, or of each list in `data` when it is an object. Anything else is left as it is.
pub(super) fn only_fields(mut doc: Value, fields: &[String]) -> Value {
    if doc["success"] != true {
        return doc;
    }
    let keep = |row: &mut Value| {
        if let Value::Object(row) = row {
            row.retain(|k, _| fields.iter().any(|f| f == k));
        }
    };
    match doc.get_mut("data") {
        Some(Value::Array(rows)) => rows.iter_mut().for_each(keep),
        Some(Value::Object(data)) => {
            for value in data.values_mut() {
                if let Value::Array(rows) = value {
                    rows.iter_mut().for_each(keep);
                }
            }
        }
        _ => {}
    }
    doc
}

/// The file of an `/api/` media route. Its `ETag` lets a page ask again with `If-None-Match` and get `304`; the
/// file of a message rarely changes (a thumbnail replaced by the full image), so it may be kept for an hour. A file
/// on disk (a video) is read as it is sent, only the part asked for: its `ETag` comes from its size and time.
pub(super) fn media_response(headers: &HeaderMap, file: in_process::MediaFile) -> Response {
    use std::hash::{Hash, Hasher};
    let (etag, len) = match &file.body {
        in_process::MediaBody::Bytes(bytes) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut hasher);
            (format!("\"{:016x}\"", hasher.finish()), bytes.len())
        }
        in_process::MediaBody::File(path) => match std::fs::metadata(path) {
            Ok(meta) => {
                let at = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                (
                    format!("\"{:x}-{at:x}\"", meta.len()),
                    usize::try_from(meta.len()).unwrap_or(usize::MAX),
                )
            }
            Err(err) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "runtime_error",
                    &format!("failed to read {}: {err}", path.display()),
                )
            }
        },
    };
    let unchanged = headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|tag| {
            let tag = tag.trim();
            tag == "*" || tag.trim_start_matches("W/") == etag
        });
    let builder = Response::builder()
        .header(header::ETAG, &etag)
        .header(header::CACHE_CONTROL, "private, max-age=3600")
        .header(header::ACCEPT_RANGES, "bytes");
    if unchanged {
        return built(builder.status(StatusCode::NOT_MODIFIED).body(Body::empty()));
    }
    // `If-Range` asks for the part only while the file is the one it names
    let same = headers
        .get(header::IF_RANGE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|tag| tag.trim() == etag);
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .filter(|_| same)
        .and_then(|v| byte_range(v, len));
    let builder = builder.header(header::CONTENT_TYPE, file.mime);
    let (builder, first, last) = match range {
        None => (builder.status(StatusCode::OK), 0, len.saturating_sub(1)),
        Some(Ok((first, last))) => (
            builder
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, format!("bytes {first}-{last}/{len}")),
            first,
            last,
        ),
        Some(Err(())) => {
            return built(
                builder
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                    .body(Body::empty()),
            )
        }
    };
    let count = if len == 0 { 0 } else { last + 1 - first };
    let body = match file.body {
        in_process::MediaBody::Bytes(mut bytes) => {
            bytes.truncate(first + count);
            bytes.drain(..first);
            Body::from(bytes)
        }
        in_process::MediaBody::File(path) => file_part(path, first as u64, count as u64),
    };
    built(builder.header(header::CONTENT_LENGTH, count).body(body))
}

/// `count` bytes of a file from `first` on, read as they are sent: a video is not held in memory.
fn file_part(path: PathBuf, first: u64, count: u64) -> Body {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let chunks = async_stream::stream! {
        let mut file = match tokio::fs::File::open(&path).await {
            Ok(file) => file,
            Err(err) => {
                yield Err(err);
                return;
            }
        };
        if let Err(err) = file.seek(std::io::SeekFrom::Start(first)).await {
            yield Err(err);
            return;
        }
        let mut left = count;
        let mut buf = vec![0u8; 256 * 1024];
        while left > 0 {
            let want = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
            match file.read(&mut buf[..want]).await {
                // shorter than it was: the answer ends early, as its length says it should not
                Ok(0) => {
                    yield Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
                    return;
                }
                Ok(n) => {
                    left -= n as u64;
                    yield Ok::<_, std::io::Error>(axum::body::Bytes::copy_from_slice(&buf[..n]));
                }
                Err(err) => {
                    yield Err(err);
                    return;
                }
            }
        }
    };
    Body::from_stream(chunks)
}

/// The one range of a `Range: bytes=…` header for a file of `len` bytes, as (first, last) byte, clamped to the file;
/// `Err` when it starts past the end. `None` (the whole file) for a header that names several ranges or that this
/// does not read, as RFC 9110 allows.
fn byte_range(header: &str, len: usize) -> Option<Result<(usize, usize), ()>> {
    let spec = header.trim().strip_prefix("bytes=")?.trim();
    if spec.contains(',') {
        return None;
    }
    let (from, to) = spec.split_once('-')?;
    let (from, to) = (from.trim(), to.trim());
    if from.is_empty() {
        // the last `to` bytes
        let n: usize = to.parse().ok()?;
        if n == 0 {
            return Some(Err(()));
        }
        return Some(Ok((len.saturating_sub(n), len.checked_sub(1)?)));
    }
    let first: usize = from.parse().ok()?;
    let last = match to {
        "" => len.saturating_sub(1),
        to => to.parse::<usize>().ok()?.min(len.saturating_sub(1)),
    };
    if first >= len {
        return Some(Err(()));
    }
    (first <= last).then_some(Ok((first, last)))
}

/// A command's `--json` document, with its error code in a header.
pub(super) fn document_response(
    doc: &Value,
    status: StatusCode,
    body: impl Into<Body>,
) -> Response {
    let mut resp = json_body(status, body);
    if let Some(code) = doc["error"]["code"].as_str().filter(|c| !c.is_empty()) {
        if let Ok(v) = HeaderValue::from_str(code) {
            resp.headers_mut().insert("x-weflow-error-code", v);
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_ranges_are_read_as_rfc_9110_has_them() {
        let r = |h: &str| byte_range(h, 100);
        assert_eq!(r("bytes=0-9"), Some(Ok((0, 9))));
        assert_eq!(r("bytes=90-"), Some(Ok((90, 99))));
        assert_eq!(r("bytes=90-500"), Some(Ok((90, 99))), "clamped to the file");
        assert_eq!(r("bytes=-10"), Some(Ok((90, 99))), "the last ten");
        assert_eq!(r("bytes=-500"), Some(Ok((0, 99))));
        assert_eq!(r(" bytes= 5 - 6 "), Some(Ok((5, 6))));
        // past the end: 416
        assert_eq!(r("bytes=100-"), Some(Err(())));
        assert_eq!(r("bytes=-0"), Some(Err(())));
        // the whole file: several ranges, other units, nonsense, a range that ends before it starts
        for h in [
            "bytes=0-1,5-6",
            "items=0-1",
            "bytes=a-b",
            "bytes=5",
            "bytes=9-3",
            "",
        ] {
            assert_eq!(r(h), None, "{h}");
        }
        assert_eq!(byte_range("bytes=-5", 0), None);
    }
}
