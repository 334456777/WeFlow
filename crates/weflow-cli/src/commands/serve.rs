//! `weflow serve`: the command API and the WebUI, and with `--insight` the AI insight engine; the image
//! auto-download whitelist it shares with `image auto-download`.

use std::path::PathBuf;

use clap::Args;

use crate::commands::insight::print_insight;

use super::web::{open_in_browser, query_escape, serve_webui, web_backend};
use crate::serve_api;
use serde_json::{json, Value};
use weflow_core::error::{AppError, AppResult};
use weflow_core::services::ServiceHub;

#[derive(Args, Debug)]
pub(crate) struct ServeCommand {
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
    pub(super) ui_dir: Option<PathBuf>,
    /// Do not open the WebUI in the browser when the server starts
    #[arg(long)]
    no_browser: bool,
    /// Show this WebUI version for this run (downloaded first when needed), instead of the one picked with `weflow web use`
    #[arg(long, value_name = "VERSION", conflicts_with = "ui_dir")]
    pub(super) web_version: Option<String>,
    /// Serve the API alone, without the WebUI
    #[arg(long, conflicts_with_all = ["ui_dir", "web_version", "no_browser"])]
    pub(super) api_only: bool,
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

pub(crate) fn config_whitelist(hub: &ServiceHub) -> Vec<String> {
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

/// `serve` runs the command API (every CLI command over HTTP, see `serve_api`), next to the optional insight and image
/// auto-download services.
pub(crate) async fn handle_serve(command: ServeCommand, hub: &ServiceHub) -> AppResult<Value> {
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
    let generated_token = token.is_none().then(serve_api::http::generate_token);
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
