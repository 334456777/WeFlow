//! `weflow config`: the config file and its keys, and the progress settings it holds.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use serde_json::{json, Value};
use weflow_core::config::{AppContext, ConfigStore};
use weflow_core::error::{AppError, AppResult};

#[derive(Args, Debug)]
pub(crate) struct ConfigCommand {
    #[command(subcommand)]
    pub(crate) command: ConfigSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ConfigSubcommand {
    /// Show the path of the config file
    Path,
    /// Show the whole config file: its own settings and every profile
    List,
    /// List the profiles: account, data directory and whether the keys are set (never the keys themselves)
    Profiles,
    /// Show one key (or every key of the active profile when omitted)
    Get {
        /// Key name (default: every key of the active profile)
        key: Option<String>,
    },
    /// Set a key
    Set {
        #[arg(
            help = "Key name; `<profile>.<key>` (for example `work.wxid`) sets a key of another profile\n  db_path                  WeChat data directory (`db detect` finds it)\n  wxid                     Account wxid (`db wxid` prints it)\n  decrypt_key              Database key in hex (`key db` prints it)\n  image_xor_key            Image XOR key, a number (`key image` prints it)\n  image_aes_key            Image AES key (`key image` prints it)\n  cache_path               Folder for cached images, voices, stickers and Moments (default: `cache` next to the config file)\n  http_api_token           Access token of the HTTP API (`serve`)\n  ai_model_api_base_url    Base URL of the AI model API used by `insight`\n  ai_model_api_key         API key of the AI model API\n  ai_model_api_model       Model name\n  ai_model_api_max_tokens  Most tokens in one AI reply\n  ai_insight_enabled       true or false: turn AI insight on or off\n  no_progress              true or false: true hides the automatic progress bar\n  progress_delay_seconds   Seconds a command runs before the progress bar appears (default 5; 0 = always)\nSettings of the config file itself (stored in that file, not in a profile; each config file has its own):\n  lang                     Output language, en or zh (the same as `weflow lang`)\n  current_profile          Active profile name, created when it does not exist (default: default)\nOptions of `serve`, used when its command line does not give them:\n  serve_host               Listen address (as --host)\n  serve_port               Listen port (as --port)\n  serve_max_tasks          Tasks run at once (as --max-tasks)\n  serve_max_reads          Other commands run at once (as --max-reads)\n  serve_max_media          Images and voices decoded at once (as --max-media)\n  serve_log_requests       true or false: true logs each request (as --log-requests)\n  serve_insight            true or false: true runs the AI insight engine (as --insight)\n  serve_image_auto_download  true or false: true runs the image auto-download hook (as --image-auto-download)\n  serve_open_browser       true or false: false does not open the WebUI at the start (as --no-browser)\nWhich config file is used (kept next to the default location, not in any config file):\n  config_path              Config file used from now on; `config unset config_path` or the default path switches back"
        )]
        key: String,
        /// Value to store
        value: String,
    },
    /// Remove a key
    Unset {
        /// Key name
        key: String,
    },
    /// Reset the whole config file: every profile (all accounts and their keys) and its own settings are removed
    Clear,
}

/// Progress bar settings from the config (`no_progress`, `progress_delay_seconds`) and `WEFLOW_PROGRESS_DELAY`;
/// `--progress` keeps NDJSON events whatever the config says.
pub(crate) fn apply_progress_settings(
    config: &ConfigStore,
    progress: weflow_core::output::ProgressMode,
) {
    if progress == weflow_core::output::ProgressMode::Auto {
        let off = config.get_key(None, "no_progress");
        if off
            .as_bool()
            .unwrap_or_else(|| off.as_str() == Some("true"))
        {
            weflow_core::output::set_progress_mode(weflow_core::output::ProgressMode::Off);
        }
    }
    let from_env = std::env::var("WEFLOW_PROGRESS_DELAY")
        .ok()
        .and_then(|v| weflow_core::output::parse_delay(&v));
    let v = config.get_key(None, "progress_delay_seconds");
    let delay = from_env.or_else(|| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(weflow_core::output::parse_delay))
    });
    if let Some(d) = delay {
        weflow_core::output::set_progress_delay(d);
    }
}

pub(crate) fn handle_config(
    command: ConfigCommand,
    ctx: &AppContext,
    config: &mut ConfigStore,
) -> AppResult<Value> {
    use weflow_core::contract::ConfigWritten;
    let written = |config_path: Option<PathBuf>| json!(ConfigWritten { config_path });
    match command.command {
        ConfigSubcommand::Path => Ok(json!(ctx.config_path.to_string_lossy())),
        ConfigSubcommand::List => serde_json::to_value(config)
            .map_err(|err| AppError::runtime(format!("cannot serialize the config: {err}"))),
        ConfigSubcommand::Profiles | ConfigSubcommand::Get { .. } => unreachable!("a read"),
        ConfigSubcommand::Set { key, value } => {
            if matches!(key.as_str(), "config_path" | "configPath") {
                let path = std::path::absolute(&value)
                    .map_err(|err| AppError::usage(format!("invalid path {value}: {err}")))?;
                if path.is_dir() {
                    return Err(AppError::usage(format!(
                        "{} is a folder; give the config file path",
                        path.display()
                    )));
                }
                weflow_core::config::set_saved_config_path(&path)?;
                return Ok(written(Some(path)));
            }
            if matches!(
                key.as_str(),
                "progress_delay_seconds" | "progressDelaySeconds"
            ) && weflow_core::output::parse_delay(&value).is_none()
            {
                return Err(AppError::usage(format!(
                    "invalid value '{value}' for {key}; use a whole number of seconds (0 or more)"
                )));
            }
            let value = parse_config_value(&value);
            ConfigStore::update(&ctx.config_path, |config| config.set_key(None, &key, value))?;
            Ok(written(Some(ctx.config_path.clone())))
        }
        ConfigSubcommand::Unset { key } => {
            if matches!(key.as_str(), "config_path" | "configPath") {
                weflow_core::config::clear_saved_config_path()?;
                return Ok(written(weflow_core::config::default_config_path()));
            }
            ConfigStore::update(&ctx.config_path, |config| {
                config.unset_key(None, &key);
                Ok(())
            })?;
            Ok(written(Some(ctx.config_path.clone())))
        }
        ConfigSubcommand::Clear => {
            ConfigStore::update(&ctx.config_path, |config| {
                *config = ConfigStore::default();
                Ok(())
            })?;
            Ok(written(Some(ctx.config_path.clone())))
        }
    }
}

fn parse_config_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}
