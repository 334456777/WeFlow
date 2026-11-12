//! `weflow key`: the database and image keys, saved to the profile with `--save`, and their summaries.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use super::cache::cache_summary;
use crate::{serve_api, KeyShown, Summary};
use serde_json::{json, Value};
use weflow_core::config::ConfigStore;
use weflow_core::error::{AppError, AppResult};
use weflow_core::services::ServiceHub;

#[derive(Args, Debug)]
pub(crate) struct KeyCommand {
    #[command(subcommand)]
    pub(crate) command: KeySubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum KeySubcommand {
    /// Hook WeChat and wait for the database key (Windows: keep the command running and log in to WeChat)
    Db {
        /// WeChat process id (default: the first Weixin.exe / WeChat.exe found)
        #[arg(long)]
        pid: Option<u32>,
        /// How long to wait for the key, in seconds
        #[arg(long, default_value_t = 180)]
        timeout: u64,
        /// Save the key to the active profile instead of printing it
        #[arg(long)]
        save: bool,
    },
    /// Derive the image keys from WeChat's kvcomm cache (verified against a .dat template)
    Image {
        /// Account directory to search for templates (default: the configured account directory)
        #[arg(long)]
        user_dir: Option<String>,
        /// kvcomm directories (repeat for net, net_1, etc.; default on Windows: the existing xwechat net*/kvcomm directories)
        #[arg(long, action = clap::ArgAction::Append)]
        kvcomm_dir: Vec<PathBuf>,
        /// Maximum filesystem entries scanned for templates in the selected account (default: 10000)
        #[arg(long)]
        scan_budget: Option<usize>,
        /// Save the keys to the active profile instead of printing them
        #[arg(long)]
        save: bool,
    },
    /// Scan WeChat's memory for the image AES key (macOS)
    ScanImage {
        /// Account directory (user directory) to scan for the image key
        user_dir: String,
    },
}

/// `--save` (#216): writes the keys a `key` command found to the active profile, and answers their names for the
/// result's `saved`. The caller leaves the keys out of the result, so they go from WeChat to the config file without
/// being printed, nor reaching a WebUI through `serve`.
fn save_keys(config_path: &Path, keys: &[(&str, Value)]) -> AppResult<Vec<String>> {
    ConfigStore::update(config_path, |config| {
        keys.iter()
            .try_for_each(|(name, value)| config.set_key(None, name, value.clone()))
    })?;
    Ok(keys.iter().map(|(name, _)| name.to_string()).collect())
}

/// `key db` run by `weflow serve`, which cannot send it Ctrl+C: the end of stdin (the job cancelled, the request or
/// the server gone) interrupts it as Ctrl+C would, so a hook it set is taken out of WeChat before it ends (#287).
fn interrupt_when_stdin_ends() {
    std::thread::spawn(|| {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
        // a hook may be about to be set: interrupt it too, until the command has ended
        loop {
            weflow_core::services::interrupt_key_db();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

pub(crate) fn handle_key(
    command: &KeyCommand,
    hub: &ServiceHub,
    config_path: &Path,
) -> AppResult<Value> {
    match &command.command {
        KeySubcommand::Db { pid, timeout, save } => {
            if std::env::var_os(serve_api::CANCEL_ON_STDIN).is_some() {
                interrupt_when_stdin_ends();
            }
            let mut data = hub.key_db(*pid, *timeout)?;
            if *save {
                let key = data
                    .decrypt_key
                    .take()
                    .ok_or_else(|| AppError::runtime("no decrypt_key to save"))?;
                data.saved = save_keys(config_path, &[("decrypt_key", json!(key))])?;
            }
            Ok(json!(data))
        }
        KeySubcommand::Image {
            user_dir,
            kvcomm_dir,
            scan_budget,
            save,
        } => {
            if !cfg!(windows) {
                return Err(AppError::native(
                    "`key image` is not supported on this platform yet",
                ));
            }
            let mut directories: Vec<_> = kvcomm_dir
                .iter()
                .map(|p| weflow_core::config::expand_home(&p.to_string_lossy()))
                .collect();
            if directories.is_empty() {
                directories = weflow_core::image_keys::default_kvcomm_dirs();
            }
            let mut data = hub.key_image(
                user_dir.as_deref(),
                &directories,
                scan_budget.unwrap_or(weflow_core::image_keys::DEFAULT_SCAN_BUDGET),
            )?;
            if *save {
                let (xor, aes) = data
                    .image_xor_key
                    .take()
                    .zip(data.image_aes_key.take())
                    .ok_or_else(|| AppError::runtime("no image keys to save"))?;
                data.saved = save_keys(
                    config_path,
                    &[("image_xor_key", json!(xor)), ("image_aes_key", json!(aes))],
                )?;
            }
            Ok(json!(data))
        }
        KeySubcommand::ScanImage { user_dir } => Ok(json!(hub.key_scan_image(user_dir)?)),
    }
}

/// The answer of `summary`'s command in words, or `None` to show it as it is.
pub(crate) fn key_summary(summary: Summary, data: &Value) -> Option<String> {
    use weflow_core::locale::tr;
    match summary {
        Summary::Cache(shown) => cache_summary(shown, data),
        Summary::ConfigSaved => Some(String::new()),
        Summary::DbDetect => {
            // only the directories that exist, named like the setting that takes them
            let found: Vec<&str> = data["candidates"]
                .as_array()?
                .iter()
                .filter(|c| c["exists"] == true)
                .filter_map(|c| c["path"].as_str())
                .collect();
            Some(if found.is_empty() {
                format!(
                    "{}\n",
                    tr("No WeChat data directory found", "未找到微信数据目录")
                )
            } else {
                found.iter().map(|p| format!("db_path: {p}\n")).collect()
            })
        }
        Summary::DbWxid => {
            let accounts = data["accounts"].as_array()?;
            Some(match accounts.as_slice() {
                [one] => format!("wxid: {}\n", one["wxid"].as_str()?),
                many => many
                    .iter()
                    .map(|a| {
                        format!(
                            "wxid: {}  {}\n",
                            a["wxid"].as_str().unwrap_or(""),
                            a["path"].as_str().unwrap_or("")
                        )
                    })
                    .collect(),
            })
        }
        Summary::Key(shown) => keys_summary(shown, data),
    }
}

/// `key db`, `image` and `scan-image`: the keys under the names `config set` expects, so they can be copied over,
/// or that they were saved.
fn keys_summary(shown: KeyShown, data: &Value) -> Option<String> {
    use weflow_core::locale::tr;
    if data.get("saved").is_some() {
        let what = match shown {
            KeyShown::Db => tr("Database key", "数据库密钥"),
            KeyShown::Image | KeyShown::ScanImage => tr("Image keys", "图片密钥"),
        };
        let mut text = format!(
            "{}\n",
            tr(
                "{what} obtained and saved to the active profile",
                "已获取{what}，已保存到当前档案"
            )
            .replace("{what}", what)
        );
        if data["verified"] == false {
            text.push_str(tr(
                "(AES header verified; XOR and the complete pair are not verified)\n",
                "（AES 首块已验证；XOR 与整组密钥尚未验证）\n",
            ));
        }
        return Some(text);
    }
    match shown {
        KeyShown::Db => {
            let key = data["decrypt_key"].as_str()?;
            Some(format!(
                "{}\ndecrypt_key: {key}\n",
                tr("Database key obtained", "已获取数据库密钥")
            ))
        }
        KeyShown::Image => {
            let xor = &data["image_xor_key"];
            let aes = data["image_aes_key"].as_str()?;
            let mut text = format!(
                "{}\nimage_xor_key: {}\nimage_aes_key: {aes}\n",
                tr("Image keys obtained", "已获取图片密钥"),
                xor.as_i64()
                    .map_or_else(|| xor.to_string(), |n| n.to_string())
            );
            if data["verified"] == false {
                text.push_str(tr(
                    "(AES header verified; XOR and the complete pair are not verified)\n",
                    "（AES 首块已验证；XOR 与整组密钥尚未验证）\n",
                ));
            }
            Some(text)
        }
        KeyShown::ScanImage => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn saved_keys_go_to_the_profile() {
        let dir = weflow_native::fixture::TestDir::new("weflow-key-save");
        let path = dir.join("config.json");
        let saved = save_keys(
            &path,
            &[
                ("image_xor_key", json!(83)),
                ("image_aes_key", json!("0123456789abcdef")),
            ],
        )
        .unwrap();
        assert_eq!(saved, ["image_xor_key", "image_aes_key"]);
        let config = ConfigStore::load(&path).unwrap();
        let profile = config.profile(None).unwrap();
        assert_eq!(profile.image_xor_key, Some(83));
        assert_eq!(profile.image_aes_key.as_deref(), Some("0123456789abcdef"));
    }

    #[test]
    fn image_key_arguments() {
        let parse =
            |args: &[&str]| Cli::try_parse_from(["weflow", "key", "image"].iter().chain(args));
        let Commands::Key(KeyCommand {
            command:
                KeySubcommand::Image {
                    kvcomm_dir,
                    scan_budget,
                    ..
                },
        }) = parse(&[]).unwrap().command
        else {
            panic!("image command")
        };
        assert!(kvcomm_dir.is_empty());
        assert_eq!(scan_budget, None);
        let Commands::Key(KeyCommand {
            command:
                KeySubcommand::Image {
                    kvcomm_dir,
                    scan_budget,
                    ..
                },
        }) = parse(&[
            "--kvcomm-dir",
            "net/kvcomm",
            "--kvcomm-dir",
            "net_1/kvcomm",
            "--scan-budget",
            "10",
        ])
        .unwrap()
        .command
        else {
            panic!("image command")
        };
        assert_eq!(
            kvcomm_dir,
            [PathBuf::from("net/kvcomm"), PathBuf::from("net_1/kvcomm")]
        );
        assert_eq!(scan_budget, Some(10));
    }

    #[test]
    fn partial_image_key_summary_does_not_claim_missing_templates() {
        let command = Cli::try_parse_from(["weflow", "key", "image"])
            .unwrap()
            .command;
        let summary = key_summary(
            Summary::of(&command).unwrap(),
            &json!({
                "image_xor_key": 123, "image_aes_key": "0123456789abcdef",
                "verified": false, "scan": {"truncated": true},
            }),
        )
        .unwrap();
        assert!(summary.contains("XOR"));
        assert!(!summary.contains("no .dat template"));
        assert!(!summary.contains("--json"));
    }
}
