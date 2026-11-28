//! 导出会话的消息：收集或流式读取消息、导出器，以及写出各种格式。

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{export_stream, ServiceHub};
use crate::config::clean_account_dir_name;
use crate::error::{AppError, AppResult};
use weflow_native::wcdb::Databases;

impl ServiceHub {
    /// 按桌面版的一种格式（json、arkme-json、chatlab、chatlab-jsonl、excel、txt、weclone、html、sql）导出会话的消息。
    ///
    /// # Errors
    ///
    /// 格式不支持或 `sender` 是账号目录名时返回用法错误，账号没有配置好时返回配置错误，
    /// 范围内没有消息时返回运行时错误；读取或写入失败时返回错误，并删掉写了一部分的文件。
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

    /// [`export_messages`](Self::export_messages)，同时把选中的媒体（图片、语音、视频、表情）复制到
    /// `<out dir>/media/<out name>/`，并让导出的消息指向这些副本。
    ///
    /// # Errors
    ///
    /// 同 [`export_messages`](Self::export_messages)。
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

    /// 一次把一个会话导出成多种格式（#342），每种写到 `outs` 中各自的文件，都在同一个文件夹里。
    ///
    /// 消息只读一次，各格式同时写出；选中的媒体在此之前复制一次，放在 `<out dir>/media/<first out name>/`，
    /// 每种格式都指向那里。
    ///
    /// 只读一次要把整个会话放在内存里，而单一格式是流式读写的：每条消息多用约 1.6 KiB 内存，换来一半的 CPU 时间，
    /// 八种格式时总时间少 45%（#628）。只有一种格式且不带媒体时仍是流式的。
    ///
    /// # Errors
    ///
    /// 同 [`export_messages`](Self::export_messages)；任何一种格式写出失败时返回它的错误。
    ///
    /// # Panics
    ///
    /// 写某种格式的线程 panic 时，把它的 panic 传给调用方。
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
        // 导出要从联系人取发送者的名字，再读消息
        wcdb.prefetch(&[Databases::Contacts, Databases::Messages])
            .map_err(|e| AppError::native(format!("{e:#}")))?;
        let opts = CollectOptions {
            session_id: &req.session_id,
            my_wxid: &my_wxid,
            start: req.start,
            end: req.end,
            sender_filter: req.sender.as_deref(),
        };
        // 按导出游标的顺序，与流式导出写出的顺序相同
        let collected = export_stream::collect_export_messages(&wcdb, req, &opts)?;
        if collected.is_empty() {
            return Err(no_messages(req));
        }
        Ok((wcdb, collected))
    }

    /// 边读消息边写文件的导出，内存占用不随会话的大小增长。
    ///
    /// 复制媒体的导出要先读完全部消息，见 `export_collect`。
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
        // 导出要从联系人取发送者的名字，再读消息
        wcdb.prefetch(&[Databases::Contacts, Databases::Messages])
            .map_err(|e| AppError::native(format!("{e:#}")))?;
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
            // 各条目互不依赖且渲染较慢：由读取的工作线程一并渲染，这个线程只按顺序写出
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
            // 读取或写入（如磁盘已满）在文件已按读到的消息写出一部分后失败：不留下看似完整、实际被截断的导出
            remove_if_written(out, before);
        }
        let count = read??.map_err(|e| AppError::runtime(format!("{e:#}")))?;
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

    /// 为会话建好 [`Exporter`](crate::export_msg::Exporter)（联系人的名字、群昵称、账号自己的 ID），交给 `f`。
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

    /// 导出 `session_id` 时各个 [`Exporter`](crate::export_msg::Exporter) 共用的部分：只读一次，
    /// 之后每个渲染的线程用它建自己的导出器（见 [`ExporterParts::exporter`]）。
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

    /// 把 `collected` 按 `req` 的格式写出；`avatars` 是用户名到头像路径（相对导出文件）的对应表，用在 HTML 页面上。
    fn export_write(
        &self,
        req: &MessageExportRequest,
        out: &Path,
        wcdb: &weflow_native::wcdb::Wcdb,
        collected: &[crate::message::ExportMsg],
        avatars: &std::collections::HashMap<String, String>,
    ) -> AppResult<crate::contract::MessagesExported> {
        let format = req.format.to_ascii_lowercase();
        let runtime = |e: anyhow::Error| AppError::runtime(format!("{e:#}"));
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
        if !matches!(written, Ok(Ok(()))) {
            // 写到一半失败（如磁盘已满）时不留下被截断的导出
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

/// `export messages` 能写的格式，写法与 `--format` 接受的一致，供报错等提示列出。
pub const MESSAGE_EXPORT_FORMATS: &str =
    "json, arkme-json, chatlab, chatlab-jsonl, excel, txt, weclone, html, sql";

/// `export messages` 导出一个会话的哪些消息、写成什么样。
#[derive(Clone, Debug)]
pub struct MessageExportRequest {
    /// 会话：私聊为对方的 wxid，群聊为 `…@chatroom`。
    pub session_id: String,
    /// [`MESSAGE_EXPORT_FORMATS`] 中的一种，不区分大小写。
    pub format: String,
    /// 导出的消息的起始秒（Unix 时间），包含在这一秒发送的消息。
    pub start: Option<i64>,
    /// 导出的消息的最后一秒（Unix 时间），包含在这一秒发送的消息，与消息游标和
    /// [`collect_rows`](crate::message::collect_rows) 的处理一致。
    pub end: Option<i64>,
    /// 只导出这个发送者（裸 wxid）的消息。
    pub sender: Option<String>,
    /// 导出中发送者显示哪个名字。
    pub display_pref: crate::export_msg::names::DisplayPref,
    /// Excel 只写时间、发送者、类型和内容这几列。
    pub excel_compact: bool,
}

/// [`Exporter`](crate::export_msg::Exporter) 中导出期间不变的部分（除名字缓存以外的全部），
/// 渲染同一次导出的各线程各用它建一个导出器。
struct ExporterParts {
    session: crate::export_msg::SessionInfo,
    my_wxid: String,
    raw_my_wxid: String,
    my_display: String,
    group_nicks: std::collections::HashMap<String, String>,
    group_members: Vec<String>,
    /// 含导出时间：同一次导出的每个导出器给出的时间相同。
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

/// 判断导出是否写过 `path` 的依据：文件的大小和修改时间，文件不存在时为 `None`。
type FileStamp = Option<(u64, Option<std::time::SystemTime>)>;

fn file_stamp(path: &Path) -> FileStamp {
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

/// 导出失败后，如果导出写过 `out`（与导出前的 `before` 比较），就删掉它，免得被截断的文件被当成完整的导出。
fn remove_if_written(out: &Path, before: FileStamp) {
    if file_stamp(out) != before {
        let _ = std::fs::remove_file(out);
    }
}

/// 读取期间消息的顺序变了时再运行一次导出。
///
/// 第二次从行中读出违反 `sort_seq` 规则的表的键（见 `Account::open_export_cursor`）。失败的导出会删掉它写的文件，
/// 所以重新开始是安全的。
fn again_if_keys_changed<T>(mut export: impl FnMut() -> AppResult<T>) -> AppResult<T> {
    match export() {
        Err(e) if e.message.contains(weflow_native::msg::KEYS_CHANGED) => export(),
        done => done,
    }
}

/// `--sender` 只接受裸 wxid：它是每条消息都有、唯一且不变的标识。
///
/// 账号目录名（账号所有者的 wxid 加 `_xxxx` 后缀）按身份规则也能匹配，所以拒绝它，并在报错里给出应使用的裸 wxid。
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

/// 导出没有找到消息时的错误；带 `--sender` 时提醒它接受什么。
fn no_messages(req: &MessageExportRequest) -> AppError {
    match req.sender.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(sender) => AppError::runtime(format!(
            "no messages from {sender} in this session in the given range\n--sender takes the bare wxid, as `chat contacts` lists it."
        )),
        None => AppError::runtime("no messages found for this session in the given range"),
    }
}

/// 导出时按需从 `wcdb` 查的联系人名字，查过的会缓存。
fn name_book(wcdb: &weflow_native::wcdb::Wcdb) -> crate::export_msg::names::NameBook<'_> {
    use crate::export_msg::names::{ContactInfo, NameBook};
    NameBook::new(move |username: &str| {
        wcdb.contact(username)
            .ok()
            .filter(|v| v.is_object() && !v.as_object().is_none_or(|o| o.is_empty()))
            .map(|v| ContactInfo::from_value(username, &v))
    })
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
