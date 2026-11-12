//! Exporting a conversation's messages: collecting or streaming them, the exporter, and writing each format.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{export_stream, ServiceHub};
use crate::config::clean_account_dir_name;
use crate::error::{AppError, AppResult};

impl ServiceHub {
    /// Export a conversation's messages in one of WeFlow's desktop app's formats
    /// (json, arkme-json, chatlab, chatlab-jsonl, excel, txt, weclone, html, sql).
    pub fn export_messages(
        &self,
        req: &MessageExportRequest,
        out: &Path,
    ) -> AppResult<crate::contract::MessagesExported> {
        let format = req.format.to_ascii_lowercase();
        if !MESSAGE_EXPORT_FORMATS
            .split(',')
            .any(|f| f.trim() == format)
        {
            return Err(AppError::usage(format!(
                "unsupported message export format: {format}; supported: {MESSAGE_EXPORT_FORMATS}"
            )));
        }
        again_if_keys_changed(|| self.export_streaming(req, out))
    }

    /// [`export_messages`](Self::export_messages) that also copies the selected media (images, voices, videos,
    /// stickers) to `<out dir>/media/<out name>/` and points the exported messages at the copies.
    pub async fn export_messages_with_media(
        &self,
        req: &MessageExportRequest,
        out: &Path,
        media: &crate::api::ApiMediaOptions,
    ) -> AppResult<crate::contract::MessagesExported> {
        if !media.enabled {
            return self.export_messages(req, out);
        }
        let (wcdb, mut collected) = again_if_keys_changed(|| self.export_collect(req))?;
        let media = self
            .attach_export_media(&wcdb, &mut collected, &req.session_id, out, media)
            .await;
        let mut result = self.export_write(req, out, &wcdb, &collected, &media.avatars)?;
        result.media = Some(media.copied);
        Ok(result)
    }

    /// One conversation in several formats at once (#342), each to its file in `outs`, all in one folder. The messages
    /// are read once and the formats written side by side, the selected media copied once before them, to
    /// `<out dir>/media/<first out name>/`, which every format points at.
    ///
    /// Reading once holds the whole conversation in memory, where one format alone streams: about 1.6 KiB a message
    /// more for half the CPU time and, in eight formats, 45% less time (#628). One format without media still streams.
    pub async fn export_messages_formats(
        &self,
        req: &MessageExportRequest,
        outs: &[(String, PathBuf)],
        media: &crate::api::ApiMediaOptions,
    ) -> AppResult<Vec<crate::contract::MessagesExported>> {
        let request = |format: &str| MessageExportRequest {
            format: format.to_string(),
            ..req.clone()
        };
        if let ([(format, out)], false) = (outs, media.enabled) {
            return Ok(vec![self.export_messages(&request(format), out)?]);
        }
        let Some((_, first)) = outs.first() else {
            return Ok(Vec::new());
        };
        let (wcdb, mut collected) = again_if_keys_changed(|| self.export_collect(req))?;
        let (stats, avatars) = if media.enabled {
            let media = self
                .attach_export_media(&wcdb, &mut collected, &req.session_id, first, media)
                .await;
            (Some(media.copied), media.avatars)
        } else {
            (None, std::collections::HashMap::new())
        };
        let lanes: Vec<_> = outs
            .iter()
            .map(|(format, _)| (format.clone(), false))
            .collect();
        crate::output::progress_formats(&lanes);
        let lanes = std::sync::Mutex::new(lanes);
        let (wcdb, collected, stats, lanes, avatars) =
            (&wcdb, &collected, &stats, &lanes, &avatars);
        std::thread::scope(|scope| {
            let writers: Vec<_> = outs
                .iter()
                .enumerate()
                .map(|(index, (format, out))| {
                    let request = request(format);
                    scope.spawn(move || {
                        let written = self.export_write(&request, out, wcdb, collected, avatars);
                        use crate::sync::MutexExt as _;
                        let mut lanes = lanes.lock_or_panic();
                        lanes[index].1 = true;
                        crate::output::progress_formats(&lanes);
                        written.map(|mut result| {
                            result.media.clone_from(stats);
                            result
                        })
                    })
                })
                .collect();
            writers
                .into_iter()
                .map(|writer| {
                    writer
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                })
                .collect()
        })
    }

    fn export_collect(
        &self,
        req: &MessageExportRequest,
    ) -> AppResult<(weflow_native::wcdb::Wcdb, Vec<crate::message::ExportMsg>)> {
        use crate::message::CollectOptions;
        let (_, _, wxid) = self.connection_inputs()?;
        let my_wxid = clean_account_dir_name(&wxid.unwrap_or_default());
        check_sender(req.sender.as_deref(), &my_wxid)?;
        let wcdb = self.open_wcdb()?;
        let opts = CollectOptions {
            session_id: &req.session_id,
            my_wxid: &my_wxid,
            start: req.start,
            end: req.end,
            sender_filter: req.sender.as_deref(),
        };
        // in the order of the cursor, as a streaming export writes them
        let collected = export_stream::collect_export_messages(&wcdb, req, &opts)?;
        if collected.is_empty() {
            return Err(no_messages(req));
        }
        Ok((wcdb, collected))
    }

    /// Export that writes the file while the messages are read, so memory stays flat whatever the size of the
    /// conversation. (An export that copies media reads everything first: see `export_collect`.)
    fn export_streaming(
        &self,
        req: &MessageExportRequest,
        out: &Path,
    ) -> AppResult<crate::contract::MessagesExported> {
        use crate::message::CollectOptions;
        let (_, _, wxid) = self.connection_inputs()?;
        let my_wxid = clean_account_dir_name(&wxid.unwrap_or_default());
        check_sender(req.sender.as_deref(), &my_wxid)?;
        let wcdb = self.open_wcdb()?;
        let opts = CollectOptions {
            session_id: &req.session_id,
            my_wxid: &my_wxid,
            start: req.start,
            end: req.end,
            sender_filter: req.sender.as_deref(),
        };
        let format = req.format.to_ascii_lowercase();
        let before = file_stamp(out);
        let render_ahead =
            crate::export_msg::spool::EntryFormat::parse(&format).filter(|f| f.renders_ahead());
        let read = match render_ahead {
            // entries that do not depend on each other and are slow to render: the reading workers render them
            // too, this thread only writes them in order
            Some(entries) => {
                let parts = self.exporter_parts(
                    &req.session_id,
                    req.display_pref,
                    req.excel_compact,
                    &wcdb,
                )?;
                let render: &export_stream::WithFinisher<
                    '_,
                    crate::export_msg::spool::RenderedEntries,
                > = &|body| {
                    let mut names = name_book(&wcdb);
                    let mut exporter = parts.exporter(&mut names);
                    body(&mut |msgs| exporter.render_entries(entries, &msgs));
                };
                export_stream::read_export_pages(&wcdb, req, &opts, render, |pages| {
                    let mut names = name_book(&wcdb);
                    let result = parts.exporter(&mut names).write_rendered(
                        entries,
                        out,
                        &mut |_| pages.next(),
                        false,
                    );
                    Ok(result)
                })
            }
            None => export_stream::read_export_stream(&wcdb, req, &opts, |stream| {
                self.with_exporter(
                    &req.session_id,
                    req.display_pref,
                    req.excel_compact,
                    &wcdb,
                    |ex| ex.write_streamed(&format, out, stream),
                )
            }),
        };
        if !matches!(read, Ok(Ok(Ok(_)))) {
            // the reading or the writing (a full disk) failed after the file was written from the messages read so
            // far: do not leave a truncated export that looks complete
            remove_if_written(out, before);
        }
        let count = read??.map_err(|e| AppError::runtime(e.to_string()))?;
        if count == 0 {
            return Err(no_messages(req));
        }
        Ok(crate::contract::MessagesExported {
            out: out.to_path_buf(),
            count,
            session: req.session_id.clone(),
            format,
            media: None,
        })
    }

    /// 按会话建好 [`Exporter`](crate::export_msg::Exporter)（联系人名、群昵称、自己的账号），再交给 `f` 使用。
    fn with_exporter<R>(
        &self,
        session_id: &str,
        display_pref: crate::export_msg::names::DisplayPref,
        excel_compact: bool,
        wcdb: &weflow_native::wcdb::Wcdb,
        f: impl FnOnce(&mut crate::export_msg::Exporter<'_, '_>) -> R,
    ) -> AppResult<R> {
        let parts = self.exporter_parts(session_id, display_pref, excel_compact, wcdb)?;
        let mut names = name_book(wcdb);
        Ok(f(&mut parts.exporter(&mut names)))
    }

    /// What every [`Exporter`](crate::export_msg::Exporter) of one export of `session_id` shares: read once, then
    /// each thread that renders builds its own exporter from it (see [`ExporterParts::exporter`]).
    fn exporter_parts(
        &self,
        session_id: &str,
        display_pref: crate::export_msg::names::DisplayPref,
        excel_compact: bool,
        wcdb: &weflow_native::wcdb::Wcdb,
    ) -> AppResult<ExporterParts> {
        use crate::export_msg::names::build_trusted_group_nicknames;
        use crate::export_msg::*;
        let (_, _, wxid) = self.connection_inputs()?;
        let raw_my_wxid = wxid.unwrap_or_default();
        let my_wxid = clean_account_dir_name(&raw_my_wxid);
        let mut names = name_book(wcdb);
        let is_group = session_id.ends_with("@chatroom");
        let (group_nicks, group_members) = if is_group {
            let nick_value = wcdb.group_nicknames(session_id).unwrap_or(Value::Null);
            let nick_obj = nick_value
                .get("nicknames")
                .and_then(Value::as_object)
                .or_else(|| nick_value.as_object());
            let entries: Vec<(String, String)> = nick_obj
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| v.as_str().map(|n| (k.clone(), n.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            let members_value = wcdb.group_members(session_id).unwrap_or(Value::Null);
            (
                build_trusted_group_nicknames(entries),
                extract_member_ids(&members_value),
            )
        } else {
            (std::collections::HashMap::new(), Vec::new())
        };

        let session_display = names.display_name(session_id);
        let session_contact = names.get(session_id);
        let my_display = names.display_name(&my_wxid);
        Ok(ExporterParts {
            session: SessionInfo {
                id: session_id.to_string(),
                display_name: session_display,
                nickname: session_contact
                    .as_ref()
                    .map(|c| c.nickname.clone())
                    .unwrap_or_default(),
                remark: session_contact
                    .as_ref()
                    .map(|c| c.remark.clone())
                    .unwrap_or_default(),
                is_group,
            },
            my_wxid: my_wxid.clone(),
            raw_my_wxid,
            my_display: if my_display == my_wxid {
                String::new()
            } else {
                my_display
            },
            group_nicks,
            group_members,
            settings: Settings {
                display_pref,
                excel_compact,
                ..Default::default()
            },
        })
    }

    /// `collected` written in `req`'s format; `avatars` (`{ username: path from the export file }`) go on the HTML page.
    fn export_write(
        &self,
        req: &MessageExportRequest,
        out: &Path,
        wcdb: &weflow_native::wcdb::Wcdb,
        collected: &[crate::message::ExportMsg],
        avatars: &std::collections::HashMap<String, String>,
    ) -> AppResult<crate::contract::MessagesExported> {
        let format = req.format.to_ascii_lowercase();
        let runtime = |e: anyhow::Error| AppError::runtime(e.to_string());
        let before = file_stamp(out);
        let written = self.with_exporter(
            &req.session_id,
            req.display_pref,
            req.excel_compact,
            wcdb,
            |exporter| match format.as_str() {
                "chatlab" => exporter
                    .write_chatlab(collected, out, false)
                    .map_err(runtime),
                "chatlab-jsonl" => exporter
                    .write_chatlab(collected, out, true)
                    .map_err(runtime),
                "json" => exporter.write_json(collected, out, false).map_err(runtime),
                "arkme-json" => exporter.write_json(collected, out, true).map_err(runtime),
                "excel" | "xlsx" => exporter.write_excel(collected, out).map_err(runtime),
                "txt" => exporter.write_txt(collected, out).map_err(runtime),
                "weclone" => exporter.write_weclone(collected, out).map_err(runtime),
                "html" => {
                    exporter.settings.avatars.clone_from(avatars);
                    exporter.write_html(collected, out).map_err(runtime)
                }
                "sql" => exporter.write_sql(collected, out).map_err(runtime),
                other => Err(AppError::usage(format!(
                    "unsupported message export format: {other}; supported: {MESSAGE_EXPORT_FORMATS}"
                ))),
            },
        );
        if !matches!(written, Ok(Ok(_))) {
            // a write that failed part-way (a full disk) leaves no truncated export behind
            remove_if_written(out, before);
        }
        written??;
        Ok(crate::contract::MessagesExported {
            out: out.to_path_buf(),
            count: collected.len(),
            session: req.session_id.clone(),
            format,
            media: None,
        })
    }
}

pub const MESSAGE_EXPORT_FORMATS: &str =
    "json, arkme-json, chatlab, chatlab-jsonl, excel, txt, weclone, html, sql";

#[derive(Clone)]
pub struct MessageExportRequest {
    pub session_id: String,
    pub format: String,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub sender: Option<String>,
    pub display_pref: crate::export_msg::names::DisplayPref,
    pub excel_compact: bool,
}

/// The parts of an [`Exporter`](crate::export_msg::Exporter) that do not change during an export (all but its name
/// cache), so threads that render the same export each build one.
struct ExporterParts {
    session: crate::export_msg::SessionInfo,
    my_wxid: String,
    raw_my_wxid: String,
    my_display: String,
    group_nicks: std::collections::HashMap<String, String>,
    group_members: Vec<String>,
    /// Holds the export time: every exporter of one export reports the same.
    settings: crate::export_msg::Settings,
}

impl ExporterParts {
    fn exporter<'a, 'n>(
        &self,
        names: &'a mut crate::export_msg::names::NameBook<'n>,
    ) -> crate::export_msg::Exporter<'a, 'n> {
        crate::export_msg::Exporter {
            session: self.session.clone(),
            my_wxid: self.my_wxid.clone(),
            raw_my_wxid: self.raw_my_wxid.clone(),
            my_display: self.my_display.clone(),
            group_nicks: self.group_nicks.clone(),
            group_members: self.group_members.clone(),
            names,
            settings: self.settings.clone(),
        }
    }
}

/// What tells whether an export wrote to `path`: its size and modification time, `None` when it does not exist.
type FileStamp = Option<(u64, Option<std::time::SystemTime>)>;

fn file_stamp(path: &Path) -> FileStamp {
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

/// Removes `out` after a failed export if the export wrote to it (compared with `before`, its stamp from before the
/// export), so a truncated file is not taken for a complete export.
fn remove_if_written(out: &Path, before: FileStamp) {
    if file_stamp(out) != before {
        let _ = std::fs::remove_file(out);
    }
}

/// Runs an export once more when the order of its messages changed while it was read: the second run reads the keys
/// of the tables that broke the `sort_seq` rule from their rows (see `Account::open_export_cursor`). An export
/// that failed removes what it wrote, so starting over is safe.
fn again_if_keys_changed<T>(mut export: impl FnMut() -> AppResult<T>) -> AppResult<T> {
    match export() {
        Err(e) if e.message.contains(weflow_native::msg::KEYS_CHANGED) => export(),
        done => done,
    }
}

/// `--sender` takes the bare wxid: it is the one identifier every message has, unique and fixed. The account folder
/// name (the owner's wxid with a `_xxxx` suffix) would match too, by the identity rule, so it is refused with the
/// bare form to use instead.
fn check_sender(sender: Option<&str>, my_wxid: &str) -> AppResult<()> {
    let Some(sender) = sender.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    if sender.len() > my_wxid.len() && crate::message::transfer::is_same_wxid(sender, my_wxid) {
        return Err(AppError::usage(format!(
            "--sender takes the bare wxid: use {my_wxid}, not the account folder name {sender}"
        )));
    }
    Ok(())
}

/// The error of an export that found nothing; with `--sender`, a reminder of what it takes.
fn no_messages(req: &MessageExportRequest) -> AppError {
    match req.sender.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(sender) => AppError::runtime(format!(
            "no messages from {sender} in this session in the given range (--sender takes the bare wxid, as `chat contacts` lists it)"
        )),
        None => AppError::runtime("no messages found for this session in the given range"),
    }
}

/// Contact names looked up in `wcdb` as an export needs them (cached).
fn name_book(wcdb: &weflow_native::wcdb::Wcdb) -> crate::export_msg::names::NameBook<'_> {
    use crate::export_msg::{names::ContactInfo, names::NameBook};
    NameBook::new(move |username: &str| {
        wcdb.contact(username)
            .ok()
            .filter(|v| v.is_object() && !v.as_object().is_none_or(|o| o.is_empty()))
            .map(|v| ContactInfo::from_value(username, &v))
    })
}

/// A sticker: a plain emoji message, or an appmsg of type 8 (a sticker sent as an attachment).
pub(super) fn is_sticker(m: &crate::message::ExportMsg) -> bool {
    m.local_type == 47 || m.sticker_md5.is_some()
}

/// 查表情描述（用户自己的表情文字，否则商店表情的中文描述），写进 `emoji_caption`，
/// 导出时显示为 `[表情：描述]`。描述表（md5 → 描述）整张一次读出，见 `emoticon_captions`。
pub(super) fn attach_emoji_captions(
    table: &std::collections::HashMap<String, String>,
    messages: &mut [crate::message::ExportMsg],
) {
    for m in messages.iter_mut() {
        let md5 = match m.local_type {
            47 => m.emoji_md5.clone(),
            _ => m.sticker_md5.clone(),
        };
        let Some(md5) = md5 else { continue };
        m.emoji_caption = table
            .get(&md5.to_lowercase())
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty());
    }
}

fn extract_member_ids(value: &Value) -> Vec<String> {
    let list = value
        .as_array()
        .cloned()
        .or_else(|| value.get("members").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for item in list {
        let id = match &item {
            Value::String(s) => s.clone(),
            other => ["username", "userName", "wxid", "user_name"]
                .iter()
                .find_map(|k| other.get(*k).and_then(Value::as_str))
                .unwrap_or("")
                .to_string(),
        };
        if !id.is_empty() && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn member_ids_from_various_shapes() {
        let v = json!({"members": [{"username": "a"}, {"userName": "b"}, "c", {"username": "a"}]});
        assert_eq!(extract_member_ids(&v), vec!["a", "b", "c"]);
        assert_eq!(extract_member_ids(&json!(["x", "y"])), vec!["x", "y"]);
        assert!(extract_member_ids(&Value::Null).is_empty());
    }
}
