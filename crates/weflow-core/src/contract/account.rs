//! The account and its setup: profiles, where `weflow` keeps its files, WeChat's data directory and keys,
//! backups and the configuration.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Serialize;

/// A profile of the configuration (`config profiles`): its account and how far it is set up, without its keys.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub name: String,
    /// Whether the commands read it.
    pub current: bool,
    pub wxid: Option<String>,
    /// The WeChat data directory.
    pub db_path: Option<String>,
    /// Whether the database key is set.
    pub decrypt_key: bool,
    /// Whether both image keys are set.
    pub image_keys: bool,
    /// Whether the key of the AI model API is set.
    pub ai_key: bool,
    /// Whether it can open the databases: the data directory and the database key are set.
    pub complete: bool,
}

/// Where `weflow` keeps its files and what it is (`runtime info`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub home_dir: std::path::PathBuf,
    pub config_path: std::path::PathBuf,
    pub runtime_dir: std::path::PathBuf,
    pub version: String,
    /// The target it was built for, such as `x86_64-pc-windows-msvc`.
    pub target: String,
}

/// The default places of WeChat's data directory (`db detect`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbCandidates {
    pub candidates: Vec<DbCandidate>,
}

/// A place WeChat's data directory may be.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbCandidate {
    pub path: std::path::PathBuf,
    pub exists: bool,
}

/// The accounts in WeChat's data directory (`db wxid`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbAccounts {
    pub accounts: Vec<DbAccount>,
}

/// An account of WeChat's data directory.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbAccount {
    /// As `config set wxid` takes it, without the suffix of its folder's name.
    pub wxid: String,
    /// Its folder.
    pub path: std::path::PathBuf,
}

/// That the databases open with the configured key (`db test`); a failure is an error instead.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DbConnection {
    pub account_dir: std::path::PathBuf,
    /// Always true.
    pub connected: bool,
}

/// The accounts `db scan` found under a folder: those with a `db_storage` folder.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbScan {
    pub root: PathBuf,
    pub accounts: Vec<ScannedAccount>,
}

/// An account folder `db scan` found.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ScannedAccount {
    /// The folder's name.
    pub wxid: String,
    pub path: PathBuf,
}

/// The database key `key db` read from WeChat.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbKey {
    /// Left out with `save`: it went to the profile instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decrypt_key: Option<String>,
    /// `wx_key` (WeChat hooked) or `key_helper`.
    pub method: String,
    /// The WeChat process hooked (`wx_key`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// With `save`: the profile keys written.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub saved: Vec<String>,
}

/// The image keys `key image` derived from the `kvcomm` cache and checked against the account's images.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ImageKeys {
    /// Left out with `save`, as `image_aes_key`: they went to the profile instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_xor_key: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_aes_key: Option<String>,
    /// Both keys were checked: the AES key on an image header and the XOR key on a whole image.
    pub verified: bool,
    pub aes_verified: bool,
    pub xor_verified: bool,
    /// How: `jpeg_decode_and_xor_tail` or `aes_header_only`.
    pub verification: String,
    /// The `kvcomm` files the keys came from.
    pub sources: Vec<crate::image_keys::CandidateSource>,
    /// What was read of the `kvcomm` folders.
    pub collection: serde_json::Value,
    /// What was read of the account's images.
    pub scan: serde_json::Value,
    /// With `save`: the profile keys written.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub saved: Vec<String>,
}

/// What `key scan-image` (macOS) answers: the helper's own result.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScannedImageKeys {
    /// The helper's answer; `{"raw": …}` when it is not JSON.
    pub result: serde_json::Value,
    pub method: String,
    pub user_dir: String,
}

/// The backup `backup create` wrote.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackupCreated {
    pub out: PathBuf,
    /// The files in it.
    pub entries: usize,
    pub wxid: String,
    /// Unix seconds.
    pub created_at: u64,
}

/// Where `backup restore` put the files.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackupRestored {
    pub restored_to: PathBuf,
    /// WeFlow's sticker cache, where the stickers of the backup went; left out when it held none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stickers_restored_to: Option<PathBuf>,
}

/// The config file a `config` change was written to.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfigWritten {
    /// Null when `config unset config_path` finds no default place for it.
    pub config_path: Option<PathBuf>,
}

/// The language `weflow lang` saved: `en` or `zh`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct LangSaved {
    pub lang: String,
}
