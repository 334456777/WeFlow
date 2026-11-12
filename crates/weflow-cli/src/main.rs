use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[cfg(feature = "alloc-stats")]
mod alloc_stats;
mod commands;
mod files;
mod i18n;
mod reads;
mod serve_api;
mod term;

use commands::analytics::{handle_analytics, AnalyticsCommand};
use commands::backup::{handle_backup, BackupCommand};
use commands::biz::BizCommand;
use commands::cache::{handle_cache, CacheCommand, CacheSubcommand};
use commands::chat::{handle_chat, ChatCommand};
use commands::config::{apply_progress_settings, handle_config, ConfigCommand, ConfigSubcommand};
use commands::db::{handle_db, DbCommand, DbSubcommand};
use commands::export::{handle_export, ExportCommand};
use commands::group::{handle_group, GroupCommand};
use commands::image::{AutoDownloadSubcommand, ImageCommand, ImageSubcommand};
use commands::insight::{handle_insight, print_insight, InsightCommand};
use commands::key::{handle_key, key_summary, KeyCommand, KeySubcommand};
use commands::report::{handle_report, ReportCommand};
use commands::runtime::{handle_runtime, RuntimeCommand};
use commands::sns::{handle_sns, SnsCommand};
use commands::video::{VideoCommand, VideoSubcommand};
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

#[derive(Parser, Debug)]
#[command(name = "weflow", version = concat!(env!("WEFLOW_VERSION"), " (", env!("WEFLOW_BUILD_INFO"), ")"), about = "Native CLI for WeFlow")]
struct Cli {
    /// Print JSON (for scripts) instead of the human-readable output
    #[arg(long, global = true)]
    json: bool,
    /// Emit NDJSON progress events on stderr (machine-readable)
    #[arg(long, global = true)]
    progress: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LangArg {
    En,
    Zh,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Read and write configuration
    Config(ConfigCommand),
    /// Detect, scan and test WeChat database locations
    Db(DbCommand),
    /// Extract database and image keys
    Key(KeyCommand),
    /// Read sessions, messages and contacts; anti-revoke triggers; media
    Chat(ChatCommand),
    /// Export sessions, contacts, messages, footprint and media
    Export(ExportCommand),
    /// Overall statistics, rankings and time distribution
    Analytics(AnalyticsCommand),
    /// Group chat members and statistics
    Group(GroupCommand),
    /// Annual and dual-person reports
    Report(ReportCommand),
    /// Moments (SNS) timeline, export and block-delete trigger
    Sns(SnsCommand),
    /// Official accounts and WeChat Pay records
    Biz(BizCommand),
    /// AI insights
    Insight(InsightCommand),
    /// Locate videos stored by WeChat and parse video md5s
    Video(VideoCommand),
    /// Locate, decrypt and cache message images
    Image(ImageCommand),
    /// Run the HTTP API, message push, insight and image auto-download services
    Serve(ServeCommand),
    /// Show the embedded runtime and manifest
    Runtime(RuntimeCommand),
    /// Create, inspect and restore backups
    Backup(BackupCommand),
    /// Clear WeFlow's caches
    Cache(CacheCommand),
    /// Download, pick and remove versions of the WebUI that `serve` shows
    Web(WebCommand),
    /// Save the output language
    Lang {
        /// Output language
        #[arg(value_enum)]
        lang: LangArg,
    },
}

#[derive(Args, Debug)]
struct WebCommand {
    #[command(subcommand)]
    command: WebSubcommand,
}

#[derive(Subcommand, Debug)]
enum WebSubcommand {
    /// The WebUI `serve` shows: the version in use, the one picked, the default one of this weflow, and where they are kept
    Info,
    /// WebUI versions kept on this machine, or published ones (--remote) and whether they suit this weflow
    List {
        /// List the versions published in the registry instead
        #[arg(long)]
        remote: bool,
    },
    /// Download a WebUI version, check it and keep it
    Install {
        /// WebUI version, such as 0.1.0
        version: String,
    },
    /// Pick the WebUI version `serve` shows (downloaded first when needed)
    Use {
        /// WebUI version, such as 0.1.0
        version: String,
    },
    /// Download and pick the newest published WebUI version that suits this weflow
    Update,
    /// Remove the WebUI versions kept here that are neither picked nor the default one of this weflow, nor shown by a
    /// running serve
    Prune,
}

#[derive(Args, Debug)]
struct ServeCommand {
    /// Run the AI insight engine
    #[arg(long)]
    insight: bool,
    /// Run the image auto-download hook (Windows only)
    #[arg(long)]
    image_auto_download: bool,
    /// Listen address (default: 127.0.0.1). On a loopback address the command API only answers requests to localhost, 127.0.0.1 or [::1].
    #[arg(long)]
    host: Option<String>,
    /// Listen port (default: 18108).
    #[arg(long)]
    port: Option<u16>,
    /// Access token for the HTTP API (default: `http_api_token` from the config; without one the command API makes one up for each run).
    #[arg(long, env = "WEFLOW_HTTP_TOKEN")]
    api_token: Option<String>,
    /// Tasks (`POST` commands: exports, keys, backups…) the command API runs at once; more are queued or refused (default: 1)
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    max_tasks: Option<u16>,
    /// Other commands the command API runs at once (default: the number of CPUs)
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    max_reads: Option<u16>,
    /// Images and voices the command API's /api/ media routes decode at once; more wait in line (default: the number of CPUs)
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    max_media: Option<u16>,
    /// Print a line on stderr for each request of the command API: method, path (no query), status and time
    #[arg(long)]
    log_requests: bool,
    /// Serve the WebUI at /ui/ from this folder (a built WebUI, such as web/dist) for this run only, instead of the installed one
    #[arg(long, value_name = "DIR")]
    ui_dir: Option<PathBuf>,
    /// Do not open the WebUI in the browser when the server starts
    #[arg(long)]
    no_browser: bool,
    /// Show this WebUI version for this run (downloaded first when needed), instead of the one picked with `weflow web use`
    #[arg(long, value_name = "VERSION", conflicts_with = "ui_dir")]
    web_version: Option<String>,
    /// Serve the API alone, without the WebUI
    #[arg(long, conflicts_with_all = ["ui_dir", "web_version", "no_browser"])]
    api_only: bool,
}

impl ServeCommand {
    /// These options, with those the command line does not give taken from the config file (#406): a saved switch
    /// turns on what its flag turns on.
    fn or_saved(mut self, saved: &weflow_core::config::ServeSettings) -> Self {
        self.host = self.host.or_else(|| saved.host.clone());
        self.port = self.port.or(saved.port);
        self.max_tasks = self.max_tasks.or(saved.max_tasks);
        self.max_reads = self.max_reads.or(saved.max_reads);
        self.max_media = self.max_media.or(saved.max_media);
        self.log_requests |= saved.log_requests == Some(true);
        self.insight |= saved.insight == Some(true);
        self.image_auto_download |= saved.image_auto_download == Some(true);
        self.no_browser |= saved.open_browser == Some(false);
        self
    }
}

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

/// The background engine of `serve --insight`: polls for new messages (WeFlow's desktop app reacts to
/// database-change events) and runs the silence scan on its own schedule.
fn spawn_insight_engine(hub: ServiceHub, stop: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        rt.block_on(async move {
            let mut next_scan = std::time::Instant::now() + std::time::Duration::from_secs(3 * 60);
            while !stop.load(Ordering::Relaxed) {
                if hub.insight_enabled() {
                    let notify =
                        hub.config_value("aiInsightNotificationEnabled") != Value::Bool(false);
                    let mut emit = |r: &weflow_core::insight::InsightRecord| {
                        if notify {
                            print_insight(r);
                        }
                    };
                    if std::time::Instant::now() >= next_scan {
                        hub.insight_silence_scan(&mut emit).await;
                        let hours = hub
                            .config_value("aiInsightScanIntervalHours")
                            .as_f64()
                            .filter(|h| *h != 0.0)
                            .unwrap_or(4.0)
                            .max(0.1);
                        next_scan = std::time::Instant::now()
                            + std::time::Duration::from_secs_f64(hours * 3600.0);
                    }
                    hub.insight_analyze_activity(&mut emit).await;
                }
                for _ in 0..50 {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        });
    });
}

fn config_whitelist(hub: &ServiceHub) -> Vec<String> {
    hub.config_value("autoDownloadWhitelist")
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The WebUI version this build defaults to (web/package.json at build time): only the version number is in the
/// binary, the page itself is downloaded.
const WEB_VERSION: &str = env!("WEFLOW_WEB_VERSION");

/// What a WebUI has to suit: this command API and this weflow.
fn web_backend() -> weflow_core::webui::Backend<'static> {
    weflow_core::webui::Backend {
        api_version: serve_api::API_VERSION,
        version: env!("WEFLOW_VERSION"),
    }
}

/// The version `serve` shows without `--ui-dir` and `--web-version`: the one picked with `weflow web use`, else the
/// default version of this weflow.
fn web_wanted(store: &weflow_core::webui::WebStore) -> Option<String> {
    store
        .chosen()
        .or_else(|| (!WEB_VERSION.is_empty()).then(|| WEB_VERSION.to_string()))
}

/// The WebUI `serve` starts with, and the version to download beside it (issue #148): none with `--api-only`; the
/// folder of `--ui-dir`; `--web-version` (downloaded first); else the version wanted, or while it is downloaded,
/// the newest suitable one kept here.
async fn serve_webui(
    command: &ServeCommand,
    hub: &ServiceHub,
) -> AppResult<(Option<serve_api::ui::UiDir>, Option<String>)> {
    use serve_api::ui::UiDir;
    if command.api_only {
        return Ok((None, None));
    }
    if let Some(dir) = &command.ui_dir {
        let ui = UiDir::open(dir).ok_or_else(|| {
            AppError::usage(format!(
                "--ui-dir {}: no index.html there; pass the folder of a built WebUI (such as web/dist)",
                dir.display()
            ))
        })?;
        return Ok((Some(ui), None));
    }
    let store = hub.web_store();
    if let Some(version) = &command.web_version {
        let registry = weflow_core::webui::Registry::default_registry()?;
        let progress = |done: u64, total: u64| {
            weflow_core::output::progress(
                "web",
                "downloading the WebUI",
                usize::try_from(done).unwrap_or(usize::MAX),
                usize::try_from(total).unwrap_or(usize::MAX),
            )
        };
        let meta =
            weflow_core::webui::install(&store, &registry, version, web_backend(), &progress)
                .await?;
        return Ok((UiDir::open_version(&store, &meta.version), None));
    }
    let Some(wanted) = web_wanted(&store) else {
        return Ok((None, None));
    };
    let backend = web_backend();
    // the newest suitable version kept here: shown while the one wanted downloads, or instead of one kept that
    // does not suit this weflow
    let fallback = || {
        store
            .installed()
            .into_iter()
            .find(|m| {
                backend
                    .refusal(&m.api_version, &m.min_backend_version)
                    .is_none()
            })
            .and_then(|m| UiDir::open_version(&store, &m.version))
    };
    match weflow_core::webui::kept(&store, &wanted, backend) {
        Ok(Some(meta)) => Ok((UiDir::open_version(&store, &meta.version), None)),
        Ok(None) => Ok((fallback(), Some(wanted))),
        // kept by another weflow: downloading it again would not help, but with nothing else suitable kept here (a
        // version picked before this weflow changed its API, say) the default version of this weflow does
        Err(refused) => {
            eprintln!(
                "{}{}",
                weflow_core::locale::tr("warning: ", "警告："),
                weflow_core::locale::localize(refused.message.clone())
            );
            let shown = fallback();
            let default_version =
                (shown.is_none() && !WEB_VERSION.is_empty() && wanted != WEB_VERSION)
                    .then(|| WEB_VERSION.to_string());
            Ok((shown, default_version))
        }
    }
}

/// `weflow web …`: the WebUI versions kept here and published (issue #148).
async fn handle_web(command: &WebCommand, hub: &ServiceHub) -> AppResult<Value> {
    use weflow_core::contract::{
        WebChosen, WebInfo, WebInstalled, WebPruned, WebPublished, WebUpdated,
    };
    use weflow_core::webui::{self, Registry, WebMeta};
    let store = hub.web_store();
    let backend = web_backend();
    let chosen = store.chosen();
    let wanted = web_wanted(&store);
    let entry = |m: &WebMeta| WebInstalled {
        version: m.version.clone(),
        api_version: m.api_version.clone(),
        min_backend_version: m.min_backend_version.clone(),
        path: store.dir(&m.version),
        in_use: wanted.as_deref() == Some(m.version.as_str()),
        picked: chosen.as_deref() == Some(m.version.as_str()),
        suits: backend
            .refusal(&m.api_version, &m.min_backend_version)
            .is_none(),
    };
    let progress = |done: u64, total: u64| {
        weflow_core::output::progress(
            "web",
            "downloading the WebUI",
            usize::try_from(done).unwrap_or(usize::MAX),
            usize::try_from(total).unwrap_or(usize::MAX),
        )
    };
    match &command.command {
        WebSubcommand::Info => {
            let in_use = wanted.as_deref().and_then(|v| store.meta(v));
            Ok(json!(WebInfo {
                in_use: in_use.as_ref().map(|m| m.version.clone()),
                path: in_use.as_ref().map(|m| store.dir(&m.version)),
                picked: chosen.clone(),
                built_in: WEB_VERSION.to_string(),
                wanted: wanted.clone(),
                downloaded: in_use.is_some(),
                store: store.root().to_path_buf(),
                installed: store.installed().into_iter().map(|m| m.version).collect(),
            }))
        }
        WebSubcommand::List { remote: false } => Ok(json!(store
            .installed()
            .iter()
            .map(entry)
            .collect::<Vec<_>>())),
        WebSubcommand::List { remote: true } => {
            let registry = Registry::default_registry()?;
            let mut out = Vec::new();
            for version in registry.versions().await?.into_iter().take(30) {
                let published = registry.published(&version).await?;
                let refusal = match (&published.api_version, &published.min_backend_version) {
                    (Some(api), Some(min)) => backend.refusal(api, min),
                    _ => None,
                };
                out.push(WebPublished {
                    size: published.size,
                    api_version: published.api_version,
                    min_backend_version: published.min_backend_version,
                    suits: refusal.is_none(),
                    why: refusal,
                    downloaded: store.meta(&version).is_some(),
                    version,
                });
            }
            Ok(json!(out))
        }
        WebSubcommand::Install { version } => {
            let registry = Registry::default_registry()?;
            let meta = webui::install(&store, &registry, version, backend, &progress).await?;
            Ok(json!(entry(&meta)))
        }
        WebSubcommand::Use { version } => {
            let registry = Registry::default_registry()?;
            let meta = webui::install(&store, &registry, version, backend, &progress).await?;
            store.choose(&meta.version)?;
            Ok(json!(WebChosen {
                path: store.dir(&meta.version),
                version: meta.version,
            }))
        }
        WebSubcommand::Update => {
            let registry = Registry::default_registry()?;
            let mut suitable = Vec::new();
            for version in registry.versions().await?.into_iter().take(30) {
                let published = registry.published(&version).await?;
                if let (Some(api), Some(min)) =
                    (&published.api_version, &published.min_backend_version)
                {
                    if backend.refusal(api, min).is_none() {
                        suitable.push(version);
                    }
                }
            }
            // the newest release, or the newest pre-release when there is no release yet
            let newest = suitable
                .iter()
                .find(|v| !v.contains('-'))
                .or(suitable.first())
                .cloned()
                .ok_or_else(|| {
                    AppError::new(
                        "not_found",
                        "no published WebUI version suits this weflow",
                        1,
                    )
                })?;
            let before = wanted.clone();
            let meta = webui::install(&store, &registry, &newest, backend, &progress).await?;
            store.choose(&meta.version)?;
            Ok(json!(WebUpdated {
                path: store.dir(&meta.version),
                version: meta.version,
                before,
            }))
        }
        WebSubcommand::Prune => {
            let (mut removed, mut shown) = (Vec::new(), Vec::new());
            for meta in store.installed() {
                let keep =
                    chosen.as_deref() == Some(meta.version.as_str()) || meta.version == WEB_VERSION;
                if keep {
                    continue;
                }
                match store.remove(&meta.version) {
                    Ok(true) => removed.push(meta.version),
                    Ok(false) => {}
                    // a running serve shows it: kept until that stops
                    Err(e) if e.code == "in_use" => shown.push(meta.version),
                    Err(e) => return Err(e),
                }
            }
            let kept: Vec<String> = store.installed().into_iter().map(|m| m.version).collect();
            Ok(json!(WebPruned {
                removed,
                kept,
                shown
            }))
        }
    }
}

/// `value` as a URL query value: every byte but the unreserved ones (RFC 3986) percent-encoded, so a token with `+`,
/// `&`, `#` or `=` reaches the page as it is.
fn query_escape(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Opens `url` in the default browser, as `serve` does with the WebUI; nothing happens where there is none.
fn open_in_browser(url: &str) {
    let mut opener = if cfg!(windows) {
        let mut cmd = std::process::Command::new("rundll32");
        cmd.args(["url.dll,FileProtocolHandler", url]);
        cmd
    } else if cfg!(target_os = "macos") {
        let mut cmd = std::process::Command::new("open");
        cmd.arg(url);
        cmd
    } else {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(url);
        cmd
    };
    let _ = opener
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// `serve` runs the command API (every CLI command over HTTP, see `serve_api`), next to the optional insight and image
/// auto-download services.
async fn handle_serve(command: ServeCommand, hub: &ServiceHub) -> AppResult<Value> {
    use weflow_core::sync::RwLockExt;
    let command = command.or_saved(hub.serve_settings());
    let (ui, fetch) = serve_webui(&command, hub).await?;
    let ui = std::sync::Arc::new(std::sync::RwLock::new(ui));

    let cfg_str = |key: &str| {
        hub.config_value(key)
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let host = command.host.unwrap_or_else(|| "127.0.0.1".into());
    let port = command.port.unwrap_or(serve_api::DEFAULT_PORT);
    let token = command
        .api_token
        .clone()
        .or_else(|| cfg_str("httpApiToken"));
    // without a configured token the command API makes one up
    let generated_token = token.is_none().then(serve_api::generate_token);
    let addr = format!("{host}:{port}")
        .parse::<std::net::SocketAddr>()
        .map_err(|err| AppError::usage(format!("invalid listen address: {err}")))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|err| AppError::runtime(format!("failed to bind HTTP server: {err}")))?;
    let bound = listener.local_addr().unwrap_or(addr);
    // the command API runs every request in this same executable
    let exe = std::env::current_exe()
        .map_err(|err| AppError::runtime(format!("cannot locate the weflow executable: {err}")))?;

    let auto_download = if command.image_auto_download {
        let svc = weflow_core::image_download::ImageAutoDownload::new(hub.runtime_dir());
        let started = svc.start(config_whitelist(hub));
        if started["success"] != true {
            eprintln!(
                "{}{}",
                weflow_core::locale::tr(
                    "warning: image auto download not started: ",
                    "警告：图片自动下载未启动："
                ),
                weflow_core::locale::localize(
                    started["error"]
                        .as_str()
                        .unwrap_or("unknown error")
                        .to_string()
                )
            );
        }
        Some(svc)
    } else {
        None
    };

    let insight_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if command.insight {
        spawn_insight_engine(hub.clone(), insight_stop.clone());
    }

    // files returned by an earlier run of the command API are not needed any more
    let files_dir = hub.serve_files_dir();
    if let Err(err) = std::fs::remove_dir_all(&files_dir) {
        if err.kind() != std::io::ErrorKind::NotFound {
            eprintln!(
                "{}{}: {err}",
                weflow_core::locale::tr("warning: cannot clear ", "警告：无法清空 "),
                files_dir.display()
            );
        }
    }

    let mut started = json!({
        "type": "server_started",
        "url": format!("http://{bound}"),
        "tokenConfigured": token.is_some(),
        "insight": command.insight,
        "imageAutoDownload": command.image_auto_download
    });
    if let Some(generated) = &generated_token {
        started["generatedToken"] = json!(generated);
    }
    // the page's address carries the token, as Jupyter's does; the page keeps it for its tab and drops it from the
    // address bar. A server listening on every address is still reached here as 127.0.0.1.
    let page_url = {
        let at = if bound.ip().is_unspecified() {
            std::net::SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), bound.port())
        } else {
            bound
        };
        let token = token.as_ref().or(generated_token.as_ref());
        format!(
            "http://{at}/ui/{}",
            token
                .map(|t| format!("?token={}", query_escape(t)))
                .unwrap_or_default()
        )
    };
    // only its version: a clone of the page kept here would hold its version after the page is replaced
    let shown = ui
        .read_or_panic()
        .as_ref()
        .map(|ui| ui.version().map(str::to_string));
    started["ui"] = json!(shown.is_some());
    if let Some(version) = &shown {
        started["uiUrl"] = json!(page_url);
        if let Some(version) = version {
            started["uiVersion"] = json!(version);
        }
    }
    if let Some(version) = &fetch {
        started["uiDownloading"] = json!(version);
    }
    weflow_core::output::event(started);
    let browse = !command.no_browser && std::io::IsTerminal::is_terminal(&std::io::stderr());
    if shown.is_some() && browse {
        open_in_browser(&page_url);
    }
    // the version wanted is downloaded beside the server, which shows it once it is in place
    if let Some(version) = fetch {
        let (store, ui, page_url, had_page) = (
            hub.web_store(),
            ui.clone(),
            page_url.clone(),
            shown.is_some(),
        );
        tokio::spawn(async move {
            let quiet = |_: u64, _: u64| {};
            let installed = match weflow_core::webui::Registry::default_registry() {
                Ok(registry) => {
                    weflow_core::webui::install(&store, &registry, &version, web_backend(), &quiet)
                        .await
                }
                Err(e) => Err(e),
            };
            match installed.map(|meta| serve_api::ui::UiDir::open_version(&store, &meta.version)) {
                Ok(Some(dir)) => {
                    *ui.write_or_panic() = Some(dir);
                    weflow_core::output::event(
                        json!({ "type": "webui_ready", "version": version, "uiUrl": page_url }),
                    );
                    if browse && !had_page {
                        open_in_browser(&page_url);
                    }
                }
                Ok(None) => {}
                Err(e) => weflow_core::output::event(json!({
                    "type": "webui_unavailable", "version": version, "error": e.message.clone(), "kept": had_page,
                })),
            }
        });
    }
    if !bound.ip().is_loopback() {
        eprintln!(
            "{}",
            weflow_core::locale::tr(
                "warning: the command API listens beyond this machine; anyone with the token can run any weflow command here",
                "警告：命令 API 监听在本机以外的地址；持有令牌的人可以在这台机器上运行任何 weflow 命令"
            )
        );
    }
    let farewell = serve_api::Farewell::default();
    let server = async {
        let stopped = {
            let cfg = serve_api::ApiConfig {
                check_host: bound.ip().is_loopback(),
                exe,
                token: token.or(generated_token).unwrap_or_default(),
                files_dir,
                max_tasks: command.max_tasks.map_or(1, usize::from),
                max_reads: command
                    .max_reads
                    .map_or_else(serve_api::default_max_reads, usize::from),
                max_media: command
                    .max_media
                    .map_or_else(serve_api::default_max_reads, usize::from),
                log_requests: command.log_requests,
                ui,
                web_store: (!command.api_only
                    && command.ui_dir.is_none()
                    && command.web_version.is_none())
                .then(|| hub.web_store()),
                config_path: hub.config_path().to_path_buf(),
                farewell: farewell.clone(),
                insight: command.insight,
            };
            serve_api::serve(cfg, listener).await
        };
        stopped.map_err(|err| AppError::runtime(format!("HTTP server stopped: {err}")))
    };
    let outcome = tokio::select! {
        result = server => result.map(|_| ()),
        _ = tokio::signal::ctrl_c() => {
            // the open event streams end with `bye`, so a page tells a server that stopped from one it lost
            farewell.say(std::time::Duration::from_millis(500)).await;
            Err(AppError::new("user_interrupt", "interrupted by user", 130))
        }
    };
    insight_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Some(svc) = &auto_download {
        svc.stop();
    }
    outcome?;
    Ok(json!({ "stopped": true }))
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
