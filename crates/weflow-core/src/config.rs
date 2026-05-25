use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConfigStore {
    pub current_profile: String,
    pub profiles: BTreeMap<String, ProfileConfig>,
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileConfig {
    pub db_path: Option<String>,
    pub decrypt_key: Option<String>,
    pub wxid: Option<String>,
    pub image_xor_key: Option<i64>,
    pub image_aes_key: Option<String>,
    pub cache_path: Option<String>,
    pub log_enabled: bool,
    pub http_api_token: Option<String>,
    pub http_api_host: Option<String>,
    pub http_api_port: Option<u16>,
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

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(path, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("failed to write {}", path.display()))
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
                target.image_xor_key = Some(as_i64(value)?);
            }
            "image_aes_key" | "imageAesKey" => target.image_aes_key = Some(as_string(value)?),
            "cache_path" | "cachePath" => target.cache_path = Some(as_string(value)?),
            "log_enabled" | "logEnabled" => target.log_enabled = as_bool(value)?,
            "http_api_token" | "httpApiToken" => target.http_api_token = Some(as_string(value)?),
            "http_api_host" | "httpApiHost" => target.http_api_host = Some(as_string(value)?),
            "http_api_port" | "httpApiPort" => target.http_api_port = Some(as_i64(value)? as u16),
            "ai_model_api_base_url" | "aiModelApiBaseUrl" => target.ai_model_api_base_url = Some(as_string(value)?),
            "ai_model_api_key" | "aiModelApiKey" => target.ai_model_api_key = Some(as_string(value)?),
            "ai_model_api_model" | "aiModelApiModel" => target.ai_model_api_model = Some(as_string(value)?),
            "ai_model_api_max_tokens" | "aiModelApiMaxTokens" => target.ai_model_api_max_tokens = Some(as_i64(value)? as u32),
            "ai_insight_enabled" | "aiInsightEnabled" => target.ai_insight_enabled = Some(as_bool(value)?),
            other => {
                target.extra.insert(other.to_string(), value);
            }
        }
        Ok(())
    }

    pub fn unset_key(&mut self, profile: Option<&str>, key: &str) {
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
            "http_api_host" | "httpApiHost" => target.http_api_host = None,
            "http_api_port" | "httpApiPort" => target.http_api_port = None,
            "ai_model_api_base_url" | "aiModelApiBaseUrl" => target.ai_model_api_base_url = None,
            "ai_model_api_key" | "aiModelApiKey" => target.ai_model_api_key = None,
            "ai_model_api_model" | "aiModelApiModel" => target.ai_model_api_model = None,
            "ai_model_api_max_tokens" | "aiModelApiMaxTokens" => target.ai_model_api_max_tokens = None,
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

    pub fn import_electron_config(
        &mut self,
        path: &Path,
        profile: Option<&str>,
    ) -> Result<Vec<String>> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let value: Value = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let mut skipped = Vec::new();
        let Some(map) = value.as_object() else {
            return Ok(skipped);
        };
        for (source, dest) in [
            ("dbPath", "db_path"),
            ("decryptKey", "decrypt_key"),
            ("myWxid", "wxid"),
            ("imageXorKey", "image_xor_key"),
            ("imageAesKey", "image_aes_key"),
            ("cachePath", "cache_path"),
            ("logEnabled", "log_enabled"),
            ("httpApiToken", "http_api_token"),
            ("httpApiHost", "http_api_host"),
            ("httpApiPort", "http_api_port"),
            ("aiModelApiBaseUrl", "ai_model_api_base_url"),
            ("aiModelApiKey", "ai_model_api_key"),
            ("aiModelApiModel", "ai_model_api_model"),
            ("aiModelApiMaxTokens", "ai_model_api_max_tokens"),
            ("aiInsightEnabled", "ai_insight_enabled"),
        ] {
            if let Some(v) = map.get(source).cloned() {
                if is_encrypted_value(&v) {
                    skipped.push(source.to_string());
                    continue;
                }
                let _ = self.set_key(profile, dest, v);
            }
        }
        Ok(skipped)
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
            "log_enabled" | "logEnabled" => Value::Bool(self.log_enabled),
            "http_api_token" | "httpApiToken" => self
                .http_api_token
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "http_api_host" | "httpApiHost" => self
                .http_api_host
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "http_api_port" | "httpApiPort" => self
                .http_api_port
                .map(|value| Value::Number(value.into()))
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

pub fn old_electron_config_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = dirs::config_dir() {
        candidates.push(dir.join("WeFlow").join("config.json"));
        candidates.push(dir.join("weflow").join("config.json"));
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("Library/Application Support/WeFlow/config.json"));
    }
    candidates
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

fn is_account_dir(path: &Path) -> bool {
    path.join("db_storage").exists()
        || path.join("FileStorage/Image").exists()
        || path.join("FileStorage/Image2").exists()
}

fn is_encrypted_value(value: &Value) -> bool {
    value
        .as_str()
        .map(|s| s.starts_with("safe:") || s.starts_with("lock:"))
        .unwrap_or(false)
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
            | "log_enabled"
            | "logEnabled"
            | "http_api_token"
            | "httpApiToken"
            | "http_api_host"
            | "httpApiHost"
            | "http_api_port"
            | "httpApiPort"
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
    fn imports_plain_electron_config_and_skips_encrypted_values() {
        let dir = temp_dir("weflow-config-import");
        let source = dir.join("config.json");
        fs::write(
            &source,
            serde_json::to_vec(&json!({
                "dbPath": "/tmp/wechat",
                "decryptKey": "safe:encrypted",
                "myWxid": "wxid_test",
                "imageXorKey": 123,
                "logEnabled": true
            }))
            .unwrap(),
        )
        .unwrap();

        let mut store = ConfigStore::default();
        let skipped = store
            .import_electron_config(&source, Some("migrated"))
            .unwrap();
        let profile = store.profile(Some("migrated")).unwrap();

        assert_eq!(profile.db_path.as_deref(), Some("/tmp/wechat"));
        assert_eq!(profile.decrypt_key, None);
        assert_eq!(profile.wxid.as_deref(), Some("wxid_test"));
        assert_eq!(profile.image_xor_key, Some(123));
        assert!(profile.log_enabled);
        assert_eq!(skipped, vec!["decryptKey"]);
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
