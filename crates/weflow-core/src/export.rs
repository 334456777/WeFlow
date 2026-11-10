use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::locale::{self, Lang};

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

/// A list of objects (`export sessions`, `export contacts`) as a table (#659): the columns are the rows' keys in the
/// order they first appear, and a row has `Null` in the columns it has no key for. What is not a list of objects
/// gives no columns and no rows.
pub fn list_table(list: &Value) -> (Vec<String>, Vec<Vec<Value>>) {
    let objects: Vec<&serde_json::Map<String, Value>> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .collect();
    let mut columns: Vec<String> = Vec::new();
    for object in &objects {
        for key in object.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }
    let rows = objects
        .iter()
        .map(|object| {
            columns
                .iter()
                .map(|column| object.get(column).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    (columns, rows)
}

/// A cell as text: a string as it is, nothing for `Null`, anything else as JSON.
fn cell_text(value: &Value) -> std::borrow::Cow<'_, str> {
    match value {
        Value::String(text) => text.into(),
        Value::Null => "".into(),
        other => other.to_string().into(),
    }
}

/// A table as an HTML page: `title` as the heading, the header row, then a line for each row. Every cell is text,
/// so an address in a cell is shown and not loaded.
pub fn export_table_html(
    title: &str,
    headers: &[impl AsRef<str>],
    rows: &[Vec<Value>],
    out: &Path,
) -> Result<()> {
    write_file(out, &table_html(title, headers, rows))
}

fn table_html(title: &str, headers: &[impl AsRef<str>], rows: &[Vec<Value>]) -> String {
    let title = html_escape(title);
    let mut html = format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>{title}</title><style>
body{{font-family:-apple-system,BlinkMacSystemFont,sans-serif;margin:20px}}
table{{border-collapse:collapse}}
th,td{{border:1px solid #ddd;padding:4px 8px;text-align:left;vertical-align:top;white-space:pre-wrap}}
th{{background:#f5f5f5;position:sticky;top:0}}
</style></head><body><h2>{title}</h2><table><thead><tr>"#
    );
    for header in headers {
        html.push_str(&format!("<th>{}</th>", html_escape(header.as_ref())));
    }
    html.push_str("</tr></thead><tbody>\n");
    for row in rows {
        html.push_str("<tr>");
        for cell in row {
            html.push_str(&format!("<td>{}</td>", html_escape(&cell_text(cell))));
        }
        html.push_str("</tr>\n");
    }
    html.push_str("</tbody></table></body></html>\n");
    html
}

/// A table as SQL in SQLite's dialect: `CREATE TABLE` with a column for each header, then an `INSERT` for each row.
/// A column that holds only whole numbers and booleans is `INTEGER` (a boolean is 1 or 0), one that holds only
/// numbers and booleans `REAL`, any other `TEXT`; `Null` is `NULL`. A table without columns is a comment saying so.
pub fn export_table_sql(
    table: &str,
    headers: &[impl AsRef<str>],
    rows: &[Vec<Value>],
    out: &Path,
) -> Result<()> {
    write_file(out, &table_sql(table, headers, rows))
}

fn table_sql(table: &str, headers: &[impl AsRef<str>], rows: &[Vec<Value>]) -> String {
    let quoted = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
    let table = quoted(table);
    if headers.is_empty() {
        return format!("-- {table}: nothing to export\n");
    }
    let types: Vec<SqlType> = (0..headers.len())
        .map(|column| SqlType::of(rows.iter().filter_map(|row| row.get(column))))
        .collect();
    let names: Vec<String> = headers.iter().map(|h| quoted(h.as_ref())).collect();
    let columns = names
        .iter()
        .zip(&types)
        .map(|(name, kind)| format!("\t{name} {}", kind.name()))
        .collect::<Vec<_>>()
        .join(",\n");
    let mut sql = format!("CREATE TABLE IF NOT EXISTS {table} (\n{columns}\n);\n\n");
    let names = names.join(", ");
    for row in rows {
        let values = row
            .iter()
            .zip(&types)
            .map(|(value, kind)| kind.literal(value))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(
            "INSERT INTO {table} ({names}) VALUES ({values});\n"
        ));
    }
    sql
}

/// The type of a column of [`table_sql`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SqlType {
    Integer,
    Real,
    Text,
}

impl SqlType {
    /// The type that holds every one of `values`; `Text` when there is none but `Null`.
    fn of<'a>(values: impl Iterator<Item = &'a Value>) -> Self {
        let mut found = None;
        for value in values {
            match value {
                Value::Null => {}
                Value::Bool(_) => found = found.or(Some(Self::Integer)),
                Value::Number(n) if n.is_i64() || n.is_u64() => {
                    found = found.or(Some(Self::Integer));
                }
                Value::Number(_) => found = Some(Self::Real),
                _ => return Self::Text,
            }
        }
        found.unwrap_or(Self::Text)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Integer => "INTEGER",
            Self::Real => "REAL",
            Self::Text => "TEXT",
        }
    }

    /// `value` as a literal of a column of this type.
    fn literal(self, value: &Value) -> String {
        match (value, self) {
            (Value::Null, _) => "NULL".into(),
            (Value::Bool(yes), Self::Integer | Self::Real) => u8::from(*yes).to_string(),
            (Value::Number(n), Self::Integer | Self::Real) => n.to_string(),
            (other, _) => format!("'{}'", sql_escape(&cell_text(other))),
        }
    }
}

/// A table as a worksheet: the bold header, then a line for each row.
pub fn export_table_excel(
    headers: &[impl AsRef<str>],
    rows: &[Vec<Value>],
    out: &Path,
) -> Result<()> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let worksheet = workbook.add_worksheet();
    let bold = rust_xlsxwriter::Format::new().set_bold();
    for (col, header) in headers.iter().enumerate() {
        worksheet.write_string_with_format(0, u16::try_from(col)?, header.as_ref(), &bold)?;
    }
    for (r, row) in rows.iter().enumerate() {
        for (col, value) in row.iter().enumerate() {
            let (r, col) = (u32::try_from(r + 1)?, u16::try_from(col)?);
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

/// `YYYY-MM-DD HH:MM:SS` in the machine's local time zone, like WeFlow's desktop app.
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

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("write {}", path.display()))
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
    fn a_list_becomes_a_table_of_its_rows_keys() {
        let list = json!([
            { "username": "wxid_bob", "unread": 2 },
            { "username": "room@chatroom", "name": "The room" },
            "not an object",
        ]);
        let (columns, rows) = list_table(&list);
        // serde_json keeps an object's keys sorted, or in the order written with `preserve_order`
        let at = |name: &str| columns.iter().position(|c| c == name).unwrap();
        assert_eq!(columns.len(), 3);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][at("username")], json!("wxid_bob"));
        assert_eq!(rows[0][at("unread")], json!(2));
        assert_eq!(rows[0][at("name")], Value::Null);
        assert_eq!(rows[1][at("name")], json!("The room"));
        assert_eq!(rows[1][at("unread")], Value::Null);
        for not_a_list in [json!({ "a": 1 }), json!("text"), json!([])] {
            assert_eq!(list_table(&not_a_list), (Vec::new(), Vec::new()));
        }
    }

    #[test]
    fn a_table_as_html_escapes_every_cell() {
        let html = table_html(
            "a<b",
            &["name", "n"],
            &[
                vec![json!("<img src=x>"), json!(3)],
                vec![Value::Null, json!([1, 2])],
            ],
        );
        assert!(html.contains("<h2>a&lt;b</h2>"), "{html}");
        assert!(html.contains("<th>name</th><th>n</th>"), "{html}");
        assert!(
            html.contains("<tr><td>&lt;img src=x&gt;</td><td>3</td></tr>"),
            "{html}"
        );
        assert!(html.contains("<tr><td></td><td>[1,2]</td></tr>"), "{html}");
        assert!(!html.contains("<img"), "{html}");
    }

    #[test]
    fn a_table_as_sql_types_its_columns_and_quotes_its_text() {
        let sql = table_sql(
            "the \"list\"",
            &["name", "count", "pinned", "ratio", "mixed", "empty"],
            &[
                vec![
                    json!("it's"),
                    json!(3),
                    json!(true),
                    json!(1),
                    json!(1),
                    Value::Null,
                ],
                vec![
                    Value::Null,
                    json!(4),
                    json!(false),
                    json!(0.5),
                    json!("x"),
                    Value::Null,
                ],
            ],
        );
        assert!(
            sql.starts_with("CREATE TABLE IF NOT EXISTS \"the \"\"list\"\"\" (\n"),
            "{sql}"
        );
        for column in [
            "\t\"name\" TEXT",
            "\t\"count\" INTEGER",
            "\t\"pinned\" INTEGER",
            "\t\"ratio\" REAL",
            "\t\"mixed\" TEXT",
            "\t\"empty\" TEXT",
        ] {
            assert!(sql.contains(column), "{column} in {sql}");
        }
        assert!(
            sql.contains("VALUES ('it''s', 3, 1, 1, '1', NULL);\n"),
            "{sql}"
        );
        assert!(
            sql.contains("VALUES (NULL, 4, 0, 0.5, 'x', NULL);\n"),
            "{sql}"
        );
        assert_eq!(
            table_sql("t", &[] as &[&str], &[]),
            "-- \"t\": nothing to export\n"
        );
    }

    #[test]
    fn format_timestamp_works() {
        let ts = 1700000000i64;
        let s = format_timestamp(ts);
        assert!(s.contains("2023"));
        assert!(s.contains(':'));
    }
}
