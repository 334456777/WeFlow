//! Exports written a piece at a time: the spool file, entries rendered ahead and the tally of senders.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use super::format::write_pretty;

/// The senders of an export, in order of first appearance, and how many messages each sent (by lower-case id):
/// what the member lists need, collected message by message.
#[derive(Default)]
pub(super) struct SenderTally {
    pub(super) order: Vec<String>,
    seen: HashSet<String>,
    pub(super) counts: HashMap<String, i64>,
}

impl SenderTally {
    pub(super) fn add(&mut self, sender: &str) {
        if !sender.is_empty() && self.seen.insert(sender.to_string()) {
            self.order.push(sender.to_string());
        }
        *self.counts.entry(sender.to_lowercase()).or_insert(0) += 1;
    }
}

/// Entries of an export that are written to a file next to the output while the messages are read, then copied
/// into the output between a header and a footer that need the totals. Removed when dropped.
pub(super) struct Spool {
    path: std::path::PathBuf,
    writer: Option<std::io::BufWriter<std::fs::File>>,
    entries: usize,
}

impl Spool {
    pub(super) fn create(out: &Path) -> Result<Self> {
        if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut name = out.as_os_str().to_os_string();
        name.push(".part");
        let path = std::path::PathBuf::from(name);
        let file = std::fs::File::create(&path)?;
        Ok(Self {
            path,
            writer: Some(std::io::BufWriter::with_capacity(1 << 20, file)),
            entries: 0,
        })
    }

    /// Appends one entry, which `write` writes, preceded by `separator` unless it is the first.
    pub(super) fn push_with(
        &mut self,
        separator: &str,
        write: impl FnOnce(&mut std::io::BufWriter<std::fs::File>) -> io::Result<()>,
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

    /// Appends one entry, preceded by `separator` unless it is the first.
    pub(super) fn push(&mut self, separator: &str, entry: &[u8]) -> Result<()> {
        use std::io::Write;
        self.push_with(separator, |w| w.write_all(entry))
    }

    /// Appends `entry` pretty-printed as one JSON value nested `indent` spaces deep (see [`write_pretty`]),
    /// preceded by `separator` unless it is the first.
    pub(super) fn push_json(
        &mut self,
        separator: &str,
        entry: &impl Serialize,
        indent: usize,
    ) -> Result<()> {
        self.push_with(separator, |w| write_pretty(w, entry, indent))
    }

    /// Copies everything written so far to `out`.
    pub(super) fn copy_to(&mut self, out: &mut impl std::io::Write) -> Result<()> {
        use std::io::Write;
        if let Some(mut w) = self.writer.take() {
            w.flush()?;
        }
        std::io::copy(&mut std::fs::File::open(&self.path)?, out)?;
        Ok(())
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        self.writer = None;
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Formats whose entries do not depend on each other, so a page of them can be rendered on another thread ahead of
/// the writer: [`Exporter::render_entries`](super::Exporter::render_entries) renders,
/// [`Exporter::write_rendered`](super::Exporter::write_rendered) writes them in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryFormat {
    Txt,
    Sql,
    Chatlab,
    ChatlabJsonl,
    /// Detailed JSON. Not arkme-json: its sender table numbers the senders in the order the messages show them.
    Json,
}

impl EntryFormat {
    /// Whether rendering is slow enough to be worth doing on the reading threads. A ChatLab or detailed-JSON entry
    /// resolves names and prints a JSON object, which keeps the writer behind the readers; TXT and SQL entries are
    /// cheap enough that the writer keeps up, and rendering them elsewhere only holds more text in flight (measured
    /// in issue #14).
    pub fn renders_ahead(self) -> bool {
        matches!(self, Self::Chatlab | Self::ChatlabJsonl | Self::Json)
    }

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

/// A page of messages rendered ahead of writing (see [`Exporter::render_entries`](super::Exporter::render_entries)).
#[derive(Debug, Default)]
pub struct RenderedEntries {
    /// The entries of the page, one after another, so a page takes a few allocations rather than one or more per
    /// message.
    pub(super) text: Vec<u8>,
    /// Where each entry ends in `text`, one per message, in order.
    pub(super) ends: Vec<usize>,
    /// ChatLab: the sender of each message (the member list follows their first appearance).
    pub senders: Vec<String>,
    /// ChatLab: the people named inside forwarded chat histories, in order (they become members too).
    pub record_names: Vec<String>,
    /// Detailed JSON: the newest `create_time` of the page (the session header has the newest of all).
    pub latest: Option<i64>,
}

impl RenderedEntries {
    /// The entries, in order.
    pub(super) fn entries(&self) -> impl Iterator<Item = &[u8]> {
        let starts = std::iter::once(0).chain(self.ends.iter().copied());
        starts
            .zip(&self.ends)
            .map(|(start, &end)| &self.text[start..end])
    }

    /// Ends the entry written to `text` since the previous one.
    pub(super) fn end_entry(&mut self) {
        self.ends.push(self.text.len());
    }
}
