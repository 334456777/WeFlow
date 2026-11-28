//! 账号及其设置：配置的档案，微信的数据目录、账号和进程，密钥，备份，以及配置文件和语言。

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Serialize;

/// 配置中的一个档案（`config profiles`）：它的账号和设置到了哪一步，不含密钥。
///
/// 密钥只说明设置了没有，只有空白的值算作没有设置。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    /// 档案的名字，即 `config set current_profile` 接受的写法。
    pub name: String,
    /// 是否为当前档案（`current_profile`），命令读取的就是它。
    pub current: bool,
    /// 账号的 wxid（`wxid`）；没有设置时为 `null`。
    pub wxid: Option<String>,
    /// 微信的数据目录（`db_path`）；没有设置时为 `null`。
    pub db_path: Option<String>,
    /// 是否设置了数据库密钥。
    pub decrypt_key: bool,
    /// 两个图片密钥是否都已设置。
    pub image_keys: bool,
    /// 是否设置了 AI 模型接口的密钥。
    pub ai_key: bool,
    /// 能否打开数据库，即数据目录和数据库密钥都已设置。
    pub complete: bool,
}

/// 微信数据目录的默认位置（`db detect`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbCandidates {
    /// 本系统上微信默认存放数据目录的位置，不论是否存在。
    pub candidates: Vec<DbCandidate>,
}

/// 微信数据目录可能在的一个位置。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbCandidate {
    /// 目录，主目录已展开。
    pub path: PathBuf,
    /// 是否存在。
    pub exists: bool,
}

/// 微信数据目录中的账号（`db wxid`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbAccounts {
    /// 其中找到的账号目录。
    pub accounts: Vec<DbAccount>,
}

/// 微信数据目录中的一个账号。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbAccount {
    /// 写成 `config set wxid` 接受的形式，即去掉账号目录名的后缀。
    pub wxid: String,
    /// 账号目录。
    pub path: PathBuf,
}

/// 数据库能用配置的密钥打开（`db test`）；打不开时回答错误。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DbConnection {
    /// 打开了其中数据库的账号目录。
    pub account_dir: PathBuf,
}

/// `db scan` 在一个文件夹下找到的账号，即含 `db_storage` 文件夹的子文件夹。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbScan {
    /// 扫描的文件夹，主目录已展开。
    pub root: PathBuf,
    /// 其中是账号目录的子文件夹；文件夹读不了时为空。
    pub accounts: Vec<ScannedAccount>,
}

/// `db scan` 找到的一个账号目录。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ScannedAccount {
    /// 文件夹的名字，没有去掉后缀。
    pub wxid: String,
    /// 文件夹。
    pub path: PathBuf,
}

/// `key db` 从微信读到的数据库密钥。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DbKey {
    /// 数据库密钥；带 `save` 时写进了档案，这里省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decrypt_key: Option<String>,
    /// 总是 `wx_key`，即用 weflow 自带的实现挂钩微信。
    pub method: String,
    /// 挂钩的微信进程的 ID。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// 带 `save` 时写入的档案键；不带时省略。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub saved: Vec<String>,
}

/// `key image` 从 `kvcomm` 缓存推导、并用账号的图片校验过的图片密钥。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ImageKeys {
    /// XOR 密钥；带 `save` 时写进了档案，这里省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_xor_key: Option<u8>,
    /// AES 密钥，16 个十六进制字符；带 `save` 时写进了档案，这里省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_aes_key: Option<String>,
    /// 两个密钥是否都校验过：AES 密钥能把缩略图的文件头解密成图片，并且有一张缩略图用这对密钥解码成完整的 JPEG；
    /// 为 `false` 时只校验了 AES 密钥。
    pub verified: bool,
    /// 是否有一张缩略图用这对密钥解码成完整的 JPEG，与 `verified` 相同。
    pub xor_verified: bool,
    /// 校验的方式：`jpeg_decode_and_xor_tail` 或 `aes_header_only`。
    pub verification: String,
    /// 推导出密钥的 code 来自哪些 `kvcomm` 文件。
    pub sources: Vec<crate::image_keys::CandidateSource>,
    /// 读取 `kvcomm` 文件夹的情况：能列出的文件夹数 `readable_directories`、候选 code 的个数 `candidate_codes`，
    /// 以及读不了的路径 `errors`。
    pub collection: serde_json::Value,
    /// 扫描账号图片的情况：查看的目录项数 `entries_scanned`、可用的缩略图数 `valid_templates` 等。
    pub scan: serde_json::Value,
    /// 带 `save` 时写入的档案键；不带时省略。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub saved: Vec<String>,
}

/// `key image-mem`（Linux x64 和 macOS）的回答：在微信的内存中找到的 AES 密钥。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ImageMemKey {
    /// 16 字节的 AES 密钥，写成 32 个十六进制字符；没有找到，或带 `save` 写进了档案时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_aes_key: Option<String>,
    /// 不带 `--ciphertext` 时，账号的一张缩略图与 AES 密钥一起解码所用的 XOR 密钥；
    /// 没有一张缩略图的 XOR 尾部通过校验，或带 `save` 写进了档案时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_xor_key: Option<u8>,
    /// 找到 AES 密钥时确认它的方式：`jpeg_decode_and_xor_tail`（账号的一张缩略图用这对密钥解码成完整的 JPEG）、
    /// `sample_first_blocks`（两张以上缩略图的第一个块解密成图片文件头）或 `first_block`（只检查了给出的密文）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
    /// 读取了内存的微信进程的 ID。
    pub pid: u32,
    /// 带 `save` 时写入的档案键；不带时省略。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub saved: Vec<String>,
}

/// `key processes`（macOS）的回答：正在运行的微信客户端，主程序在前。
///
/// 不含 helper、`WeChatAppEx` 和 crashpad 进程。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WeChatProcesses {
    /// 这些进程；微信没有运行时为空。
    pub processes: Vec<WeChatProcess>,
}

/// 一个正在运行的微信客户端。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WeChatProcess {
    /// 进程 ID。
    pub pid: u32,
    /// 进程名。
    pub name: String,
    /// 可执行文件的路径。
    pub path: PathBuf,
    /// 是否为应用包的主程序（`…/Contents/MacOS/WeChat`）。
    pub main: bool,
}

/// `backup create` 写出的备份。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackupCreated {
    /// 写出的备份文件：`out` 给出的 `.zip` 文件，或者写在 `out` 给出的文件夹中的
    /// `weflow-backup-YYYYMMDD-HHMMSS.zip`。
    pub out: PathBuf,
    /// 其中的文件数。
    pub entries: usize,
    /// 备份的账号，即账号目录的名字（带后缀）。
    pub wxid: String,
    /// 创建备份的时间，Unix 秒。
    pub created_at: u64,
    /// 读不了、因而不在备份中的媒体和表情文件；没有时省略。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<crate::backup::SkippedFile>,
    /// 复制期间微信仍在写入的数据库，备份中它们的主文件和 WAL 可能来自两个时刻；没有时省略。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unsettled: Vec<String>,
}

/// `backup restore` 把文件放到了哪里。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackupRestored {
    /// 恢复到的文件夹。
    pub restored_to: PathBuf,
    /// WeFlow 的表情缓存，备份中的表情放回了这里；备份中没有表情时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stickers_restored_to: Option<PathBuf>,
}

/// `config` 的修改写入的配置文件。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfigWritten {
    /// 写入的配置文件；`config set config_path` 和 `config unset config_path` 时为此后使用的配置文件，
    /// 后者找不到默认位置时为 `null`。
    pub config_path: Option<PathBuf>,
    /// `config set` 的键 WeFlow 不读取、但与一个读取的键只差一点（如 `decrypt_ky`）时，为那个读取的键；
    /// 键仍按输入的写法保存。其他情况省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub did_you_mean: Option<String>,
}

/// `weflow lang` 保存的语言：`en` 或 `zh`。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct LangSaved {
    /// `en` 或 `zh`。
    pub lang: String,
}
