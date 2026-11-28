//! `?return=file`：命令写出的文件与 JSON 结果一起放在回答里送回（`multipart/form-data`，含 `result` 和 `file`
//! 两部分）；作业的文件由 [`attachment`] 单独发送。

use std::path::{Path, PathBuf};

use axum::body::{Body, Bytes};
use axum::http::{header, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

use super::endpoints::{find_param, flag_value, texts, Endpoint, Params};

/// 只写出一个文件的命令，及其默认的 `--out` 和结果中保存写出的路径的字段。
///
/// 默认的 `--out` 在请求没有给 `--out` 时使用，位于请求自己的文件夹里；为空时就是这个文件夹，由命令给文件取名。
/// 字段为 `None` 时，文件就是 `--out` 本身。
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

/// 命令 `name` 在 [`FILE_COMMANDS`] 中的默认 `--out` 和路径字段；不只写出一个文件的命令为 `None`。
pub(super) fn file_command(name: &str) -> Option<(&'static str, Option<&'static str>)> {
    FILE_COMMANDS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, out, field)| (*out, *field))
}

/// 命令运行之前说明 `?return=file` 为什么不能发送 `command` 用这些参数写出的东西：`export messages`
/// 导出多种格式时写出一个文件夹，除非用 `zip` 打包。能发送时为 `None`。
pub(super) fn not_one_file(
    command: &str,
    endpoint: &Endpoint,
    params: &Params,
) -> Option<&'static str> {
    if command != "export messages" {
        return None;
    }
    let mut formats: Vec<String> = Vec::new();
    let mut zip = false;
    for (key, values) in params {
        let Some(param) = find_param(endpoint, key) else {
            continue;
        };
        match param.name.as_str() {
            // 命令行上也可以用逗号分隔
            "format" => {
                let named = texts(param, values);
                for format in named
                    .iter()
                    .flat_map(|text| text.split(','))
                    .map(|f| f.trim().to_ascii_lowercase())
                    .filter(|f| !f.is_empty())
                {
                    if !formats.contains(&format) {
                        formats.push(format);
                    }
                }
            }
            "zip" => zip |= values.iter().any(|v| flag_value(v) == Some(true)),
            _ => {}
        }
    }
    (formats.len() > 1 && !zip).then_some(
        "several formats are written to a folder, which return=file cannot send: add zip=true to pack them into \
         one zip",
    )
}

/// 成功运行写出的文件：结果中的路径字段，没有时是 `--out`；相对路径按服务的工作目录解析，
/// 命令与服务使用同一个工作目录。
pub(super) fn written_file(doc: &Value, field: Option<&str>, out: &str) -> PathBuf {
    let path = field
        .and_then(|f| doc["data"][f].as_str())
        .filter(|p| !p.is_empty())
        .unwrap_or(out);
    std::path::absolute(path).unwrap_or_else(|_| PathBuf::from(path))
}

/// 服务端取名的、没有扩展名的文件（如 `image`），按内容加上扩展名；认不出内容或改名失败时保持原名。
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

/// 按开头的字节认出几种媒体格式，返回扩展名。
fn sniff(head: &[u8]) -> Option<&'static str> {
    if let Some(picture) = weflow_core::image::ImageFormat::of(head) {
        return Some(picture.extension().trim_start_matches('.'));
    }
    let at = |i: usize, magic: &[u8]| head.get(i..i + magic.len()) == Some(magic);
    Some(if at(0, b"RIFF") && at(8, b"WAVE") {
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

/// 按扩展名确定的 `Content-Type`，文本格式加上 `charset=utf-8`。
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

/// `<kind>; filename="…"`（按浏览器读取的方式写成 UTF-8，引号、反斜杠和控制字符换成 `_`），再加上 RFC 5987 的
/// `filename*`。头的值里不能有控制字符，而 Linux 的文件名里可能有。
fn disposition(kind: &str, name: &str) -> String {
    let plain: String = name
        .chars()
        .map(|c| match c {
            '"' | '\\' => '_',
            c if c.is_control() => '_',
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

/// 打开 `path` 处的文件，连同它的长度和文件名一起返回；打不开或不是文件时，`Err` 是说明。
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

/// 先发送 `head`，再边读边发送磁盘上的文件，最后发送 `tail`。请求自己的文件夹 `folder` 跟着回答走：
/// 文件发完并关闭后删除，客户端提前断开时也删除。
fn between(
    head: String,
    mut file: tokio::fs::File,
    tail: String,
    folder: Option<super::RequestFolder>,
) -> Body {
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
        // 先关闭文件，再删除它所在的文件夹
        drop(file);
        drop(folder);
        if !tail.is_empty() {
            yield Ok(Bytes::from(tail));
        }
    })
}

/// `multipart/form-data` 的回答：先是 `{"success": true, "data": {"path": …}}`，然后是边读边发送的文件；
/// 请求自己的文件夹 `folder` 在文件发完后删除。
pub(super) async fn multipart(
    path: &Path,
    folder: Option<super::RequestFolder>,
) -> Result<Response, String> {
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
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .header(header::CONTENT_LENGTH, length)
        .body(between(head, file, tail, folder))
        .map_err(|e| format!("cannot answer with {}: {e}", path.display()))
}

/// 只发送文件，供客户端保存（`GET /jobs/{id}/file`）：类型按扩展名确定，文件名作为附件的名字。
pub(super) async fn attachment(path: &Path) -> Result<Response, String> {
    let (file, length, name) = open(path).await?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type(path))
        .header(
            header::CONTENT_DISPOSITION,
            disposition("attachment", &name),
        )
        .header(header::CONTENT_LENGTH, length)
        .body(between(String::new(), file, String::new(), None))
        .map_err(|e| format!("cannot answer with {}: {e}", path.display()))
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
        let value = disposition("attachment", "a\tb\u{1}\u{7f}\r\n.txt");
        assert!(
            value.starts_with("attachment; filename=\"a_b____.txt\""),
            "{value}"
        );
        assert!(header::HeaderValue::from_str(&value).is_ok(), "{value}");
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
    fn several_formats_need_a_zip_to_be_one_file() {
        use crate::serve_api::endpoints::Endpoints;
        use crate::serve_api::tests::params;
        let endpoints = Endpoints::new();
        let messages = endpoints.0.get("export messages").unwrap();
        let refused = |pairs: &[(&str, Value)]| {
            not_one_file("export messages", messages, &params(pairs)).is_some()
        };
        assert!(!refused(&[]));
        assert!(!refused(&[("format", json!("html"))]));
        assert!(!refused(&[("format", json!("html,HTML, "))]));
        assert!(refused(&[("format", json!(["html", "txt"]))]));
        assert!(refused(&[
            ("format", json!("html")),
            ("format", json!("txt"))
        ]));
        assert!(refused(&[("format", json!("html,txt"))]));
        assert!(refused(&[
            ("format", json!(["html", "txt"])),
            ("zip", json!("false"))
        ]));
        assert!(!refused(&[
            ("format", json!(["html", "txt"])),
            ("zip", json!(true))
        ]));
        let sessions = endpoints.0.get("export sessions").unwrap();
        let more = params(&[("format", json!(["json", "csv"]))]);
        assert!(not_one_file("export sessions", sessions, &more).is_none());
    }

    #[test]
    fn the_written_file_comes_from_the_result_or_out() {
        let doc = json!({ "success": true, "data": { "out": "/x/sessions.json" } });
        assert!(written_file(&doc, Some("out"), "/x").ends_with("sessions.json"));
        assert!(written_file(&doc, None, "/x/m.xlsx").ends_with("m.xlsx"));
        assert!(written_file(&json!({}), Some("path"), "rel.wav").is_absolute());
    }
}
