use std::process::ExitCode;

#[cfg(feature = "alloc-stats")]
mod alloc_stats;
mod cli;
mod commands;
mod files;
mod i18n;
mod reads;
mod serve_api;
mod term;

use cli::{Cli, Commands, LangArg};
use commands::analytics::handle_analytics;
use commands::backup::handle_backup;
use commands::cache::{handle_cache, CacheCommand, CacheSubcommand};
use commands::chat::handle_chat;
use commands::config::{apply_progress_settings, handle_config, ConfigCommand, ConfigSubcommand};
use commands::db::{handle_db, DbCommand, DbSubcommand};
use commands::export::handle_export;
use commands::group::handle_group;
use commands::image::{AutoDownloadSubcommand, ImageCommand, ImageSubcommand};
use commands::insight::handle_insight;
use commands::key::{handle_key, key_summary, KeyCommand, KeySubcommand};
use commands::report::handle_report;
use commands::runtime::handle_runtime;
use commands::serve::{config_whitelist, handle_serve};
use commands::sns::handle_sns;
use commands::video::VideoSubcommand;
use commands::web::handle_web;
use reads::{json_document, read_of};

use serde_json::{json, Value};
use tracing_subscriber::EnvFilter;
use weflow_core::config::{AppContext, ConfigStore};
use weflow_core::error::{AppError, AppResult};
use weflow_core::output::success;
use weflow_core::services::ServiceHub;

/// Exports read rows on one thread and free them on another; with the system allocator (glibc in particular) that
/// hand-over costs more than the parsing it runs next to. See issue #14.
#[cfg(not(feature = "alloc-stats"))]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The same mimalloc, counting each allocation for the benchmark (#513).
#[cfg(feature = "alloc-stats")]
#[global_allocator]
static ALLOCATOR: alloc_stats::Counting = alloc_stats::Counting;

/// On Windows mimalloc commits memory from the system as it goes, and an export made it commit fresh pages over and
/// over: on a 200,000-message group about 400,000 extra page faults and 1.2 s of kernel time, enough to make the SQL
/// export slower than with the system allocator. An arena reserved and committed up front, which mimalloc uses
/// before it asks the system again, removes that; 128 MiB did as well as 1 GiB. See issue #45.
#[cfg(windows)]
const RESERVED_ARENA: usize = 128 << 20;

#[cfg(windows)]
fn reserve_allocator_arena() {
    extern "C" {
        // mimalloc's own function, linked in with the `mimalloc` crate
        fn mi_reserve_os_memory(
            size: usize,
            commit: bool,
            allow_large: bool,
        ) -> std::os::raw::c_int;
    }
    // MIMALLOC_RESERVE_OS_MEMORY makes mimalloc reserve its own arena at startup (with the same call)
    if std::env::var_os("MIMALLOC_RESERVE_OS_MEMORY").is_none() {
        // a failure leaves mimalloc taking memory as it goes, as without the reservation
        // SAFETY: mimalloc's own function, with the signature declared above; it takes only a size and two flags.
        unsafe { mi_reserve_os_memory(RESERVED_ARENA, true, true) };
    }
}

const VERSION: &str = env!("WEFLOW_VERSION");

#[tokio::main]
async fn main() -> ExitCode {
    // dropped last, when `main` returns: the counts then cover the whole command
    #[cfg(feature = "alloc-stats")]
    let _alloc_report = alloc_stats::Report;
    #[cfg(windows)]
    reserve_allocator_arena();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let Cli {
        json,
        progress,
        command,
    } = i18n::parse();
    // a command run by `serve` gets the database keys the server already derived on stdin (see `serve_api::spawn`)
    if std::env::var_os(serve_api::KEYS_ON_STDIN).is_some() {
        let mut stdin = std::io::stdin();
        let mut keys = Vec::new();
        if !std::io::IsTerminal::is_terminal(&stdin)
            && std::io::Read::read_to_end(&mut stdin, &mut keys).is_ok()
        {
            weflow_core::page_keys::import_keys(&keys);
        }
    }
    weflow_core::output::set_json_output(json);
    let progress = if progress {
        weflow_core::output::ProgressMode::Ndjson
    } else {
        weflow_core::output::ProgressMode::Auto
    };
    weflow_core::output::set_progress_mode(progress);
    let summary = Summary::of(&command);
    let outcome = run(command, progress).await;
    weflow_core::output::finish_progress();
    if json {
        let (doc, exit) = json_document(outcome);
        println!("{doc}");
        return ExitCode::from(exit);
    }
    match checked(outcome) {
        Ok(value) => {
            print_response(&success(value), summary);
            ExitCode::SUCCESS
        }
        Err(err) => {
            print_failure(&err);
            ExitCode::from(exit_status(err.exit_code))
        }
    }
}

async fn run(command: Commands, progress: weflow_core::output::ProgressMode) -> AppResult<Value> {
    let ctx = AppContext::new(VERSION)?;
    let mut config =
        ConfigStore::load(&ctx.config_path).map_err(|err| AppError::config(err.to_string()))?;

    apply_progress_settings(&config, progress);

    let command = match read_of(command) {
        Ok(read) => return read.run(&ServiceHub::new(ctx, config)),
        Err(command) => command,
    };
    let command = match command {
        Commands::Config(command) => return handle_config(command, &ctx, &mut config),
        Commands::Lang { lang } => {
            let code = match lang {
                LangArg::En => "en",
                LangArg::Zh => "zh",
            };
            ConfigStore::update(&ctx.config_path, |config| {
                config.lang = Some(code.to_string());
                Ok(())
            })?;
            return Ok(json!(weflow_core::contract::LangSaved {
                lang: code.to_string()
            }));
        }
        Commands::Runtime(command) => return handle_runtime(&command, &ctx),
        Commands::Cache(command) => return handle_cache(&command, &ctx, &mut config),
        command => command,
    };

    let config_path = ctx.config_path.clone();
    let hub = ServiceHub::new(ctx, config);
    match command {
        Commands::Db(command) => handle_db(&command, &hub),
        Commands::Chat(command) => handle_chat(command, &hub).await,
        Commands::Key(command) => handle_key(&command, &hub, &config_path),
        Commands::Export(command) => handle_export(command, &hub).await,
        Commands::Analytics(command) => handle_analytics(&command, &hub),
        Commands::Group(command) => handle_group(&command, &hub),
        Commands::Report(command) => handle_report(&command, &hub),
        Commands::Sns(command) => handle_sns(command, &hub).await,
        Commands::Insight(command) => handle_insight(command, &hub).await,
        Commands::Image(ImageCommand {
            command: ImageSubcommand::AutoDownload { command },
        }) => match command {
            AutoDownloadSubcommand::Start { whitelist } => {
                let svc = weflow_core::image_download::ImageAutoDownload::new(hub.runtime_dir());
                let list = if whitelist.is_empty() {
                    config_whitelist(&hub)
                } else {
                    whitelist
                };
                let started = svc.start(list);
                if started["success"] != true {
                    return Err(AppError::runtime(
                        started["error"]
                            .as_str()
                            .unwrap_or("auto download failed to start")
                            .to_string(),
                    ));
                }
                weflow_core::output::event(
                    json!({ "type": "auto_download_started", "status": svc.status() }),
                );
                let _ = tokio::signal::ctrl_c().await;
                svc.stop();
                Ok(json!(weflow_core::contract::AutoDownloadStopped {
                    stopped: true
                }))
            }
            AutoDownloadSubcommand::Status => {
                Ok(json!(weflow_core::contract::AutoDownloadStatus {
                    is_hooked: false,
                    pid: None,
                    supported: weflow_core::image_download::supported(),
                }))
            }
        },
        Commands::Image(command) => Ok(match &command.command {
            ImageSubcommand::AutoDownload { .. } => unreachable!("handled above"),
            ImageSubcommand::Decrypt { target, force } => {
                json!(hub.image_decrypt(&target.payload(*force)))
            }
            ImageSubcommand::ResolveCache { target } => {
                json!(hub.image_resolve_cache(&target.payload(false)))
            }
            ImageSubcommand::ResolveBatch { payloads_json } => {
                let list: Vec<Value> = serde_json::from_str(payloads_json).map_err(|e| {
                    AppError::usage(format!("payloads_json must be a JSON array: {e}"))
                })?;
                let payloads: Vec<_> = list
                    .iter()
                    .map(weflow_core::services::ImagePayload::from_json)
                    .collect();
                json!(hub.image_resolve_cache_batch(&payloads))
            }
        }),
        Commands::Video(command) => match &command.command {
            VideoSubcommand::Info {
                md5,
                no_poster,
                file_url,
            } => Ok(json!(hub.video_info(
                md5,
                !*no_poster,
                if *file_url {
                    weflow_core::video::PosterFormat::FileUrl
                } else {
                    weflow_core::video::PosterFormat::DataUrl
                },
            )?)),
            VideoSubcommand::ParseMd5 { content } => Ok(json!(weflow_core::contract::VideoMd5 {
                md5: weflow_core::video::parse_video_md5(content),
            })),
        },
        Commands::Serve(command) => handle_serve(command, &hub).await,
        Commands::Web(command) => handle_web(&command, &hub).await,
        Commands::Backup(command) => handle_backup(&command, &hub),
        Commands::Biz(_) => unreachable!("a read"),
        Commands::Runtime(_) | Commands::Cache(_) | Commands::Config(_) | Commands::Lang { .. } => {
            unreachable!()
        }
    }
}

/// A result that reports `success: false` itself is a failure.
fn checked(outcome: AppResult<Value>) -> AppResult<Value> {
    outcome.and_then(|value| match reported_failure(&value) {
        Some(err) => Err(err),
        None => Ok(value),
    })
}

/// The human-readable output: the payload itself, without the {success, data} envelope.
fn print_response(response: &weflow_core::output::CliResponse<Value>, summary: Option<Summary>) {
    #[expect(
        clippy::expect_used,
        reason = "a document of JSON values and strings always serializes"
    )]
    let mut value = serde_json::to_value(response).expect("a response serializes");
    weflow_core::locale::localize_json(&mut value);
    let data = value.get("data").cloned().unwrap_or(Value::Null);
    match summary.and_then(|summary| key_summary(summary, &data)) {
        Some(text) => print!("{text}"),
        None => print!("{}", weflow_core::render::render(&data)),
    }
}

/// The commands whose answer the terminal shows in words of its own, and which words: chosen before the command
/// runs, which takes the command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Summary {
    Cache(CacheShown),
    /// `config set`, `unset` and `clear`, which succeed silently: the config file path is not news.
    ConfigSaved,
    DbDetect,
    DbWxid,
    Key(KeyShown),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CacheShown {
    List,
    Cleared,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyShown {
    Db,
    Image,
    ScanImage,
}

impl Summary {
    fn of(command: &Commands) -> Option<Self> {
        Some(match command {
            Commands::Cache(CacheCommand { command }) => Self::Cache(match command {
                CacheSubcommand::List => CacheShown::List,
                CacheSubcommand::Clear(_) | CacheSubcommand::ClearAccount { .. } => {
                    CacheShown::Cleared
                }
            }),
            Commands::Config(ConfigCommand {
                command:
                    ConfigSubcommand::Set { .. }
                    | ConfigSubcommand::Unset { .. }
                    | ConfigSubcommand::Clear,
            }) => Self::ConfigSaved,
            Commands::Db(DbCommand { command }) => match command {
                DbSubcommand::Detect => Self::DbDetect,
                DbSubcommand::Wxid { .. } => Self::DbWxid,
                _ => return None,
            },
            Commands::Key(KeyCommand { command }) => Self::Key(match command {
                KeySubcommand::Db { .. } => KeyShown::Db,
                KeySubcommand::Image { .. } => KeyShown::Image,
                KeySubcommand::ScanImage { .. } => KeyShown::ScanImage,
            }),
            _ => return None,
        })
    }
}

/// Some services report a failure in their result (`{"success": false, "error" | "message": …}`), the shape WeFlow's
/// desktop app expects. For the CLI that is a failed command: an error envelope and a non-zero exit code, with the
/// other fields (such as `failureKind`) kept as details.
fn reported_failure(value: &Value) -> Option<AppError> {
    let fields = value.as_object()?;
    if fields.get("success") != Some(&Value::Bool(false)) {
        return None;
    }
    let message = ["error", "message"]
        .iter()
        .find_map(|key| fields.get(*key).and_then(Value::as_str))
        .filter(|text| !text.is_empty())
        .unwrap_or("the operation failed");
    let mut err = AppError::runtime(message);
    let details: serde_json::Map<String, Value> = fields
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "success" | "error" | "message"))
        .map(|(key, v)| (key.clone(), v.clone()))
        .collect();
    if !details.is_empty() {
        err.details = Some(Value::Object(details));
    }
    Some(err)
}

/// The process exit status for an error's exit code; one outside 0 to 255 becomes 1 rather than wrapping (256 would
/// read as success).
fn exit_status(code: i32) -> u8 {
    u8::try_from(code).unwrap_or(1)
}

fn print_failure(err: &AppError) {
    let details = err.details.as_ref();
    eprint!(
        "{}",
        weflow_core::render::render_error(&err.message, details)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reported_failures_become_errors() {
        let err = reported_failure(
            &json!({ "success": false, "error": "not found", "failureKind": "not_found" }),
        )
        .unwrap();
        assert_eq!((err.code.as_str(), err.exit_code), ("runtime_error", 1));
        assert_eq!(err.message, "not found");
        assert_eq!(err.details, Some(json!({ "failureKind": "not_found" })));

        let err = reported_failure(&json!({ "success": false, "message": "no key" })).unwrap();
        assert_eq!(
            (err.message.as_str(), err.details.is_none()),
            ("no key", true)
        );
        assert_eq!(
            reported_failure(&json!({ "success": false }))
                .unwrap()
                .message,
            weflow_core::locale::localize("the operation failed".to_string())
        );

        for ok in [
            json!({ "success": true, "path": "x" }),
            json!({ "out": "x" }),
            json!([{ "success": false }]),
            json!({ "results": [{ "success": false }] }),
        ] {
            assert!(reported_failure(&ok).is_none(), "{ok}");
        }
    }
}
