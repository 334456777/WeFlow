//! Videos, files and stickers of messages: where their files are, and downloading stickers.

use std::path::{Path, PathBuf};

use serde_json::json;

use super::{chat, ServiceHub};
use crate::error::{AppError, AppResult};

impl ServiceHub {
    /// `video:getVideoInfo`
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

    /// The files WeChat keeps of a message's video (`msg/video/<yyyy-mm>/<md5>.mp4`, its cover and thumbnail),
    /// found by the md5 in the message. Not there (WeChat has not downloaded it): a `not_found` failure.
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
            .ok_or_else(|| AppError::runtime("message not found"))?;
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

    /// Path of the on-disk video of a message, if WeChat stored one: by the first of `md5s` in the month folders,
    /// else where WeChat's media index puts one of them.
    pub(super) fn video_file_path(&self, md5s: &[&str]) -> Option<PathBuf> {
        let account_dir = self.account_dir_only().ok()?;
        let info = crate::video::video_info(
            &account_dir.join("msg").join("video"),
            md5s.first()?,
            false,
            crate::video::PosterFormat::DataUrl,
        );
        info.video_url
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .or_else(|| {
                let wcdb = self.open_wcdb().ok()?;
                linked_video(&wcdb, &account_dir, md5s).map(|files| files.video)
            })
    }

    /// A file message's file on disk, and where an export puts it below `files/`: `<month>/<file name>`, as WeChat
    /// keeps it in `msg/file/<month>/`, so files of one name sent in different months stay apart. WeChat's media
    /// index is asked first (by the file's `md5`), then the message's file `name` is looked for in the month it was
    /// sent (local time).
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

    // ── Media export ─────────────────────────────────────────────────────────

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
                files: None,
            });
        }
        std::fs::create_dir_all(out)
            .map_err(|e| AppError::runtime(format!("create {}: {e}", out.display())))?;
        let results = crate::media::download_emojis(&metas, out, &|current, t| {
            self.emit_progress("emoji", "downloading emojis", current, t)
        })
        .await
        .map_err(|e| AppError::runtime(e.to_string()))?;
        let downloaded = results.iter().filter(|f| !f.cached).count();
        Ok(crate::contract::EmojisDownloaded {
            found: total,
            downloaded,
            files: Some(results),
        })
    }
}

/// The files of the video that WeChat's media index (`hardlink.db`) lists for the first of `md5s` that has one on
/// disk (#651). The month folders miss a video whose file is named otherwise than the message's md5; the index is
/// asked only after them, because many videos have no row in it.
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
