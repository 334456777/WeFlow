//! `weflow web`: the WebUI versions kept here and published, the one `serve` shows, and opening it in a browser.

use clap::{Args, Subcommand};

use super::serve::ServeCommand;
use crate::serve_api;
use serde_json::{json, Value};
use weflow_core::error::{AppError, AppResult};
use weflow_core::services::ServiceHub;
use weflow_core::webui::{WebMeta, WebStore};

#[derive(Args, Debug)]
pub(crate) struct WebCommand {
    #[command(subcommand)]
    command: WebSubcommand,
}

#[derive(Subcommand, Debug)]
enum WebSubcommand {
    /// The WebUI `serve` shows: the version in use, the one picked, the default one of this weflow, and where they are kept
    Info,
    /// WebUI versions kept on this machine, or with --remote the published ones, and whether they suit this weflow
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
    /// Pick the WebUI version `serve` shows; it is downloaded first when needed
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

/// The WebUI version this build defaults to (web/package.json at build time): only the version number is in the
/// binary, the page itself is downloaded.
const WEB_VERSION: &str = env!("WEFLOW_WEB_VERSION");

/// What a WebUI has to suit: this command API and this weflow.
pub(super) fn web_backend() -> weflow_core::webui::Backend<'static> {
    weflow_core::webui::Backend {
        api_version: serve_api::API_VERSION,
        version: env!("WEFLOW_VERSION"),
    }
}

/// The version `serve` aims to show without `--ui-dir` and `--web-version`: the one picked with `weflow web use`,
/// else the default version of this weflow.
fn web_wanted(store: &WebStore) -> Option<String> {
    store
        .chosen()
        .or_else(|| (!WEB_VERSION.is_empty()).then(|| WEB_VERSION.to_string()))
}

/// What `serve` shows of the versions kept here when it starts without `--ui-dir` and `--web-version`.
#[derive(Debug, Default)]
struct WebShown {
    /// The version shown when the server starts: kept here, and suits this weflow.
    now: Option<WebMeta>,
    /// The version downloaded beside the server, shown once it is in place.
    fetch: Option<String>,
    /// Why the version wanted is passed over, and what to do about it, in the language of this run.
    note: Option<String>,
}

impl WebShown {
    /// The version `serve` ends up showing: the one downloaded beside it, else the one it starts with.
    fn target(&self) -> Option<&str> {
        self.fetch
            .as_deref()
            .or(self.now.as_ref().map(|m| m.version.as_str()))
    }
}

/// The version wanted when it is kept and suits this weflow; while it is downloaded, the newest suitable one kept
/// here. One kept that does not suit (picked before this weflow changed its API, say) gives way to the newest
/// suitable one kept here, else to the default version of this weflow, downloaded: downloading the one kept again
/// would not help.
fn web_shown(store: &WebStore) -> WebShown {
    let Some(wanted) = web_wanted(store) else {
        return WebShown::default();
    };
    let backend = web_backend();
    let newest_suitable = || {
        store.installed().into_iter().find(|m| {
            backend
                .refusal(&m.api_version, &m.min_backend_version)
                .is_none()
        })
    };
    match weflow_core::webui::kept(store, &wanted, backend) {
        Ok(Some(meta)) => WebShown {
            now: Some(meta),
            ..WebShown::default()
        },
        Ok(None) => WebShown {
            now: newest_suitable(),
            fetch: Some(wanted),
            note: None,
        },
        Err(refused) => {
            let now = newest_suitable();
            let fetch = (now.is_none() && !WEB_VERSION.is_empty() && wanted != WEB_VERSION)
                .then(|| WEB_VERSION.to_string());
            let note = format!(
                "{}\n{}",
                weflow_core::locale::localize(refused.message.clone()),
                weflow_core::locale::tr(
                    "Run `weflow web update` to pick the newest WebUI that suits this weflow, or `weflow web use <version>` to pick another.",
                    "运行 `weflow web update` 选用适合这个 weflow 的最新网页，或用 `weflow web use <版本>` 选用其他版本。"
                )
            );
            WebShown {
                now,
                fetch,
                note: Some(note),
            }
        }
    }
}

/// The WebUI `serve` starts with, and the version to download beside it (issue #148): none with `--api-only`; the
/// folder of `--ui-dir`; `--web-version` (downloaded first); else what [`web_shown`] gives, after a warning when the
/// version wanted is passed over.
pub(super) async fn serve_webui(
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
                "--ui-dir {}: no index.html there\nPass the folder of a built WebUI, such as web/dist.",
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
    let shown = web_shown(&store);
    if let Some(note) = &shown.note {
        weflow_core::output::warn(note);
    }
    let ui = shown
        .now
        .and_then(|meta| UiDir::open_version(&store, &meta.version));
    Ok((ui, shown.fetch))
}

/// `weflow web …`: the WebUI versions kept here and published (issue #148).
pub(crate) async fn handle_web(command: &WebCommand, hub: &ServiceHub) -> AppResult<Value> {
    use weflow_core::contract::{
        WebChosen, WebInfo, WebInstalled, WebPruned, WebPublished, WebUpdated,
    };
    use weflow_core::webui::{self, Registry};
    let store = hub.web_store();
    let backend = web_backend();
    let chosen = store.chosen();
    let shown = web_shown(&store);
    let in_use = shown.now.as_ref().map(|m| m.version.clone());
    let entry = |m: &WebMeta| WebInstalled {
        version: m.version.clone(),
        api_version: m.api_version.clone(),
        min_backend_version: m.min_backend_version.clone(),
        path: store.dir(&m.version),
        in_use: in_use.as_deref() == Some(m.version.as_str()),
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
        WebSubcommand::Info => Ok(json!(WebInfo {
            path: in_use.as_deref().map(|v| store.dir(v)),
            in_use: in_use.clone(),
            picked: chosen.clone(),
            built_in: WEB_VERSION.to_string(),
            wanted: shown.target().map(str::to_string),
            downloaded: shown.fetch.is_none() && shown.now.is_some(),
            store: store.root().to_path_buf(),
            installed: store.installed().into_iter().map(|m| m.version).collect(),
            note: shown.note.clone(),
        })),
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
                let refusal = published_refusal(
                    &backend,
                    published.api_version.as_deref(),
                    published.min_backend_version.as_deref(),
                );
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
                let refusal = published_refusal(
                    &backend,
                    published.api_version.as_deref(),
                    published.min_backend_version.as_deref(),
                );
                if refusal.is_none() {
                    suitable.push(version);
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
            let before = shown.target().map(str::to_string);
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

/// Why a published version does not suit this weflow, or `None` when it does: one whose metadata leaves out the API
/// or the lowest weflow it needs cannot be told to suit.
fn published_refusal(
    backend: &weflow_core::webui::Backend<'_>,
    api_version: Option<&str>,
    min_backend_version: Option<&str>,
) -> Option<String> {
    match (api_version, min_backend_version) {
        (Some(api), Some(min)) => backend.refusal(api, min).map(weflow_core::locale::localize),
        _ => Some(
            weflow_core::locale::tr(
                "its metadata does not say which API and weflow version it needs",
                "它的元数据没有写明需要的 API 和 weflow 版本",
            )
            .to_string(),
        ),
    }
}

/// `value` as a URL query value: every byte but the unreserved ones (RFC 3986) percent-encoded, so a token with `+`,
/// `&`, `#` or `=` reaches the page as it is.
pub(super) fn query_escape(value: &str) -> String {
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
pub(super) fn open_in_browser(url: &str) {
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
    let spawned = opener
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = spawned {
        // waited for off this thread, so the opener, which ends once it handed the address over, is reaped (on
        // Unix it would stay a zombie while the server runs)
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}
