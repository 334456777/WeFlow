//! 按部分列出和清理 WeFlow 的缓存（`weflow cache list`、`cache clear`），
//! 以及删除当前账号的数据（`weflow cache clear-account`）。
//! 只动 WeFlow 自己的文件，从不写微信的数据。

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::*;
use crate::contract::CachePart;

/// `path` 下文件的总字节数和个数（`path` 是文件时就是它自己）；不存在时为 0。
pub fn disk_usage(path: &Path) -> (u64, u64) {
    WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .fold((0, 0), |(bytes, files), e| {
            (bytes + e.metadata().map_or(0, |m| m.len()), files + 1)
        })
}

/// 文件名或文件夹名中以完整的词出现了某个（小写的）ID 时，它属于这个账号，不区分大小写：`wxid_abc`、
/// `wxid_abc_1a2b`、`wxid_abc-moments.json` 和 `group_my_message_counts_wxid_abc.json` 属于，
/// `wxid_abc123.json`（另一个账号）不属于。
fn account_name_matches(name: &str, ids: &[String]) -> bool {
    let lowered = name.trim().to_lowercase();
    ids.iter().any(|id| contains_word(&lowered, id))
}

/// `id` 是否出现在 `name` 中，且紧挨着的前后都不是 ASCII 字母或数字。
///
/// 微信 ID 由 ASCII 字母、数字、`_` 和 `-` 组成，挨着字母或数字说明是另一个更长的 ID；
/// `_`、`-`、`.` 和其他字符都是 ID 的边界。
fn contains_word(name: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    let (name, id) = (name.as_bytes(), id.as_bytes());
    let part_of_id = |at: Option<&u8>| at.is_some_and(u8::is_ascii_alphanumeric);
    name.windows(id.len()).enumerate().any(|(start, window)| {
        window == id
            && !part_of_id(start.checked_sub(1).and_then(|before| name.get(before)))
            && !part_of_id(name.get(start + id.len()))
    })
}

/// 删除一个文件或文件夹；它存在并且已删掉时返回 true。
fn remove_path(path: &Path, removed: &mut Vec<String>, warnings: &mut Vec<String>) -> bool {
    if !path.exists() {
        return false;
    }
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match result {
        Ok(()) => {
            removed.push(path.to_string_lossy().to_string());
            true
        }
        Err(e) => {
            warnings.push(format!("{}: {e}", path.display()));
            false
        }
    }
}

/// `root` 下以账号命名的条目。
fn matched_entries(root: &Path, ids: &[String]) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| account_name_matches(&e.file_name().to_string_lossy(), ids))
        .map(|e| e.path())
        .collect();
    paths.sort();
    paths
}

impl ServiceHub {
    /// 存放 `part` 的文件和文件夹，存在与否都列出；不用的程序、运行中的服务没有显示的 WebUI 版本只列出存在的。
    pub fn cache_part_paths(&self, part: CachePart) -> Vec<PathBuf> {
        let base = self.cache_base();
        match part {
            CachePart::Images => vec![base.join("Images")],
            CachePart::Voices => vec![base.join("Voices")],
            CachePart::Emojis => vec![base.join("Emojis")],
            CachePart::Sns => vec![base.join("sns_cache")],
            CachePart::Analytics => vec![base.join("analytics_cache.json")],
            CachePart::Api => vec![
                base.join("api-media"),
                base.join("push-avatar-files"),
                self.serve_files_dir(),
            ],
            // 校验通过的密钥的指纹，之后的命令可以跳过较慢的校验
            CachePart::Keys => vec![self.ctx.cache_dir().join("verified-keys")],
            CachePart::Runtime => self.unused_programs(),
            // 下载的 WebUI 版本（#148），运行中的服务正在显示的除外（#1139）
            CachePart::Web => self.web_store().clearable(),
        }
    }

    /// 不用的程序：早期版本解压到 `runtime/` 的运行时目录，
    /// 以及这些版本安装在 `ffmpeg/<release>/` 的 ffmpeg（用户自己放在 `ffmpeg/` 下的文件保留）。
    fn unused_programs(&self) -> Vec<PathBuf> {
        let subfolders = |dir: PathBuf| -> Vec<PathBuf> {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return Vec::new();
            };
            let mut dirs: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect();
            dirs.sort();
            dirs
        };
        let mut dirs = subfolders(self.ctx.home_dir.join("runtime"));
        dirs.extend(subfolders(self.ctx.home_dir.join("ffmpeg")));
        dirs
    }

    /// `weflow serve` 为没有指定 `--out` 的 `?return=file` 请求写文件的目录，
    /// 其中每个运行中的服务各有自己的文件夹（见 [`crate::serve_files`]）。
    pub fn serve_files_dir(&self) -> PathBuf {
        self.cache_base().join("serve-files")
    }

    /// `weflow cache list`：每个部分的路径、大小和文件数。
    pub fn cache_list(&self) -> Vec<crate::contract::CacheUsage> {
        CachePart::ALL
            .iter()
            .map(|&part| {
                let paths = self.cache_part_paths(part);
                let (bytes, files) = paths
                    .iter()
                    .map(|p| disk_usage(p))
                    .fold((0, 0), |(b, f), (pb, pf)| (b + pb, f + pf));
                crate::contract::CacheUsage {
                    part,
                    paths,
                    bytes,
                    files,
                }
            })
            .collect()
    }

    /// `weflow cache clear`：删除 `parts` 的文件，并清掉内存中保存的这些部分的内容。
    pub fn cache_clear(&self, parts: &[CachePart]) -> crate::contract::CacheCleared {
        let (mut removed, mut warnings) = (Vec::new(), Vec::new());
        let mut freed = 0u64;
        let serve_files = self.serve_files_dir();
        for &part in parts {
            if part == CachePart::Web {
                // 与 `web prune` 一样，保留运行中的服务正在显示的版本（#1139）
                let store = self.web_store();
                let before = disk_usage(store.root()).0;
                let (gone, failed) = store.clear();
                removed.extend(gone.iter().map(|p| p.to_string_lossy().into_owned()));
                warnings.extend(failed);
                freed += before.saturating_sub(disk_usage(store.root()).0);
                continue;
            }
            for path in self.cache_part_paths(part) {
                let (bytes, _) = disk_usage(&path);
                if path == serve_files {
                    // 保留运行中的服务的文件夹（#815）
                    let (gone, failed) = crate::serve_files::remove_unheld(&path);
                    removed.extend(gone.iter().map(|p| p.to_string_lossy().into_owned()));
                    warnings.extend(failed);
                    freed += bytes.saturating_sub(disk_usage(&path).0);
                } else if remove_path(&path, &mut removed, &mut warnings) {
                    freed += bytes;
                }
            }
            match part {
                CachePart::Images => self.image_forget_cached(),
                CachePart::Sns => self.sns_clear_memory_cache(),
                _ => {}
            }
        }
        crate::contract::CacheCleared {
            removed_paths: removed,
            freed_bytes: freed,
            warnings,
        }
    }

    /// [`Self::clear_current_account_data`] 要删除的路径，只列出存在的。
    ///
    /// 包括 WeFlow 为当前账号保存的缓存（图片、语音、表情、朋友圈、统计、群提示、密钥指纹），
    /// 账号的见解记录（以及记录文件读不出时另存的副本），和 `export_dirs` 中以账号命名的条目。
    ///
    /// WeFlow 7.0.0 及更早版本写的、含所有账号见解记录的文件先拆分成各账号的文件（不改动任何记录），
    /// 列出的就是这个账号自己的文件。
    ///
    /// # Errors
    ///
    /// 没有设置 wxid 时返回配置错误。
    pub fn account_data_paths(&self, export_dirs: &[PathBuf]) -> AppResult<Vec<PathBuf>> {
        let id = self.my_wxid_cleaned();
        if id.is_empty() {
            return Err(AppError::config(
                "no current account configured: wxid is not set, so there is nothing to clear",
            ));
        }
        let ids = [id.to_lowercase()];
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut add = |path: PathBuf| {
            if path.exists() && !paths.contains(&path) {
                paths.push(path);
            }
        };
        let base = self.cache_base();
        let roots = [
            base.clone(),
            base.join("Images"),
            base.join("Voices"),
            base.join("Emojis"),
        ];
        for root in &roots {
            add(root.join(&id));
        }
        for root in &roots {
            matched_entries(root, &ids).into_iter().for_each(&mut add);
        }
        add(base.join("analytics_cache.json"));
        // `cache_path` 指向别的目录时，主目录的缓存里还有群提示（#1013）
        if let Some(path) = self.legacy_hint_path() {
            add(path);
        }
        // 校验通过的密钥的指纹（档案里的密钥随账号设置一起清除）
        add(self.ctx.cache_dir().join("verified-keys"));
        let home = &self.ctx.home_dir;
        crate::insight::split_legacy_records(home);
        let records = crate::insight::records_file(home, &self.insight_scope());
        let set_aside = crate::insight::set_aside_records_files(&records);
        add(records);
        set_aside.into_iter().for_each(&mut add);
        for dir in export_dirs {
            matched_entries(dir, &ids).into_iter().for_each(&mut add);
        }
        Ok(paths)
    }

    /// 删除 [`Self::account_data_paths`] 列出的路径。
    ///
    /// 之后由调用方重置档案的账号设置，与桌面版的做法相同。
    ///
    /// # Errors
    ///
    /// 同 [`Self::account_data_paths`]；删不掉的路径写进结果的 `warnings`，不算失败。
    pub fn clear_current_account_data(
        &self,
        export_dirs: &[PathBuf],
    ) -> AppResult<crate::contract::AccountCleared> {
        let paths = self.account_data_paths(export_dirs)?;
        let (mut removed, mut warnings) = (Vec::new(), Vec::new());
        for path in &paths {
            remove_path(path, &mut removed, &mut warnings);
        }
        self.sns_clear_memory_cache();
        self.image_forget_cached();
        Ok(crate::contract::AccountCleared {
            removed_paths: removed,
            profile_reset: true,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cache_part_is_written_by_the_name_cache_clear_takes() {
        for part in CachePart::ALL {
            assert_eq!(serde_json::to_value(part).unwrap(), part.name());
        }
    }

    #[test]
    fn names_match_the_account_ids() {
        let ids = vec!["wxid_abc".to_string()];
        assert!(account_name_matches("wxid_abc", &ids));
        assert!(account_name_matches("WXID_ABC_1a2b", &ids));
        assert!(account_name_matches("export-wxid_abc-2026", &ids));
        // WeFlow 为账号写的名字：账号的文件夹、以账号命名的文件、群提示文件
        assert!(account_name_matches("wxid_abc_xxxx", &ids));
        assert!(account_name_matches("wxid_abc-moments.json", &ids));
        assert!(account_name_matches("wxid_abc.json", &ids));
        assert!(account_name_matches(
            "group_my_message_counts_wxid_abc.json",
            &ids
        ));
        assert!(!account_name_matches("wxid_other", &ids));
        assert!(!account_name_matches("", &ids));
    }

    #[test]
    fn a_longer_id_is_another_account() {
        let ids = vec!["wxid_abc".to_string()];
        for name in [
            "wxid_abc123.json",
            "xwxid_abc",
            "wxid_abcd_1a2b",
            "2wxid_abc.json",
        ] {
            assert!(!account_name_matches(name, &ids), "{name}");
        }
        // 老账号自定义的短微信号
        let ids = vec!["john".to_string()];
        assert!(account_name_matches("john_1a2b", &ids));
        assert!(account_name_matches("chat-john.json", &ids));
        for name in ["johnny.json", "littlejohn", "john2"] {
            assert!(!account_name_matches(name, &ids), "{name}");
        }
    }

    #[test]
    fn an_empty_id_matches_nothing() {
        assert!(!account_name_matches("anything", &[String::new()]));
        assert!(!account_name_matches("wxid_abc", &[]));
    }

    #[test]
    fn an_id_is_found_after_a_partial_match() {
        // 第一个 `wxid_abc` 是更长的 ID 的一部分，第二个是独立的
        let ids = vec!["wxid_abc".to_string()];
        assert!(account_name_matches("wxid_abcd-wxid_abc.json", &ids));
        assert!(account_name_matches("聊天记录wxid_abc", &ids));
    }
}
