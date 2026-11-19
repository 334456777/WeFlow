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
    /// Hook WeChat and wait for the database key (keep the command running and log in to WeChat)
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
    /// Derive the image keys from WeChat's kvcomm cache (verified against a .dat template; Windows, Linux x64 and macOS)
    Image {
        /// Account directory to search for templates (default: the configured account directory)
        #[arg(long)]
        user_dir: Option<String>,
        /// kvcomm directories (repeat for net, net_1, etc.; default on Windows, Linux x64 and macOS: WeChat's existing net*/kvcomm directories)
        #[arg(long, action = clap::ArgAction::Append)]
        kvcomm_dir: Vec<PathBuf>,
        /// Maximum filesystem entries scanned for templates in the selected account (default: 10000)
        #[arg(long)]
        scan_budget: Option<usize>,
        /// Save the keys to the active profile instead of printing them
        #[arg(long)]
        save: bool,
    },
    /// Scan WeChat's memory for the image AES key (Linux x64 and macOS)
    ImageMem {
        /// WeChat process id (default: the running WeChat)
        #[arg(long)]
        pid: Option<u32>,
        /// 32 hex characters of the 16-byte encrypted block (default: from the newest thumbnails of the configured account, which also give the XOR key)
        #[arg(long, value_name = "HEX")]
        ciphertext: Option<String>,
        /// Save the keys to the active profile instead of printing them
        #[arg(long)]
        save: bool,
    },
    /// List the running WeChat processes (macOS)
    Processes,
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
            if std::env::var_os(serve_api::child::CANCEL_ON_STDIN).is_some() {
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
            if kvcomm_dir.is_empty() && !weflow_core::image_keys::HAS_DEFAULT_KVCOMM_DIRS {
                return Err(AppError::native(
                    "`key image` does not know where WeChat keeps its kvcomm cache on this platform; pass --kvcomm-dir",
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
        KeySubcommand::ImageMem {
            pid,
            ciphertext,
            save,
        } => {
            if *save && ciphertext.is_some() {
                return Err(AppError::usage(
                    "a key found with --ciphertext is checked against that block only, so it is not saved; run `key image-mem --save` without --ciphertext to check the key against the account's thumbnails",
                ));
            }
            let mut data = hub.key_image_mem(*pid, ciphertext.as_deref())?;
            if *save {
                let aes = data.image_aes_key.take().ok_or_else(|| {
                    AppError::runtime(
                        "no image AES key found in WeChat's memory; open a few images in WeChat, then try again",
                    )
                })?;
                let mut keys = vec![("image_aes_key", json!(aes))];
                keys.extend(
                    data.image_xor_key
                        .take()
                        .map(|xor| ("image_xor_key", json!(xor))),
                );
                data.saved = save_keys(config_path, &keys)?;
            }
            Ok(json!(data))
        }
        KeySubcommand::Processes => Ok(json!(hub.key_processes()?)),
    }
}

/// The answer of `summary`'s command in words, or `None` to show it as it is.
pub(crate) fn key_summary(summary: Summary, data: &Value) -> Option<String> {
    use weflow_core::locale::tr;
    match summary {
        Summary::Cache(shown) => cache_summary(shown, data),
        // silent unless the key is spelt almost as one WeFlow reads (#813)
        Summary::ConfigSaved => Some(match data["didYouMean"].as_str() {
            Some(known) => format!(
                "{}\n",
                tr(
                    "Saved, but WeFlow does not read this key; did you mean {known}?",
                    "已保存，但 WeFlow 不读取这个键；是否想写 {known}？"
                )
                .replace("{known}", known)
            ),
            None => String::new(),
        }),
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

/// `key db`, `image` and `image-mem`: the keys under the names `config set` expects, so they can be copied over,
/// or that they were saved.
fn keys_summary(shown: KeyShown, data: &Value) -> Option<String> {
    use weflow_core::locale::tr;
    if data.get("saved").is_some() {
        let what = match shown {
            KeyShown::Db => tr("Database key", "数据库密钥"),
            KeyShown::Image => tr("Image keys", "图片密钥"),
            KeyShown::ImageMem
                if data["saved"]
                    .as_array()
                    .is_some_and(|saved| saved.iter().any(|key| key == "image_xor_key")) =>
            {
                tr("Image keys", "图片密钥")
            }
            KeyShown::ImageMem => tr("Image AES key", "图片 AES 密钥"),
            KeyShown::Processes => tr("WeChat processes", "微信进程"),
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
        KeyShown::ImageMem => {
            let Some(key) = data["image_aes_key"].as_str() else {
                return Some(format!(
                    "{}\n",
                    tr("No image AES key found", "未找到图片 AES 密钥")
                ));
            };
            let mut text = match data["image_xor_key"].as_u64() {
                Some(xor) => format!(
                    "{}\nimage_xor_key: {xor}\nimage_aes_key: {key}\n",
                    tr("Image keys obtained", "已获取图片密钥")
                ),
                None => format!(
                    "{}\nimage_aes_key: {key}\n",
                    tr("Image AES key obtained", "已获取图片 AES 密钥")
                ),
            };
            if data["verification"] == "first_block" {
                text.push_str(tr(
                    "(only the given block was checked; the key is not verified against the account's images)\n",
                    "（只验证了给定的这一块，未用账号的图片验证这个密钥）\n",
                ));
            }
            Some(text)
        }
        KeyShown::Processes => {
            let processes = data["processes"].as_array()?;
            Some(if processes.is_empty() {
                format!("{}\n", tr("No WeChat process found", "未找到微信进程"))
            } else {
                processes
                    .iter()
                    .map(|process| {
                        let main = if process["main"] == true { "*" } else { " " };
                        format!(
                            "{main} {}  {}  {}\n",
                            process["pid"].as_u64().unwrap_or(0),
                            process["name"].as_str().unwrap_or(""),
                            process["path"].as_str().unwrap_or("")
                        )
                    })
                    .collect()
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
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
    fn image_mem_arguments() {
        let parse =
            |args: &[&str]| Cli::try_parse_from(["weflow", "key", "image-mem"].iter().chain(args));
        let Commands::Key(KeyCommand {
            command:
                KeySubcommand::ImageMem {
                    pid,
                    ciphertext,
                    save,
                },
        }) = parse(&[
            "--pid",
            "1234",
            "--ciphertext",
            "00112233445566778899aabbccddeeff",
        ])
        .unwrap()
        .command
        else {
            panic!("image-mem command")
        };
        assert_eq!(pid, Some(1234));
        assert_eq!(
            ciphertext.as_deref(),
            Some("00112233445566778899aabbccddeeff")
        );
        assert!(!save);
        // both may be left out: the running WeChat and a thumbnail of the account
        let Commands::Key(KeyCommand {
            command:
                KeySubcommand::ImageMem {
                    pid,
                    ciphertext,
                    save,
                },
        }) = parse(&["--save"]).unwrap().command
        else {
            panic!("image-mem command")
        };
        assert_eq!((pid, ciphertext), (None, None));
        assert!(save);
        assert!(parse(&["--pid", "not-a-pid"]).is_err());
        // both are options: a bare value is refused
        assert!(parse(&["1234"]).is_err());
    }

    #[test]
    fn image_mem_summary_shows_the_key_or_that_none_was_found() {
        let found = keys_summary(
            KeyShown::ImageMem,
            &json!({ "image_aes_key": "00112233445566778899aabbccddeeff", "pid": 1 }),
        )
        .unwrap();
        assert!(found.contains("image_aes_key: 00112233445566778899aabbccddeeff"));
        let one_block = keys_summary(
            KeyShown::ImageMem,
            &json!({ "image_aes_key": "00112233445566778899aabbccddeeff", "verification": "first_block", "pid": 1 }),
        )
        .unwrap();
        assert!(
            one_block.starts_with(&found) && one_block.len() > found.len(),
            "a key checked on one block says so: {one_block}"
        );
        let missing = keys_summary(KeyShown::ImageMem, &json!({ "pid": 1 })).unwrap();
        assert!(missing.contains("AES") || missing.contains("图片"));
        let pair = keys_summary(
            KeyShown::ImageMem,
            &json!({ "image_aes_key": "00112233445566778899aabbccddeeff", "image_xor_key": 55, "pid": 1 }),
        )
        .unwrap();
        assert!(pair.contains("image_xor_key: 55\n"), "{pair}");
        let aes_only = keys_summary(
            KeyShown::ImageMem,
            &json!({ "saved": ["image_aes_key"], "pid": 1 }),
        )
        .unwrap();
        let both = keys_summary(
            KeyShown::ImageMem,
            &json!({ "saved": ["image_aes_key", "image_xor_key"], "pid": 1 }),
        )
        .unwrap();
        assert_ne!(aes_only, both, "saving the XOR key too says so");
    }

    #[test]
    fn processes_summary_lists_the_main_client() {
        let text = keys_summary(
            KeyShown::Processes,
            &json!({ "processes": [
                { "pid": 7, "name": "WeChat", "path": "/Applications/WeChat.app/Contents/MacOS/WeChat", "main": true },
                { "pid": 9, "name": "WeChat", "path": "/Applications/WeChat.app/Contents/Frameworks/x/WeChat", "main": false }
            ] }),
        )
        .unwrap();
        assert!(text.contains("7"));
        assert!(text.contains("WeChat.app"));
        let empty = keys_summary(KeyShown::Processes, &json!({ "processes": [] })).unwrap();
        assert!(empty.contains("No WeChat process") || empty.contains("未找到"));
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
