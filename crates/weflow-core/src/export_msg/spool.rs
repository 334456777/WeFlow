//! 分段写出的导出：暂存文件、预先渲染的条目和发送者的统计。

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use super::format::write_pretty;

/// 导出中的发送者（按第一次出现的顺序）和各自发送的消息数（按小写的 ID 计）：成员列表需要的信息，逐条消息收集。
#[derive(Default)]
pub(super) struct SenderTally {
    pub(super) order: Vec<String>,
    seen: HashSet<String>,
    pub(super) counts: HashMap<String, i64>,
}

impl SenderTally {
    /// 记下 `sender` 发送的一条消息；`sender` 为空时只计数，不加进顺序。
    pub(super) fn add(&mut self, sender: &str) {
        if !sender.is_empty() && self.seen.insert(sender.to_string()) {
            self.order.push(sender.to_string());
        }
        *self.counts.entry(sender.to_lowercase()).or_insert(0) += 1;
    }
}

/// 导出的条目：读取消息时先写进输出文件旁边的暂存文件，最后复制到输出里，夹在需要总数的文件头和文件尾之间。
/// 丢弃时删除暂存文件。
pub(super) struct Spool {
    path: std::path::PathBuf,
    writer: Option<io::BufWriter<std::fs::File>>,
    entries: usize,
}

impl Spool {
    /// 在 `out` 旁边创建暂存文件 `<out>.part`。
    pub(super) fn create(out: &Path) -> Result<Self> {
        let mut name = out.as_os_str().to_os_string();
        name.push(".part");
        let path = std::path::PathBuf::from(name);
        let writer = super::create_output(&path)?;
        Ok(Self {
            path,
            writer: Some(writer),
            entries: 0,
        })
    }

    /// 追加一个由 `write` 写出的条目；不是第一个时先写 `separator`。
    pub(super) fn push_with(
        &mut self,
        separator: &str,
        write: impl FnOnce(&mut io::BufWriter<std::fs::File>) -> io::Result<()>,
    ) -> Result<()> {
        use std::io::Write;
        if let Some(w) = self.writer.as_mut() {
            if self.entries > 0 {
                w.write_all(separator.as_bytes())?;
            }
            write(w)?;
        }
        self.entries += 1;
        Ok(())
    }

    /// 追加一个条目；不是第一个时先写 `separator`。
    pub(super) fn push(&mut self, separator: &str, entry: &[u8]) -> Result<()> {
        use std::io::Write;
        self.push_with(separator, |w| w.write_all(entry))
    }

    /// 把 `entry` 美化输出成缩进 `indent` 个空格的一个 JSON 值（见 [`write_pretty`]）后追加；
    /// 不是第一个时先写 `separator`。
    pub(super) fn push_json(
        &mut self,
        separator: &str,
        entry: &impl Serialize,
        indent: usize,
    ) -> Result<()> {
        self.push_with(separator, |w| write_pretty(w, entry, indent))
    }

    /// 把到目前为止写进的全部内容复制到 `out`。
    pub(super) fn copy_to(&mut self, out: &mut impl io::Write) -> Result<()> {
        use std::io::Write;
        if let Some(mut w) = self.writer.take() {
            w.flush()?;
        }
        io::copy(&mut std::fs::File::open(&self.path)?, out)?;
        Ok(())
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        self.writer = None;
        let _ = std::fs::remove_file(&self.path);
    }
}

/// 各条目互不依赖的格式，一页条目可以在别的线程上先于写入渲染：
/// [`Exporter::render_entries`](super::Exporter::render_entries) 负责渲染，
/// [`Exporter::write_rendered`](super::Exporter::write_rendered) 按顺序写出。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryFormat {
    /// 纯文本 TXT。
    Txt,
    /// SQL 语句：建表，再逐条插入。
    Sql,
    /// ChatLab 的 JSON 文档。
    Chatlab,
    /// ChatLab 的 JSONL。
    ChatlabJsonl,
    /// 详细 JSON。不含 arkme-json：它的发送者表按消息中出现的顺序给发送者编号，条目之间互相依赖。
    Json,
}

impl EntryFormat {
    /// 渲染是否慢到值得放在读取线程上做。ChatLab 或详细 JSON 的条目要查名字、输出 JSON 对象，写入会跟不上读取；
    /// TXT 和 SQL 的条目足够快，写入跟得上，放到别处渲染只会让更多文字积压在途中（测量见 #14）。
    pub fn renders_ahead(self) -> bool {
        matches!(self, Self::Chatlab | Self::ChatlabJsonl | Self::Json)
    }

    /// 读取格式名（不分大小写）；不是这几种格式时为 `None`。
    pub fn parse(format: &str) -> Option<Self> {
        match format.to_ascii_lowercase().as_str() {
            "txt" => Some(Self::Txt),
            "sql" => Some(Self::Sql),
            "chatlab" => Some(Self::Chatlab),
            "chatlab-jsonl" => Some(Self::ChatlabJsonl),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// 先于写入渲染好的一页消息（见 [`Exporter::render_entries`](super::Exporter::render_entries)）。
#[derive(Debug, Default)]
pub struct RenderedEntries {
    /// 这一页的条目，一个接一个地放在一起，这样一页只需要几次分配，而不是每条消息一次或更多。
    pub(super) text: Vec<u8>,
    /// 每个条目在 `text` 中的结束位置，每条消息一个，按顺序排列。
    pub(super) ends: Vec<usize>,
    /// ChatLab：每条消息的发送者（成员列表按他们第一次出现的顺序排列）。
    pub senders: Vec<String>,
    /// ChatLab：转发的聊天记录里出现的人名，按顺序排列（他们也会成为成员）。
    pub record_names: Vec<String>,
    /// 详细 JSON：这一页中最新的 `create_time`（会话信息里写全部消息中最新的）。
    pub latest: Option<i64>,
}

impl RenderedEntries {
    /// 按顺序列出各条目。
    pub(super) fn entries(&self) -> impl Iterator<Item = &[u8]> {
        let starts = std::iter::once(0).chain(self.ends.iter().copied());
        starts
            .zip(&self.ends)
            .map(|(start, &end)| &self.text[start..end])
    }

    /// 结束上一个条目之后写进 `text` 的条目。
    pub(super) fn end_entry(&mut self) {
        self.ends.push(self.text.len());
    }
}
