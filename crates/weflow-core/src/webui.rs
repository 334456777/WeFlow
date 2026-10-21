//! The WebUI's versions (issue #148): built pages published as OCI artifacts on GHCR
//! (`ghcr.io/334456777/weflow-web:<version>`), downloaded, checked and kept by `weflow` itself, without Docker, ORAS
//! or Node.js.
//!
//! - Each version is kept with WeFlow's other caches, in `<home>/cache/web/<version>/` (the page and its
//!   `weflow-web.json`); the one picked with `weflow web use` is named in `<home>/cache/web/current`.
//! - A download is checked against its SHA-256 digest and size, unpacked into a folder of its own under limits (no
//!   path out of it, no links, a size and a number of files at most), checked (`index.html`, the metadata, the
//!   version asked for, compatibility), and only then moved into place in one rename. A failed or interrupted
//!   download leaves the versions already there as they were.
//! - The registry is reached as the OCI Distribution API describes: an anonymous token from the realm a `401` names,
//!   then the manifest and the blob.

use std::cmp::Ordering;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

/// Where the versions are published.
pub const REGISTRY: &str = "https://ghcr.io";
pub const REPOSITORY: &str = "334456777/weflow-web";
/// The artifact type of a WebUI version, and the media type of its one layer (the page, zipped).
pub const ARTIFACT_TYPE: &str = "application/vnd.weflow.web.v1";
pub const LAYER_TYPE: &str = "application/vnd.weflow.web.layer.v1+zip";
/// Annotations of the manifest, read before the download: the API it needs and the oldest `weflow` it runs on.
pub const ANNOTATION_API: &str = "io.weflow.web.api-version";
pub const ANNOTATION_MIN_BACKEND: &str = "io.weflow.web.min-backend-version";

/// The most a package may weigh, and the most it may unpack to, in how many files.
pub const MAX_PACKAGE: u64 = 64 << 20;
pub const MAX_UNPACKED: u64 = 256 << 20;
pub const MAX_FILES: usize = 10_000;

/// What a built page says about itself (`weflow-web.json`, written by the WebUI's build).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebMeta {
    pub name: String,
    pub version: String,
    pub api_version: String,
    pub min_backend_version: String,
}

/// What a page has to suit: the command API's version, and this `weflow`'s.
#[derive(Debug, Clone, Copy)]
pub struct Backend<'a> {
    pub api_version: &'a str,
    pub version: &'a str,
}

impl Backend<'_> {
    /// Why a page with this API version and lowest `weflow` cannot run here, or `None` when it can.
    pub fn refusal(&self, api_version: &str, min_backend_version: &str) -> Option<String> {
        if api_version != self.api_version {
            return Some(format!(
                "it needs API {api_version}, this weflow serves API {}",
                self.api_version
            ));
        }
        (compare_versions(self.version, min_backend_version) == Ordering::Less).then(|| {
            format!(
                "it needs weflow {min_backend_version} or newer, this is {}",
                self.version
            )
        })
    }
}

/// Compares versions as SemVer does (`5.0.0-rc.2` < `5.0.0-rc.10` < `5.0.0`), leading `v` or not. A development
/// build's `git describe` (`5.0.0-rc.5-3-gabc1234`) comes after its tag and before the next one.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    fn parts(v: &str) -> (Vec<u64>, Vec<String>, u64) {
        let v = v.trim().trim_start_matches('v');
        let v = v.split('+').next().unwrap_or("");
        // `<tag>-<n>-g<hash>`: n commits after the tag
        let (v, after) = match v.rsplitn(3, '-').collect::<Vec<_>>()[..] {
            [hash, n, tag] if hash.starts_with('g') && n.chars().all(|c| c.is_ascii_digit()) => {
                (tag, n.parse().unwrap_or(0))
            }
            _ => (v, 0),
        };
        let (core, pre) = v.split_once('-').unwrap_or((v, ""));
        let core = core.split('.').map(|n| n.parse().unwrap_or(0)).collect();
        let pre = if pre.is_empty() {
            Vec::new()
        } else {
            pre.split('.').map(str::to_string).collect()
        };
        (core, pre, after)
    }
    let (ac, ap, aa) = parts(a);
    let (bc, bp, ba) = parts(b);
    let core = (0..ac.len().max(bc.len()))
        .map(|i| ac.get(i).unwrap_or(&0).cmp(bc.get(i).unwrap_or(&0)))
        .find(|o| o.is_ne())
        .unwrap_or(Ordering::Equal);
    let pre = match (ap.is_empty(), bp.is_empty()) {
        (true, true) => Ordering::Equal,
        // a pre-release comes before its release
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => ap
            .iter()
            .zip(&bp)
            .map(|(x, y)| match (x.parse::<u64>(), y.parse::<u64>()) {
                (Ok(x), Ok(y)) => x.cmp(&y),
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                (Err(_), Err(_)) => x.cmp(y),
            })
            .find(|o| o.is_ne())
            .unwrap_or_else(|| ap.len().cmp(&bp.len())),
    };
    core.then(pre).then(aa.cmp(&ba))
}

/// A version as a folder name: digits, letters, dots, dashes and pluses only.
pub fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version.chars().next().is_some_and(|c| c.is_ascii_digit())
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
}

fn checked_version(version: &str) -> AppResult<&str> {
    let version = version.trim().trim_start_matches('v');
    if valid_version(version) {
        Ok(version)
    } else {
        Err(AppError::usage(format!(
            "`{version}` is not a WebUI version (such as 0.1.0)"
        )))
    }
}

/// The versions kept on this machine: `<home>/cache/web/`.
#[derive(Debug, Clone)]
pub struct WebStore {
    root: PathBuf,
}

impl WebStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn dir(&self, version: &str) -> PathBuf {
        self.root.join(version)
    }

    /// The metadata of a version kept here, when it is whole (its page and metadata, of that version).
    pub fn meta(&self, version: &str) -> Option<WebMeta> {
        if !valid_version(version) {
            return None;
        }
        let dir = self.dir(version);
        let meta = read_meta(&dir).ok()?;
        (meta.version == version && dir.join("index.html").is_file()).then_some(meta)
    }

    /// Every version kept here, newest first.
    pub fn installed(&self) -> Vec<WebMeta> {
        let mut found: Vec<WebMeta> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| self.meta(e.file_name().to_str()?))
            .collect();
        found.sort_by(|a, b| compare_versions(&b.version, &a.version));
        found
    }

    /// The version picked with `weflow web use`.
    pub fn chosen(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.root.join("current")).ok()?;
        let version = text.trim();
        valid_version(version).then(|| version.to_string())
    }

    /// Picks a version kept here (written beside and renamed, so a reader never sees half of it).
    pub fn choose(&self, version: &str) -> AppResult<()> {
        let fail = |e: std::io::Error| {
            AppError::runtime(format!("cannot write {}: {e}", self.root.display()))
        };
        std::fs::create_dir_all(&self.root).map_err(fail)?;
        let tmp = self.root.join(format!(".current-{}", std::process::id()));
        std::fs::write(&tmp, format!("{version}\n")).map_err(fail)?;
        std::fs::rename(&tmp, self.root.join("current")).map_err(fail)
    }

    /// Removes a version kept here; the pick goes with it. A version a running `weflow serve` shows (see
    /// [`WebStore::hold`]) is left as it is: the error's code is `in_use`.
    pub fn remove(&self, version: &str) -> AppResult<bool> {
        let version = checked_version(version)?;
        let dir = self.dir(version);
        if !dir.exists() {
            return Ok(false);
        }
        let lock = self.lock_file(version)?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(AppError::new(
                    "in_use",
                    format!("WebUI {version} is shown by a running `weflow serve`; stop it first"),
                    1,
                ))
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(AppError::runtime(format!(
                    "cannot lock WebUI {version}: {e}"
                )))
            }
        }
        std::fs::remove_dir_all(&dir)
            .map_err(|e| AppError::runtime(format!("cannot remove {}: {e}", dir.display())))?;
        if self.chosen().as_deref() == Some(version) {
            let _ = std::fs::remove_file(self.root.join("current"));
        }
        let _ = std::fs::remove_file(self.lock_path(version));
        Ok(true)
    }

    /// Keeps a version from being removed while the file returned is open: a shared lock on `.lock-<version>`
    /// beside it, which [`WebStore::remove`] asks for alone. It holds across processes, and ends with the process
    /// that holds it, however that ends.
    pub fn hold(&self, version: &str) -> AppResult<std::fs::File> {
        let version = checked_version(version)?;
        let lock = self.lock_file(version)?;
        lock.lock_shared()
            .map_err(|e| AppError::runtime(format!("cannot lock WebUI {version}: {e}")))?;
        Ok(lock)
    }

    fn lock_path(&self, version: &str) -> PathBuf {
        self.root.join(format!(".lock-{version}"))
    }

    fn lock_file(&self, version: &str) -> AppResult<std::fs::File> {
        let path = self.lock_path(version);
        std::fs::create_dir_all(&self.root)
            .and_then(|()| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .write(true)
                    .open(&path)
            })
            .map_err(|e| AppError::runtime(format!("cannot open {}: {e}", path.display())))
    }

    /// A fresh folder for one download, beside the versions (so the last step is a rename on one disk).
    fn scratch(&self, version: &str) -> AppResult<PathBuf> {
        let nonce: u64 = rand::random();
        let dir = self.root.join(format!(
            ".download-{version}-{}-{nonce:x}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir)
            .map_err(|e| AppError::runtime(format!("cannot create {}: {e}", dir.display())))?;
        Ok(dir)
    }
}

fn read_meta(dir: &Path) -> AppResult<WebMeta> {
    let path = dir.join("weflow-web.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| AppError::runtime(format!("cannot read {}: {e}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|e| AppError::runtime(format!("{} is not WebUI metadata: {e}", path.display())))
}

/// One version as the registry lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Published {
    pub version: String,
    pub digest: String,
    pub size: u64,
    pub api_version: Option<String>,
    pub min_backend_version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    #[serde(default)]
    artifact_type: Option<String>,
    #[serde(default)]
    config: Option<Descriptor>,
    #[serde(default)]
    layers: Vec<Descriptor>,
    #[serde(default)]
    annotations: std::collections::HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Descriptor {
    media_type: String,
    digest: String,
    size: u64,
}

/// The registry the versions come from: GHCR, or another (`WEFLOW_WEB_REGISTRY`, for tests and mirrors).
pub struct Registry {
    base: String,
    repository: String,
    client: reqwest::Client,
    token: tokio::sync::Mutex<Option<String>>,
}

impl Registry {
    pub fn new(base: &str, repository: &str) -> AppResult<Self> {
        let client = crate::http_client::builder()
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(60))
            .user_agent(concat!("weflow/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AppError::runtime(e.to_string()))?;
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            repository: repository.to_string(),
            client,
            token: Default::default(),
        })
    }

    /// GHCR, unless `WEFLOW_WEB_REGISTRY` names another.
    pub fn default_registry() -> AppResult<Self> {
        let base = std::env::var("WEFLOW_WEB_REGISTRY").unwrap_or_else(|_| REGISTRY.into());
        Self::new(&base, REPOSITORY)
    }

    fn failed(&self, what: &str, e: impl std::fmt::Display) -> AppError {
        AppError::new(
            "network_error",
            format!(
                "cannot reach the WebUI registry {} ({what}): {e}",
                self.base
            ),
            1,
        )
    }

    /// A GET of the registry, with the token a `401` asks for (fetched once, anonymously).
    async fn get(&self, path: &str, accept: &str) -> AppResult<reqwest::Response> {
        let url = format!("{}/v2/{}/{path}", self.base, self.repository);
        for attempt in 0..2 {
            let mut req = self
                .client
                .get(&url)
                .header(reqwest::header::ACCEPT, accept);
            if let Some(token) = self.token.lock().await.as_deref() {
                req = req.bearer_auth(token);
            }
            let resp = req.send().await.map_err(|e| self.failed(path, e))?;
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                let challenge = resp
                    .headers()
                    .get(reqwest::header::WWW_AUTHENTICATE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let token = self.fetch_token(&challenge).await?;
                *self.token.lock().await = Some(token);
                continue;
            }
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(AppError::new(
                    "not_found",
                    format!("not in the WebUI registry: {path}"),
                    1,
                ));
            }
            if !resp.status().is_success() {
                return Err(self.failed(path, resp.status()));
            }
            return Ok(resp);
        }
        Err(self.failed(path, "unauthorized"))
    }

    /// The token of a `Bearer realm="…",service="…",scope="…"` challenge.
    async fn fetch_token(&self, challenge: &str) -> AppResult<String> {
        let params: std::collections::HashMap<String, String> = challenge
            .trim_start_matches("Bearer ")
            .split(',')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| {
                (
                    k.trim().to_ascii_lowercase(),
                    v.trim().trim_matches('"').to_string(),
                )
            })
            .collect();
        let realm = params
            .get("realm")
            .ok_or_else(|| self.failed("token", format!("unexpected challenge `{challenge}`")))?;
        let mut query = Vec::new();
        for key in ["service", "scope"] {
            if let Some(v) = params.get(key) {
                query.push((key, v.as_str()));
            }
        }
        let resp = self
            .client
            .get(realm)
            .query(&query)
            .send()
            .await
            .map_err(|e| self.failed("token", e))?;
        if !resp.status().is_success() {
            return Err(self.failed("token", resp.status()));
        }
        let body: Value = resp.json().await.map_err(|e| self.failed("token", e))?;
        body["token"]
            .as_str()
            .or(body["access_token"].as_str())
            .map(str::to_string)
            .ok_or_else(|| self.failed("token", "no token in the answer"))
    }

    /// The tags that are versions, newest first.
    pub async fn versions(&self) -> AppResult<Vec<String>> {
        let resp = self.get("tags/list?n=1000", "application/json").await?;
        let body: Value = resp.json().await.map_err(|e| self.failed("tags", e))?;
        let mut tags: Vec<String> = body["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|t| valid_version(t))
            .map(str::to_string)
            .collect();
        tags.sort_by(|a, b| compare_versions(b, a));
        Ok(tags)
    }

    /// A version's manifest: its package and what it says it needs.
    pub async fn published(&self, version: &str) -> AppResult<Published> {
        let resp = self
            .get(
                &format!("manifests/{version}"),
                "application/vnd.oci.image.manifest.v1+json",
            )
            .await?;
        let manifest: Manifest = resp.json().await.map_err(|e| self.failed("manifest", e))?;
        let kind = manifest
            .artifact_type
            .as_deref()
            .or(manifest.config.as_ref().map(|c| c.media_type.as_str()));
        if kind != Some(ARTIFACT_TYPE) {
            return Err(AppError::runtime(format!(
                "{version} in the registry is not a WebUI (artifact type {})",
                kind.unwrap_or("none")
            )));
        }
        let layer = manifest
            .layers
            .iter()
            .find(|l| l.media_type == LAYER_TYPE)
            .ok_or_else(|| AppError::runtime(format!("{version} in the registry has no page")))?;
        Ok(Published {
            version: version.to_string(),
            digest: layer.digest.clone(),
            size: layer.size,
            api_version: manifest.annotations.get(ANNOTATION_API).cloned(),
            min_backend_version: manifest.annotations.get(ANNOTATION_MIN_BACKEND).cloned(),
        })
    }

    /// Downloads a package to `file`, checking its size and digest as it comes.
    async fn download(
        &self,
        published: &Published,
        file: &Path,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> AppResult<()> {
        if published.size > MAX_PACKAGE {
            return Err(AppError::runtime(format!(
                "the WebUI package of {} is {} bytes, more than the {MAX_PACKAGE} allowed",
                published.version, published.size
            )));
        }
        let expected = published
            .digest
            .strip_prefix("sha256:")
            .ok_or_else(|| AppError::runtime(format!("unknown digest {}", published.digest)))?
            .to_ascii_lowercase();
        let mut resp = self
            .get(&format!("blobs/{}", published.digest), "*/*")
            .await?;
        let mut out = std::fs::File::create(file)
            .map_err(|e| AppError::runtime(format!("cannot write {}: {e}", file.display())))?;
        let mut hash = Sha256::new();
        let mut got = 0u64;
        while let Some(chunk) = resp.chunk().await.map_err(|e| self.failed("download", e))? {
            got += chunk.len() as u64;
            if got > published.size {
                return Err(AppError::runtime(
                    "the WebUI package is bigger than its manifest says",
                ));
            }
            hash.update(&chunk);
            out.write_all(&chunk)
                .map_err(|e| AppError::runtime(format!("cannot write {}: {e}", file.display())))?;
            progress(got, published.size);
        }
        let digest: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != published.size || digest != expected {
            return Err(AppError::runtime(format!(
                "the WebUI package of {} is damaged: expected {} bytes with SHA-256 {expected}, got {got} bytes with {digest}",
                published.version, published.size
            )));
        }
        Ok(())
    }
}

/// Unpacks a package into `dir` under the limits: only paths inside it, no links, at most [`MAX_UNPACKED`] bytes
/// in [`MAX_FILES`] files.
pub fn unpack(package: &Path, dir: &Path) -> AppResult<()> {
    let bad = |what: String| AppError::runtime(format!("the WebUI package is refused: {what}"));
    let file = std::fs::File::open(package)
        .map_err(|e| AppError::runtime(format!("cannot read {}: {e}", package.display())))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| bad(e.to_string()))?;
    if zip.len() > MAX_FILES {
        return Err(bad(format!("{} files, more than {MAX_FILES}", zip.len())));
    }
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| bad(e.to_string()))?;
        let name = entry.name().to_string();
        let Some(relative) = entry.enclosed_name() else {
            return Err(bad(format!("`{name}` leads out of the folder")));
        };
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(bad(format!("`{name}` is a link")));
        }
        let target = dir.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| bad(e.to_string()))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| bad(e.to_string()))?;
        }
        let mut out = std::fs::File::create(&target).map_err(|e| bad(e.to_string()))?;
        // read at most what is left of the allowance, plus one byte to tell when it is over
        let left = MAX_UNPACKED - total;
        let copied = std::io::copy(&mut (&mut entry).take(left + 1), &mut out)
            .map_err(|e| bad(e.to_string()))?;
        total += copied;
        if total > MAX_UNPACKED {
            return Err(bad(format!("it unpacks to more than {MAX_UNPACKED} bytes")));
        }
    }
    Ok(())
}

/// A version kept here, when it suits this backend: an `incompatible` error when it does not (kept by a newer
/// weflow, say), and `None` when it is not kept.
pub fn kept(store: &WebStore, version: &str, backend: Backend<'_>) -> AppResult<Option<WebMeta>> {
    let version = checked_version(version)?;
    match store.meta(version) {
        Some(meta) => match backend.refusal(&meta.api_version, &meta.min_backend_version) {
            Some(why) => Err(incompatible(version, &why)),
            None => Ok(Some(meta)),
        },
        None => Ok(None),
    }
}

/// Downloads, checks and keeps a version for this backend; one kept already is checked again, as it may have been
/// kept by another weflow.
pub async fn install(
    store: &WebStore,
    registry: &Registry,
    version: &str,
    backend: Backend<'_>,
    progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> AppResult<WebMeta> {
    let version = checked_version(version)?;
    if let Some(meta) = kept(store, version, backend)? {
        return Ok(meta);
    }
    let published = registry.published(version).await?;
    if let (Some(api), Some(min)) = (&published.api_version, &published.min_backend_version) {
        if let Some(why) = backend.refusal(api, min) {
            return Err(incompatible(version, &why));
        }
    }
    let scratch = store.scratch(version)?;
    let result = async {
        let package = scratch.join("package.zip");
        registry.download(&published, &package, progress).await?;
        let site = scratch.join("site");
        unpack(&package, &site)?;
        let meta = read_meta(&site)?;
        if meta.name != "weflow-web" || meta.version != version {
            return Err(AppError::runtime(format!(
                "the package of {version} holds {} {}",
                meta.name, meta.version
            )));
        }
        if !site.join("index.html").is_file() {
            return Err(AppError::runtime(format!(
                "the package of {version} has no index.html"
            )));
        }
        if let Some(why) = backend.refusal(&meta.api_version, &meta.min_backend_version) {
            return Err(incompatible(version, &why));
        }
        // the last step: one rename; another run that got there first leaves its copy, which is the same
        match std::fs::rename(&site, store.dir(version)) {
            Ok(()) => Ok(meta),
            Err(_) if store.meta(version).is_some() => Ok(meta),
            Err(e) => Err(AppError::runtime(format!(
                "cannot move the WebUI into {}: {e}",
                store.dir(version).display()
            ))),
        }
    }
    .await;
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn incompatible(version: &str, why: &str) -> AppError {
    AppError::new(
        "incompatible",
        format!("WebUI {version} cannot run with this weflow: {why}"),
        1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_semver_and_git_describe() {
        let sorted = [
            "4.2.1",
            "5.0.0-rc.3",
            "5.0.0-rc.5",
            "5.0.0-rc.5-3-gabc1234",
            "5.0.0-rc.10",
            "5.0.0",
            "v5.0.1",
            "5.1.0",
        ];
        for pair in sorted.windows(2) {
            assert_eq!(
                compare_versions(pair[0], pair[1]),
                Ordering::Less,
                "{pair:?}"
            );
            assert_eq!(
                compare_versions(pair[1], pair[0]),
                Ordering::Greater,
                "{pair:?}"
            );
        }
        assert_eq!(compare_versions("0.1.0", "v0.1.0"), Ordering::Equal);
        assert!(valid_version("0.1.0-rc.1"));
        for bad in ["", "latest", "../x", "1/2", "0.1.0 ", "a1"] {
            assert!(!valid_version(bad), "{bad}");
        }
    }

    #[test]
    fn compatibility_is_the_api_and_the_lowest_weflow() {
        let backend = Backend {
            api_version: "1",
            version: "5.0.0-rc.5-2-gabc",
        };
        assert!(backend.refusal("1", "5.0.0-rc.5").is_none());
        assert!(backend
            .refusal("2", "5.0.0-rc.5")
            .unwrap()
            .contains("API 2"));
        assert!(backend
            .refusal("1", "5.0.0")
            .unwrap()
            .contains("5.0.0 or newer"));
    }
}
