//! `weflow cache`: what WeFlow's cache takes, and clearing it (after a confirmation, or `--dry-run`).

use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::term::{align_columns, format_bytes, green, is_yes};
use crate::CacheShown;
use serde_json::{json, Value};
use weflow_core::config::{AppContext, ConfigStore};
use weflow_core::contract::{PlannedRemoval, RemovalPlan};
use weflow_core::error::{AppError, AppResult};
use weflow_core::services::{CachePart, ServiceHub};

#[derive(Args, Debug)]
pub(crate) struct CacheCommand {
    #[command(subcommand)]
    pub(crate) command: CacheSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum CacheSubcommand {
    /// Show each part of the cache: the option of `cache clear` that removes it, its size and where it is
    List,
    /// Remove the chosen parts of the cache; shows what goes and asks [y/N] first
    Clear(CacheClearArgs),
    /// Remove WeFlow's data of the current account and take the account out of this profile: its cached images,
    /// voices, stickers, Moments, analytics and key fingerprints, then db_path, wxid, decrypt_key and the image keys
    /// of the profile. Shows what goes and asks [y/N] first. WeChat's own files are never touched.
    ClearAccount {
        /// Folder of exports; files and folders named after the account are removed too (repeatable)
        #[arg(long = "exports-dir")]
        exports_dir: Vec<PathBuf>,
        /// Do not ask before removing
        #[arg(short = 'y', long)]
        yes: bool,
        /// Only list what would be removed, with its size; remove nothing
        #[arg(long, conflicts_with = "yes")]
        dry_run: bool,
    },
}

#[derive(Args, Debug)]
pub(crate) struct CacheClearArgs {
    /// Decrypted images
    #[arg(long)]
    images: bool,
    /// Decoded voice messages
    #[arg(long)]
    voices: bool,
    /// Stickers
    #[arg(long)]
    emojis: bool,
    /// Images and videos of Moments
    #[arg(long)]
    sns: bool,
    /// The analytics aggregate earlier versions kept
    #[arg(long)]
    analytics: bool,
    /// Media exported and avatars pushed by the HTTP API, and files returned by `serve` (`?return=file`)
    #[arg(long)]
    api: bool,
    /// Fingerprints of keys that worked (the next command checks the key again)
    #[arg(long)]
    keys: bool,
    /// Runtimes unpacked by other WeFlow versions (the running version's stays), and the ffmpeg earlier versions installed in ffmpeg/
    #[arg(long)]
    runtime: bool,
    /// WebUI versions downloaded by `weflow web` and `serve` (the one `serve` needs is downloaded again)
    #[arg(long)]
    web: bool,
    /// Every part above
    #[arg(long)]
    all: bool,
    /// Do not ask before removing
    #[arg(short = 'y', long)]
    yes: bool,
    /// Only list what would be removed, with its size; remove nothing
    #[arg(long, conflicts_with = "yes")]
    dry_run: bool,
}

impl CacheClearArgs {
    fn parts(&self) -> Vec<CachePart> {
        // in the order of `CachePart::ALL`
        let chosen = [
            self.images,
            self.voices,
            self.emojis,
            self.sns,
            self.analytics,
            self.api,
            self.keys,
            self.runtime,
            self.web,
        ];
        CachePart::ALL
            .into_iter()
            .zip(chosen)
            .filter(|&(_, on)| on || self.all)
            .map(|(part, _)| part)
            .collect()
    }
}

/// `weflow cache …`; `clear-account` also changes the profile, so this runs before the shared hub is built.
pub(crate) fn handle_cache(
    command: &CacheCommand,
    ctx: &AppContext,
    config: &mut ConfigStore,
) -> AppResult<Value> {
    use weflow_core::contract::{CacheCleared, Cancelled};
    use weflow_core::locale::tr;
    use weflow_core::services::disk_usage;

    let hub = ServiceHub::new(ctx.clone(), config.clone());
    match &command.command {
        CacheSubcommand::List => unreachable!("a read"),
        CacheSubcommand::Clear(args) => {
            let parts = args.parts();
            if parts.is_empty() {
                let options: Vec<String> = CachePart::ALL
                    .iter()
                    .map(|p| format!("--{}", p.name()))
                    .chain(["--all".to_string()])
                    .collect();
                return Err(AppError::usage(format!(
                    "choose what to clear: {} (`weflow cache list` shows each part)",
                    options.join(", ")
                )));
            }
            let targets: Vec<RemovalTarget> = parts
                .iter()
                .flat_map(|&part| {
                    hub.cache_part_paths(part)
                        .into_iter()
                        .filter(|p| p.exists())
                        .map(move |path| RemovalTarget {
                            label: cache_part_label(part),
                            bytes: disk_usage(&path).0,
                            path,
                        })
                })
                .collect();
            if args.dry_run {
                return Ok(json!(removal_plan(targets, &[])));
            }
            if targets.is_empty() {
                return Ok(json!(CacheCleared {
                    removed_paths: Vec::new(),
                    freed_bytes: 0,
                    warnings: Vec::new(),
                }));
            }
            if !confirm_removal(tr("About to remove:", "将要清理："), &targets, args.yes)? {
                return Ok(json!(Cancelled { cancelled: true }));
            }
            Ok(json!(hub.cache_clear(&parts)))
        }
        CacheSubcommand::ClearAccount {
            exports_dir,
            yes,
            dry_run,
        } => {
            let targets: Vec<RemovalTarget> = hub
                .account_data_paths(exports_dir)?
                .into_iter()
                .map(|path| RemovalTarget {
                    label: "",
                    bytes: disk_usage(&path).0,
                    path,
                })
                .collect();
            if *dry_run {
                return Ok(json!(removal_plan(targets, &ACCOUNT_KEYS)));
            }
            let heading = tr(
                "About to remove these files and take the account out of the profile (db_path, wxid, decrypt_key and the image keys):",
                "将要删除以下文件，并把当前账号移出配置档案（db_path、wxid、decrypt_key 和图片密钥）：",
            );
            if !confirm_removal(heading, &targets, *yes)? {
                return Ok(json!(Cancelled { cancelled: true }));
            }
            let result = hub.clear_current_account_data(exports_dir)?;
            // like WeFlow's desktop app: the account is signed out of this profile
            ConfigStore::update(&ctx.config_path, |config| {
                for key in ACCOUNT_KEYS {
                    config.unset_key(None, key);
                }
                Ok(())
            })?;
            Ok(json!(result))
        }
    }
}

/// The keys `cache clear-account` takes out of the profile.
const ACCOUNT_KEYS: [&str; 5] = [
    "db_path",
    "wxid",
    "decrypt_key",
    "image_xor_key",
    "image_aes_key",
];

/// `--dry-run` (#215): what `cache clear` / `cache clear-account` would remove, and the profile keys it would unset,
/// for a page that asks before it removes anything.
fn removal_plan(targets: Vec<RemovalTarget>, keys: &[&str]) -> RemovalPlan {
    RemovalPlan {
        dry_run: true,
        bytes: targets.iter().map(|t| t.bytes).sum(),
        paths: targets
            .into_iter()
            .map(|t| PlannedRemoval {
                path: t.path,
                bytes: t.bytes,
            })
            .collect(),
        profile_keys: keys.iter().map(|k| k.to_string()).collect(),
    }
}

/// A file or folder `cache clear` / `cache clear-account` is about to remove.
struct RemovalTarget {
    label: &'static str,
    bytes: u64,
    path: PathBuf,
}

fn cache_part_label(part: CachePart) -> &'static str {
    use weflow_core::locale::tr;
    match part {
        CachePart::Images => tr("images", "图片"),
        CachePart::Voices => tr("voices", "语音"),
        CachePart::Emojis => tr("stickers", "表情"),
        CachePart::Sns => tr("Moments", "朋友圈"),
        CachePart::Analytics => tr("analytics", "统计"),
        CachePart::Api => tr("HTTP API", "API 导出"),
        CachePart::Keys => tr("key fingerprints", "密钥指纹"),
        CachePart::Runtime => tr("unused programs", "不再使用的程序"),
        CachePart::Web => tr("WebUI versions", "网页版本"),
    }
}

/// Lists `targets` on stderr under `heading` and asks [y/N]; only `y` or `yes` goes ahead, anything else (Enter
/// included) is a no. `yes` skips the question; without a terminal to ask on, it is required.
fn confirm_removal(heading: &str, targets: &[RemovalTarget], yes: bool) -> AppResult<bool> {
    use std::io::{IsTerminal, Write};
    use weflow_core::locale::tr;

    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(AppError::usage("this removes files; add --yes to confirm"));
    }
    let mut text = format!("{heading}\n");
    if targets.is_empty() {
        text.push_str(&format!("  {}\n", tr("(no files)", "（没有文件）")));
    }
    let rows: Vec<Vec<String>> = targets
        .iter()
        .map(|t| {
            vec![
                t.label.to_string(),
                format_bytes(t.bytes),
                t.path.display().to_string(),
            ]
        })
        .collect();
    for line in align_columns(&rows) {
        text.push_str(&format!("  {line}\n"));
    }
    text.push_str(tr("Remove them? [y/N] ", "确认清理？[y/N] "));
    let mut err = std::io::stderr();
    let _ = err.write_all(text.as_bytes());
    let _ = err.flush();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|e| AppError::runtime(format!("failed to read the answer: {e}")))?;
    Ok(is_yes(&answer))
}

/// `cache list` as a table under a green header; `cache clear` / `clear-account`: what was removed.
pub(super) fn cache_summary(shown: CacheShown, data: &Value) -> Option<String> {
    use weflow_core::locale::tr;
    match shown {
        CacheShown::List => {
            let header = [
                tr("Part", "部分"),
                tr("Option", "选项"),
                tr("Size", "大小"),
                tr("Path", "路径"),
            ];
            let mut rows: Vec<Vec<String>> = vec![header.iter().map(|h| h.to_string()).collect()];
            for (part, item) in CachePart::ALL.iter().zip(data.as_array()?) {
                let paths: Vec<&str> = item["paths"]
                    .as_array()?
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                rows.push(vec![
                    cache_part_label(*part).to_string(),
                    format!("--{}", part.name()),
                    format_bytes(item["bytes"].as_u64().unwrap_or(0)),
                    if paths.is_empty() {
                        tr("(none)", "（无）").to_string()
                    } else {
                        // one path a line, under the header
                        paths.join("\n")
                    },
                ]);
            }
            let mut lines = align_columns(&rows).into_iter();
            let mut out = format!("{}\n", green(&lines.next()?));
            lines.for_each(|l| out.push_str(&format!("{l}\n")));
            Some(out)
        }
        CacheShown::Cleared => {
            if data["dryRun"] == true {
                let paths = data["paths"].as_array()?;
                let mut out = String::from(if paths.is_empty() {
                    tr("Nothing would be removed.\n", "没有需要清理的文件。\n")
                } else {
                    tr("Would remove:\n", "将会清理：\n")
                });
                let rows: Vec<Vec<String>> = paths
                    .iter()
                    .map(|p| {
                        vec![
                            format_bytes(p["bytes"].as_u64().unwrap_or(0)),
                            p["path"].as_str().unwrap_or("").to_string(),
                        ]
                    })
                    .collect();
                for line in align_columns(&rows) {
                    out.push_str(&format!("  {line}\n"));
                }
                return Some(out);
            }
            if data["cancelled"] == true {
                return Some(format!(
                    "{}\n",
                    tr(
                        "Cancelled; nothing was removed.",
                        "已取消，没有删除任何文件。"
                    )
                ));
            }
            let removed: Vec<&str> = data["removedPaths"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect();
            let mut out = String::new();
            if removed.is_empty() {
                out.push_str(tr("Nothing to remove.\n", "没有需要清理的文件。\n"));
            } else {
                out.push_str(tr("Removed:\n", "已清理：\n"));
                removed
                    .iter()
                    .for_each(|p| out.push_str(&format!("  {p}\n")));
            }
            if let Some(freed) = data["freedBytes"].as_u64().filter(|&b| b > 0) {
                out.push_str(&format!(
                    "{}{}\n",
                    tr("Freed ", "共释放 "),
                    format_bytes(freed)
                ));
            }
            if data["profileReset"] == true {
                out.push_str(tr(
                    "The account was taken out of the profile.\n",
                    "已把当前账号移出配置档案。\n",
                ));
            }
            for warning in data["warnings"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "{}{}\n",
                    tr("warning: ", "警告："),
                    warning.as_str().unwrap_or_default()
                ));
            }
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn cache_clear_options_pick_their_parts() {
        let parse = |args: &[&str]| {
            let cli = Cli::try_parse_from(["weflow", "cache", "clear"].iter().chain(args)).unwrap();
            match cli.command {
                Commands::Cache(CacheCommand {
                    command: CacheSubcommand::Clear(args),
                }) => args.parts(),
                other => panic!("{other:?}"),
            }
        };
        assert!(parse(&[]).is_empty());
        assert_eq!(
            parse(&["--voices", "--images", "-y"]),
            [CachePart::Images, CachePart::Voices]
        );
        assert_eq!(parse(&["--keys"]), [CachePart::Keys]);
        assert_eq!(parse(&["--runtime"]), [CachePart::Runtime]);
        assert_eq!(parse(&["--all"]), CachePart::ALL);
        assert_eq!(parse(&["--all", "--images"]), CachePart::ALL);
    }
}
