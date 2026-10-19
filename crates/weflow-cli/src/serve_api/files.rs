//! `?return=file`: the file a command wrote, sent back in the response next to a JSON result
//! (`multipart/form-data` with a `result` and a `file` part).

use std::path::{Path, PathBuf};

use axum::body::{Body, Bytes};
use axum::http::{header, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

/// Commands that write one file, with the `--out` used when the request gives none (inside the request's folder;
/// empty = the folder itself, the command names the file) and the field of the result holding the written path
/// (`None`: the file is `--out` itself).
pub(super) const FILE_COMMANDS: &[(&str, &str, Option<&str>)] = &[
    ("chat voice-data", "voice.wav", Some("path")),
    ("chat image-data", "image", Some("path")),
    ("chat video-data", "video", Some("path")),
    ("export sessions", "", Some("out")),
    ("export contacts", "", Some("out")),
    ("export footprint", "", Some("out")),
    ("export messages", "", Some("out")),
    ("group export-member-messages", "member-messages.xlsx", None),
    ("group export-members", "members.xlsx", None),
    ("sns media", "media", Some("out")),
    ("sns download-image", "image", Some("out")),
    ("backup create", "backup.zip", Some("out")),
];

pub(super) fn file_command(name: &str) -> Option<(&'static str, Option<&'static str>)> {
    FILE_COMMANDS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, out, field)| (*out, *field))
}

/// The file a successful run wrote: the result's path field, or `--out`; relative paths are taken from the
/// server's working folder, which the command shared.
pub(super) fn written_file(doc: &Value, field: Option<&str>, out: &str) -> PathBuf {
    let path = field
        .and_then(|f| doc["data"][f].as_str())
        .filter(|p| !p.is_empty())
        .unwrap_or(out);
    std::path::absolute(path).unwrap_or_else(|_| PathBuf::from(path))
}

/// Gives a file named by the server without an extension (`image`) the one its content shows.
pub(super) fn add_extension(path: &Path) -> PathBuf {
    if path.extension().is_some() {
        return path.to_path_buf();
    }
    let mut head = [0u8; 16];
    let n = std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
        .unwrap_or(0);
    let Some(ext) = sniff(&head[..n]) else {
        return path.to_path_buf();
    };
    let named = path.with_extension(ext);
    match std::fs::rename(path, &named) {
        Ok(()) => named,
        Err(_) => path.to_path_buf(),
    }
}

/// The extension of a few media formats, from their first bytes.
fn sniff(head: &[u8]) -> Option<&'static str> {
    let at = |i: usize, magic: &[u8]| head.get(i..i + magic.len()) == Some(magic);
    Some(if at(0, &[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if at(0, b"\x89PNG") {
        "png"
    } else if at(0, b"GIF8") {
        "gif"
    } else if at(0, b"RIFF") && at(8, b"WEBP") {
        "webp"
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        "wav"
    } else if at(0, b"BM") {
        "bmp"
    } else if at(4, b"ftyp") {
        "mp4"
    } else if at(0, b"wxgf") {
        "wxgf"
    } else if at(0, b"PK\x03\x04") {
        "zip"
    } else {
        return None;
    })
}

fn content_type(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "json" => "application/json; charset=utf-8".into(),
        "jsonl" => "application/x-ndjson; charset=utf-8".into(),
        "txt" | "csv" | "html" | "sql" => format!(
            "{}; charset=utf-8",
            mime_guess::from_ext(&ext).first_or_text_plain()
        ),
        _ => mime_guess::from_ext(&ext)
            .first_or_octet_stream()
            .to_string(),
    }
}

/// `<kind>; filename="…"` (UTF-8 as browsers read it, quotes and line breaks escaped) and RFC 5987 `filename*`.
fn disposition(kind: &str, name: &str) -> String {
    let plain: String = name
        .chars()
        .map(|c| match c {
            '"' | '\\' | '\r' | '\n' => '_',
            c => c,
        })
        .collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    format!("{kind}; filename=\"{plain}\"; filename*=UTF-8''{encoded}")
}

/// The file at `path`, opened, with its length and name.
async fn open(path: &Path) -> Result<(tokio::fs::File, u64, String), String> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let meta = file
        .metadata()
        .await
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    Ok((file, meta.len(), name))
}

/// `head`, then the file read from disk as it is sent, then `tail`.
fn between(head: String, mut file: tokio::fs::File, tail: String) -> Body {
    Body::from_stream(async_stream::stream! {
        if !head.is_empty() {
            yield Ok::<_, std::io::Error>(Bytes::from(head));
        }
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match file.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => yield Ok(Bytes::copy_from_slice(&buf[..n])),
                Err(e) => {
                    yield Err(e);
                    return;
                }
            }
        }
        if !tail.is_empty() {
            yield Ok(Bytes::from(tail));
        }
    })
}

/// The `multipart/form-data` answer: `{"success": true, "data": {"path": …}}`, then the file read from disk as it is
/// sent.
pub(super) async fn multipart(path: &Path) -> Result<Response, String> {
    let (file, length, name) = open(path).await?;
    let boundary = format!("weflow-{}", super::generate_token());
    let result = json!({ "success": true, "data": { "path": path } });
    let head = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"result\"\r\nContent-Type: application/json; charset=utf-8\r\n\r\n{result}\r\n--{boundary}\r\nContent-Disposition: {}\r\nContent-Type: {}\r\n\r\n",
        disposition("form-data; name=\"file\"", &name),
        content_type(path)
    );
    let tail = format!("\r\n--{boundary}--\r\n");
    let length = head.len() as u64 + length + tail.len() as u64;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .header(header::CONTENT_LENGTH, length)
        .body(between(head, file, tail))
        .unwrap())
}

/// The file alone, to be saved (`GET /jobs/{id}/file`): its type from its extension, its name as an attachment.
pub(super) async fn attachment(path: &Path) -> Result<Response, String> {
    let (file, length, name) = open(path).await?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type(path))
        .header(
            header::CONTENT_DISPOSITION,
            disposition("attachment", &name),
        )
        .header(header::CONTENT_LENGTH, length)
        .body(between(String::new(), file, String::new()))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_named_by_their_content() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff(b"\0\0\0\x20ftypisom"), Some("mp4"));
        assert_eq!(sniff(b"hello"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn names_survive_the_header() {
        assert_eq!(
            disposition("form-data; name=\"file\"", "a b.wav"),
            "form-data; name=\"file\"; filename=\"a b.wav\"; filename*=UTF-8''a%20b.wav"
        );
        assert_eq!(
            disposition("attachment", "群\"1.xlsx"),
            "attachment; filename=\"群_1.xlsx\"; filename*=UTF-8''%E7%BE%A4%221.xlsx"
        );
    }

    #[test]
    fn content_types_follow_the_extension() {
        assert_eq!(content_type(Path::new("a.wav")), "audio/wav");
        assert_eq!(
            content_type(Path::new("a.json")),
            "application/json; charset=utf-8"
        );
        assert_eq!(content_type(Path::new("a.csv")), "text/csv; charset=utf-8");
        assert_eq!(
            content_type(Path::new("a.xlsx")),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
        assert_eq!(content_type(Path::new("a")), "application/octet-stream");
    }

    #[test]
    fn the_written_file_comes_from_the_result_or_out() {
        let doc = json!({ "success": true, "data": { "out": "/x/sessions.json" } });
        assert!(written_file(&doc, Some("out"), "/x").ends_with("sessions.json"));
        assert!(written_file(&doc, None, "/x/m.xlsx").ends_with("m.xlsx"));
        assert!(written_file(&json!({}), Some("path"), "rel.wav").is_absolute());
    }
}
