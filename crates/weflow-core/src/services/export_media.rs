//! 从微信中复制出消息的媒体（`export media`，以及带 `--media` 的导出）：每条消息的图片、语音、视频、表情或文件，
//! 以及发言者或列表各行的头像。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::image::{CacheWrites, Since};
use super::*;
use crate::api::{self, ApiExportedMedia, ApiMediaOptions};
use crate::chat_msg::{self, ChatMessage};
use crate::fs_atomic::{copy_atomically, write_atomically};
use crate::message::xml::rx;
use crate::sync::MutexExt;
use weflow_native::msg::Order;

impl ServiceHub {
    /// 按 CDN 地址下载表情，已下载过的直接取缓存。
    async fn chat_download_emoji(&self, cdn_url: &str, md5: Option<&str>) -> AppResult<PathBuf> {
        if cdn_url.is_empty() {
            return Err(AppError::usage("invalid CDN URL"));
        }
        let key = md5
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                use md5::{Digest, Md5};
                let mut h = Md5::new();
                h.update(cdn_url.as_bytes());
                h.finalize().iter().map(|b| format!("{b:02x}")).collect()
            });
        let dir = self.emoji_cache_dir();
        for ext in [".gif", ".png", ".webp", ".jpg", ".jpeg"] {
            let p = dir.join(format!("{key}{ext}"));
            if p.exists() {
                return Ok(p);
            }
        }
        // 校验证书，只有证书上写着别的主机名的那几台表情主机除外（#814）
        let client = crate::http_client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .danger_accept_invalid_certs(crate::http_client::sticker_certificate_unchecked(cdn_url))
            .build()
            .map_err(|e| AppError::runtime(format!("{e:#}")))?;
        // 客户端自己跟随重定向（reqwest 的默认策略）：拿到的响应是最后一个
        let resp = client
            .get(cdn_url)
            .send()
            .await
            .map_err(|e| AppError::runtime(crate::http_client::failure(e)))?;
        if resp.status().as_u16() != 200 {
            return Err(AppError::runtime("download failed"));
        }
        // 重定向之后表情的实际地址：字节看不出格式时，地址里的文件名也许能看出来
        let url = resp.url().to_string();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::runtime(crate::http_client::failure(e)))?;
        if bytes.is_empty() {
            return Err(AppError::runtime("download failed"));
        }
        let ext = match crate::decrypt::detect_image_extension(&bytes) {
            ".bin" | ".wxgf" => rx(r"(?i)\.(gif|png|jpe?g|webp)(?:\?|$)")
                .captures(&url)
                .map(|c| format!(".{}", c[1].to_lowercase()))
                .unwrap_or_else(|| ".gif".into()),
            e => e.to_string(),
        };
        let path = dir.join(format!("{key}{ext}"));
        write_atomically(&path, &bytes)
            .map_err(|e| AppError::runtime(format!("failed to write {}: {e}", path.display())))?;
        Ok(path)
    }

    async fn export_media_for_message(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        msg: &ChatMessage,
        talker: &str,
        safe_talker: &str,
        session_dir: &Path,
        opts: &ApiMediaOptions,
    ) -> Option<ApiExportedMedia> {
        if opts.files && is_file_message(msg.xml_type()) {
            let (source, file_name) =
                self.message_file(wcdb, msg.file_md5(), msg.file_name(), msg.create_time)?;
            let to = MediaTarget {
                session_dir,
                safe_talker,
                written: None,
            };
            return put_media(
                &to,
                ("file", "files"),
                file_name,
                MediaSource::Copy(&source),
                false,
            );
        }
        if msg.local_type == 47 && opts.emojis {
            let url = msg.emoji_cdn_url.as_deref().filter(|u| !u.is_empty())?;
            let path = self
                .chat_download_emoji(url, msg.emoji_md5.as_deref())
                .await
                .ok()?;
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_else(|| ".gif".into());
            let base = api::sanitize_file_name(
                msg.emoji_md5
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(""),
                &message_file_stem("emoji", msg),
            );
            let to = MediaTarget {
                session_dir,
                safe_talker,
                written: None,
            };
            return put_media(
                &to,
                ("emoji", "emojis"),
                format!("{base}{ext}"),
                MediaSource::Copy(&path),
                false,
            );
        }
        self.export_local_media(msg, talker, safe_talker, session_dir, opts, None)
    }

    /// 把 `msgs` 的媒体复制到 `<out dir>/media/<out name>/{images,voices,videos,emojis,files}`，
    /// 并把每个文件记在它的消息上；要求的头像放在旁边的 `avatars/` 中（见 [`Self::export_avatars`]）。
    pub(super) async fn attach_export_media(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        msgs: &mut [crate::message::ExportMsg],
        session_id: &str,
        out: &Path,
        opts: &ApiMediaOptions,
    ) -> ExportMedia {
        use crate::message::MediaRef;
        let (session_dir, prefix) = media_dir(out);
        let wanted = |m: &crate::message::ExportMsg| match m.local_type {
            3 => opts.images,
            34 => opts.voices,
            43 => opts.videos,
            47 => opts.emojis,
            _ => opts.files && is_file_message(m.xml_type.as_deref()),
        };
        let todo: Vec<usize> = (0..msgs.len()).filter(|i| wanted(&msgs[*i])).collect();
        let in_order =
            |m: &crate::message::ExportMsg| copied_in_order(m.local_type, m.xml_type.as_deref());
        let chat_msg = |m: &crate::message::ExportMsg| ChatMessage {
            local_id: m.local_id,
            server_id: m.server_id,
            server_id_raw: m.server_id_raw.clone().unwrap_or_default(),
            local_type: m.local_type,
            create_time: m.create_time,
            image_md5: m.image_md5.clone(),
            image_dat_name: m.image_dat_name.clone(),
            video_md5: m.video_md5.clone(),
            emoji_cdn_url: m.emoji_cdn_url.clone(),
            emoji_md5: m.emoji_md5.clone(),
            // 用来找到文件消息的文件
            t49: is_file_message(m.xml_type.as_deref()).then(|| {
                Box::new(chat_msg::Type49Info {
                    xml_type: m.xml_type.clone(),
                    file_name: m.file_name.clone(),
                    file_md5: m.file_md5.clone(),
                    ..Default::default()
                })
            }),
            ..Default::default()
        };
        let safe = api::sanitize_file_name(session_id, "session");
        let total = todo.len();
        let mut results: Vec<Option<ApiExportedMedia>> = (0..total).map(|_| None).collect();
        // 两轮共用一个计数器，进度不会倒退
        let finished = std::sync::atomic::AtomicUsize::new(0);
        {
            let local: Vec<usize> = (0..total).filter(|k| !in_order(&msgs[todo[*k]])).collect();
            let workers = std::thread::available_parallelism()
                .map_or(2, |n| n.get())
                .clamp(1, 8)
                .min(local.len().max(1));
            let next = std::sync::atomic::AtomicUsize::new(0);
            let found = std::sync::Mutex::new(Vec::<(usize, Option<ApiExportedMedia>)>::new());
            let run = ExportRun::new(self);
            let shared: &[crate::message::ExportMsg] = msgs;
            std::thread::scope(|scope| {
                for _ in 0..workers {
                    scope.spawn(|| loop {
                        let n = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&k) = local.get(n) else { break };
                        let r = self.export_local_media(
                            &chat_msg(&shared[todo[k]]),
                            session_id,
                            &safe,
                            &session_dir,
                            opts,
                            Some(&run),
                        );
                        found.lock_or_panic().push((k, r));
                        let done = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        self.emit_progress("export", "copying media", done, total);
                    });
                }
            });
            for (k, r) in found.into_inner_or_panic() {
                results[k] = r;
            }
            // 图片自己的文件不在磁盘上、但另一条消息显示同一张图（md5 相同）且那条先找到时，仍然导出：
            // 那次的结果记在 `image_state` 中。哪条消息先处理取决于线程，所以等全部完成后再找一次。
            for &k in &local {
                if results[k].is_none() && shared[todo[k]].local_type == 3 {
                    results[k] = self.export_local_media(
                        &chat_msg(&shared[todo[k]]),
                        session_id,
                        &safe,
                        &session_dir,
                        opts,
                        Some(&run),
                    );
                }
            }
        }
        for k in 0..total {
            let m = &msgs[todo[k]];
            if in_order(m) {
                results[k] = self
                    .export_media_for_message(
                        wcdb,
                        &chat_msg(m),
                        session_id,
                        &safe,
                        &session_dir,
                        opts,
                    )
                    .await;
                let done = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                self.emit_progress("export", "copying media", done, total);
            }
        }
        let mut exported = 0usize;
        for (k, r) in results.into_iter().enumerate() {
            let Some(r) = r else { continue };
            let sub = match r.kind {
                "image" => "images",
                "voice" => "voices",
                "video" => "videos",
                "file" => "files",
                _ => "emojis",
            };
            msgs[todo[k]].media = Some(MediaRef {
                kind: r.kind,
                relative_path: format!("{prefix}/{sub}/{}", r.file_name),
            });
            exported += 1;
        }
        let avatars = match opts.avatars {
            Some(source) => {
                self.export_avatars(wcdb, msgs, session_id, &session_dir, &prefix, source)
                    .await
            }
            None => HashMap::new(),
        };
        ExportMedia {
            copied: crate::contract::MediaCopied {
                requested: total,
                exported,
                missing: total - exported,
                dir: session_dir,
            },
            avatars,
        }
    }

    /// `msgs` 中发言者（账号所有者、私聊的对方、群成员）的头像，
    /// 写到 `<session_dir>/avatars/`（见 [`Self::write_avatars`]）。
    async fn export_avatars(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        msgs: &[crate::message::ExportMsg],
        session_id: &str,
        session_dir: &Path,
        prefix: &str,
        source: api::AvatarSource,
    ) -> HashMap<String, String> {
        let me = self.wmy();
        let is_group = session_id.ends_with("@chatroom");
        let mut users: Vec<String> = Vec::new();
        for m in msgs {
            let user = if m.is_send {
                me.as_str()
            } else if is_group {
                m.sender_username.as_str()
            } else {
                session_id
            };
            if !user.is_empty() && !users.iter().any(|u| u == user) {
                users.push(user.to_string());
            }
        }
        self.write_avatars(wcdb, users, session_dir, prefix, source)
            .await
    }

    /// 列表（带 `--media avatar` 的 `export sessions`、`export contacts`）各行的头像，供它的 HTML 页面使用。
    ///
    /// 与 `write_avatars` 一样写到 `out` 旁边的 `media/<out name>/avatars/`，返回写出的那些头像：
    /// 用户名到相对页面的路径的对应表。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，数据库打不开时返回数据库层的错误。
    pub async fn export_list_avatars(
        &self,
        users: Vec<String>,
        out: &Path,
        source: api::AvatarSource,
    ) -> AppResult<HashMap<String, String>> {
        let wcdb = self.open_wcdb()?;
        let (dir, prefix) = media_dir(out);
        Ok(self
            .write_avatars(&wcdb, users, &dir, &prefix, source)
            .await)
    }

    /// 把 `users` 的头像写到 `<dir>/avatars/`：用本机上微信保存的头像；没有时，
    /// 来源为 [`AvatarSource::Download`](api::AvatarSource::Download) 就下载联系人头像地址上的图片。
    ///
    /// 返回写出的那些：`{ username: <prefix>/avatars/<file> }`；其余的在页面上仍显示首字。
    async fn write_avatars(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        users: Vec<String>,
        dir: &Path,
        prefix: &str,
        source: api::AvatarSource,
    ) -> HashMap<String, String> {
        use futures::StreamExt as _;
        let dir = dir.join("avatars");
        if users.is_empty() || std::fs::create_dir_all(&dir).is_err() {
            return HashMap::new();
        }
        let mut pictures: Vec<(String, Vec<u8>)> = Vec::new();
        let mut elsewhere: Vec<String> = Vec::new();
        for user in users {
            match wcdb.head_image(&user) {
                Ok(Some(bytes)) => pictures.push((user, bytes)),
                _ => elsewhere.push(user),
            }
        }
        if source == api::AvatarSource::Download && !elsewhere.is_empty() {
            let urls = wcdb.avatar_urls(&elsewhere).ok().unwrap_or(Value::Null);
            let wanted: Vec<(String, String)> = elsewhere
                .into_iter()
                .filter_map(|user| {
                    let url = urls.get(&user)?.as_str()?.to_string();
                    Some((user, url))
                })
                .collect();
            let total = wanted.len();
            self.emit_progress("export", "downloading avatars", 0, total);
            let fetched: Vec<(String, Option<Vec<u8>>)> = futures::stream::iter(wanted)
                .map(|(user, url)| async move { (user, avatar_picture(&url).await) })
                .buffer_unordered(5)
                .collect()
                .await;
            self.emit_progress("export", "downloading avatars", total, total);
            pictures.extend(fetched.into_iter().filter_map(|(u, b)| Some((u, b?))));
        }
        pictures
            .into_iter()
            .filter_map(|(user, bytes)| {
                let ext = crate::image::ImageFormat::of(&bytes).map_or(".jpg", |f| f.extension());
                // 用用户 ID 的哈希命名：ID 里可能有文件名不能用的字符
                let hash: String = {
                    use md5::{Digest, Md5};
                    Md5::digest(user.as_bytes())
                        .iter()
                        .take(6)
                        .map(|b| format!("{b:02x}"))
                        .collect()
                };
                let file = format!("{hash}{ext}");
                write_atomically(&dir.join(&file), &bytes).ok()?;
                Some((user, format!("{prefix}/avatars/{file}")))
            })
            .collect()
    }

    /// 图片、语音和视频：不需要联网的媒体，所以可以在多个线程上处理（共用 `run`，即它们所属的这次导出，
    /// 见 [`ExportRun`]）。
    fn export_local_media(
        &self,
        msg: &ChatMessage,
        talker: &str,
        safe_talker: &str,
        session_dir: &Path,
        opts: &ApiMediaOptions,
        run: Option<&ExportRun>,
    ) -> Option<ApiExportedMedia> {
        let to = MediaTarget {
            session_dir,
            safe_talker,
            written: run.map(|r| &r.files),
        };
        let put = |kind: &'static str,
                   sub: &str,
                   file_name: String,
                   source: MediaSource<'_>,
                   thumbnail: bool| {
            put_media(&to, (kind, sub), file_name, source, thumbnail)
        };
        if msg.local_type == 3 && opts.images {
            // 查找图片用的目录和索引在整次导出中只检查一次
            let since = run.map_or_else(Since::now, |r| r.since);
            let payload = ImagePayload {
                session_id: Some(talker.to_string()),
                image_md5: msg.image_md5.clone().filter(|m| !m.is_empty()),
                image_dat_name: msg.image_dat_name.clone().filter(|m| !m.is_empty()),
                create_time: Some(msg.create_time).filter(|t| *t != 0),
                force: true,
                prefer_file_path: true,
                ..Default::default()
            };
            // 刚解密的图片写在这里；只有从 WXGF 转换来的图片同时写进缓存
            let decrypted = self.image_decrypt_data(&payload, since, CacheWrites::OnlyWxgf);
            let (bytes, thumbnail) = match decrypted.fresh {
                Some(fresh) => (fresh.data, fresh.thumbnail),
                None => {
                    let path = match decrypted.result.local_path {
                        Some(path) => path,
                        None => {
                            self.image_resolve_cache_since(
                                &ImagePayload {
                                    force: false,
                                    ..payload
                                },
                                since,
                            )
                            .local_path?
                        }
                    };
                    let thumbnail =
                        !path.starts_with("data:") && crate::image::is_thumbnail_path(&path);
                    let bytes = match path.strip_prefix("data:") {
                        Some(rest) => {
                            use base64::Engine;
                            let b64 = rest.split_once(";base64,")?.1;
                            base64::engine::general_purpose::STANDARD.decode(b64).ok()?
                        }
                        None => std::fs::read(&path).ok()?,
                    };
                    (bytes, thumbnail)
                }
            };
            let ext = api_detect_image_ext(&bytes);
            let base = api::sanitize_file_name(
                msg.image_md5
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .or(msg.image_dat_name.as_deref())
                    .unwrap_or(""),
                &message_file_stem("image", msg),
            );
            return put(
                "image",
                "images",
                format!("{base}{ext}"),
                MediaSource::Bytes(&bytes),
                thumbnail,
            )
            .map(|mut m| {
                m.thumbnail = thumbnail;
                m
            });
        }
        if msg.local_type == 34 && opts.voices {
            let server = if msg.server_id_raw.is_empty() || msg.server_id_raw == "0" {
                msg.server_id.to_string()
            } else {
                msg.server_id_raw.clone()
            };
            let server = if server == "0" { None } else { Some(server) };
            let (msg_id, time) = (
                msg.local_id.to_string(),
                Some(msg.create_time).filter(|t| *t != 0),
            );
            let wav = match run.and_then(|run| run.wcdb(self)) {
                Some(wcdb) => {
                    self.voice_data_with(wcdb, talker, &msg_id, time, server.as_deref(), None)
                }
                None => self.voice_data(talker, &msg_id, time, server.as_deref(), None),
            }
            .ok()?;
            return put(
                "voice",
                "voices",
                format!("{}.wav", message_file_stem("voice", msg)),
                MediaSource::Bytes(&wav),
                false,
            );
        }
        if msg.local_type == 43 && opts.videos {
            let md5 = msg.video_md5.as_deref().filter(|m| !m.is_empty())?;
            let xml_md5 = crate::video::parse_video_md5(&msg.raw_content).unwrap_or_default();
            let own;
            let videos = match run {
                Some(run) => run.videos.as_ref()?,
                None => {
                    own = self.video_folder()?;
                    &own
                }
            };
            let month = crate::image::year_month_from_create_time(Some(msg.create_time));
            let video = self.video_file_path(videos, &[md5, &xml_md5], &month)?;
            let ext = video
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_else(|| ".mp4".into());
            let base = api::sanitize_file_name(md5, &message_file_stem("video", msg));
            return put(
                "video",
                "videos",
                format!("{base}{ext}"),
                MediaSource::Copy(&video),
                false,
            );
        }
        None
    }
}

/// 导出文件内容的优劣，最好的最大：完整版胜过缩略图，其次比文件大小。
type MediaRank = (bool, u64);

/// 一次媒体导出写的文件，以及每个文件内容的优劣。
///
/// 共用一张图片的消息共用它的文件，媒体线程同时导出它们：不论谁先到，高清版都会替换缩略图，
/// 所以导出的结果不取决于线程的先后（#52）。
///
/// 新文件直接写在原位，不经过临时文件再改名：在 Windows 上改名会再经过一遍所有文件系统过滤驱动，
/// 约占带媒体的导出一半的时间。进程被杀掉时写了一半的文件由 [`INCOMPLETE`] 发现：导出往某个文件夹写文件期间，
/// 文件夹里有这个标记，丢弃 `ExportedFiles` 时再把标记删掉。
#[derive(Default)]
pub(super) struct ExportedFiles {
    written: std::sync::Mutex<Written>,
    /// 某个线程写完一个文件时发出信号，通知等着写同一个文件的线程。
    done: std::sync::Condvar,
}

/// [`ExportedFiles`] 保存的内容。
#[derive(Default)]
struct Written {
    ranks: HashMap<PathBuf, MediaRank>,
    /// 到目前为止写过的文件夹，以及每个文件夹在导出之前的状态。
    folders: HashMap<PathBuf, Folder>,
    /// 线程正在写的文件。只有写同一个文件的线程才互相等待：文件在锁外写，所以不同的文件可以同时写。
    writing: HashSet<PathBuf>,
}

/// 导出第一次往文件夹里写文件时，文件夹的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Folder {
    /// 这次导出建的：没有以前的导出留下的文件。
    Made,
    /// 以前的导出在这里完成了：保留它留下的文件。
    Finished,
    /// 以前的导出往这里写文件时被中止了（有 [`INCOMPLETE`]）：它留下的文件都可能不完整，所以每个都重新写。
    Stopped,
}

/// 导出往媒体文件夹写文件期间放在其中的标记（见 [`ExportedFiles`]）。
const INCOMPLETE: &str = ".weflow-export-incomplete";

impl ExportedFiles {
    fn lock(&self) -> std::sync::MutexGuard<'_, Written> {
        self.written
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Drop for ExportedFiles {
    /// 删掉写过的文件夹中的标记：导出已经完成，或者在删掉写了一半的文件之后失败了。
    fn drop(&mut self) {
        for dir in self.lock().folders.keys() {
            // 标记已经不在时无需处理
            let _ = std::fs::remove_file(dir.join(INCOMPLETE));
        }
    }
}

/// 让文件夹 `dir` 可以写入：确保它存在并放好 [`INCOMPLETE`]，返回它在这之前的状态。
fn open_folder(dir: &Path) -> std::io::Result<Folder> {
    let made = make_dir(dir)?;
    let marker = dir.join(INCOMPLETE);
    let stopped = !made && marker.exists();
    std::fs::write(&marker, b"")?;
    Ok(if made {
        Folder::Made
    } else if stopped {
        Folder::Stopped
    } else {
        Folder::Finished
    })
}

/// 把 `bytes` 直接写到还不存在的 `path`；写入失败时删掉写了一半的文件。
/// `path` 其实已经存在时，经临时文件替换它（[`write_atomically`]）。
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return write_atomically(path, bytes)
        }
        Err(e) => return Err(e),
    };
    let written = file.write_all(bytes);
    drop(file);
    if written.is_err() {
        // 文件已经不在时无需删除
        let _ = std::fs::remove_file(path);
    }
    written
}

/// 一次媒体导出中，各条消息并行复制媒体时共用的东西。
struct ExportRun {
    /// 到目前为止写过的文件。
    files: ExportedFiles,
    /// 微信的视频目录，整次导出最多列一次（没有账号目录时为 `None`）。
    videos: Option<crate::video::VideoFolder>,
    /// 从这一时刻起，查找图片用的目录检查过一次就沿用，不再重新检查（#1099）。
    since: Since,
    /// 数据库，第一条语音需要时打开，整次导出只打开一次（打不开时为 `None`：这时每条语音像单独导出时一样自己去打开）。
    wcdb: std::sync::OnceLock<Option<weflow_native::wcdb::Wcdb>>,
}

impl ExportRun {
    /// `hub` 的账号的一次媒体导出。
    fn new(hub: &ServiceHub) -> Self {
        Self {
            files: ExportedFiles::default(),
            videos: hub.video_folder(),
            since: Since::now(),
            wcdb: std::sync::OnceLock::new(),
        }
    }

    /// `hub` 的账号的数据库，由第一个调用者打开。
    fn wcdb(&self, hub: &ServiceHub) -> Option<&weflow_native::wcdb::Wcdb> {
        self.wcdb.get_or_init(|| hub.open_wcdb().ok()).as_ref()
    }
}

/// 一个会话的媒体放到哪里。
struct MediaTarget<'a> {
    session_dir: &'a Path,
    safe_talker: &'a str,
    /// 它所属的这次导出写过的文件；一次导出多条消息时才有。
    written: Option<&'a ExportedFiles>,
}

/// 导出的媒体文件从哪里来。
enum MediaSource<'a> {
    Bytes(&'a [u8]),
    Copy(&'a Path),
}

/// 在 `<session_dir>/<sub>` 下写出（或复制）一个导出的文件；`file_name` 也可以带下一级文件夹（`2024-01/report.pdf`）。
///
/// 已有的文件保留，除非是这次导出（`written`）从较差的版本写的：这时完整版（不是 `thumbnail`），
/// 或者同类中更大的文件，会替换它。
fn put_media(
    to: &MediaTarget<'_>,
    (kind, sub): (&'static str, &str),
    file_name: String,
    source: MediaSource<'_>,
    thumbnail: bool,
) -> Option<ApiExportedMedia> {
    let (session_dir, safe_talker) = (to.session_dir, to.safe_talker);
    let full = file_name
        .split('/')
        .fold(session_dir.join(sub), |path, part| path.join(part));
    let dir = full.parent()?;
    let size = match source {
        MediaSource::Bytes([]) => return None,
        MediaSource::Bytes(b) => b.len() as u64,
        MediaSource::Copy(src) => std::fs::metadata(src).ok()?.len(),
    };
    // 先写临时文件再改名到位：别的线程一看到这个文件，它就是完整的
    let write = || match source {
        MediaSource::Bytes(b) => write_atomically(&full, b),
        MediaSource::Copy(src) => copy_atomically(src, &full),
    };
    match to.written {
        None => {
            std::fs::create_dir_all(dir).ok()?;
            if !full.exists() {
                write().ok()?;
            }
        }
        Some(written) => {
            let rank = (!thumbnail, size);
            // 等之前对同一个文件的写入完成后再决定，更好的版本就不会被更差的覆盖
            let mut files = written.lock();
            while files.writing.contains(&full) {
                files = written
                    .done
                    .wait(files)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            // 往文件夹里写的第一个文件在锁内建好文件夹并放上标记（`INCOMPLETE`），然后才写任何文件
            let folder = match files.folders.get(dir) {
                Some(folder) => *folder,
                None => {
                    let folder = open_folder(dir).ok()?;
                    files.folders.insert(dir.to_path_buf(), folder);
                    folder
                }
            };
            // 现在那里有什么：没有文件为 `None`，否则为是否保留它
            let there = match files.ranks.get(&full) {
                Some(have) => Some(*have >= rank),
                None => match folder {
                    // 这次导出建的文件夹里没有以前的导出留下的文件
                    Folder::Made => None,
                    // 以前的导出在同一个文件夹里留下的：保留
                    Folder::Finished if full.exists() => {
                        files.ranks.insert(full.clone(), (true, u64::MAX));
                        Some(true)
                    }
                    Folder::Finished => None,
                    // 被中止的导出留下的：重新写
                    Folder::Stopped => Some(false),
                },
            };
            if there != Some(true) {
                files.writing.insert(full.clone());
                drop(files);
                let result = match (there, &source) {
                    (None, MediaSource::Bytes(b)) => write_new(&full, b),
                    // 已有的文件经临时文件替换，所以从不会被看到不完整的样子
                    _ => write(),
                };
                let mut files = written.lock();
                files.writing.remove(&full);
                if result.is_ok() {
                    files.ranks.insert(full.clone(), rank);
                }
                drop(files);
                written.done.notify_all();
                result.ok()?;
            }
        }
    }
    Some(ApiExportedMedia {
        kind,
        file_name: file_name.clone(),
        full_path: full.to_string_lossy().to_string(),
        relative_path: format!("{safe_talker}/{sub}/{file_name}"),
        thumbnail: false,
    })
}

/// 建好文件夹 `dir` 及其上级；返回 `dir` 是否是新建的（`false` 表示它本来就在）。
fn make_dir(dir: &Path) -> std::io::Result<bool> {
    let made = |result: std::io::Result<()>| match result {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    };
    match std::fs::create_dir(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = dir.parent() {
                std::fs::create_dir_all(parent)?;
            }
            made(std::fs::create_dir(dir))
        }
        result => made(result),
    }
}

/// 写到 `out` 的导出的媒体放在哪里：`<out dir>/media/<out name>`，以及 `out` 旁边的页面引用这个文件夹时用的路径
/// `media/<out name>`。
fn media_dir(out: &Path) -> (PathBuf, String) {
    let out_dir = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let name = api::sanitize_file_name(
        &out.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        "export",
    );
    (out_dir.join("media").join(&name), format!("media/{name}"))
}

/// [`ServiceHub::attach_export_media`] 复制了什么：媒体消息的计数，以及写出的头像（用户名到相对导出文件的路径）。
pub(super) struct ExportMedia {
    pub copied: crate::contract::MediaCopied,
    pub avatars: HashMap<String, String>,
}

/// 头像地址上的图片：`data:image/…;base64,` 地址直接解码，其他地址下载。地址里不是图片或取不到时为 `None`。
async fn avatar_picture(url: &str) -> Option<Vec<u8>> {
    let bytes = if let Some(rest) = url.strip_prefix("data:image/") {
        crate::sns::lenient_base64(rest.split_once(";base64,")?.1)
    } else {
        let client = crate::http_client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .ok()?;
        let resp = client.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.bytes().await.ok()?.to_vec()
    };
    crate::image::ImageFormat::of(&bytes)
        .is_some()
        .then_some(bytes)
}

/// 媒体本身没有名字时，消息的媒体文件名：`<kind>_<send time>_<local id>`。
///
/// 本地 ID 只在一个消息库内唯一，而一个会话的消息可能分布在多个消息库中，所以用发送时间区分本地 ID 相同的两条消息；
/// 两者都不变，所以同一条消息在每次导出中的文件名都相同。
fn message_file_stem(kind: &str, msg: &ChatMessage) -> String {
    format!("{kind}_{}_{}", msg.create_time, msg.local_id)
}

/// 是否是文件消息：类型为 6 的 app 消息。
fn is_file_message(xml_type: Option<&str>) -> bool {
    xml_type == Some("6")
}

/// 在线程之后、能用到数据库的地方逐个复制的媒体：表情（要下载）和文件（通过微信的媒体索引查找）。
fn copied_in_order(local_type: i64, xml_type: Option<&str>) -> bool {
    local_type == 47 || is_file_message(xml_type)
}

/// 对应桌面版 httpService 的 `detectImageExt`：图片的扩展名，位图为 `.bmp`，认不出的为 `.jpg`。
fn api_detect_image_ext(b: &[u8]) -> &'static str {
    match crate::image::ImageFormat::of(b) {
        Some(format) => format.extension(),
        None if b.starts_with(b"BM") => ".bmp",
        None => ".jpg",
    }
}

impl ServiceHub {
    /// `export media`：遍历一个会话（或全部会话）的消息，把图片、语音（转成 WAV）、视频、表情和文件复制到
    /// `<out>/<session>/<kind>s/`，每条消息的处理与 HTTP API 的 `media=1` 相同。
    ///
    /// `media_type` 为 image、voice、video、emoji、file 或 all。`start`、`end`（Unix 秒，不含 `end`）限定消息的时间。
    /// `found` 计的是媒体消息的条数（同一张图片发了两次算两条；`chat images` 列出的是去重后的文件）。
    ///
    /// # Errors
    ///
    /// `media_type` 不支持时返回用法错误，建不了 `out` 时返回运行时错误；账号没有配置好时返回配置错误，
    /// 读不出会话列表时返回数据库层的错误。
    pub async fn export_media(
        &self,
        session: Option<&str>,
        out: &Path,
        media_type: &str,
        start: Option<i64>,
        end: Option<i64>,
    ) -> AppResult<crate::contract::MediaExport> {
        let wanted = |k: &str| media_type == "all" || media_type == k;
        if !["image", "voice", "video", "emoji", "file", "all"].contains(&media_type) {
            return Err(AppError::usage(format!(
                "unsupported media type: {media_type}; use image, voice, video, emoji, file or all"
            )));
        }
        std::fs::create_dir_all(out)
            .map_err(|e| AppError::runtime(format!("create {}: {e}", out.display())))?;
        let wcdb = self.open_wcdb()?;
        let sessions: Vec<String> = match session {
            Some(s) => vec![s.to_string()],
            None => self
                .sessions_with(&wcdb)?
                .into_iter()
                .map(|s| s.username)
                .collect(),
        };
        let opts = ApiMediaOptions {
            enabled: true,
            images: wanted("image"),
            voices: wanted("voice"),
            videos: wanted("video"),
            emojis: wanted("emoji"),
            files: wanted("file"),
            // 消息本身没有头像：`export messages` 把头像放在它的页面上
            avatars: None,
        };
        // 每一类对应的 `local_type`：文件是类型为 6 的 app 消息，有几种 `local_type` 都可能是它
        let types: [(&[i64], &'static str, bool); 5] = [
            (&[3], "image", opts.images),
            (&[34], "voice", opts.voices),
            (&[43], "video", opts.videos),
            (&[47], "emoji", opts.emojis),
            (&chat::FILE_APP_TYPES, "file", opts.files),
        ];
        let my = self.wmy();

        // 第一轮：先收集媒体消息，进度条才知道总数
        self.emit_progress("media", "scanning messages", 0, sessions.len());
        let mut work: Vec<(String, &'static str, ChatMessage)> = Vec::new();
        for (i, sid) in sessions.iter().enumerate() {
            for &(codes, kind, enabled) in &types {
                if !enabled {
                    continue;
                }
                let mut seen: HashSet<(i64, i64)> = HashSet::new();
                for &code in codes {
                    let Ok(rows) = wcdb.messages_by_type(sid, code, Order::NewestFirst) else {
                        continue;
                    };
                    for m in chat_msg::map_msg_rows(rows, sid, &my) {
                        let in_range = start.is_none_or(|s| m.create_time >= s)
                            && end.is_none_or(|e| m.create_time < e);
                        let of_kind = kind != "file" || is_file_message(m.xml_type());
                        if m.local_type == code
                            && of_kind
                            && in_range
                            && seen.insert((m.local_id, m.create_time))
                        {
                            work.push((sid.clone(), kind, m));
                        }
                    }
                }
            }
            self.emit_progress("media", "scanning messages", i + 1, sessions.len());
        }

        // 第二轮：导出
        let total = work.len();
        let mut files: Vec<crate::contract::MediaFileExported> = Vec::new();
        let mut missing_by_kind: BTreeMap<&str, usize> = BTreeMap::new();
        let mut thumb_only = 0usize;
        // 图片（顺便解码 WXGF）、语音和视频不需要联网：先在几个线程上处理，再按顺序遍历列表，处理表情和文件并汇总结果。
        let mut done: Vec<Option<Option<ApiExportedMedia>>> = (0..total).map(|_| None).collect();
        {
            let local: Vec<usize> = (0..total)
                .filter(|i| !copied_in_order(work[*i].2.local_type, work[*i].2.xml_type()))
                .collect();
            let workers = std::thread::available_parallelism()
                .map_or(2, |n| n.get())
                .clamp(1, 8)
                .min(local.len().max(1));
            let next = std::sync::atomic::AtomicUsize::new(0);
            let finished = std::sync::atomic::AtomicUsize::new(0);
            let results = std::sync::Mutex::new(Vec::<(usize, Option<ApiExportedMedia>)>::new());
            let run = ExportRun::new(self);
            std::thread::scope(|scope| {
                for _ in 0..workers {
                    scope.spawn(|| loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&i) = local.get(k) else { break };
                        let (sid, _, msg) = &work[i];
                        let safe = api::sanitize_file_name(sid, "session");
                        let dir = out.join(&safe);
                        let r = self.export_local_media(msg, sid, &safe, &dir, &opts, Some(&run));
                        results.lock_or_panic().push((i, r));
                        let n = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        self.emit_progress(
                            "media",
                            &format!("exporting {media_type} media"),
                            n,
                            total,
                        );
                    });
                }
            });
            for (i, r) in results.into_inner_or_panic() {
                done[i] = Some(r);
            }
            // 见 `attach_export_media`：通过另一条消息的结果找到的图片，不论线程的先后都能导出
            for &i in &local {
                if matches!(done[i], Some(None)) && work[i].2.local_type == 3 {
                    let (sid, _, msg) = &work[i];
                    let safe = api::sanitize_file_name(sid, "session");
                    let dir = out.join(&safe);
                    done[i] =
                        Some(self.export_local_media(msg, sid, &safe, &dir, &opts, Some(&run)));
                }
            }
        }
        for (n, (sid, kind, msg)) in work.iter().enumerate() {
            let safe = api::sanitize_file_name(sid, "session");
            let session_dir = out.join(&safe);
            let (exported, in_parallel) = match done[n].take() {
                Some(r) => (r, true),
                None => (
                    self.export_media_for_message(&wcdb, msg, sid, &safe, &session_dir, &opts)
                        .await,
                    false,
                ),
            };
            match exported {
                Some(m) => {
                    if m.thumbnail {
                        thumb_only += 1;
                    }
                    files.push(crate::contract::MediaFileExported {
                        session: sid.clone(),
                        kind: m.kind.to_string(),
                        is_thumb: (m.kind == "image").then_some(m.thumbnail),
                        file_name: m.file_name,
                        path: m.full_path,
                        local_id: msg.local_id,
                        create_time: msg.create_time,
                    });
                }
                None => *missing_by_kind.entry(kind).or_default() += 1,
            }
            if !in_parallel {
                self.emit_progress(
                    "media",
                    &format!("exporting {media_type} media"),
                    n + 1,
                    total,
                );
            }
        }
        let missing: usize = missing_by_kind.values().sum();
        Ok(crate::contract::MediaExport {
            exported: files.len(),
            found: total,
            missing,
            missing_by_kind: missing_by_kind
                .into_iter()
                .map(|(k, n)| (k.to_string(), n))
                .collect(),
            thumb_only,
            note: "missing = media messages whose file is not on disk, as WeChat has not downloaded it, or could not be resolved; stickers need network access.\nthumbOnly = exported images that are only the thumbnail; open the original in WeChat, then export again for the HD image.".into(),
            sessions: sessions.len(),
            out: out.to_path_buf(),
            files,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media_dir(tag: &str) -> weflow_native::fixture::TestDir {
        weflow_native::fixture::TestDir::new(&format!("weflow-media-{tag}"))
    }

    fn put(to: &MediaTarget<'_>, bytes: &[u8], thumbnail: bool) -> Option<ApiExportedMedia> {
        put_media(
            to,
            ("image", "images"),
            "a.jpg".into(),
            MediaSource::Bytes(bytes),
            thumbnail,
        )
    }

    #[test]
    fn a_shared_image_file_ends_up_with_the_best_rendition_in_any_order() {
        for (tag, order) in [
            (
                "thumb-first",
                [(&b"thumb"[..], true), (&b"full image"[..], false)],
            ),
            (
                "full-first",
                [(&b"full image"[..], false), (&b"thumb"[..], true)],
            ),
        ] {
            let dir = media_dir(tag);
            let written = ExportedFiles::default();
            let to = MediaTarget {
                session_dir: &dir,
                safe_talker: "s",
                written: Some(&written),
            };
            for (bytes, thumbnail) in order {
                assert!(put(&to, bytes, thumbnail).is_some());
            }
            assert_eq!(
                std::fs::read(dir.join("images").join("a.jpg")).unwrap(),
                b"full image",
                "{tag}"
            );
            // 两个完整版中保留较大的
            assert!(put(&to, b"full", false).is_some());
            assert!(put(&to, b"a larger full image", false).is_some());
            assert_eq!(
                std::fs::read(dir.join("images").join("a.jpg")).unwrap(),
                b"a larger full image"
            );
        }
    }

    #[test]
    fn threads_writing_the_same_files_at_once_leave_the_best_rendition_of_each() {
        const THREADS: usize = 8;
        const FILES: usize = 6;
        // 每个线程把每个文件写一次，大小与其他线程写的都不同；每三个中有一个是缩略图
        let rendition = |thread: usize, file: usize| {
            let size = 1 + (thread * 7 + file * 3) % THREADS + THREADS * thread;
            let thumbnail = (thread + file).is_multiple_of(3);
            (vec![u8::try_from(thread).unwrap(); size], thumbnail)
        };
        for round in 0..5 {
            let dir = media_dir(&format!("threads-{round}"));
            let written = ExportedFiles::default();
            let to = MediaTarget {
                session_dir: &dir,
                safe_talker: "s",
                written: Some(&written),
            };
            std::thread::scope(|s| {
                for thread in 0..THREADS {
                    let (to, rendition) = (&to, &rendition);
                    s.spawn(move || {
                        for i in 0..FILES {
                            // 各线程按不同的顺序处理这些文件
                            let file = (i + thread) % FILES;
                            let (bytes, thumbnail) = rendition(thread, file);
                            let name = format!("{file}/a.jpg");
                            let source = MediaSource::Bytes(&bytes);
                            let put = put_media(to, ("image", "images"), name, source, thumbnail);
                            assert!(put.is_some());
                        }
                    });
                }
            });
            drop(written);
            for file in 0..FILES {
                let best = (0..THREADS)
                    .map(|thread| rendition(thread, file))
                    .max_by_key(|(bytes, thumbnail)| (!thumbnail, bytes.len()))
                    .unwrap()
                    .0;
                let folder = dir.join("images").join(file.to_string());
                assert_eq!(std::fs::read(folder.join("a.jpg")).unwrap(), best);
                let names: Vec<_> = std::fs::read_dir(&folder).unwrap().collect();
                assert_eq!(names.len(), 1, "no temporary file or marker is left behind");
            }
        }
    }

    #[test]
    fn a_folder_is_marked_while_written_into_and_a_stopped_export_has_its_files_written_again() {
        let dir = media_dir("incomplete");
        let images = dir.join("images");
        std::fs::create_dir_all(&images).unwrap();
        std::fs::write(images.join("a.jpg"), b"cut sh").unwrap();
        // 往文件夹写文件时被中止的导出在那里留下了标记
        std::fs::write(images.join(INCOMPLETE), b"").unwrap();
        let written = ExportedFiles::default();
        let to = MediaTarget {
            session_dir: &dir,
            safe_talker: "s",
            written: Some(&written),
        };
        assert!(put(&to, b"cut short? no: whole", false).is_some());
        assert_eq!(
            std::fs::read(images.join("a.jpg")).unwrap(),
            b"cut short? no: whole",
            "a file a stopped export left is written again"
        );
        // 新的文件夹在导出写入期间同样有标记
        let other = MediaTarget {
            session_dir: &dir.join("other"),
            safe_talker: "s",
            written: Some(&written),
        };
        assert!(put(&other, b"new", false).is_some());
        assert!(images.join(INCOMPLETE).exists());
        assert!(dir.join("other").join("images").join(INCOMPLETE).exists());
        drop(written);
        for folder in [images.clone(), dir.join("other").join("images")] {
            assert!(!folder.join(INCOMPLETE).exists(), "{folder:?}");
            let names: Vec<_> = std::fs::read_dir(&folder).unwrap().collect();
            assert_eq!(names.len(), 1, "{folder:?}");
        }
        // 下一次导出发现文件夹已完成：保留已有的文件
        let written = ExportedFiles::default();
        let to = MediaTarget {
            session_dir: &dir,
            safe_talker: "s",
            written: Some(&written),
        };
        assert!(put(&to, b"a later, larger image", false).is_some());
        assert_eq!(
            std::fs::read(images.join("a.jpg")).unwrap(),
            b"cut short? no: whole"
        );
    }

    #[test]
    fn a_folder_is_made_with_those_above_it_and_says_whether_it_was_missing() {
        let dir = media_dir("make-dir");
        let deep = dir.join("a").join("b");
        assert!(make_dir(&deep).unwrap());
        assert!(deep.is_dir());
        assert!(!make_dir(&deep).unwrap(), "already there");
    }

    #[test]
    fn a_file_from_an_earlier_export_is_kept_and_empty_data_is_not_written() {
        let dir = media_dir("earlier");
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::write(dir.join("images").join("a.jpg"), b"earlier").unwrap();
        let written = ExportedFiles::default();
        let to = MediaTarget {
            session_dir: &dir,
            safe_talker: "s",
            written: Some(&written),
        };
        assert!(put(&to, b"a newer, larger image", false).is_some());
        assert_eq!(
            std::fs::read(dir.join("images").join("a.jpg")).unwrap(),
            b"earlier"
        );
        assert!(put(&to, b"", false).is_none());
        drop(written);
        let names: Vec<_> = std::fs::read_dir(dir.join("images")).unwrap().collect();
        assert_eq!(names.len(), 1, "no temporary file or marker is left behind");
    }
}
