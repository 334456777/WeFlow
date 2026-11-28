//! 账号数据的备份：创建、查看和恢复。

use std::path::Path;

use super::ServiceHub;
use crate::error::{AppError, AppResult};

impl ServiceHub {
    /// 把配置的账号备份到 `out`（`backup create`）：账号的数据库、`options` 要求的媒体，以及要求时 WeFlow 的表情缓存。
    ///
    /// # Errors
    /// 账号没有配置好时返回配置错误；备份写不出来时返回运行时错误。
    pub fn backup_create(
        &self,
        out: &Path,
        options: &crate::backup::BackupOptions,
    ) -> AppResult<crate::contract::BackupCreated> {
        let (account_dir, _, _) = self.connection_inputs()?;
        let manifest = crate::backup::create_backup(
            &account_dir,
            options,
            Some(&self.cache_base().join("Emojis")),
            &self.ctx.version,
            out,
            &|current, total| self.emit_progress("backup", "creating backup", current, total),
        )
        .map_err(|e| AppError::runtime(format!("{e:#}")))?;
        Ok(crate::contract::BackupCreated {
            out: out.to_path_buf(),
            entries: manifest.entries.len(),
            wxid: manifest.wxid,
            created_at: manifest.created_at,
            skipped: manifest.skipped,
            unsettled: manifest.unsettled,
        })
    }

    /// `path` 处备份的清单。
    ///
    /// # Errors
    /// 文件不存在时返回 `not_found`（命令 API 回答 404，#822）；不能作为备份读取时返回运行时错误。
    pub fn backup_inspect(&self, path: &Path) -> AppResult<crate::backup::BackupManifest> {
        crate::backup::inspect_backup(path).map_err(|e| {
            let missing = e.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
            });
            if missing {
                AppError::not_found(format!("{e:#}"))
            } else {
                AppError::runtime(format!("{e:#}"))
            }
        })
    }

    /// 把 `path` 处的备份恢复到文件夹 `target_dir`（`backup restore`），其中的表情放回 WeFlow 的表情缓存。
    ///
    /// # Errors
    /// 备份读不出来或文件写不进去时返回运行时错误。
    pub fn backup_restore(
        &self,
        path: &Path,
        target_dir: &Path,
    ) -> AppResult<crate::contract::BackupRestored> {
        let emoji_dir = self.cache_base().join("Emojis");
        let stickers =
            crate::backup::restore_backup(path, target_dir, Some(&emoji_dir), &|current, total| {
                self.emit_progress("restore", "restoring backup", current, total);
            })
            .map_err(|e| AppError::runtime(format!("{e:#}")))?;
        Ok(crate::contract::BackupRestored {
            restored_to: target_dir.to_path_buf(),
            stickers_restored_to: stickers.then_some(emoji_dir),
        })
    }
}
