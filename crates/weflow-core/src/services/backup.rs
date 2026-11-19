//! Backups of the account's WeFlow data: create, inspect, restore.

use std::path::Path;

use super::ServiceHub;
use crate::error::{AppError, AppResult};

impl ServiceHub {
    /// Writes a backup of the configured account to `out` (`backup create`): its databases, the media `options`
    /// asks for, and WeFlow's sticker cache when asked.
    ///
    /// # Errors
    /// A config error when the account is not set up; a runtime error when the backup cannot be written.
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

    /// The manifest of the backup at `path`.
    ///
    /// # Errors
    /// A file that is not there is `not_found` (a 404 of the command API, #822); one that cannot be read as a backup
    /// is a runtime error.
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

    /// Restores the backup at `path` into the folder `target_dir` (`backup restore`), and its stickers into
    /// WeFlow's sticker cache.
    ///
    /// # Errors
    /// A runtime error when the backup cannot be read or a file cannot be written.
    pub fn backup_restore(
        &self,
        path: &Path,
        target_dir: &Path,
    ) -> AppResult<crate::contract::BackupRestored> {
        let emoji_dir = self.cache_base().join("Emojis");
        let stickers =
            crate::backup::restore_backup(path, target_dir, Some(&emoji_dir), &|current, total| {
                self.emit_progress("restore", "restoring backup", current, total)
            })
            .map_err(|e| AppError::runtime(format!("{e:#}")))?;
        Ok(crate::contract::BackupRestored {
            restored_to: target_dir.to_path_buf(),
            stickers_restored_to: stickers.then_some(emoji_dir),
        })
    }
}
