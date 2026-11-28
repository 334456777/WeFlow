//! WeFlow's configuration: the config file with its profiles and the saved options of `serve`, and the folders
//! WeFlow keeps its files in.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

mod extra_keys;
pub mod keys;
mod legacy_keys;
pub use legacy_keys::refuse_renamed;

/// Where a run of `weflow` keeps its files, and which version it is.
#[derive(Debug, Clone)]
pub struct AppContext {
    /// WeFlow's home folder: the config file's folder, and the parent of the cache.
    pub home_dir: PathBuf,
    /// The config file.
    pub config_path: PathBuf,
    /// The version of this `weflow`.
    pub version: String,
}

impl AppContext {
    /// Where this `weflow` keeps its files: beside the config file chosen with `config set config_path`, else in
    /// the home folder.
    pub fn new(version: impl Into<String>) -> AppResult<Self> {
        let version = version.into();
        let chosen = saved_config_path();
        let home_dir = if let Some(config_path) = &chosen {
            config_path.parent().map(Path::to_path_buf).ok_or_else(|| {
                AppError::config("the config file path must include a parent directory")
            })?
        } else {
            default_home_dir()?
        };
        // under `sudo`, what root makes here goes back to the caller when the command ends (#855)
        crate::sudo_caller::adopt(&home_dir);
        crate::private_files::create_dir_all(&home_dir).map_err(|err| {
            AppError::config(format!("failed to create {}: {err}", home_dir.display()))
        })?;
        let config_path = chosen.unwrap_or_else(|| home_dir.join("config.json"));
        Ok(Self {
            home_dir,
            config_path,
            version,
        })
    }

    /// The cache folder, `cache` in the home folder.
    pub fn cache_dir(&self) -> PathBuf {
        self.home_dir.join("cache")
    }
}

/// WeFlow's config file (`config list`): the profiles and the settings shared by all of them.
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ConfigStore {
    /// The profile the commands read; `default` in a new file.
    pub current_profile: String,
    /// Output language chosen with `weflow lang en|zh` (`None` = follow the system).
    pub lang: Option<String>,
    /// Options of `weflow serve` saved here (`serve_*` keys), used where its command line gives none.
    pub serve: ServeSettings,
    /// The profiles, by name.
    pub profiles: BTreeMap<String, ProfileConfig>,
    /// The file's other top-level keys, kept as they are.
    pub extra: BTreeMap<String, Value>,
}

/// The profiles as [`ProfileConfig`] shows them, and of the other keys only their names.
impl std::fmt::Debug for ConfigStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            current_profile,
            lang,
            serve,
            profiles,
            extra,
        } = self;
        f.debug_struct("ConfigStore")
            .field("current_profile", current_profile)
            .field("lang", lang)
            .field("serve", serve)
            .field("profiles", profiles)
            .field("extra", &KeyNames(extra))
            .finish()
    }
}

/// Whether a key or token is set, never its value, for `Debug`.
struct SetOrNot<'a, T>(&'a Option<T>);

impl<T> std::fmt::Debug for SetOrNot<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "Some(..)" } else { "None" })
    }
}

/// The names of keys kept as they are, for `Debug`: their values may be secrets too.
struct KeyNames<'a>(&'a BTreeMap<String, Value>);

impl std::fmt::Debug for KeyNames<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
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
    /// Whether to run the image auto-download hook; `None` is on.
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
                // what `serve --host` takes, which resolves a host name when it starts
                if keys::listen_host(&host).is_err() {
                    return Err(AppError::usage(format!(
                        "invalid value '{host}' for {name}; use an IP address such as 127.0.0.1, 0.0.0.0 or [::1], or a host name"
                    )));
                }
                self.host = Some(host);
            }
            "port" => self.port = Some(as_bounded(value, &keys::SERVE_PORT)?),
            "max_tasks" => self.max_tasks = Some(as_bounded(value, &keys::SERVE_MAX_TASKS)?),
            "max_reads" => self.max_reads = Some(as_bounded(value, &keys::SERVE_MAX_READS)?),
            "max_media" => self.max_media = Some(as_bounded(value, &keys::SERVE_MAX_MEDIA)?),
            "log_requests" => self.log_requests = Some(as_bool(value, &name)?),
            "insight" => self.insight = Some(as_bool(value, &name)?),
            "image_auto_download" => self.image_auto_download = Some(as_bool(value, &name)?),
            "open_browser" => self.open_browser = Some(as_bool(value, &name)?),
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
#[derive(Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ProfileConfig {
    /// WeChat's data directory (`db_path`).
    pub db_path: Option<String>,
    /// The database key, in hex (`decrypt_key`).
    pub decrypt_key: Option<String>,
    /// The account's wxid (`wxid`).
    pub wxid: Option<String>,
    /// The image XOR key, 0 to 255 (`image_xor_key`).
    pub image_xor_key: Option<i64>,
    /// The image AES key (`image_aes_key`).
    pub image_aes_key: Option<String>,
    /// The folder of the cached images, voices, stickers and Moments (`cache_path`); `cache` beside the config file
    /// when unset.
    pub cache_path: Option<String>,
    /// The access token of `weflow serve`'s HTTP API (`http_api_token`).
    pub http_api_token: Option<String>,
    /// The API format of the model API, `openai` or `anthropic`. Kept as text, so that a file edited by hand still
    /// loads; read as `openai` when it is neither.
    pub ai_model_api_format: Option<String>,
    /// The model API's base URL; the service of the format when unset.
    pub ai_model_api_base_url: Option<String>,
    /// The model API's key.
    pub ai_model_api_key: Option<String>,
    /// The model asked for.
    pub ai_model_api_model: Option<String>,
    /// The most tokens of one answer.
    pub ai_model_api_max_tokens: Option<u32>,
    /// Whether the AI insights run.
    pub ai_insight_enabled: Option<bool>,
    /// The profile's other keys (those the WebUI keeps, such as `auto_download_whitelist`), kept as they are.
    pub extra: BTreeMap<String, Value>,
}

/// The settings, each key and token only as set or not, and of the other keys only their names.
impl std::fmt::Debug for ProfileConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // every field named: a new one does not compile until it is shown or hidden here
        let Self {
            db_path,
            decrypt_key,
            wxid,
            image_xor_key,
            image_aes_key,
            cache_path,
            http_api_token,
            ai_model_api_format,
            ai_model_api_base_url,
            ai_model_api_key,
            ai_model_api_model,
            ai_model_api_max_tokens,
            ai_insight_enabled,
            extra,
        } = self;
        f.debug_struct("ProfileConfig")
            .field("db_path", db_path)
            .field("decrypt_key", &SetOrNot(decrypt_key))
            .field("wxid", wxid)
            .field("image_xor_key", &SetOrNot(image_xor_key))
            .field("image_aes_key", &SetOrNot(image_aes_key))
            .field("cache_path", cache_path)
            .field("http_api_token", &SetOrNot(http_api_token))
            .field("ai_model_api_format", ai_model_api_format)
            .field("ai_model_api_base_url", ai_model_api_base_url)
            .field("ai_model_api_key", &SetOrNot(ai_model_api_key))
            .field("ai_model_api_model", ai_model_api_model)
            .field("ai_model_api_max_tokens", ai_model_api_max_tokens)
            .field("ai_insight_enabled", ai_insight_enabled)
            .field("extra", &KeyNames(extra))
            .finish()
    }
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
    /// The config file at `path`, as JSON or TOML; the default config when there is none.
    ///
    /// # Errors
    /// When the file cannot be read or parsed. The message is that of the parser of the format the file is in (JSON
    /// when it starts with `{`, else TOML), with the line and column of the mistake and without the text found
    /// there: the config holds keys.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut parsed =
            Self::parse(&raw).with_context(|| format!("failed to parse {}", path.display()))?;
        parsed.rename_legacy_keys();
        parsed.settle_checked_values();
        Ok(parsed)
    }

    /// The config in `text`: JSON when it starts with `{`, else TOML, so that a mistake is reported by the parser
    /// of the format the file is in (#1044). A file with nothing in it is the default config.
    fn parse(text: &str) -> Result<Self> {
        // some editors put a byte order mark first, which neither format has
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        if text.trim_start().starts_with('{') {
            return serde_json::from_str(text)
                .map_err(|err| anyhow::anyhow!("{}", without_found_value(&err.to_string())));
        }
        toml::from_str(text).map_err(|err| {
            let message = without_found_value(err.message());
            // `Display` of the error shows the line of the file it is at: the config holds keys
            let at = err.span().and_then(|span| {
                let before = text.get(..span.start)?;
                let line = before.matches('\n').count() + 1;
                let column = before
                    .rsplit('\n')
                    .next()
                    .map_or(0, |tail| tail.chars().count())
                    + 1;
                Some(format!(" at line {line} column {column}"))
            });
            anyhow::anyhow!("TOML error: {message}{}", at.unwrap_or_default())
        })
    }

    /// Gives the values of a profile that `set_key` checks in its `extra` ([`extra_keys::KEYS`]) the form it keeps
    /// them in (#1042, #1149), for a file written before it did: `no_progress` as `1` or `yes` is `true`. A value the
    /// key does not take stays as it is.
    fn settle_checked_values(&mut self) {
        let names: Vec<String> = self.profiles.keys().cloned().collect();
        for name in names {
            for (key, _) in extra_keys::KEYS {
                let Some(value) = self
                    .profiles
                    .get(&name)
                    .and_then(|profile| profile.extra.get(*key))
                    .cloned()
                else {
                    continue;
                };
                let _ = self.set_key(Some(&name), key, value);
            }
        }
    }

    /// Gives the keys of every profile that carry a name of WeFlow's desktop app (`dbPath`, `aiInsightAllowContext`)
    /// the key's own name (#1021): a key already set under its own name keeps its value, and a value the key does not
    /// take stays under the old name.
    fn rename_legacy_keys(&mut self) {
        let names: Vec<String> = self.profiles.keys().cloned().collect();
        for name in names {
            for (old, new) in legacy_keys::RENAMED {
                let Some(value) = self
                    .profiles
                    .get_mut(&name)
                    .and_then(|profile| profile.extra.remove(*old))
                else {
                    continue;
                };
                if !self.get_key(Some(&name), new).is_null() {
                    continue;
                }
                if self.set_key(Some(&name), new, value.clone()).is_err() {
                    if let Some(profile) = self.profiles.get_mut(&name) {
                        profile.extra.insert((*old).to_string(), value);
                    }
                }
            }
        }
    }

    /// Write the config through a temporary file and a rename, so a reader never sees a half-written file.
    /// Use [`ConfigStore::update`] to change the saved config: `save` alone overwrites what other processes wrote.
    /// The file it leaves is its owner's only on Unix, whatever the mode of the one it replaces (#812).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            crate::private_files::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let failed = || format!("failed to write {}", path.display());
        let bytes = serde_json::to_vec_pretty(self).with_context(failed)?;
        crate::fs_atomic::write_private(path, &bytes).with_context(failed)
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
        let config_err = |err: anyhow::Error| AppError::config(format!("{err:#}"));
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

    /// The profile `name`, else the current one; `None` when there is no such profile.
    pub fn profile(&self, name: Option<&str>) -> Option<&ProfileConfig> {
        let name = name.unwrap_or(&self.current_profile);
        self.profiles.get(name)
    }

    /// The profile `name`, else the current one, made empty when there is no such profile.
    pub fn profile_mut(&mut self, name: Option<&str>) -> &mut ProfileConfig {
        let name = name.unwrap_or(&self.current_profile).to_string();
        self.profiles.entry(name).or_default()
    }

    /// The value of `key` (`config get`): a setting of the file itself, else of the profile `profile`, else of the
    /// one `key` names (`work.wxid`), else of the current one; `null` when it is not set.
    pub fn get_key(&self, profile: Option<&str>, key: &str) -> Value {
        if key == "current_profile" {
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

    /// The key `setting` of the profile `profile`, else of the current one, as the key reads it ([`keys::Setting`]):
    /// its default when it is not set or holds what the key does not take.
    pub fn setting<S: keys::Setting>(&self, profile: Option<&str>, setting: &S) -> S::Value {
        setting.read(&self.get_key(profile, setting.name()))
    }

    /// Sets `key` from the text a command line gives (`config set`, #813): as it is for the keys that hold text
    /// (paths, keys, tokens, names), so that `123456789012345678901234567890` or `1e5` stays what was typed; read as
    /// JSON for the others (numbers, `true`/`false`, and the arrays and objects of the keys the WebUI keeps).
    ///
    /// # Errors
    /// As [`Self::set_key`].
    pub fn set_key_from_text(
        &mut self,
        profile: Option<&str>,
        key: &str,
        text: &str,
    ) -> AppResult<()> {
        let (_, bare) = self.split_profile_key(profile, key);
        let value = if holds_text(key) || holds_text(&bare) {
            Value::String(text.to_string())
        } else {
            serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
        };
        self.set_key(profile, key, value)
    }

    /// Whether `name` is a profile: one of the file's, or the active one before it holds anything.
    fn has_profile(&self, name: &str) -> bool {
        self.profiles.contains_key(name) || self.current_profile == name
    }

    /// Sets `key` to `value` (`config set`), in the file itself or in a profile as [`Self::get_key`] finds it; a key
    /// WeFlow does not read goes to the profile's `extra`.
    ///
    /// # Errors
    /// A usage error for a name of WeFlow's desktop app ([`refuse_renamed`]), a value the key does not take, a
    /// profile named in `key` that does not exist, or a name for a new profile that cannot be addressed (empty, with a
    /// `.`, `/` or `\`, or the name of a setting).
    pub fn set_key(&mut self, profile: Option<&str>, key: &str, value: Value) -> AppResult<()> {
        refuse_renamed(key)?;
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
        if key == "current_profile" {
            let typed = as_string(value)?;
            // a profile the file has can be switched to whatever its name is (a hand-written file may hold names
            // `config set` refuses); only the name of a new one is checked (#1043)
            let name = if self.profiles.contains_key(&typed) {
                typed
            } else {
                checked_profile_name(&typed)?
            };
            self.profiles.entry(name.clone()).or_default();
            self.current_profile = name;
            return Ok(());
        }

        let named_in_key = profile.is_none();
        let (profile, key) = self.split_profile_key(profile, key);
        if let Some(name) = profile
            .as_deref()
            .filter(|name| named_in_key && !self.has_profile(name))
        {
            // `serve.port` or `defualt.wxid`: a profile is made only by `current_profile` (#813)
            let mut hints = Vec::new();
            if checked_profile_name(name).is_ok() {
                hints.push(format!("`config set current_profile {name}` makes one"));
            }
            let setting = format!("{name}_{key}");
            if is_known_key(&setting) {
                hints.push(format!("the setting is written {setting}"));
            }
            let mut message = format!("there is no profile named '{name}'");
            if !hints.is_empty() {
                message.push_str(&format!("; {}", hints.join(", and ")));
            }
            return Err(AppError::usage(message));
        }
        let target = self.profile_mut(profile.as_deref());
        match key.as_str() {
            "db_path" => target.db_path = Some(as_string(value)?),
            "decrypt_key" => target.decrypt_key = Some(as_string(value)?),
            "wxid" => target.wxid = Some(as_string(value)?),
            "image_xor_key" => {
                target.image_xor_key = Some(as_bounded(value, &keys::IMAGE_XOR_KEY)?);
            }
            "image_aes_key" => target.image_aes_key = Some(as_string(value)?),
            "cache_path" => target.cache_path = Some(as_string(value)?),
            "http_api_token" => target.http_api_token = Some(as_string(value)?),
            "ai_model_api_format" => {
                let text = as_string(value)?;
                let format = text
                    .parse::<crate::model_api::ModelApiFormat>()
                    .map_err(|_| {
                        AppError::usage(format!(
                            "invalid value '{text}' for {key}; use {}",
                            crate::model_api::ModelApiFormat::NAMES.join(" or ")
                        ))
                    })?;
                target.ai_model_api_format = Some(format.as_str().to_string());
            }
            "ai_model_api_base_url" => target.ai_model_api_base_url = Some(as_string(value)?),
            "ai_model_api_key" => target.ai_model_api_key = Some(as_string(value)?),
            "ai_model_api_model" => target.ai_model_api_model = Some(as_string(value)?),
            "ai_model_api_max_tokens" => {
                let tokens = keys::AI_MODEL_API_MAX_TOKENS;
                target.ai_model_api_max_tokens =
                    Some(as_int_in(value, tokens.name, tokens.min, tokens.max)?);
            }
            "ai_insight_enabled" => target.ai_insight_enabled = Some(as_bool(value, &key)?),
            other => {
                let value = match extra_keys::kind_of(other) {
                    Some(kind) => kind.settle(value, other)?,
                    None => value,
                };
                target.extra.insert(other.to_string(), value);
            }
        }
        Ok(())
    }

    /// Removes the profile `name` from the file (`config remove-profile`), with the account and keys it holds.
    /// WeFlow's cache is left as it is.
    ///
    /// # Errors
    /// A usage error when there is no such profile, or when it is the current one: the commands would be left with
    /// none to read.
    pub fn remove_profile(&mut self, name: &str) -> AppResult<()> {
        let shown = name.escape_debug();
        if name == self.current_profile {
            return Err(AppError::usage(format!(
                "'{shown}' is the current profile; switch to another with `config set current_profile` first"
            )));
        }
        if self.profiles.remove(name).is_none() {
            return Err(AppError::usage(format!(
                "there is no profile named '{shown}'; `config profiles` lists them"
            )));
        }
        Ok(())
    }

    /// Unsets `key` (`config unset`), found as [`Self::get_key`] finds it.
    ///
    /// # Errors
    /// A usage error for a name of WeFlow's desktop app ([`refuse_renamed`]).
    pub fn unset_key(&mut self, profile: Option<&str>, key: &str) -> AppResult<()> {
        refuse_renamed(key)?;
        if key == "lang" {
            self.lang = None;
            return Ok(());
        }
        if key
            .strip_prefix("serve_")
            .is_some_and(|k| self.serve.unset(k))
        {
            return Ok(());
        }
        if key == "current_profile" {
            self.current_profile = "default".to_string();
            self.profiles
                .entry(self.current_profile.clone())
                .or_default();
            return Ok(());
        }

        let named_in_key = profile.is_none();
        let (profile, key) = self.split_profile_key(profile, key);
        if profile
            .as_deref()
            .is_some_and(|name| named_in_key && !self.has_profile(name))
        {
            // nothing to remove, and no profile to make for it
            return Ok(());
        }
        let target = self.profile_mut(profile.as_deref());
        match key.as_str() {
            "db_path" => target.db_path = None,
            "decrypt_key" => target.decrypt_key = None,
            "wxid" => target.wxid = None,
            "image_xor_key" => target.image_xor_key = None,
            "image_aes_key" => target.image_aes_key = None,
            "cache_path" => target.cache_path = None,
            "http_api_token" => target.http_api_token = None,
            "ai_model_api_format" => target.ai_model_api_format = None,
            "ai_model_api_base_url" => target.ai_model_api_base_url = None,
            "ai_model_api_key" => target.ai_model_api_key = None,
            "ai_model_api_model" => target.ai_model_api_model = None,
            "ai_model_api_max_tokens" => target.ai_model_api_max_tokens = None,
            "ai_insight_enabled" => target.ai_insight_enabled = None,
            other => {
                target.extra.remove(other);
            }
        }
        Ok(())
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
    /// The value of `key`; `null` when it is not set.
    pub fn get_key(&self, key: &str) -> Value {
        match key {
            "db_path" => self
                .db_path
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "decrypt_key" => self
                .decrypt_key
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "wxid" => self
                .wxid
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "image_xor_key" => self
                .image_xor_key
                .map(|value| Value::Number(value.into()))
                .unwrap_or(Value::Null),
            "image_aes_key" => self
                .image_aes_key
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "cache_path" => self
                .cache_path
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "http_api_token" => self
                .http_api_token
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "ai_model_api_format" => self
                .ai_model_api_format
                .as_ref()
                .map(|v| Value::String(v.clone()))
                .unwrap_or(Value::Null),
            "ai_model_api_base_url" => self
                .ai_model_api_base_url
                .as_ref()
                .map(|v| Value::String(v.clone()))
                .unwrap_or(Value::Null),
            "ai_model_api_key" => self
                .ai_model_api_key
                .as_ref()
                .map(|v| Value::String(v.clone()))
                .unwrap_or(Value::Null),
            "ai_model_api_model" => self
                .ai_model_api_model
                .as_ref()
                .map(|v| Value::String(v.clone()))
                .unwrap_or(Value::Null),
            "ai_model_api_max_tokens" => self
                .ai_model_api_max_tokens
                .map(|v| Value::Number(v.into()))
                .unwrap_or(Value::Null),
            "ai_insight_enabled" => self
                .ai_insight_enabled
                .map(Value::Bool)
                .unwrap_or(Value::Null),
            other => self.extra.get(other).cloned().unwrap_or(Value::Null),
        }
    }
}

/// The account folder of `wxid` under the WeChat data directory `db_path` (or `db_path` itself when it is one).
///
/// `wxid` is tried as configured first, then without the folder suffix ([`clean_account_dir_name`]): `lily_1998` is
/// an id of its own as well as `lily` plus a suffix. For each, the folder named exactly that wins, then the folders
/// named the id plus the `_<4 chars>` suffix WeChat adds, in name order, so the pick does not depend on the directory
/// listing. With no account folder found, the result is `db_path/<id without suffix>`.
pub fn resolve_account_dir(db_path: &str, wxid: &str) -> PathBuf {
    let root = expand_home(db_path);
    if is_account_dir(&root) {
        return root;
    }
    let configured = wxid.trim();
    let cleaned = clean_account_dir_name(wxid);
    let mut folders: Option<Vec<(String, PathBuf)>> = None;
    for id in [configured, cleaned.as_str()] {
        if id.is_empty() {
            continue;
        }
        let direct = root.join(id);
        if is_account_dir(&direct) {
            return direct;
        }
        let folders = folders.get_or_insert_with(|| account_folders(&root));
        if let Some((_, path)) = folders.iter().find(|(name, _)| names_account(name, id)) {
            return path.clone();
        }
    }
    root.join(cleaned)
}

/// The account folders directly under `root` with their names, sorted by name.
fn account_folders(root: &Path) -> Vec<(String, PathBuf)> {
    let mut folders: Vec<(String, PathBuf)> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .filter(|(_, path)| is_account_dir(path))
        .collect();
    folders.sort();
    folders
}

/// Whether the account folder `name` is account `id`'s: the id itself, or the id plus the `_<4 chars>` suffix.
///
/// This is [`weflow_native::db::same_identity`] read one way. A folder named like a shorter id that `id` extends
/// with a suffix-shaped tail (`john` for `john_ab-c`) is another account's folder.
fn names_account(name: &str, id: &str) -> bool {
    name.trim().len() >= id.trim().len() && weflow_native::db::same_identity(name, id)
}

/// `path` with a leading `~` replaced by the home folder of the user this runs for (the caller's under `sudo`).
pub fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return crate::sudo_caller::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = crate::sudo_caller::home_dir() {
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
        crate::private_files::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    crate::fs_atomic::lock_beside(path)
        .with_context(|| format!("failed to lock {}.lock", path.display()))
}

/// Pointer file in the home folder that remembers the config path chosen with `config set config_path`.
fn config_pointer_file() -> Option<PathBuf> {
    default_home_dir().ok().map(|dir| dir.join("config_path"))
}

/// The config path chosen with `config set config_path` (`None` = the default location).
fn saved_config_path() -> Option<PathBuf> {
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
        crate::private_files::create_dir_all(parent).map_err(|err| {
            AppError::config(format!("failed to create {}: {err}", parent.display()))
        })?;
    }
    fs::write(&pointer, path.to_string_lossy().as_bytes())
        .map_err(|err| AppError::config(format!("failed to write {}: {err}", pointer.display())))
}

/// `WEFLOW_HOME`, else `weflow` in the config folder of the user this runs for (the caller's under `sudo`, #855).
fn default_home_dir() -> AppResult<PathBuf> {
    if let Ok(home) = env::var("WEFLOW_HOME") {
        if !home.trim().is_empty() {
            return Ok(PathBuf::from(home));
        }
    }
    crate::sudo_caller::config_dir()
        .map(|dir| dir.join("weflow"))
        .ok_or_else(|| AppError::config("failed to locate platform config directory"))
}

/// The account id of an account folder's name.
///
/// `wxid_abc_1a2b` is `wxid_abc` and `name_1a2b` is `name`; a name without the four-character suffix WeChat adds is
/// the id already.
pub fn clean_account_dir_name(dir_name: &str) -> String {
    let trimmed = dir_name.trim();
    if trimmed
        .get(..5)
        .is_some_and(|head| head.eq_ignore_ascii_case("wxid_"))
    {
        // `wxid_` and everything up to the next underscore
        let rest = &trimmed[5..];
        return match rest.find('_') {
            Some(0) => trimmed.to_string(),
            Some(end) => trimmed[..5 + end].to_string(),
            None => trimmed.to_string(),
        };
    }
    match trimmed.rsplit_once('_') {
        Some((head, tail))
            if !head.is_empty()
                && tail.len() == 4
                && tail.bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            head.to_string()
        }
        _ => trimmed.to_string(),
    }
}

/// A WeChat 4.x account folder: one with `db_storage`, which the database layer reads.
pub fn is_account_dir(path: &Path) -> bool {
    path.join("db_storage").is_dir()
}

fn is_profile_key(key: &str) -> bool {
    matches!(
        key,
        "db_path"
            | "decrypt_key"
            | "wxid"
            | "image_xor_key"
            | "image_aes_key"
            | "cache_path"
            | "http_api_token"
            | "ai_model_api_format"
            | "ai_model_api_base_url"
            | "ai_model_api_key"
            | "ai_model_api_model"
            | "ai_model_api_max_tokens"
            | "ai_insight_enabled"
    )
}

/// The keys that hold text: taken as they are typed, never read as a number (#813).
fn holds_text(key: &str) -> bool {
    matches!(
        key,
        "db_path"
            | "decrypt_key"
            | "wxid"
            | "image_aes_key"
            | "cache_path"
            | "http_api_token"
            | "ai_model_api_format"
            | "ai_model_api_base_url"
            | "ai_model_api_key"
            | "ai_model_api_model"
            | "lang"
            | "current_profile"
            | "serve_host"
    ) || extra_keys::holds_text(key)
}

/// The keys of the file's own settings and of `serve`'s, besides the profiles' keys.
const FILE_KEYS: &[&str] = &["lang", "current_profile"];

/// Whether `key` (without a profile) is one `config set --help` lists.
fn is_known_key(key: &str) -> bool {
    is_profile_key(key)
        || FILE_KEYS.contains(&key)
        || extra_keys::is_extra_key(key)
        || key
            .strip_prefix("serve_")
            .is_some_and(|k| ServeSettings::KEYS.contains(&k))
}

/// The known key that `key` looks like a misspelling of (`decrypt_ky`, `dbpath`), when `config set` does not know it
/// (#813).
///
/// Such a key is still saved, with the profile's other settings: the WebUI keeps settings of its own there.
pub fn near_known_key(key: &str) -> Option<String> {
    let bare = key.split_once('.').map_or(key, |(_, k)| k);
    if is_known_key(bare) {
        return None;
    }
    let plain = |k: &str| -> Vec<char> {
        k.chars()
            .filter(|c| *c != '_')
            .flat_map(char::to_lowercase)
            .collect()
    };
    let typed = plain(bare);
    let serve: Vec<String> = ServeSettings::KEYS
        .iter()
        .map(|k| format!("serve_{k}"))
        .collect();
    let snake = [
        "db_path",
        "decrypt_key",
        "wxid",
        "image_xor_key",
        "image_aes_key",
        "cache_path",
        "http_api_token",
        "ai_model_api_format",
        "ai_model_api_base_url",
        "ai_model_api_key",
        "ai_model_api_model",
        "ai_model_api_max_tokens",
        "ai_insight_enabled",
        "lang",
        "current_profile",
    ];
    snake
        .iter()
        .chain(extra_keys::KEYS.iter().map(|(k, _)| k))
        .map(|k| (*k).to_string())
        .chain(serve)
        .map(|k| {
            let known = plain(&k);
            // one slip in five letters at most, so that a short key of one's own (`name`) is not taken for `lang`
            let near = (known.len() / 5).max(1);
            (edit_distance(&typed, &known), near, k)
        })
        .filter(|(d, near, _)| d <= near)
        .min_by_key(|(d, _, _)| *d)
        .map(|(_, _, k)| k)
}

/// The slips that turn `a` into `b`: a letter put in, left out or changed, or two letters next to each other swapped
/// (`serve_prot`), each counting one (the optimal string alignment distance).
fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut two_back: Vec<usize> = vec![0; b.len() + 1];
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut current = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let changed = usize::from(a[i - 1] != b[j - 1]);
            let mut slips = (previous[j - 1] + changed)
                .min(previous[j] + 1)
                .min(current[j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                slips = slips.min(two_back[j - 2] + 1);
            }
            current[j] = slips;
        }
        two_back = std::mem::replace(&mut previous, current);
    }
    previous[b.len()]
}

/// `message` of a parser, without the value it found where it wanted another type: `invalid type: integer `42`,
/// expected a string` becomes `invalid type, expected a string`, as the value may be a key (#1044).
fn without_found_value(message: &str) -> String {
    for kind in ["invalid type", "invalid value"] {
        let wanted = message
            .strip_prefix(kind)
            .and_then(|rest| rest.strip_prefix(": "))
            .and_then(|rest| rest.rsplit_once(", expected "));
        if let Some((_, expected)) = wanted {
            return format!("{kind}, expected {expected}");
        }
    }
    message.to_string()
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

/// A whole number from `min` to `max` for the key `key`, as the type of the field it sets; anything outside is refused
/// rather than wrapped by a later `as` cast.
fn as_int_in<T: TryFrom<i64>>(value: Value, key: &str, min: i64, max: i64) -> AppResult<T> {
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

/// A whole number in the bounds of `bounds`, as the type of the field it sets ([`as_int_in`]).
fn as_bounded<T: TryFrom<i64>>(value: Value, bounds: &keys::Bounds) -> AppResult<T> {
    as_int_in(value, bounds.name, bounds.min, bounds.max)
}

/// A switch for the key `key`: `true` or `false`, or `1`/`0`, `yes`/`no`, `on`/`off`, in any case.
fn as_bool(value: Value, key: &str) -> AppResult<bool> {
    let text = match &value {
        Value::Bool(b) => return Ok(*b),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.trim().to_ascii_lowercase(),
        _ => String::new(),
    };
    match text.as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => {
            let shown = match &value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            Err(AppError::usage(format!(
                "invalid value '{shown}' for {key}; use true or false"
            )))
        }
    }
}

/// A number of seconds, 0 or more, for the key `key`; given as a number or as text that holds one.
fn as_seconds(value: Value, key: &str) -> AppResult<u64> {
    let text = match &value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    crate::output::parse_delay(&text).ok_or_else(|| {
        AppError::usage(format!(
            "invalid value '{text}' for {key}; use a whole number of seconds, 0 or more"
        ))
    })
}

/// Names a profile cannot take besides those of the keys `config set` lists: the file's own top-level keys, and
/// `config_path`, which is kept beside the config.
const RESERVED_PROFILE_NAMES: &[&str] = &["profiles", "serve", "extra", "config_path"];

/// The name `typed` makes for a new profile (#1043), without the spaces around it.
///
/// `<profile>.<key>` is read at the first `.`, so a name with one cannot be addressed, and nor can one that is also a
/// key: `wxid.db_path` is read as a key of the profile `wxid`, while `wxid` itself is the current profile's.
fn checked_profile_name(typed: &str) -> AppResult<String> {
    let name = typed.trim();
    if name.is_empty() {
        return Err(AppError::usage("a profile name cannot be empty"));
    }
    let shown = name.escape_debug();
    if name
        .chars()
        .any(|c| matches!(c, '.' | '/' | '\\') || c.is_control())
    {
        return Err(AppError::usage(format!(
            "invalid profile name '{shown}'; a name cannot hold a dot, a slash, a backslash or a control character"
        )));
    }
    if is_known_key(name) || RESERVED_PROFILE_NAMES.contains(&name) {
        return Err(AppError::usage(format!(
            "'{shown}' is reserved for the config file's own settings; choose another profile name"
        )));
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;

    #[test]
    fn debug_shows_whether_keys_are_set_but_never_their_values() {
        let profile = ProfileConfig {
            db_path: Some("data-dir".into()),
            decrypt_key: Some("db-key-secret".into()),
            image_xor_key: Some(173),
            image_aes_key: Some("aes-key-secret".into()),
            http_api_token: Some("token-secret".into()),
            ai_model_api_key: Some("model-key-secret".into()),
            extra: BTreeMap::from([("kept".to_string(), json!("extra-secret"))]),
            ..ProfileConfig::default()
        };
        let mut store = ConfigStore::default();
        store.profiles.insert("default".into(), profile);
        store
            .extra
            .insert("other".into(), json!({ "inner": "top-secret" }));
        let shown = format!("{store:?}");
        for secret in [
            "db-key-secret",
            "173",
            "aes-key-secret",
            "token-secret",
            "model-key-secret",
            "extra-secret",
            "top-secret",
        ] {
            assert!(!shown.contains(secret), "{secret} in {shown}");
        }
        for visible in [
            "data-dir",
            "decrypt_key: Some(..)",
            "wxid: None",
            "\"kept\"",
            "\"other\"",
        ] {
            assert!(shown.contains(visible), "{visible} not in {shown}");
        }
    }

    #[test]
    fn resolves_account_dir_with_wechat_suffix() {
        let dir = temp_dir("weflow-account-dir");
        let account = dir.join("wxid_abc_1234");
        fs::create_dir_all(account.join("db_storage/session")).unwrap();

        let resolved = resolve_account_dir(dir.to_str().unwrap(), "wxid_abc");
        assert_eq!(resolved, account);
    }

    #[test]
    fn account_folder_names_lose_the_suffix_wechat_adds() {
        for (name, id) in [
            ("wxid_abc123_1a2b", "wxid_abc123"),
            ("WXID_abc", "WXID_abc"),
            // four characters after `wxid_` are the id, not a suffix
            ("wxid_ab12", "wxid_ab12"),
            ("wxid__ab12", "wxid__ab12"),
            ("wxid_", "wxid_"),
            ("alice_9x9z", "alice"),
            ("alice_toolong", "alice_toolong"),
            ("_ab12", "_ab12"),
            ("plain", "plain"),
            ("  someone_a1b2 ", "someone"),
            ("  ", ""),
        ] {
            assert_eq!(clean_account_dir_name(name), id, "{name:?}");
        }
    }

    /// An id of four characters after `wxid_` names its own folder, not every account's.
    #[test]
    fn a_short_wxid_resolves_to_its_own_folder() {
        let dir = temp_dir("weflow-account-short");
        let other = dir.join("wxid_zz99_1234");
        let own = dir.join("wxid_ab12_5678");
        fs::create_dir_all(other.join("db_storage")).unwrap();
        fs::create_dir_all(own.join("db_storage")).unwrap();
        assert_eq!(resolve_account_dir(dir.to_str().unwrap(), "wxid_ab12"), own);
    }

    /// A custom id that starts another account's id resolves to its own folder, not the longer id's.
    #[test]
    fn a_custom_id_does_not_resolve_to_a_longer_id_it_starts() {
        let dir = temp_dir("weflow-account-prefix");
        let own = dir.join("john_abcd");
        let other = dir.join("john_smith_wxyz");
        fs::create_dir_all(own.join("db_storage")).unwrap();
        fs::create_dir_all(other.join("db_storage")).unwrap();
        let root = dir.to_str().unwrap();
        assert_eq!(resolve_account_dir(root, "john"), own);
        assert_eq!(resolve_account_dir(root, "john_smith"), other);
    }

    /// With only a longer id's folder present, a shorter id it starts has no folder.
    #[test]
    fn a_custom_id_without_its_folder_does_not_take_a_longer_ids() {
        let dir = temp_dir("weflow-account-prefix-only");
        fs::create_dir_all(dir.join("john_smith_wxyz/db_storage")).unwrap();
        assert_eq!(
            resolve_account_dir(dir.to_str().unwrap(), "john"),
            dir.join("john")
        );
    }

    /// A folder named like a shorter id is not the folder of an id that extends it with a suffix-shaped tail.
    #[test]
    fn an_id_does_not_resolve_to_a_shorter_id_it_extends() {
        let dir = temp_dir("weflow-account-shorter");
        fs::create_dir_all(dir.join("john/db_storage")).unwrap();
        let own = dir.join("john_ab-c_wxyz");
        fs::create_dir_all(own.join("db_storage")).unwrap();
        assert_eq!(resolve_account_dir(dir.to_str().unwrap(), "john_ab-c"), own);
    }

    /// An id whose tail looks like the folder suffix still finds its folder before the shorter id's.
    #[test]
    fn an_id_with_a_suffix_shaped_tail_resolves_to_its_own_folder() {
        let dir = temp_dir("weflow-account-tail");
        let own = dir.join("lily_1998_a1b2");
        fs::create_dir_all(own.join("db_storage")).unwrap();
        fs::create_dir_all(dir.join("lily_wxyz/db_storage")).unwrap();
        assert_eq!(resolve_account_dir(dir.to_str().unwrap(), "lily_1998"), own);
        // the folder's own name, suffix included, is an id too
        assert_eq!(
            resolve_account_dir(dir.to_str().unwrap(), "lily_wxyz"),
            dir.join("lily_wxyz")
        );
    }

    /// Several folders of one account resolve to the first by name, whatever order the directory lists them in.
    #[test]
    fn several_folders_of_one_account_resolve_to_the_first_by_name() {
        let dir = temp_dir("weflow-account-several");
        for name in ["john_zz99", "john_ab12", "john_mm55"] {
            fs::create_dir_all(dir.join(name).join("db_storage")).unwrap();
        }
        // not account folders: no `db_storage`
        fs::create_dir_all(dir.join("john_aaaa")).unwrap();
        fs::write(dir.join("john_0000"), b"").unwrap();
        assert_eq!(
            resolve_account_dir(dir.to_str().unwrap(), "john"),
            dir.join("john_ab12")
        );
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
    }

    /// A config file of WeFlow 7.0.0 or earlier, with names of the desktop app, is read with the keys' own (#1021).
    #[test]
    fn the_desktop_names_of_an_old_config_file_are_read_as_the_keys() {
        let dir = temp_dir("weflow-config-desktop-names");
        let path = dir.join("config.json");
        fs::write(
            &path,
            json!({
                "current_profile": "default",
                "profiles": {
                    "default": { "wxid": "wxid_test", "ai_model_api_model": "kept", "extra": {
                        "aiInsightAllowContext": true,
                        "autoDownloadWhitelist": ["a@chatroom"],
                        "aiInsightApiBaseUrl": "https://api.example.com",
                        "aiInsightApiModel": "old",
                        "aiModelApiMaxTokens": "many",
                        "my_own": 1
                    } },
                    "work": { "extra": { "aiFootprintEnabled": true } }
                }
            })
            .to_string(),
        )
        .unwrap();
        let store = ConfigStore::load(&path).unwrap();
        assert_eq!(store.get_key(None, "ai_insight_allow_context"), json!(true));
        assert_eq!(
            store.get_key(None, "auto_download_whitelist"),
            json!(["a@chatroom"])
        );
        assert_eq!(
            store.get_key(None, "ai_model_api_base_url"),
            json!("https://api.example.com")
        );
        // a key set under its own name keeps its value; a value the key does not take keeps its old name
        assert_eq!(store.get_key(None, "ai_model_api_model"), json!("kept"));
        assert_eq!(store.get_key(None, "ai_model_api_max_tokens"), Value::Null);
        assert_eq!(
            store.profiles["default"].extra.keys().collect::<Vec<_>>(),
            [
                "aiModelApiMaxTokens",
                "ai_insight_allow_context",
                "auto_download_whitelist",
                "my_own"
            ]
        );
        assert_eq!(
            store.get_key(None, "work.ai_footprint_enabled"),
            json!(true)
        );
        // saved with the keys' own names
        store.save(&path).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("aiInsightAllowContext"), "{saved}");
        assert!(saved.contains("ai_insight_allow_context"), "{saved}");
    }

    /// A name of the desktop app is no key: setting or unsetting it says the key's own name (#1021).
    #[test]
    fn the_desktop_names_are_refused() {
        let mut store = ConfigStore::default();
        for key in [
            "dbPath",
            "work.myWxid",
            "aiInsightAllowContext",
            "currentProfile",
        ] {
            let err = store.set_key_from_text(None, key, "x").unwrap_err();
            assert_eq!(err.code, "invalid_arguments", "{key}");
            assert!(err.message.contains("it is written"), "{}", err.message);
            assert!(store.unset_key(None, key).is_err(), "{key}");
        }
        assert_eq!(store.get_key(None, "db_path"), Value::Null);
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
                        .unwrap();
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
    }

    /// The config file holds the keys: its folder, the file and its lock are made for their owner only, and a file
    /// others could read is closed to them by the next save (#812).
    #[cfg(unix)]
    #[test]
    fn the_config_is_its_owners_only() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).map_or(0, |m| m.permissions().mode() & 0o777);
        let dir = temp_dir("weflow-config-mode");
        let path = dir.join("home/config.json");
        ConfigStore::update(&path, |c| c.set_key(None, "wxid", json!("wxid_a"))).unwrap();
        assert_eq!(mode(&dir.join("home")), 0o700);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&dir.join("home/config.json.lock")), 0o600);

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        ConfigStore::update(&path, |c| c.set_key(None, "wxid", json!("wxid_b"))).unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn numeric_keys_refuse_out_of_range_values() {
        let mut store = ConfigStore::default();
        for (key, bad) in [
            ("ai_model_api_max_tokens", json!(2_000_001)),
            ("ai_model_api_max_tokens", json!(4294967296_i64)),
            ("ai_model_api_max_tokens", json!(0)),
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
            ("ai_model_api_max_tokens", 2_000_000_i64),
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
            ("serve_port", json!(65536)),
            ("serve_port", json!(-1)),
            ("serve_max_tasks", json!(0)),
            ("serve_max_tasks", json!(65536)),
            ("serve_host", json!("")),
            ("serve_host", json!("a/b")),
            ("serve_host", json!("[::1")),
            ("serve_insight", json!("maybe")),
        ] {
            assert!(
                store.set_key(None, key, bad.clone()).is_err(),
                "{key}={bad}"
            );
        }
        assert_eq!(store.get_key(None, "serve_insight"), Value::Null);
        // what `serve --host` and `--port` take (#1265): a host name, an IPv6 address without brackets, a free port
        for (key, good) in [
            ("serve_host", json!("localhost")),
            ("serve_host", json!("::1")),
            ("serve_port", json!(0)),
        ] {
            store.set_key(None, key, good.clone()).unwrap();
            assert_eq!(store.get_key(None, key), good, "{key}");
        }
        store
            .set_key(None, "serve_host", json!(" 0.0.0.0 "))
            .unwrap();
        store.set_key(None, "serve_port", json!(8080)).unwrap();

        store.unset_key(None, "serve_port").unwrap();
        assert_eq!(store.get_key(None, "serve_port"), Value::Null);
        assert_eq!(store.serve.host.as_deref(), Some("0.0.0.0"));
    }

    /// `config set` keeps a key that holds text as it was typed, and reads the others as JSON (#813).
    #[test]
    fn text_is_kept_as_it_was_typed() {
        let mut store = ConfigStore::default();
        for typed in [
            "123456789012345678901234567890",
            "1e5",
            "1.50",
            "007",
            "0x10",
            "true",
            "null",
            "[1,2]",
            "{\"a\":1}",
        ] {
            for key in [
                "http_api_token",
                "image_aes_key",
                "default.ai_model_api_key",
            ] {
                store.set_key_from_text(None, key, typed).unwrap();
                assert_eq!(store.get_key(None, key), json!(typed), "{key}={typed}");
            }
        }
        store.set_key_from_text(None, "lang", "zh").unwrap();
        store
            .set_key_from_text(None, "serve_host", "127.0.0.1")
            .unwrap();

        store
            .set_key_from_text(None, "image_xor_key", "53")
            .unwrap();
        assert_eq!(store.get_key(None, "image_xor_key"), json!(53));
        store.set_key_from_text(None, "serve_port", "8080").unwrap();
        assert_eq!(store.get_key(None, "serve_port"), json!(8080));
        store
            .set_key_from_text(None, "ai_insight_enabled", "true")
            .unwrap();
        assert_eq!(store.get_key(None, "ai_insight_enabled"), json!(true));
        store
            .set_key_from_text(None, "auto_download_whitelist", "[\"a@chatroom\"]")
            .unwrap();
        assert_eq!(
            store.get_key(None, "auto_download_whitelist"),
            json!(["a@chatroom"])
        );
    }

    /// `<profile>.<key>` sets a key of a profile there is; it makes none, for `serve.port` reads like a setting
    /// and `defualt.wxid` like a slip (#813).
    #[test]
    fn a_profile_is_not_made_by_a_key() {
        let mut store = ConfigStore::default();
        // `serve` cannot name a profile (#1043), so only the setting is offered
        let err = store
            .set_key_from_text(None, "serve.port", "8080")
            .unwrap_err();
        assert!(
            !err.message.contains("current_profile serve"),
            "{}",
            err.message
        );
        assert!(err.message.contains("serve_port"), "{}", err.message);
        // a name that can be one is offered, for a slip or for a profile still to make
        let err = store
            .set_key_from_text(None, "work.wxid", "wxid_a")
            .unwrap_err();
        assert!(
            err.message.contains("current_profile work"),
            "{}",
            err.message
        );
        let err = store
            .set_key_from_text(None, "defualt.wxid", "wxid_a")
            .unwrap_err();
        assert!(!err.message.contains("defualt_wxid"), "{}", err.message);
        store.unset_key(None, "serve.port").unwrap();
        assert_eq!(
            store.profiles.keys().collect::<Vec<_>>(),
            ["default"],
            "only the profile every config file starts with"
        );

        // the active profile is one before it holds anything; a profile made by `current_profile` takes keys
        store
            .set_key_from_text(None, "default.wxid", "wxid_a")
            .unwrap();
        store
            .set_key(None, "current_profile", json!("work"))
            .unwrap();
        store
            .set_key_from_text(None, "default.wxid", "wxid_b")
            .unwrap();
        assert_eq!(store.get_key(None, "default.wxid"), json!("wxid_b"));
        assert_eq!(store.get_key(None, "work.wxid"), Value::Null);
    }

    /// A switch is taken as `true`/`false`, `1`/`0`, `yes`/`no` or `on`/`off` in any case, and a value that is none of
    /// them names the key and the words it takes (#1042).
    #[test]
    fn a_switch_takes_every_spelling_of_yes_and_no() {
        let mut store = ConfigStore::default();
        store
            .set_key(None, "current_profile", json!("work"))
            .unwrap();
        for (typed, on) in [
            ("true", true),
            ("TRUE", true),
            ("1", true),
            ("yes", true),
            ("Yes", true),
            ("on", true),
            (" ON ", true),
            ("false", false),
            ("False", false),
            ("0", false),
            ("no", false),
            ("OFF", false),
        ] {
            for key in [
                "serve_insight",
                "serve_log_requests",
                "serve_open_browser",
                "serve_image_auto_download",
                "ai_insight_enabled",
                "work.ai_insight_enabled",
                "no_progress",
            ] {
                store.set_key_from_text(None, key, typed).unwrap();
                assert_eq!(store.get_key(None, key), json!(on), "{key}={typed}");
            }
        }
        for (key, bad) in [
            ("serve_insight", "maybe"),
            ("serve_log_requests", "2"),
            ("serve_open_browser", "1.5"),
            ("ai_insight_enabled", "[1]"),
            ("no_progress", "null"),
            ("no_progress", ""),
        ] {
            let err = store.set_key_from_text(None, key, bad).unwrap_err();
            assert_eq!(err.code, "invalid_arguments", "{key}={bad}");
            assert!(
                err.message.contains(key) && err.message.contains("use true or false"),
                "{key}={bad}: {}",
                err.message
            );
        }
    }

    /// `no_progress` and `progress_delay_seconds` are checked by `set_key` like the other keys, wherever the value
    /// comes from, and kept as a switch and a number whatever was typed (#1042).
    #[test]
    fn the_progress_settings_are_checked_and_kept_as_one_type() {
        let mut store = ConfigStore::default();
        for typed in ["1", "yes", "on", "TRUE"] {
            store.set_key_from_text(None, "no_progress", typed).unwrap();
            assert_eq!(store.get_key(None, "no_progress"), json!(true), "{typed}");
        }
        store.set_key(None, "no_progress", json!("off")).unwrap();
        assert_eq!(store.get_key(None, "no_progress"), json!(false));
        store.set_key(None, "no_progress", json!(1)).unwrap();
        assert_eq!(store.get_key(None, "no_progress"), json!(true));
        let err = store
            .set_key_from_text(None, "no_progress", "maybe")
            .unwrap_err();
        assert!(err.message.contains("no_progress"), "{}", err.message);
        // the value that was there stays
        assert_eq!(store.get_key(None, "no_progress"), json!(true));

        for typed in ["5", " 5 ", "\"5\""] {
            store
                .set_key_from_text(None, "progress_delay_seconds", typed)
                .unwrap();
            assert_eq!(
                store.get_key(None, "progress_delay_seconds"),
                json!(5),
                "{typed}"
            );
        }
        store
            .set_key(None, "progress_delay_seconds", json!(0))
            .unwrap();
        assert_eq!(store.get_key(None, "progress_delay_seconds"), json!(0));
        for bad in ["-1", "abc", "1.5", "1e3", "true", "[1]", ""] {
            let err = store
                .set_key_from_text(None, "progress_delay_seconds", bad)
                .unwrap_err();
            assert!(
                err.message
                    .contains("use a whole number of seconds, 0 or more")
                    && err.message.contains("progress_delay_seconds"),
                "{bad}: {}",
                err.message
            );
        }
        assert_eq!(store.get_key(None, "progress_delay_seconds"), json!(0));
    }

    /// A file written before `set_key` checked the progress settings holds `1` or `yes` for `no_progress`: it is read
    /// as `true`, and a value no setting takes is left for the file's owner to see (#1042).
    #[test]
    fn the_progress_settings_of_an_old_file_are_read_as_set_keeps_them() {
        let dir = temp_dir("weflow-config-progress");
        let path = dir.join("config.json");
        fs::write(
            &path,
            json!({
                "current_profile": "a",
                "profiles": {
                    "a": { "extra": { "no_progress": 1, "progress_delay_seconds": "9" } },
                    "b": { "extra": { "no_progress": "yes", "progress_delay_seconds": 0 } },
                    "c": { "extra": { "no_progress": "on" } },
                    "d": { "extra": { "no_progress": "maybe", "progress_delay_seconds": "soon" } },
                    "e": { "extra": { "no_progress": false } },
                    "f": {}
                }
            })
            .to_string(),
        )
        .unwrap();
        let store = ConfigStore::load(&path).unwrap();
        assert_eq!(store.get_key(None, "no_progress"), json!(true));
        assert_eq!(store.get_key(None, "progress_delay_seconds"), json!(9));
        assert_eq!(store.get_key(None, "b.no_progress"), json!(true));
        assert_eq!(store.get_key(None, "b.progress_delay_seconds"), json!(0));
        assert_eq!(store.get_key(None, "c.no_progress"), json!(true));
        assert_eq!(store.get_key(None, "d.no_progress"), json!("maybe"));
        assert_eq!(
            store.get_key(None, "d.progress_delay_seconds"),
            json!("soon")
        );
        assert_eq!(store.get_key(None, "e.no_progress"), json!(false));
        assert!(store.profiles["f"].extra.is_empty());
    }

    /// A new profile is made only under a name `<profile>.<key>` can address and no key is spelt as (#1043).
    #[test]
    fn a_profile_is_not_made_under_a_name_that_cannot_be_addressed() {
        let mut store = ConfigStore::default();
        for name in [
            "",
            "  ",
            "a.b",
            ".",
            "a/b",
            "a\\b",
            "a\nb",
            "a\u{7}",
            // names of the keys `config set` lists, of the file's own and of what is kept beside it
            "wxid",
            "lang",
            "db_path",
            "current_profile",
            "serve_port",
            "no_progress",
            "profiles",
            "serve",
            "extra",
            "config_path",
        ] {
            let err = store
                .set_key(None, "current_profile", json!(name))
                .unwrap_err();
            assert_eq!(err.code, "invalid_arguments", "{name:?}");
            // said once, without the name's control characters as they are
            assert!(!err.message.contains(['\n', '\u{7}']), "{}", err.message);
            assert_eq!(store.current_profile, "default", "{name:?}");
            assert_eq!(store.profiles.len(), 1, "{name:?}: {:?}", store.profiles);
        }
        let err = store
            .set_key_from_text(None, "current_profile", "a.b")
            .unwrap_err();
        assert!(err.message.contains("'a.b'"), "{}", err.message);
        assert!(err.message.contains("dot"), "{}", err.message);

        // names that can be addressed, with the spaces around them left off; a number is one too
        for (typed, made) in [
            ("work", "work"),
            ("  home  ", "home"),
            ("家里", "家里"),
            ("my account", "my account"),
            ("1", "1"),
            ("my-profile_2", "my-profile_2"),
            // only a whole key is a key
            ("wxid2", "wxid2"),
            ("profile", "profile"),
        ] {
            store
                .set_key_from_text(None, "current_profile", typed)
                .unwrap();
            assert_eq!(store.current_profile, made, "{typed:?}");
            assert!(store.profiles.contains_key(made), "{typed:?}");
            store
                .set_key_from_text(None, &format!("{made}.wxid"), "wxid_a")
                .unwrap_or_else(|err| panic!("{made:?}: {err}"));
            assert_eq!(
                store.get_key(None, &format!("{made}.wxid")),
                json!("wxid_a")
            );
        }
        // switching to a profile there is takes no new name
        store
            .set_key(None, "current_profile", json!("default"))
            .unwrap();
        assert_eq!(store.current_profile, "default");
    }

    /// A file written by hand or by an earlier version may hold a profile `config set` would not make: it can be
    /// switched to and read, and `remove-profile` takes it away (#1043).
    #[test]
    fn a_profile_with_a_name_set_would_refuse_can_still_be_used_and_removed() {
        let dir = temp_dir("weflow-config-odd-profile");
        let path = dir.join("config.json");
        fs::write(
            &path,
            json!({
                "current_profile": "",
                "profiles": { "": {}, "a.b": { "wxid": "odd" }, "default": {} }
            })
            .to_string(),
        )
        .unwrap();
        let mut store = ConfigStore::load(&path).unwrap();
        assert_eq!(store.current_profile, "");
        store
            .set_key(None, "current_profile", json!("a.b"))
            .unwrap();
        assert_eq!(store.get_key(None, "wxid"), json!("odd"));
        store.set_key(None, "current_profile", json!("")).unwrap();
        assert_eq!(store.current_profile, "");
        store
            .set_key(None, "current_profile", json!("default"))
            .unwrap();
        store.remove_profile("").unwrap();
        store.remove_profile("a.b").unwrap();
        assert_eq!(store.profiles.keys().collect::<Vec<_>>(), ["default"]);
    }

    /// `remove_profile` takes a profile out of the file; the current one, and one there is not, are refused (#1043).
    #[test]
    fn a_profile_is_removed_unless_it_is_the_current_one() {
        let mut store = ConfigStore::default();
        store
            .set_key(None, "current_profile", json!("work"))
            .unwrap();
        store.set_key(None, "wxid", json!("wxid_work")).unwrap();
        store
            .set_key(None, "current_profile", json!("home"))
            .unwrap();
        store.set_key(None, "wxid", json!("wxid_home")).unwrap();
        let before = store.clone();

        let err = store.remove_profile("home").unwrap_err();
        assert_eq!(err.code, "invalid_arguments");
        assert!(
            err.message.contains("current profile")
                && err.message.contains("config set current_profile"),
            "{}",
            err.message
        );
        let err = store.remove_profile("nobody").unwrap_err();
        assert_eq!(err.code, "invalid_arguments");
        assert!(
            err.message.contains("'nobody'") && err.message.contains("config profiles"),
            "{}",
            err.message
        );
        assert_eq!(
            serde_json::to_value(&store).unwrap(),
            serde_json::to_value(&before).unwrap(),
            "a refusal changes nothing"
        );

        store.remove_profile("work").unwrap();
        assert_eq!(
            store.profiles.keys().collect::<Vec<_>>(),
            ["default", "home"]
        );
        assert_eq!(store.current_profile, "home");
        assert_eq!(store.get_key(None, "wxid"), json!("wxid_home"));
        assert_eq!(store.get_key(None, "work.wxid"), Value::Null);
        // `default` is a profile like the others: it goes when another is the current one
        store.remove_profile("default").unwrap();
        assert_eq!(store.profiles.keys().collect::<Vec<_>>(), ["home"]);
        // the name is taken as it is: `wxid` is no key of a profile here
        assert!(store.remove_profile("home.wxid").is_err());
    }

    /// A config in JSON that does not parse is reported by the JSON parser, with the line and column of the mistake
    /// (#1044).
    #[test]
    fn a_json_config_that_does_not_parse_is_reported_as_json() {
        let dir = temp_dir("weflow-config-bad-json");
        let path = dir.join("config.json");
        fs::write(
            &path,
            "{\n  \"current_profile\": \"default\",\n  \"profiles\": { \"default\": { \"wxid\": \"a\", } }\n}\n",
        )
        .unwrap();
        let err = format!("{:#}", ConfigStore::load(&path).unwrap_err());
        assert!(err.contains("failed to parse"), "{err}");
        assert!(err.contains("line 3"), "{err}");
        assert!(!err.contains("TOML"), "{err}");
    }

    /// An error does not show the text it was found at: the config holds keys, and the file may be on one line
    /// (#1044).
    #[test]
    fn a_config_that_does_not_parse_does_not_show_its_keys() {
        let dir = temp_dir("weflow-config-secret");
        let path = dir.join("config.json");
        let secret = "5ecretKeyThatMustNotBeShown";
        let broken = [
            // a mistake of syntax
            format!("{{ \"profiles\": {{ \"default\": {{ \"decrypt_key\": \"{secret}\", }} }} }}"),
            // a key of the wrong type, which the parser names by its value
            format!("{{ \"profiles\": {{ \"default\": {{ \"decrypt_key\": 123456789, \"wxid\": \"{secret}\" }} }} }}"),
            format!("{{ \"profiles\": {{ \"default\": {{ \"image_xor_key\": \"{secret}\" }} }} }}"),
            format!("{{ \"profiles\": {{ \"default\": {{ \"decrypt_key\": \"{secret}\"  \"wxid\": 1 }} }} }}"),
            // the same in TOML
            format!("[profiles.default]\ndecrypt_key = \"{secret}\" oops\n"),
            format!("[profiles.default]\ndecrypt_key = 123456789\nimage_xor_key = \"{secret}\"\n"),
            format!("profiles = {{ default = {{ decrypt_key = \"{secret}\", wxid = }} }}\n"),
        ];
        for text in broken {
            fs::write(&path, &text).unwrap();
            let err = format!("{:#}", ConfigStore::load(&path).unwrap_err());
            assert!(err.contains("failed to parse"), "{text}: {err}");
            assert!(!err.contains(secret), "{text}: {err}");
            assert!(!err.contains("123456789"), "{text}: {err}");
            assert!(!err.contains("decrypt_key"), "{text}: {err}");
        }
    }

    /// The parsers name the value they found where they wanted another type; the message keeps what was wanted and
    /// where, not what was found (#1044).
    #[test]
    fn what_a_parser_found_is_left_out_of_its_message() {
        let json = serde_json::from_str::<ProfileConfig>("{\n \"decrypt_key\": 123456789\n}")
            .unwrap_err()
            .to_string();
        assert!(json.contains("123456789"), "the reason for the cut: {json}");
        let kept = without_found_value(&json);
        assert!(!kept.contains("123456789"), "{kept}");
        assert!(
            kept.starts_with("invalid type, expected a string"),
            "{kept}"
        );
        assert!(kept.ends_with("at line 2 column 25"), "{kept}");

        let toml = toml::from_str::<ProfileConfig>("decrypt_key = 123456789\n")
            .unwrap_err()
            .message()
            .to_string();
        assert!(!without_found_value(&toml).contains("123456789"));
        for other in [
            "trailing comma at line 3 column 43",
            "invalid length 2, expected a tuple of size 3",
            "",
        ] {
            assert_eq!(without_found_value(other), other);
        }
    }

    /// A config that is not JSON is TOML, and says so, with the line and column of the mistake (#1044).
    #[test]
    fn a_toml_config_that_does_not_parse_is_reported_as_toml() {
        let dir = temp_dir("weflow-config-bad-toml");
        let path = dir.join("config.json");
        fs::write(
            &path,
            "current_profile = \"a\"\n\n[profiles.a\nwxid = \"x\"\n",
        )
        .unwrap();
        let err = format!("{:#}", ConfigStore::load(&path).unwrap_err());
        assert!(err.contains("TOML"), "{err}");
        assert!(err.contains("line 3"), "{err}");
    }

    /// Both formats load, and so do a file with nothing in it (the default config) and a JSON file that starts with a
    /// byte order mark (#1044).
    #[test]
    fn json_toml_and_an_empty_file_load() {
        let dir = temp_dir("weflow-config-formats");
        let path = dir.join("config.json");
        let json = "{\"current_profile\": \"work\", \"profiles\": {\"work\": {\"wxid\": \"a\"}}}";
        let toml = "current_profile = \"work\"\n[profiles.work]\nwxid = \"a\"\n";
        for (text, wxid) in [
            (json.to_string(), json!("a")),
            (format!("  \n{json}"), json!("a")),
            (format!("\u{feff}{json}"), json!("a")),
            (toml.to_string(), json!("a")),
            (format!("\u{feff}{toml}"), json!("a")),
        ] {
            fs::write(&path, &text).unwrap();
            let store = ConfigStore::load(&path).unwrap_or_else(|err| panic!("{text:?}: {err:#}"));
            assert_eq!(store.get_key(None, "wxid"), wxid, "{text:?}");
        }
        for empty in ["", "\n", "  \r\n\t"] {
            fs::write(&path, empty).unwrap();
            let store = ConfigStore::load(&path).unwrap();
            assert_eq!(store.current_profile, "default", "{empty:?}");
            assert!(store.profiles.contains_key("default"), "{empty:?}");
        }
    }

    #[test]
    fn a_key_spelt_almost_as_a_known_one_is_named() {
        for (typed, known) in [
            ("decrypt_ky", "decrypt_key"),
            ("dbpath", "db_path"),
            ("work.dbpath", "db_path"),
            ("serve_prot", "serve_port"),
            ("decrpyt_key", "decrypt_key"),
            ("wixd", "wxid"),
            ("progress_delay_second", "progress_delay_seconds"),
            ("decryptKey", "decrypt_key"),
            // the settings `config set --help` lists since #1149
            ("aiFootprintEnabled", "ai_footprint_enabled"),
            ("ai_insight_silense_days", "ai_insight_silence_days"),
        ] {
            assert_eq!(near_known_key(typed).as_deref(), Some(known), "{typed}");
        }
        for typed in [
            "decrypt_key",
            "work.wxid",
            "ai_insight_silence_days",
            "messagePushEnabled",
            "name",
            "custom",
        ] {
            assert_eq!(near_known_key(typed), None, "{typed}");
        }
    }

    fn temp_dir(prefix: &str) -> weflow_native::fixture::TestDir {
        weflow_native::fixture::TestDir::new(prefix)
    }
}
