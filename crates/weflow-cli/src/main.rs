//! `weflow`, the command line of WeFlow, and `weflow serve`, its command API.
//!
//! This file is the entry point: the allocator, the dispatch of a parsed command, and the summary and exit code of
//! the run. Each subcommand is defined and run in its own module under `commands`.

use std::process::ExitCode;

#[cfg(feature = "alloc-stats")]
mod alloc_stats;
mod cli;
mod commands;
mod files;
mod help_style;
mod i18n;
mod launch;
mod numbers;
mod reads;
mod serve_api;
mod stop;
mod term;
mod views;

use cli::{Cli, Commands, LangArg};
use commands::analytics::handle_analytics;
use commands::backup::handle_backup;
use commands::cache::{handle_cache, CacheCommand, CacheSubcommand};
use commands::chat::handle_chat;
use commands::config::{
    apply_progress_settings, handle_config, warn_if_open_to_others, ConfigCommand, ConfigSubcommand,
};
use commands::db::{handle_db, DbCommand, DbSubcommand};
use commands::export::handle_export;
use commands::group::handle_group;
use commands::image::{AutoDownloadSubcommand, ImageCommand, ImageSubcommand};
use commands::insight::handle_insight;
use commands::key::{handle_key, key_summary, KeyCommand, KeySubcommand};
use commands::report::handle_report;
use commands::serve::{config_whitelist, handle_serve};
use commands::sns::handle_sns;
use commands::video::VideoSubcommand;
use commands::web::handle_web;
use reads::{json_document, read_of};

use serde_json::{json, Value};
use tracing_subscriber::EnvFilter;
use weflow_core::colour::{Paint, Role, Stream};
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
        verbose,
        ascii,
        command,
    } = i18n::parse(launch::args());
    // a command run by `serve` gets the database keys the server already derived on stdin
    // (see `serve_api::child::spawn`)
    if std::env::var_os(serve_api::child::KEYS_ON_STDIN).is_some() {
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
    let view = if json || verbose {
        None
    } else {
        views::View::of(&command)
    };
    let outcome = run(command, progress, view.as_ref()).await;
    // under `sudo`, the config and cache files root wrote go back to the caller (#855)
    weflow_core::sudo_caller::hand_back();
    weflow_core::output::finish_progress();
    if json {
        let (doc, exit) = json_document(outcome.map(|answer| answer.data));
        return answered(weflow_core::output::print(&format!("{doc}\n")), exit);
    }
    let glyphs = if ascii {
        views::Glyphs::Ascii
    } else {
        views::Glyphs::Unicode
    };
    match outcome {
        Ok(Answer { data, names }) => answered(
            print_response(&success(data), summary, view.as_ref(), &names, glyphs),
            0,
        ),
        Err(err) => {
            print_failure(&err);
            ExitCode::from(exit_status(err.exit_code))
        }
    }
}

/// What a command answered, and the names its view shows for the usernames in it.
struct Answer {
    data: Value,
    names: views::Names,
}

impl From<Value> for Answer {
    fn from(data: Value) -> Self {
        Self {
            data,
            names: views::Names::new(),
        }
    }
}

/// Runs `command`; a read shown in `view` is read as the view needs it, and the names it shows are looked up.
async fn run(
    command: Commands,
    progress: weflow_core::output::ProgressMode,
    view: Option<&views::View>,
) -> AppResult<Answer> {
    let ctx = AppContext::new(VERSION)?;
    let config =
        ConfigStore::load(&ctx.config_path).map_err(|err| AppError::config(format!("{err:#}")))?;

    apply_progress_settings(&config, progress);
    if matches!(
        command,
        Commands::Config(ConfigCommand {
            command: ConfigSubcommand::Path
        }) | Commands::Db(DbCommand {
            command: DbSubcommand::Test
        })
    ) {
        warn_if_open_to_others(&ctx);
    }

    match read_of(command) {
        Ok(read) => {
            let hub = ServiceHub::new(ctx, config);
            let Some(view) = view else {
                return read.run(&hub).map(Answer::from);
            };
            let data = view.read(read).run(&hub)?;
            let names = view.names(&hub, &data);
            Ok(Answer { data, names })
        }
        Err(command) => run_command(command, ctx, config).await.map(Answer::from),
    }
}

/// Runs a command that is not a read.
async fn run_command(
    command: Commands,
    ctx: AppContext,
    mut config: ConfigStore,
) -> AppResult<Value> {
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
                let svc = weflow_core::image_download::ImageAutoDownload::new();
                let list = if whitelist.is_empty() {
                    config_whitelist(&hub)
                } else {
                    whitelist
                };
                if !weflow_core::image_download::supported() {
                    return Err(AppError::runtime(
                        "image auto download is only supported on Windows x64",
                    ));
                }
                if let Err(reason) = svc.start(list) {
                    // why WeChat could not be hooked, as the hook says it
                    return Err(AppError::runtime(reason));
                }
                weflow_core::output::event(
                    json!({ "type": "auto_download_started", "status": svc.status() }),
                );
                let running = weflow_core::status::begin(weflow_core::locale::tr(
                    "Image auto download is running; press Ctrl+C to stop it…",
                    "原图自动下载运行中，按 Ctrl+C 停止…",
                ));
                stop::requested().await;
                svc.stop();
                running.finish(weflow_core::locale::tr("stopped", "已停止"), Role::Success);
                Ok(json!(weflow_core::contract::AutoDownloadStopped {
                    stopped: true
                }))
            }
            // this process runs no hook; `serve` answers the request itself for the hook it runs
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
                json!(hub.image_decrypt(&target.payload(*force)).into_answer()?)
            }
            ImageSubcommand::ResolveCache { target } => {
                json!(hub
                    .image_resolve_cache(&target.payload(false))
                    .into_answer()?)
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
                no_data_url,
            } => Ok(json!(hub.video_info(
                md5,
                !*no_poster,
                if *no_data_url {
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
        Commands::Cache(_) | Commands::Config(_) | Commands::Lang { .. } => unreachable!(),
    }
}

/// The exit status of a run whose answer, with exit code `code`, was written to stdout as `written` says. An answer
/// that could not be written is a failure of its own, said on stderr; output that nothing reads any more is not one
/// (see `weflow_core::output::print`).
fn answered(written: std::io::Result<()>, code: u8) -> ExitCode {
    if let Err(err) = written {
        print_failure(&AppError::runtime(format!(
            "{}{err}",
            weflow_core::locale::tr("cannot write the output: ", "无法写出结果：")
        )));
        return ExitCode::from(code.max(1));
    }
    ExitCode::from(code)
}

/// The human-readable output: the payload itself, without the {success, data} envelope, in the command's own words,
/// else in its `view` (whose note goes to stderr), else as `weflow_core::render` shows any answer.
fn print_response(
    response: &weflow_core::output::CliResponse<Value>,
    summary: Option<Summary>,
    view: Option<&views::View>,
    names: &views::Names,
    glyphs: views::Glyphs,
) -> std::io::Result<()> {
    #[expect(
        clippy::expect_used,
        reason = "a document of JSON values and strings always serializes"
    )]
    let mut value = serde_json::to_value(response).expect("a response serializes");
    weflow_core::locale::localize_json(&mut value);
    let data = value.get("data").cloned().unwrap_or(Value::Null);
    let paint = Paint::of(Stream::Stdout);
    if let Some(text) = summary.and_then(|summary| key_summary(summary, &data, paint)) {
        return weflow_core::output::print(&text);
    }
    let shown = view.and_then(|view| view.render(&data, names, &views::Style::of_run(glyphs)));
    let Some(shown) = shown else {
        return weflow_core::output::print(&weflow_core::render::render(&data, paint));
    };
    weflow_core::output::print(&shown.body)?;
    if let Some(note) = shown.note {
        // a note that cannot be written on stderr leaves the answer whole
        let _ = weflow_core::output::eprint(&note);
    }
    Ok(())
}

/// The commands whose answer the terminal shows in words of its own, and which words: chosen before the command
/// runs, which takes the command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Summary {
    /// A command that ends with its outcome on the line that said it was running (`image auto-download start`).
    AlreadySaid,
    Cache(CacheShown),
    /// `config set`, `unset`, `clear` and `remove-profile`, which succeed silently: the config file path is not news.
    /// A key spelt almost as one WeFlow reads is named.
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
    ImageMem,
    Processes,
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
                    | ConfigSubcommand::Clear
                    | ConfigSubcommand::RemoveProfile { .. },
            }) => Self::ConfigSaved,
            Commands::Db(DbCommand { command }) => match command {
                DbSubcommand::Detect => Self::DbDetect,
                DbSubcommand::Wxid { .. } => Self::DbWxid,
                _ => return None,
            },
            Commands::Image(ImageCommand {
                command:
                    ImageSubcommand::AutoDownload {
                        command: AutoDownloadSubcommand::Start { .. },
                    },
            }) => Self::AlreadySaid,
            Commands::Key(KeyCommand { command }) => Self::Key(match command {
                KeySubcommand::Db { .. } => KeyShown::Db,
                KeySubcommand::Image { .. } => KeyShown::Image,
                KeySubcommand::ImageMem { .. } => KeyShown::ImageMem,
                KeySubcommand::Processes => KeyShown::Processes,
            }),
            _ => return None,
        })
    }
}

/// The process exit status for an error's exit code; one outside 0 to 255 becomes 1 rather than wrapping (256 would
/// read as success).
fn exit_status(code: i32) -> u8 {
    u8::try_from(code).unwrap_or(1)
}

fn print_failure(err: &AppError) {
    let details = err.details.as_ref();
    // a failure that cannot be written on stderr has nowhere else to go
    let _ = weflow_core::output::eprint(&weflow_core::render::render_error(
        &err.message,
        details,
        Paint::of(Stream::Stderr),
    ));
}
