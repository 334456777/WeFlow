//! 消息的视频、文件和表情：它们的文件在哪里，以及下载表情。

use std::path::{Path, PathBuf};

use serde_json::json;

use super::{chat, ServiceHub};
use crate::error::{AppError, AppResult};

impl ServiceHub {
    /// 按 md5 找到的消息的视频和封面在哪里（`video info`）。
    ///
    /// # Errors
    ///
    /// 档案没有数据库路径或账号，或者账号目录不存在时返回配置错误。
    pub fn video_info(
        &self,
        md5: &str,
        include_poster: bool,
        format: crate::video::PosterFormat,
    ) -> AppResult<crate::video::VideoInfo> {
        let account_dir = self.account_dir_only()?;
        let info = crate::video::video_info(
            &account_dir.join("msg").join("video"),
            md5,
            include_poster,
            format,
        );
        if info.exists {
            return Ok(info);
        }
        Ok(self
            .open_wcdb()
            .ok()
            .and_then(|wcdb| linked_video(&wcdb, &account_dir, &[md5]))
            .map_or(info, |files| {
                crate::video::info_of(&files, include_poster, format)
            }))
    }

    /// 微信为一条视频消息保存的文件（`msg/video/<yyyy-mm>/<md5>.mp4` 及其封面和缩略图），按消息中的 md5 查找。
    ///
    /// # Errors
    ///
    /// 消息 ID 无效或消息不是视频时返回用法错误，视频消息没有 md5 时返回运行时错误；找不到消息，
    /// 或视频不在本机（微信还没下载）时返回 `not_found`；账号没有配置好时返回配置错误。
    pub fn video_files_for_message(
        &self,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
    ) -> AppResult<crate::video::VideoFiles> {
        let local_id = crate::api::js_parse_int(msg_id)
            .ok_or_else(|| AppError::usage("invalid message id"))?;
        let wcdb = self.open_wcdb()?;
        let row = chat::message_by_local_id(&wcdb, session_id, local_id, create_time, Some(43))?
            .ok_or_else(|| AppError::not_found("message not found"))?;
        let msg = crate::chat_msg::map_msg_row(
            row,
            &weflow_native::msg::table_name_for(session_id),
            &self.my_wxid_cleaned(),
        );
        if msg.local_type != 43 {
            return Err(AppError::usage("the message is not a video"));
        }
        let md5 = msg
            .video_md5
            .filter(|m| !m.is_empty())
            .ok_or_else(|| AppError::runtime("the video message has no md5"))?;
        let month = crate::image::year_month_from_create_time(Some(msg.create_time));
        let account_dir = self.account_dir_only()?;
        let xml_md5 = crate::video::parse_video_md5(&msg.raw_content).unwrap_or_default();
        crate::video::video_files(&account_dir.join("msg").join("video"), &md5, &month)
            .or_else(|| linked_video(&wcdb, &account_dir, &[&md5, &xml_md5]))
            .ok_or_else(|| {
                AppError::runtime("the video is not on this computer: open it in WeChat first")
                    .with_details(json!({ "failureKind": "not_found" }))
            })
    }

    /// 配置的账号的微信视频目录，用来查找许多条消息的视频。
    pub(super) fn video_folder(&self) -> Option<crate::video::VideoFolder> {
        let account_dir = self.account_dir_only().ok()?;
        Some(crate::video::VideoFolder::new(
            account_dir.join("msg").join("video"),
        ))
    }

    /// 在 `month`（`yyyy-mm`）发送的消息在磁盘上的视频的路径，微信没有保存时为 `None`。
    ///
    /// 先按 `md5s` 的第一个在 `videos` 中找（见 [`VideoFolder::find`](crate::video::VideoFolder::find)），
    /// 找不到时用微信的媒体索引给出的其中一个的位置。
    pub(super) fn video_file_path(
        &self,
        videos: &crate::video::VideoFolder,
        md5s: &[&str],
        month: &str,
    ) -> Option<PathBuf> {
        videos
            .find(md5s.first()?, month)
            .map(|files| files.video)
            .or_else(|| {
                let account_dir = self.account_dir_only().ok()?;
                let wcdb = self.open_wcdb().ok()?;
                linked_video(&wcdb, &account_dir, md5s).map(|files| files.video)
            })
    }

    /// 文件消息在磁盘上的文件，以及导出时它在 `files/` 下的位置 `<month>/<file name>`。
    ///
    /// 位置与微信在 `msg/file/<month>/` 中的存放方式一致，不同月份发送的同名文件不会混在一起。先按文件的 `md5`
    /// 查微信的媒体索引，再在发送的月份（本地时间）里找消息中的文件名 `name`。
    pub(super) fn message_file(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        md5: Option<&str>,
        name: Option<&str>,
        create_time: i64,
    ) -> Option<(PathBuf, String)> {
        use chrono::TimeZone as _;
        let files = self.account_dir_only().ok()?.join("msg").join("file");
        let linked = md5
            .filter(|m| !m.trim().is_empty())
            .and_then(|m| wcdb.file_hardlinks(m).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|rel| {
                let month = rel.parent()?.file_name()?.to_str()?.to_string();
                let name = rel.file_name()?.to_str()?.to_string();
                Some((month, name))
            });
        let sent = name
            .map(|n| crate::api::sanitize_file_name(n, ""))
            .filter(|n| !n.is_empty())
            .zip(
                chrono::Local
                    .timestamp_opt(create_time, 0)
                    .single()
                    .map(|t| t.format("%Y-%m").to_string()),
            )
            .map(|(name, month)| (month, name));
        linked.chain(sent).find_map(|(month, name)| {
            let path = files.join(&month).join(&name);
            path.is_file().then(|| (path, format!("{month}/{name}")))
        })
    }

    // ── 媒体导出 ─────────────────────────────────────────────────────────

    /// 把会话最近 500 条消息中的表情下载到文件夹 `out`（`chat emoji`），`out` 中已有的不重新下载。
    ///
    /// # Errors
    /// 读不出消息或建不了 `out` 时返回错误。
    pub async fn emoji_download(
        &self,
        session_id: &str,
        out: &Path,
    ) -> AppResult<crate::contract::EmojisDownloaded> {
        let messages = self.messages(session_id, 500, 0)?;
        let metas = crate::media::extract_emoji_urls(&messages);
        let total = metas.len();
        if total == 0 {
            return Ok(crate::contract::EmojisDownloaded {
                found: 0,
                downloaded: 0,
                failed: 0,
                files: None,
            });
        }
        std::fs::create_dir_all(out)
            .map_err(|e| AppError::runtime(format!("create {}: {e}", out.display())))?;
        let crate::media::EmojiDownloads { files, failed } =
            crate::media::download_emojis(&metas, out, &|current, t| {
                self.emit_progress("emoji", "downloading emojis", current, t);
            })
            .await
            .map_err(|e| AppError::runtime(format!("{e:#}")))?;
        let downloaded = files.iter().filter(|f| !f.cached).count();
        Ok(crate::contract::EmojisDownloaded {
            found: total,
            downloaded,
            failed,
            files: Some(files),
        })
    }
}

/// 微信的媒体索引（`hardlink.db`）为 `md5s` 中第一个在磁盘上有文件的 md5 列出的视频文件（#651）。
///
/// 文件名与消息的 md5 不同的视频在月份目录里找不到；索引放在月份目录之后查，因为许多视频在索引里没有记录。
fn linked_video(
    wcdb: &weflow_native::wcdb::Wcdb,
    account_dir: &Path,
    md5s: &[&str],
) -> Option<crate::video::VideoFiles> {
    md5s.iter()
        .filter(|md5| !md5.trim().is_empty())
        .find_map(|md5| {
            wcdb.video_hardlinks(md5)
                .ok()?
                .into_iter()
                .map(|rel| account_dir.join(rel))
                .find(|path| path.is_file())
        })
        .map(crate::video::files_beside)
}
