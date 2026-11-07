use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct AppContext {
    pub home_dir: PathBuf,
    pub config_path: PathBuf,
    pub runtime_dir: PathBuf,
    pub version: String,
}

impl AppContext {
    pub fn new(config_override: Option<PathBuf>, version: impl Into<String>) -> AppResult<Self> {
        let version = version.into();
        let config_override = config_override.or_else(saved_config_path);
        let home_dir = if let Some(config_path) = &config_override {
            config_path.parent().map(Path::to_path_buf).ok_or_else(|| {
                AppError::config("config override must include a parent directory")
            })?
        } else {
            default_home_dir()?
        };
        fs::create_dir_all(&home_dir).map_err(|err| {
            AppError::config(format!("failed to create {}: {err}", home_dir.display()))
        })?;
        let config_path = config_override.unwrap_or_else(|| home_dir.join("config.json"));
        let runtime_dir = weflow_assets::ensure_runtime(&home_dir, &version)
            .map_err(|err| AppError::native(format!("failed to prepare runtime assets: {err}")))?;
        Ok(Self {
            home_dir,
            config_path,
            runtime_dir,
            version,
        })
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.home_dir.join("cache")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.home_dir.join("logs")
    }
}

/// WeFlow's config file (`config list`): the profiles and the settings shared by all of them.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ConfigStore {
    pub current_profile: String,
    /// Output language chosen with `weflow lang en|zh` (`None` = follow the system).
    pub lang: Option<String>,
    /// Options of `weflow serve` saved here (`serve_*` keys), used where its command line gives none.
    pub serve: ServeSettings,
    pub profiles: BTreeMap<String, ProfileConfig>,
    pub extra: BTreeMap<String, Value>,
}

/// Options of `weflow serve` saved in the config file (#406), each used when the command line does not give it; the
/// switches turn on what their flag turns on, and `None` leaves the default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ServeSettings {
    /// Listen address (`--host`).
    pub host: Option<String>,
    /// Listen port (`--port`).
    pub port: Option<u16>,
    /// Tasks run at once (`--max-tasks`).
    pub max_tasks: Option<u16>,
    /// Other commands run at once (`--max-reads`).
    pub max_reads: Option<u16>,
    /// Images and voices decoded at once (`--max-media`).
    pub max_media: Option<u16>,
    /// `--log-requests`.
    pub log_requests: Option<bool>,
    /// `--insight`.
    pub insight: Option<bool>,
    /// `--image-auto-download`.
    pub image_auto_download: Option<bool>,
    /// Whether to open the WebUI in the browser at the start; `false` is `--no-browser`.
    pub open_browser: Option<bool>,
}

impl ServeSettings {
    /// The keys, as `config set` names them after `serve_`.
    pub const KEYS: [&'static str; 9] = [
        "host",
        "port",
        "max_tasks",
        "max_reads",
        "max_media",
        "log_requests",
        "insight",
        "image_auto_download",
        "open_browser",
    ];

    /// The value of `key` (`host` of `serve_host`), or `None` when it is not one of these.
    fn get(&self, key: &str) -> Option<Value> {
        let value = match key {
            "host" => json!(self.host),
            "port" => json!(self.port),
            "max_tasks" => json!(self.max_tasks),
            "max_reads" => json!(self.max_reads),
            "max_media" => json!(self.max_media),
            "log_requests" => json!(self.log_requests),
            "insight" => json!(self.insight),
            "image_auto_download" => json!(self.image_auto_download),
            "open_browser" => json!(self.open_browser),
            _ => return None,
        };
        Some(value)
    }

    /// Sets `key`, one of [`Self::KEYS`], to `value`, checked.
    fn set(&mut self, key: &str, value: Value) -> AppResult<()> {
        let name = format!("serve_{key}");
        match key {
            "host" => {
                let host = as_string(value)?.trim().to_string();
                if format!("{host}:0").parse::<std::net::SocketAddr>().is_err() {
                    return Err(AppError::usage(format!(
                        "invalid value '{host}' for {name}; use an IP address such as 127.0.0.1, 0.0.0.0 or [::1]"
                    )));
                }
                self.host = Some(host);
            }
            "port" => self.port = Some(as_int_in(value, &name, 1, u16::MAX)?),
            "max_tasks" => self.max_tasks = Some(as_int_in(value, &name, 1, u16::MAX)?),
            "max_reads" => self.max_reads = Some(as_int_in(value, &name, 1, u16::MAX)?),
            "max_media" => self.max_media = Some(as_int_in(value, &name, 1, u16::MAX)?),
            "log_requests" => self.log_requests = Some(as_bool(value)?),
            "insight" => self.insight = Some(as_bool(value)?),
            "image_auto_download" => self.image_auto_download = Some(as_bool(value)?),
            "open_browser" => self.open_browser = Some(as_bool(value)?),
            _ => unreachable!("`{key}` is not one of ServeSettings::KEYS"),
        }
        Ok(())
    }

    /// Unsets `key` (`host` of `serve_host`); `false` when it is not one of these.
    fn unset(&mut self, key: &str) -> bool {
        match key {
            "host" => self.host = None,
            "port" => self.port = None,
            "max_tasks" => self.max_tasks = None,
            "max_reads" => self.max_reads = None,
            "max_media" => self.max_media = None,
            "log_requests" => self.log_requests = None,
            "insight" => self.insight = None,
            "image_auto_download" => self.image_auto_download = None,
            "open_browser" => self.open_browser = None,
            _ => return false,
        }
        true
    }
}

/// One profile of the config file: an account and its settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ProfileConfig {
    pub db_path: Option<String>,
    pub decrypt_key: Option<String>,
    pub wxid: Option<String>,
    pub image_xor_key: Option<i64>,
    pub image_aes_key: Option<String>,
    pub cache_path: Option<String>,
    pub http_api_token: Option<String>,
    pub ai_model_api_base_url: Option<String>,
    pub ai_model_api_key: Option<String>,
    pub ai_model_api_model: Option<String>,
    pub ai_model_api_max_tokens: Option<u32>,
    pub ai_insight_enabled: Option<bool>,
    pub extra: BTreeMap<String, Value>,
}

impl Default for ConfigStore {
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        profiles.insert("default".to_string(), ProfileConfig::default());
        Self {
            current_profile: "default".to_string(),
            lang: None,
            serve: ServeSettings::default(),
            profiles,
            extra: BTreeMap::new(),
        }
    }
}

impl ConfigStore {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let parsed = serde_json::from_str(&raw)
            .or_else(|_| toml::from_str::<Self>(&raw))
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(parsed)
    }

    /// Write the config through a temporary file and a rename, so a reader never sees a half-written file.
    /// Use [`ConfigStore::update`] to change the saved config: `save` alone overwrites what other processes wrote.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        static NEXT_TMP: AtomicU64 = AtomicU64::new(0);
        let tmp = path.with_file_name(format!(
            "{}.{}-{}.tmp",
            path.file_name().unwrap_or_default().to_string_lossy(),
            std::process::id(),
            NEXT_TMP.fetch_add(1, Ordering::Relaxed)
        ));
        let written = (|| -> Result<()> {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(&serde_json::to_vec_pretty(self)?)?;
            file.sync_all()?;
            drop(file);
            rename_replacing(&tmp, path)
        })();
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written.with_context(|| format!("failed to write {}", path.display()))
    }

    /// Read the saved config, change it with `change` and save it, holding a lock on `<config>.lock` meanwhile, so
    /// that concurrent `weflow` processes cannot overwrite each other's changes. A missing file starts from the
    /// default config.
    pub fn update<R>(path: &Path, change: impl FnOnce(&mut Self) -> AppResult<R>) -> AppResult<R> {
        Self::update_or(path, Self::default, change)
    }

    /// [`ConfigStore::update`], starting from `missing()` when the config file does not exist yet.
    pub fn update_or<R>(
        path: &Path,
        missing: impl FnOnce() -> Self,
        change: impl FnOnce(&mut Self) -> AppResult<R>,
    ) -> AppResult<R> {
        let config_err = |err: anyhow::Error| AppError::config(err.to_string());
        let _lock = lock_config(path).map_err(config_err)?;
        let mut config = if path.exists() {
            Self::load(path).map_err(config_err)?
        } else {
            missing()
        };
        let result = change(&mut config)?;
        config.save(path).map_err(config_err)?;
        Ok(result)
    }

    pub fn profile(&self, name: Option<&str>) -> Option<&ProfileConfig> {
        let name = name.unwrap_or(&self.current_profile);
        self.profiles.get(name)
    }

    pub fn profile_mut(&mut self, name: Option<&str>) -> &mut ProfileConfig {
        let name = name.unwrap_or(&self.current_profile).to_string();
        self.profiles.entry(name).or_default()
    }

    pub fn get_key(&self, profile: Option<&str>, key: &str) -> Value {
        if key == "current_profile" || key == "currentProfile" {
            return Value::String(self.current_profile.clone());
        }
        if key == "lang" {
            return self.lang.clone().map_or(Value::Null, Value::String);
        }
        if let Some(value) = key.strip_prefix("serve_").and_then(|k| self.serve.get(k)) {
            return value;
        }
        if key == "profiles" {
            return serde_json::to_value(&self.profiles).unwrap_or(Value::Null);
        }
        if let Some(value) = self.extra.get(key) {
            return value.clone();
        }

        let (profile, key) = self.split_profile_key(profile, key);
        let Some(target) = self.profile(profile.as_deref()) else {
            return Value::Null;
        };
        target.get_key(&key)
    }

    pub fn set_key(&mut self, profile: Option<&str>, key: &str, value: Value) -> AppResult<()> {
        if key == "lang" {
            let lang = as_string(value)?;
            if !matches!(lang.as_str(), "en" | "zh") {
                return Err(AppError::usage(format!(
                    "invalid value '{lang}' for lang; use en or zh"
                )));
            }
            self.lang = Some(lang);
            return Ok(());
        }
        if let Some(name) = key
            .strip_prefix("serve_")
            .filter(|k| ServeSettings::KEYS.contains(k))
        {
            return self.serve.set(name, value);
        }
        if key == "current_profile" || key == "currentProfile" {
            self.current_profile = as_string(value)?;
            self.profiles
                .entry(self.current_profile.clone())
                .or_default();
            return Ok(());
        }

        let (profile, key) = self.split_profile_key(profile, key);
        let target = self.profile_mut(profile.as_deref());
        match key.as_str() {
            "db_path" | "dbPath" => target.db_path = Some(as_string(value)?),
            "decrypt_key" | "decryptKey" => target.decrypt_key = Some(as_string(value)?),
            "wxid" | "myWxid" => target.wxid = Some(as_string(value)?),
            "image_xor_key" | "imageXorKey" => {
                target.image_xor_key = Some(as_int_in(value, &key, 0, u8::MAX.into())?);
            }
            "image_aes_key" | "imageAesKey" => target.image_aes_key = Some(as_string(value)?),
            "cache_path" | "cachePath" => target.cache_path = Some(as_string(value)?),
            "http_api_token" | "httpApiToken" => target.http_api_token = Some(as_string(value)?),
            "ai_model_api_base_url" | "aiModelApiBaseUrl" => {
                target.ai_model_api_base_url = Some(as_string(value)?)
            }
            "ai_model_api_key" | "aiModelApiKey" => {
                target.ai_model_api_key = Some(as_string(value)?)
            }
            "ai_model_api_model" | "aiModelApiModel" => {
                target.ai_model_api_model = Some(as_string(value)?)
            }
            "ai_model_api_max_tokens" | "aiModelApiMaxTokens" => {
                target.ai_model_api_max_tokens = Some(as_int_in(value, &key, 1, u32::MAX)?)
            }
            "ai_insight_enabled" | "aiInsightEnabled" => {
                target.ai_insight_enabled = Some(as_bool(value)?)
            }
            other => {
                target.extra.insert(other.to_string(), value);
            }
        }
        Ok(())
    }

    pub fn unset_key(&mut self, profile: Option<&str>, key: &str) {
        if key == "lang" {
            self.lang = None;
            return;
        }
        if key
            .strip_prefix("serve_")
            .is_some_and(|k| self.serve.unset(k))
        {
            return;
        }
        if key == "current_profile" || key == "currentProfile" {
            self.current_profile = "default".to_string();
            self.profiles
                .entry(self.current_profile.clone())
                .or_default();
            return;
        }

        let (profile, key) = self.split_profile_key(profile, key);
        let target = self.profile_mut(profile.as_deref());
        match key.as_str() {
            "db_path" | "dbPath" => target.db_path = None,
            "decrypt_key" | "decryptKey" => target.decrypt_key = None,
            "wxid" | "myWxid" => target.wxid = None,
            "image_xor_key" | "imageXorKey" => target.image_xor_key = None,
            "image_aes_key" | "imageAesKey" => target.image_aes_key = None,
            "cache_path" | "cachePath" => target.cache_path = None,
            "http_api_token" | "httpApiToken" => target.http_api_token = None,
            "ai_model_api_base_url" | "aiModelApiBaseUrl" => target.ai_model_api_base_url = None,
            "ai_model_api_key" | "aiModelApiKey" => target.ai_model_api_key = None,
            "ai_model_api_model" | "aiModelApiModel" => target.ai_model_api_model = None,
            "ai_model_api_max_tokens" | "aiModelApiMaxTokens" => {
                target.ai_model_api_max_tokens = None
            }
            "ai_insight_enabled" | "aiInsightEnabled" => target.ai_insight_enabled = None,
            other => {
                target.extra.remove(other);
            }
        }
    }

    fn split_profile_key(
        &self,
        explicit_profile: Option<&str>,
        key: &str,
    ) -> (Option<String>, String) {
        if explicit_profile.is_some() {
            return (explicit_profile.map(ToString::to_string), key.to_string());
        }
        if let Some((candidate, rest)) = key.split_once('.') {
            if self.profiles.contains_key(candidate) || !is_profile_key(candidate) {
                return (Some(candidate.to_string()), rest.to_string());
            }
        }
        (explicit_profile.map(ToString::to_string), key.to_string())
    }
}

impl ProfileConfig {
    pub fn get_key(&self, key: &str) -> Value {
        match key {
            "db_path" | "dbPath" => self
                .db_path
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "decrypt_key" | "decryptKey" => self
                .decrypt_key
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "wxid" | "myWxid" => self
                .wxid
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "image_xor_key" | "imageXorKey" => self
                .image_xor_key
                .map(|value| Value::Number(value.into()))
                .unwrap_or(Value::Null),
            "image_aes_key" | "imageAesKey" => self
                .image_aes_key
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "cache_path" | "cachePath" => self
                .cache_path
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "http_api_token" | "httpApiToken" => self
                .http_api_token
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "ai_model_api_base_url" | "aiModelApiBaseUrl" => self
                .ai_model_api_base_url
                .as_ref()
                .map(|v| Value::String(v.clone()))
                .unwrap_or(Value::Null),
            "ai_model_api_key" | "aiModelApiKey" => self
                .ai_model_api_key
                .as_ref()
                .map(|v| Value::String(v.clone()))
                .unwrap_or(Value::Null),
            "ai_model_api_model" | "aiModelApiModel" => self
                .ai_model_api_model
                .as_ref()
                .map(|v| Value::String(v.clone()))
                .unwrap_or(Value::Null),
            "ai_model_api_max_tokens" | "aiModelApiMaxTokens" => self
                .ai_model_api_max_tokens
                .map(|v| Value::Number(v.into()))
                .unwrap_or(Value::Null),
            "ai_insight_enabled" | "aiInsightEnabled" => self
                .ai_insight_enabled
                .map(Value::Bool)
                .unwrap_or(Value::Null),
            other => self.extra.get(other).cloned().unwrap_or(Value::Null),
        }
    }
}

pub fn resolve_account_dir(db_path: &str, wxid: &str) -> PathBuf {
    let root = expand_home(db_path);
    if is_account_dir(&root) {
        return root;
    }
    let cleaned = clean_account_dir_name(wxid);
    let direct = root.join(&cleaned);
    if is_account_dir(&direct) {
        return direct;
    }
    if let Ok(entries) = fs::read_dir(&root) {
        let lower = cleaned.to_lowercase();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let is_match = if lower.starts_with("wxid_") {
                name.starts_with(&format!("{lower}_"))
            } else {
                name == lower || name.starts_with(&format!("{lower}_"))
            };
            if is_match && is_account_dir(&path) {
                return path;
            }
        }
    }
    direct
}

pub fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Where the config file lives: the path chosen with `config set config_path`, else `config.json` in the home folder.
pub fn default_config_path() -> Option<PathBuf> {
    saved_config_path().or_else(|| default_home_dir().ok().map(|dir| dir.join("config.json")))
}

/// Exclusive lock on `<config>.lock`, released when the returned file is dropped.
fn lock_config(path: &Path) -> Result<fs::File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let lock_path = path.with_file_name(format!(
        "{}.lock",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("failed to open {}", lock_path.display()))?;
    file.lock()
        .with_context(|| format!("failed to lock {}", lock_path.display()))?;
    Ok(file)
}

/// `fs::rename` over an existing file. On Windows the replace can fail for a moment while another process (a reader,
/// an antivirus scan) has the old file open, so it is retried briefly there.
fn rename_replacing(from: &Path, to: &Path) -> Result<()> {
    let mut attempts = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(err)
                if cfg!(windows)
                    && err.kind() == std::io::ErrorKind::PermissionDenied
                    && attempts < 20 =>
            {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(err) => return Err(err.into()),
        }
    }
}

/// Pointer file in the home folder that remembers the config path chosen with `config set config_path`.
fn config_pointer_file() -> Option<PathBuf> {
    default_home_dir().ok().map(|dir| dir.join("config_path"))
}

/// The config path chosen with `config set config_path` (`None` = the default location).
pub fn saved_config_path() -> Option<PathBuf> {
    let text = fs::read_to_string(config_pointer_file()?).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// Go back to the default config file location.
pub fn clear_saved_config_path() -> AppResult<()> {
    let pointer = config_pointer_file()
        .ok_or_else(|| AppError::config("failed to locate platform config directory"))?;
    set_saved_config_path(&pointer.with_file_name("config.json"))
}

/// Remember `path` as the config file for later runs; the default location clears the setting.
pub fn set_saved_config_path(path: &Path) -> AppResult<()> {
    let pointer = config_pointer_file()
        .ok_or_else(|| AppError::config("failed to locate platform config directory"))?;
    let default_path = pointer.with_file_name("config.json");
    if path == default_path {
        return match fs::remove_file(&pointer) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(AppError::config(
                format!("failed to update {}: {err}", pointer.display()),
            )),
            _ => Ok(()),
        };
    }
    if let Some(parent) = pointer.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            AppError::config(format!("failed to create {}: {err}", parent.display()))
        })?;
    }
    fs::write(&pointer, path.to_string_lossy().as_bytes())
        .map_err(|err| AppError::config(format!("failed to write {}: {err}", pointer.display())))
}

fn default_home_dir() -> AppResult<PathBuf> {
    if let Ok(home) = env::var("WEFLOW_HOME") {
        if !home.trim().is_empty() {
            return Ok(PathBuf::from(home));
        }
    }
    dirs::config_dir()
        .map(|dir| dir.join("weflow"))
        .ok_or_else(|| AppError::config("failed to locate platform config directory"))
}

fn clean_account_dir_name(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.to_lowercase().starts_with("wxid_") {
        if let Some(idx) = trimmed[5..].find('_') {
            return trimmed[..5 + idx].to_string();
        }
    }
    if let Some(idx) = trimmed.rfind('_') {
        let suffix = &trimmed[idx + 1..];
        if suffix.len() == 4 && suffix.chars().all(|c| c.is_ascii_alphanumeric()) {
            return trimmed[..idx].to_string();
        }
    }
    trimmed.to_string()
}

pub fn is_account_dir(path: &Path) -> bool {
    path.join("db_storage").exists()
        || path.join("FileStorage/Image").exists()
        || path.join("FileStorage/Image2").exists()
}

fn is_profile_key(key: &str) -> bool {
    matches!(
        key,
        "db_path"
            | "dbPath"
            | "decrypt_key"
            | "decryptKey"
            | "wxid"
            | "myWxid"
            | "image_xor_key"
            | "imageXorKey"
            | "image_aes_key"
            | "imageAesKey"
            | "cache_path"
            | "cachePath"
            | "http_api_token"
            | "httpApiToken"
            | "ai_model_api_base_url"
            | "aiModelApiBaseUrl"
            | "ai_model_api_key"
            | "aiModelApiKey"
            | "ai_model_api_model"
            | "aiModelApiModel"
            | "ai_model_api_max_tokens"
            | "aiModelApiMaxTokens"
            | "ai_insight_enabled"
            | "aiInsightEnabled"
    )
}

fn as_string(value: Value) -> AppResult<String> {
    match value {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => Err(AppError::usage("expected scalar string value")),
    }
}

fn as_i64(value: Value) -> AppResult<i64> {
    match value {
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| AppError::usage("expected integer value")),
        Value::String(s) => s
            .parse::<i64>()
            .map_err(|_| AppError::usage("expected integer value")),
        _ => Err(AppError::usage("expected integer value")),
    }
}

/// An integer in `min..=max`; anything outside is refused rather than wrapped by a later `as` cast.
/// A whole number from `min` to `max`, as the type of the field it sets.
fn as_int_in<T>(value: Value, key: &str, min: T, max: T) -> AppResult<T>
where
    T: Copy + Into<i64> + TryFrom<i64>,
{
    let (min, max) = (min.into(), max.into());
    let n = as_i64(value)?;
    T::try_from(n)
        .ok()
        .filter(|_| (min..=max).contains(&n))
        .ok_or_else(|| {
            AppError::usage(format!(
                "invalid value {n} for {key}; use a whole number from {min} to {max}"
            ))
        })
}

fn as_bool(value: Value) -> AppResult<bool> {
    match value {
        Value::Bool(b) => Ok(b),
        Value::String(s) => match s.as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => Err(AppError::usage("expected boolean value")),
        },
        _ => Err(AppError::usage("expected boolean value")),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;

    #[test]
    fn resolves_account_dir_with_wechat_suffix() {
        let dir = temp_dir("weflow-account-dir");
        let account = dir.join("wxid_abc_1234");
        fs::create_dir_all(account.join("db_storage/session")).unwrap();

        let resolved = resolve_account_dir(dir.to_str().unwrap(), "wxid_abc");
        assert_eq!(resolved, account);
    }

    /// A config file from a version that had `weflow ffmpeg set baseurl` still loads: the key is left out.
    #[test]
    fn a_config_with_the_old_ffmpeg_mirror_still_loads() {
        let dir = temp_dir("weflow-config-ffmpeg");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        fs::write(
            &path,
            json!({
                "current_profile": "default",
                "lang": "zh",
                "ffmpeg_base_url": "https://registry.npmmirror.com/-/binary/ffmpeg-static",
                "profiles": { "default": { "wxid": "wxid_test" } }
            })
            .to_string(),
        )
        .unwrap();
        let store = ConfigStore::load(&path).unwrap();
        assert_eq!(store.lang.as_deref(), Some("zh"));
        assert_eq!(store.get_key(None, "default.wxid"), json!("wxid_test"));
        assert!(!serde_json::to_string(&store).unwrap().contains("ffmpeg"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn gets_profile_keys_and_extra_values() {
        let mut store = ConfigStore::default();
        store
            .set_key(None, "default.wxid", Value::String("wxid_test".to_string()))
            .unwrap();
        store
            .set_key(
                None,
                "default.custom",
                Value::String("custom_value".to_string()),
            )
            .unwrap();

        assert_eq!(
            store.get_key(None, "default.wxid"),
            Value::String("wxid_test".to_string())
        );
        assert_eq!(
            store.get_key(None, "custom"),
            Value::String("custom_value".to_string())
        );
    }

    #[test]
    fn concurrent_updates_keep_every_key() {
        let dir = temp_dir("weflow-config-concurrent");
        let path = dir.join("config.json");
        std::thread::scope(|s| {
            for i in 0..32 {
                let path = &path;
                s.spawn(move || {
                    ConfigStore::update(path, |c| c.set_key(None, &format!("k{i}"), json!(i)))
                        .unwrap()
                });
            }
        });
        let saved = ConfigStore::load(&path).unwrap();
        for i in 0..32 {
            assert_eq!(
                saved.get_key(None, &format!("k{i}")),
                json!(i),
                "k{i} was lost"
            );
        }
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names
                .iter()
                .all(|n| n == "config.json" || n == "config.json.lock"),
            "{names:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_update_leaves_the_file_unchanged() {
        let dir = temp_dir("weflow-config-failed-update");
        let path = dir.join("config.json");
        ConfigStore::update(&path, |c| c.set_key(None, "wxid", json!("wxid_a"))).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(
            ConfigStore::update(&path, |c| c.set_key(None, "image_xor_key", json!(256))).is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn numeric_keys_refuse_out_of_range_values() {
        let mut store = ConfigStore::default();
        for (key, bad) in [
            ("ai_model_api_max_tokens", json!(4294967296_i64)),
            ("ai_model_api_max_tokens", json!(-1)),
            ("image_xor_key", json!(256)),
            ("image_xor_key", json!("-1")),
        ] {
            let err = store.set_key(None, key, bad.clone()).unwrap_err();
            assert!(
                err.to_string().contains("use a whole number from"),
                "{key}={bad}: {err}"
            );
            assert_eq!(
                store.get_key(None, key),
                Value::Null,
                "{key}={bad} was saved"
            );
        }
        for (key, good) in [
            ("ai_model_api_max_tokens", 4294967295_i64),
            ("image_xor_key", 0),
            ("image_xor_key", 255),
        ] {
            store.set_key(None, key, json!(good.to_string())).unwrap();
            assert_eq!(store.get_key(None, key), json!(good));
        }
    }

    #[test]
    fn serve_options_are_settings_of_the_file_and_checked() {
        let mut store = ConfigStore::default();
        store.set_key(None, "serve_port", json!("8080")).unwrap();
        store
            .set_key(None, "serve_host", json!(" 0.0.0.0 "))
            .unwrap();
        store
            .set_key(None, "serve_open_browser", json!("false"))
            .unwrap();
        assert_eq!(store.get_key(None, "serve_port"), json!(8080));
        assert_eq!(store.get_key(None, "serve_host"), json!("0.0.0.0"));
        assert_eq!(store.serve.open_browser, Some(false));
        // not a profile's: another profile sees them too
        assert_eq!(store.get_key(Some("work"), "serve_port"), json!(8080));
        assert!(store.profile(None).unwrap().extra.is_empty());

        for (key, bad) in [
            ("serve_port", json!(0)),
            ("serve_max_tasks", json!(65536)),
            ("serve_host", json!("localhost")),
            ("serve_insight", json!("maybe")),
        ] {
            assert!(
                store.set_key(None, key, bad.clone()).is_err(),
                "{key}={bad}"
            );
        }
        assert_eq!(store.get_key(None, "serve_insight"), Value::Null);

        store.unset_key(None, "serve_port");
        assert_eq!(store.get_key(None, "serve_port"), Value::Null);
        assert_eq!(store.serve.host.as_deref(), Some("0.0.0.0"));
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
