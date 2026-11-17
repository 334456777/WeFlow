//! The account and its setup: profiles, where `weflow` keeps its files, WeChat's data directory and keys,
//! backups and the configuration.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    /// Always `wx_key`: WeChat hooked by the binary's own implementation.
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

/// What `key image-local` (Linux) answers: the accounts and keys read from local `kvcomm` files.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct ImageLocalKeys {
    pub accounts: Vec<ImageLocalAccount>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct ImageLocalAccount {
    pub wxid: String,
    pub keys: Vec<ImageLocalKey>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageLocalKey {
    pub code: u32,
    pub xor_key: u8,
    pub aes_key: String,
}

/// What `key image-mem` (Linux x64 and macOS) and `key scan-image` (macOS)
/// answer: the AES key found in WeChat's memory, if any.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ImageMemKey {
    /// The 16-byte AES key as 32 hex characters; left out when nothing matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_aes_key: Option<String>,
    /// `key image-mem` without a ciphertext: the XOR key a thumbnail of the account decodes with together with the
    /// AES key; left out when no thumbnail has an XOR tail that verifies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_xor_key: Option<u8>,
    /// How the AES key was confirmed, when one was found: `jpeg_decode_and_xor_tail` (a thumbnail of the account
    /// decodes as a whole JPEG with both keys), `sample_first_blocks` (the first blocks of two or more thumbnails
    /// decrypt into image headers) or `first_block` (only the given ciphertext was checked).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
    pub pid: u32,
    /// With `save`: the profile keys written.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub saved: Vec<String>,
}

/// What `key processes` (macOS) answers: the running WeChat clients, the main
/// application first. Helper, AppEx and crashpad processes are left out.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WeChatProcesses {
    pub processes: Vec<WeChatProcess>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WeChatProcess {
    pub pid: u32,
    pub name: String,
    pub path: PathBuf,
    /// Whether it is the main client rather than a helper.
    pub main: bool,
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
    /// Files of the media and stickers that could not be read and are not in it; left out when there are none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<crate::backup::SkippedFile>,
    /// Databases WeChat kept writing to while they were copied, whose main file and WAL in the backup may be of two
    /// moments; left out when there are none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unsettled: Vec<String>,
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
    /// For `config set` of a key WeFlow does not read but spelt almost as one it does (`decrypt_ky`), that one:
    /// the key was saved as it was typed all the same. Left out otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub did_you_mean: Option<String>,
}

/// The language `weflow lang` saved: `en` or `zh`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct LangSaved {
    pub lang: String,
}
