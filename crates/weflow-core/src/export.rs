use std::collections::HashMap;
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

// ── TXT export ───────────────────────────────────────────────────────────────

/// Export messages to human-readable TXT.
/// Format per message:
///   YYYY-MM-DD HH:MM:SS 'Nickname'
///
///   content
///
pub fn export_txt(
    messages: &[TxtMessage],
    nickname_map: &HashMap<String, String>,
    out: &Path,
) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let file = fs::File::create(out).with_context(|| format!("write {}", out.display()))?;
    let mut w = std::io::BufWriter::new(file);
    for msg in messages {
        let content: String = match msg.local_type {
            1 => {
                let t = extract_text_after_sender(&msg.content);
                if t.is_empty() {
                    continue;
                }
                t
            }
            3 => locale::tr("[Image]", "[图片]").to_string(),
            34 => locale::tr("[Voice]", "[语音]").to_string(),
            43 => locale::tr("[Video]", "[视频]").to_string(),
            47 => locale::tr("[Sticker]", "[表情]").to_string(),
            49 => crate::message::parse_message_content(
                &msg.content,
                msg.local_type,
                None,
                Some(&msg.sender),
                None,
            )
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| locale::tr("[Link/File]", "[链接/文件]").to_string()),
            10000 => {
                let t = extract_text_after_sender(&msg.content);
                if t.is_empty() {
                    continue;
                }
                format!("[{}: {t}]", locale::tr("System", "系统"))
            }
            // 引用消息、聊天记录、小程序、文件、转账、拍一拍等：localType 高 32 位带有子类型，
            // 交给与 JSON / ChatLab 相同的解析逻辑，避免整类消息被丢掉。
            _ => match crate::message::parse_message_content(
                &msg.content,
                msg.local_type,
                None,
                Some(&msg.sender),
                None,
            ) {
                Some(t) if !t.trim().is_empty() => t,
                _ => continue,
            },
        };
        let nickname = nickname_map.get(&msg.sender).unwrap_or(&msg.sender);
        let dt = format_timestamp(msg.create_time);
        write!(w, "{dt} '{nickname}'\n\n{content}\n\n")
            .with_context(|| format!("write {}", out.display()))?;
    }
    w.flush()
        .with_context(|| format!("write {}", out.display()))
}

/// One message of the TXT export: just what its layout prints.
pub struct TxtMessage {
    pub create_time: i64,
    pub sender: String,
    pub local_type: i64,
    pub content: String,
}

impl TxtMessage {
    /// From a raw message row; every kind is kept (kinds the layout cannot render are skipped when printing).
    pub fn from_row(row: &Value) -> Option<Self> {
        let int = |key: &str, default: i64| {
            row.get(key)
                .and_then(|v| {
                    v.as_str()
                        .and_then(|s| s.parse::<i64>().ok())
                        .or_else(|| v.as_i64())
                })
                .unwrap_or(default)
        };
        let local_type = int("local_type", 1);
        let content = match local_type {
            3 | 34 | 43 | 47 => String::new(),
            _ => decode_wcdb_content(row),
        };
        Some(Self {
            create_time: int("create_time", 0),
            sender: row
                .get("sender_username")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            local_type,
            content,
        })
    }
}

fn decode_wcdb_content(msg: &Value) -> String {
    let ct_flag = msg
        .get("WCDB_CT_message_content")
        .and_then(Value::as_str)
        .unwrap_or("0");
    let raw = msg
        .get("message_content")
        .and_then(Value::as_str)
        .unwrap_or("");

    if ct_flag == "4" && !raw.is_empty() {
        decompress_zstd_hex(raw).unwrap_or_else(|| raw.to_string())
    } else {
        raw.to_string()
    }
}

fn extract_text_after_sender(raw: &str) -> String {
    // WCDB text messages start with "sender_wxid:\ncontent"
    if let Some(nl) = raw.find('\n') {
        let prefix = &raw[..nl];
        if prefix.ends_with(':') && prefix.len() < 80 && !prefix.contains('\n') {
            return raw[nl + 1..].trim().to_string();
        }
    }
    raw.trim().to_string()
}

fn decompress_zstd_hex(hex: &str) -> Option<String> {
    let bytes = hex_decode(hex)?;
    let decoded = zstd::decode_all(bytes.as_slice()).ok()?;
    String::from_utf8(decoded).ok()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
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

    #[test]
    fn txt_keeps_quote_and_other_app_messages() {
        let quote = "wxid_a:\n<msg><appmsg><title>老师我又进步了</title><type>57</type>\
            <refermsg><type>1</type><displayname>菠萝</displayname>\
            <content>外星人要变异了</content></refermsg></appmsg></msg>";
        let forward = "wxid_a:\n<msg><appmsg><title>聊天记录</title><type>19</type></appmsg></msg>";
        let mk = |local_type: i64, content: &str| TxtMessage {
            create_time: 1_700_000_000,
            sender: "wxid_a".into(),
            local_type,
            content: content.into(),
        };
        let msgs = vec![
            mk(244_813_135_921, quote),
            mk(81_604_378_673, forward),
            mk(999, ""),
        ];
        let out = std::env::temp_dir().join("weflow_txt_quote_test.txt");
        export_txt(&msgs, &HashMap::new(), &out).unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&out);
        assert!(
            text.contains("老师我又进步了[引用 菠萝：外星人要变异了]"),
            "{text}"
        );
        assert!(text.contains("聊天记录"), "{text}");
        assert_eq!(text.matches(" 'wxid_a'").count(), 2, "{text}");
    }
}
