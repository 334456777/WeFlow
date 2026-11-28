//! SQL 导出。

use std::path::Path;

use anyhow::Result;

use crate::message::kind::message_type_name_in;
use crate::message::xml::MsgXml;
use crate::message::*;

use super::format::sql_escape;
use super::{write_bytes, Exporter};

impl Exporter<'_, '_> {
    // ───────────────────────────────── SQL 导出 ─────────────────────────────────

    /// SQL 导出的文件头：建 `messages` 表的语句。
    pub(super) const SQL_HEADER: &'static str = "CREATE TABLE IF NOT EXISTS \"messages\" (\n\tid INTEGER PRIMARY KEY AUTOINCREMENT,\n\tsession_id TEXT,\n\tlocal_id INTEGER,\n\tserver_id TEXT,\n\tcreate_time INTEGER,\n\tsender TEXT,\n\tis_send INTEGER,\n\tlocal_type INTEGER,\n\ttype_name TEXT,\n\tcontent TEXT\n);\n\n";

    /// 把 `msg` 的 SQL `INSERT` 语句追加到 `out`。
    pub(super) fn write_sql_insert(&mut self, msg: &ExportMsg, out: &mut Vec<u8>) {
        use std::io::Write;
        let x = MsgXml::of_export(msg);
        let (content, _) = self.plain_row_content(msg, &x);
        // 写进 Vec 不会失败
        let _ = writeln!(
            out,
            "INSERT INTO \"messages\" (session_id, local_id, server_id, create_time, sender, is_send, local_type, type_name, content) VALUES ('{}', {}, {}, {}, '{}', {}, {}, '{}', '{}');",
            sql_escape(&self.session.id),
            msg.local_id,
            msg.platform_message_id().map(|s| format!("'{s}'")).unwrap_or_else(|| "NULL".into()),
            msg.create_time,
            sql_escape(&msg.sender_username),
            if msg.is_send { 1 } else { 0 },
            msg.local_type,
            sql_escape(message_type_name_in(msg.local_type, &x)),
            sql_escape(&content),
        );
    }

    /// 整个 SQL 导出的文字：文件头加上每条消息的 `INSERT`。
    pub(super) fn render_sql(&mut self, msgs: &[ExportMsg]) -> String {
        let mut sql = Vec::from(Self::SQL_HEADER);
        for msg in msgs {
            self.write_sql_insert(msg, &mut sql);
        }
        // 各条 INSERT 都由字符串写成，字节一定是 UTF-8
        String::from_utf8(sql).unwrap_or_default()
    }

    /// 把 `msgs` 写成 SQL 文件 `out`：先建表，再逐条插入。
    ///
    /// # Errors
    ///
    /// 无法写入文件时返回错误。
    pub fn write_sql(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        let sql = self.render_sql(msgs);
        write_bytes(out, sql.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use crate::export_msg::tests::with_exporter;

    #[test]
    fn sql_inserts() {
        let sql = with_exporter(false, |ex, msgs| ex.render_sql(msgs));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS \"messages\""));
        assert_eq!(sql.matches("INSERT INTO").count(), 5);
        assert!(sql.contains("hello \"world\", ok"));
    }
}
