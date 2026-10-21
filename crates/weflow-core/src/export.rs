use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::locale::{self, Lang};

pub fn export_html(
    title: &str,
    messages: &Value,
    contacts_value: &Value,
    out: &Path,
) -> Result<()> {
    let contact_map = build_contact_map(contacts_value);
    let items = messages.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let mut body = String::new();
    for (idx, msg) in items.iter().enumerate() {
        let sender = msg
            .get("sender")
            .or_else(|| msg.get("from"))
            .or_else(|| msg.get("talker"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let nickname = contact_map
            .get(sender)
            .cloned()
            .unwrap_or_else(|| sender.to_string());
        let content = msg
            .get("content")
            .or_else(|| msg.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let time = msg
            .get("createTime")
            .or_else(|| msg.get("create_time"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let msg_type = msg
            .get("type")
            .or_else(|| msg.get("msgType"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let type_label = message_type_label(msg_type);
        let is_self = msg
            .get("isSelf")
            .or_else(|| msg.get("is_sent"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let align = if is_self { "right" } else { "left" };
        let time_str = format_timestamp(time);
        let escaped = html_escape(content);
        body.push_str(&format!(
            r#"<div class="msg {align}"><div class="sender">{nickname}</div><div class="bubble"><span class="type">{type_label}</span>{escaped}</div><div class="time">{time_str}</div></div>"#,
        ));
        if idx > 0 && idx % 100 == 0 {
            body.push('\n');
        }
    }

    let html = format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>{title}</title><style>
body{{font-family:-apple-system,BlinkMacSystemFont,sans-serif;max-width:800px;margin:0 auto;padding:20px;background:#f5f5f5}}
.msg{{margin:10px 0;padding:10px 15px;border-radius:12px;max-width:70%;clear:both}}
.msg.left{{float:left;background:white;box-shadow:0 1px 2px rgba(0,0,0,.1)}}
.msg.right{{float:right;background:#95ec69;box-shadow:0 1px 2px rgba(0,0,0,.1)}}
.sender{{font-size:12px;color:#888;margin-bottom:4px}}
.bubble{{word-wrap:break-word;white-space:pre-wrap}}
.type{{display:inline-block;font-size:10px;background:#eee;padding:1px 4px;border-radius:3px;margin-right:4px}}
.time{{font-size:11px;color:#aaa;margin-top:4px;text-align:right}}
.container{{overflow:hidden}}
</style></head><body><h2>{title}</h2><div class="container">{body}</div></body></html>"#,
    );
    write_file(out, &html)
}

pub fn export_excel(messages: &Value, out: &Path) -> Result<()> {
    let items = messages.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let worksheet = workbook.add_worksheet();
    let header_format = rust_xlsxwriter::Format::new().set_bold();
    let headers = match locale::current() {
        Lang::En => ["No.", "Time", "Sender", "Message type", "Content"],
        Lang::Zh => ["序号", "时间", "发送者", "消息类型", "内容"],
    };
    for (col, header) in headers.iter().enumerate() {
        worksheet.write_string_with_format(0, col as u16, *header, &header_format)?;
    }
    for (row, msg) in items.iter().enumerate() {
        let r = (row + 1) as u32;
        worksheet.write_number(r, 0, (row + 1) as f64)?;
        let time = msg
            .get("createTime")
            .or_else(|| msg.get("create_time"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        worksheet.write_string(r, 1, format_timestamp(time))?;
        let sender = msg
            .get("sender")
            .or_else(|| msg.get("from"))
            .or_else(|| msg.get("talker"))
            .and_then(Value::as_str)
            .unwrap_or("");
        worksheet.write_string(r, 2, sender)?;
        let msg_type = msg
            .get("type")
            .or_else(|| msg.get("msgType"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        worksheet.write_string(r, 3, message_type_label(msg_type))?;
        let content = msg
            .get("content")
            .or_else(|| msg.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        worksheet.write_string(r, 4, content)?;
    }
    workbook.save(out)?;
    Ok(())
}

/// The footprint (`export footprint`) as a timeline, for CSV and Excel (#315): the column names, and a row for each
/// private-chat segment and each message that @-mentions me, oldest first. `names` gives conversations' and senders'
/// names by username.
pub fn footprint_timeline(
    data: &Value,
    names: &std::collections::HashMap<String, String>,
) -> (Vec<&'static str>, Vec<Vec<Value>>) {
    let zh = locale::current() == Lang::Zh;
    let columns: [&'static str; 9] = if zh {
        [
            "时间",
            "类型",
            "会话",
            "发送者",
            "收到",
            "发出",
            "已回复",
            "内容",
            "会话 ID",
        ]
    } else {
        [
            "Time",
            "Kind",
            "Conversation",
            "Sender",
            "Received",
            "Sent",
            "Replied",
            "Content",
            "Conversation ID",
        ]
    };
    let name = |id: &str| {
        names
            .get(id)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| id.to_string())
    };
    let row = |values: [Value; 9]| -> Vec<Value> { values.into() };
    let text = |v: &Value, key: &str| v[key].as_str().unwrap_or("").to_string();
    let mut rows: Vec<(i64, Vec<Value>)> = Vec::new();
    for s in data["private_segments"].as_array().into_iter().flatten() {
        let id = text(s, "session_id");
        let at = s["start_ts"].as_i64().unwrap_or(0);
        let yes = s["replied"].as_bool().unwrap_or(false);
        let yes = match (zh, yes) {
            (true, true) => "是",
            (true, false) => "否",
            (false, true) => "yes",
            (false, false) => "no",
        };
        rows.push((
            at,
            row([
                json!(format_timestamp(at)),
                json!(if zh { "私聊" } else { "private chat" }),
                json!(name(&id)),
                json!(""),
                s["incoming_count"].clone(),
                s["outgoing_count"].clone(),
                json!(yes),
                json!(""),
                json!(id),
            ]),
        ));
    }
    for m in data["mentions"].as_array().into_iter().flatten() {
        let id = text(m, "session_id");
        let from = text(m, "sender_username");
        let at = m["create_time"].as_i64().unwrap_or(0);
        // a group message is stored as "<sender>:\n<text>"
        let body = text(m, "message_content");
        let body = body
            .strip_prefix(&format!("{from}:\n"))
            .unwrap_or(&body)
            .to_string();
        rows.push((
            at,
            row([
                json!(format_timestamp(at)),
                json!(if zh { "@我" } else { "mention" }),
                json!(name(&id)),
                json!(name(&from)),
                json!(""),
                json!(""),
                json!(""),
                json!(body),
                json!(id),
            ]),
        ));
    }
    rows.sort_by_key(|(at, _)| *at);
    (
        columns.to_vec(),
        rows.into_iter().map(|(_, row)| row).collect(),
    )
}

/// A table as a worksheet: the bold header, then a line for each row.
pub fn export_table_excel(headers: &[&str], rows: &[Vec<Value>], out: &Path) -> Result<()> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let worksheet = workbook.add_worksheet();
    let bold = rust_xlsxwriter::Format::new().set_bold();
    for (col, header) in headers.iter().enumerate() {
        worksheet.write_string_with_format(0, col as u16, *header, &bold)?;
    }
    for (r, row) in rows.iter().enumerate() {
        for (col, value) in row.iter().enumerate() {
            let (r, col) = (r as u32 + 1, col as u16);
            match value {
                Value::Number(n) => worksheet.write_number(r, col, n.as_f64().unwrap_or(0.0))?,
                Value::String(s) => worksheet.write_string(r, col, s)?,
                Value::Null => worksheet,
                other => worksheet.write_string(r, col, other.to_string())?,
            };
        }
    }
    workbook.save(out)?;
    Ok(())
}

pub fn export_sql(table_name: &str, messages: &Value, out: &Path) -> Result<()> {
    let items = messages.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let mut sql = String::new();
    sql.push_str(&format!(
        "CREATE TABLE IF NOT EXISTS \"{table_name}\" (\n\
         \tid INTEGER PRIMARY KEY AUTOINCREMENT,\n\
         \tsession_id TEXT,\n\
         \tlocal_id INTEGER,\n\
         \tcreate_time INTEGER,\n\
         \tsender TEXT,\n\
         \ttype INTEGER,\n\
         \tcontent TEXT\n\
         );\n\n"
    ));
    for msg in items {
        let session_id = sql_escape(
            msg.get("sessionId")
                .or_else(|| msg.get("session_id"))
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        let local_id = msg
            .get("localId")
            .or_else(|| msg.get("local_id"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let create_time = msg
            .get("createTime")
            .or_else(|| msg.get("create_time"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let sender = sql_escape(
            msg.get("sender")
                .or_else(|| msg.get("from"))
                .or_else(|| msg.get("talker"))
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        let msg_type = msg
            .get("type")
            .or_else(|| msg.get("msgType"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let content = sql_escape(
            msg.get("content")
                .or_else(|| msg.get("text"))
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        sql.push_str(&format!(
            "INSERT INTO \"{table_name}\" (session_id, local_id, create_time, sender, type, content) VALUES ('{session_id}', {local_id}, {create_time}, '{sender}', {msg_type}, '{content}');\n"
        ));
    }
    write_file(out, &sql)
}

pub fn export_chatlab(
    session_id: &str,
    messages: &Value,
    contacts: &Value,
    out: &Path,
) -> Result<()> {
    let contact_map = build_contact_map(contacts);
    let items = messages.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let chatlab_messages: Vec<Value> = items
        .iter()
        .map(|msg| {
            let sender = msg
                .get("sender")
                .or_else(|| msg.get("from"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let account_name = contact_map
                .get(sender)
                .cloned()
                .unwrap_or_else(|| sender.to_string());
            let msg_type = msg
                .get("type")
                .or_else(|| msg.get("msgType"))
                .and_then(Value::as_i64)
                .unwrap_or(1);
            let chatlab_type = match msg_type {
                1 => 0,      // TEXT
                3 => 1,      // IMAGE
                34 => 2,     // VOICE
                43 => 3,     // VIDEO
                47 => 5,     // EMOJI
                48 => 8,     // LOCATION
                49 => 7,     // LINK
                42 => 27,    // CONTACT
                50 => 23,    // CALL
                10000 => 80, // SYSTEM
                _ => 0,
            };
            let content = msg
                .get("content")
                .or_else(|| msg.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let timestamp = msg
                .get("createTime")
                .or_else(|| msg.get("create_time"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            json!({
                "sender": sender,
                "accountName": account_name,
                "timestamp": timestamp,
                "type": chatlab_type,
                "content": content
            })
        })
        .collect();

    let output = json!({
        "chatlab": {
            "version": "1.0",
            "exportedAt": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        },
        "meta": {
            "sessionId": session_id,
            "messageCount": chatlab_messages.len()
        },
        "members": contact_map.into_iter().map(|(k, v)| json!({"username": k, "nickname": v})).collect::<Vec<_>>(),
        "messages": chatlab_messages
    });
    let bytes = serde_json::to_vec_pretty(&output)?;
    write_bytes(out, &bytes)
}

pub fn export_weclone(wxid: &str, messages: &Value, contacts: &Value, out: &Path) -> Result<()> {
    let contact_map = build_contact_map(contacts);
    let items = messages.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let mut csv = String::from("talker,type,text\n");
    for msg in items {
        let sender = msg
            .get("sender")
            .or_else(|| msg.get("from"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let is_self = sender == wxid;
        let talker = if is_self {
            "self".to_string()
        } else {
            contact_map
                .get(sender)
                .cloned()
                .unwrap_or_else(|| sender.to_string())
        };
        let msg_type = msg
            .get("type")
            .or_else(|| msg.get("msgType"))
            .and_then(Value::as_i64)
            .unwrap_or(1);
        let type_name = match msg_type {
            1 => "text",
            3 => "image",
            47 => "sticker",
            43 => "video",
            34 => "voice",
            _ => "text",
        };
        let content = msg
            .get("content")
            .or_else(|| msg.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        csv.push_str(&format!(
            "{},{},{}\n",
            csv_escape(&talker),
            type_name,
            csv_escape(content)
        ));
    }
    write_file(out, &csv)
}

fn build_contact_map(contacts: &Value) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Some(items) = contacts.as_array() else {
        return map;
    };
    for c in items {
        let username = c
            .get("username")
            .or_else(|| c.get("userName"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let nickname = c
            .get("nickname")
            .or_else(|| c.get("alias"))
            .or_else(|| c.get("remark"))
            .and_then(Value::as_str)
            .unwrap_or(username.as_str())
            .to_string();
        if !username.is_empty() {
            map.insert(username, nickname);
        }
    }
    map
}

fn message_type_label(msg_type: i64) -> &'static str {
    message_type_label_in(locale::current(), msg_type)
}

fn message_type_label_in(lang: Lang, msg_type: i64) -> &'static str {
    let (en, zh) = match msg_type {
        1 => ("Text", "文本"),
        3 => ("Image", "图片"),
        34 => ("Voice", "语音"),
        43 => ("Video", "视频"),
        47 => ("Sticker", "表情"),
        49 => ("Link", "链接"),
        10000 => ("System", "系统"),
        _ => ("Other", "其他"),
    };
    match lang {
        Lang::En => en,
        Lang::Zh => zh,
    }
}

/// `YYYY-MM-DD HH:MM:SS` in the machine's local time zone, like the desktop app.
fn format_timestamp(ts: i64) -> String {
    if ts <= 0 {
        String::new()
    } else {
        crate::message::format_timestamp(ts)
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn sql_escape(s: &str) -> String {
    s.replace('\'', "''")
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("write {}", path.display()))
}

fn write_bytes(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(path, data).with_context(|| format!("write {}", path.display()))
}

/// Packs everything under `dir` into the zip `out` (#342), paths relative to `dir` with `/`; pictures, videos and
/// other files that are compressed already are stored as they are.
pub fn zip_folder(dir: &Path, out: &Path) -> Result<()> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    const STORED: &[&str] = &[
        "jpg", "jpeg", "png", "gif", "webp", "heic", "mp4", "mov", "mp3", "silk", "amr", "zip",
        "xlsx",
    ];
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(folder) = stack.pop() {
        for entry in fs::read_dir(&folder).with_context(|| format!("read {}", folder.display()))? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut zip = zip::ZipWriter::new(
        fs::File::create(out).with_context(|| format!("create {}", out.display()))?,
    );
    for path in files {
        let name = path
            .strip_prefix(dir)
            .unwrap_or(&path)
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let stored = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| STORED.contains(&e.to_ascii_lowercase().as_str()));
        let options = SimpleFileOptions::default()
            .compression_method(if stored {
                zip::CompressionMethod::Stored
            } else {
                zip::CompressionMethod::Deflated
            })
            .large_file(
                fs::metadata(&path)
                    .map(|m| m.len() >= u32::MAX as u64)
                    .unwrap_or(false),
            );
        zip.start_file(&name, options)
            .with_context(|| format!("zip {name}"))?;
        let mut file = fs::File::open(&path).with_context(|| format!("open {}", path.display()))?;
        std::io::copy(&mut file, &mut zip).with_context(|| format!("zip {name}"))?;
    }
    zip.finish()?.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escapes() {
        assert_eq!(html_escape("<b>hi</b>"), "&lt;b&gt;hi&lt;/b&gt;");
    }

    #[test]
    fn sql_escapes() {
        assert_eq!(sql_escape("it's"), "it''s");
    }

    #[test]
    fn the_footprint_timeline_has_a_row_per_segment_and_mention_oldest_first() {
        let data = json!({
            "private_segments": [
                { "session_id": "wxid_bob", "start_ts": 1_700_000_300, "incoming_count": 2, "outgoing_count": 0, "replied": false },
                { "session_id": "wxid_bob", "start_ts": 1_700_000_000, "incoming_count": 1, "outgoing_count": 1, "replied": true },
            ],
            "mentions": [
                { "session_id": "room@chatroom", "sender_username": "wxid_amy", "create_time": 1_700_000_100,
                  "message_content": "wxid_amy:\n@me look" },
            ],
        });
        let names = std::collections::HashMap::from([
            ("wxid_bob".to_string(), "Bob".to_string()),
            ("room@chatroom".to_string(), "The room".to_string()),
        ]);
        let (columns, rows) = footprint_timeline(&data, &names);
        assert_eq!(columns.len(), 9);
        assert!(rows.iter().all(|r| r.len() == columns.len()));
        // oldest first: the replied segment, the mention, the later segment
        let at: Vec<&str> = rows.iter().map(|r| r[8].as_str().unwrap()).collect();
        assert_eq!(at, ["wxid_bob", "room@chatroom", "wxid_bob"]);
        assert_eq!(
            (&rows[0][2], &rows[0][4], &rows[0][5]),
            (&json!("Bob"), &json!(1), &json!(1))
        );
        assert!(["是", "yes"].contains(&rows[0][6].as_str().unwrap()));
        assert!(["否", "no"].contains(&rows[2][6].as_str().unwrap()));
        // the mention: the group's and the sender's names (the sender unknown here: the username), the text alone
        assert_eq!(
            (&rows[1][2], &rows[1][3], &rows[1][7]),
            (&json!("The room"), &json!("wxid_amy"), &json!("@me look"))
        );
        assert_eq!(rows[1][4], json!(""));
    }

    #[test]
    fn csv_escapes() {
        assert_eq!(csv_escape("hello"), "hello");
        assert_eq!(csv_escape("he said,\"hello\""), "\"he said,\"\"hello\"\"\"");
    }

    #[test]
    fn format_timestamp_works() {
        let ts = 1700000000i64;
        let s = format_timestamp(ts);
        assert!(s.contains("2023"));
        assert!(s.contains(':'));
    }

    #[test]
    fn message_type_labels() {
        assert_eq!(message_type_label_in(Lang::En, 1), "Text");
        assert_eq!(message_type_label_in(Lang::En, 3), "Image");
        assert_eq!(message_type_label_in(Lang::En, 999), "Other");
        assert_eq!(message_type_label_in(Lang::Zh, 1), "文本");
        assert_eq!(message_type_label_in(Lang::Zh, 3), "图片");
        assert_eq!(message_type_label_in(Lang::Zh, 43), "视频");
    }
}
