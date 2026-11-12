//! Backups of the account's WeFlow data: create, inspect, restore.

use std::path::Path;

use super::ServiceHub;
use crate::error::{AppError, AppResult};

impl ServiceHub {
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
        .map_err(|e| AppError::runtime(e.to_string()))?;
        Ok(crate::contract::BackupCreated {
            out: out.to_path_buf(),
            entries: manifest.entries.len(),
            wxid: manifest.wxid,
            created_at: manifest.created_at,
        })
    }

    pub fn backup_inspect(&self, path: &Path) -> AppResult<crate::backup::BackupManifest> {
        crate::backup::inspect_backup(path).map_err(|e| AppError::runtime(e.to_string()))
    }

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
            .map_err(|e| AppError::runtime(e.to_string()))?;
        Ok(crate::contract::BackupRestored {
            restored_to: target_dir.to_path_buf(),
            stickers_restored_to: stickers.then_some(emoji_dir),
        })
    }
}
