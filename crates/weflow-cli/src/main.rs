use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};

mod i18n;
use serde_json::{json, Value};
use tracing_subscriber::EnvFilter;
use weflow_core::config::{old_electron_config_candidates, AppContext, ConfigStore};
use weflow_core::error::{AppError, AppResult};
use weflow_core::output::{failure, success};
use weflow_core::services::ServiceHub;

/// Exports read rows on one thread and free them on another; with the system allocator (glibc in particular) that
/// hand-over costs more than the parsing it runs next to. See issue #14.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

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
        unsafe { mi_reserve_os_memory(RESERVED_ARENA, true, true) };
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(name = "weflow", version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("WEFLOW_BUILD_INFO"), ")"), about = "Native CLI for WeFlow")]
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
    /// Install and locate the ffmpeg that converts WXGF images
    Ffmpeg(FfmpegCommand),
    /// Save the output language
    Lang {
        /// Output language
        #[arg(value_enum)]
        lang: LangArg,
    },
}

#[derive(Args, Debug)]
struct FfmpegCommand {
    #[command(subcommand)]
    command: FfmpegSubcommand,
}

#[derive(Subcommand, Debug)]
enum FfmpegSubcommand {
    /// Download the ffmpeg build the desktop app bundles (ffmpeg-static b6.1.1), check its SHA-256 and unpack it into WeFlow's folder; used when FFMPEG_PATH is unset and no ffmpeg is on PATH
    Install {
        /// Download it again even when the installed copy is intact
        #[arg(long)]
        force: bool,
    },
    /// Show the ffmpeg that WXGF images use and where it was found (FFMPEG_PATH, PATH, installed or missing)
    Path,
    /// Change a setting of the ffmpeg download
    Set {
        #[command(subcommand)]
        setting: FfmpegSetting,
    },
    /// Go back to the default of a setting of the ffmpeg download
    Unset {
        #[command(subcommand)]
        setting: FfmpegUnsetting,
    },
}

#[derive(Subcommand, Debug)]
enum FfmpegSetting {
    /// Download from a mirror of the ffmpeg-static releases instead of GitHub, for example https://registry.npmmirror.com/-/binary/ffmpeg-static (the files are checked either way)
    Baseurl {
        /// Address the release folder `b6.1.1` is found under (http:// or https://)
        url: String,
    },
}

#[derive(Subcommand, Debug)]
enum FfmpegUnsetting {
    /// Download from GitHub again
    Baseurl,
}

#[derive(Args, Debug)]
struct CacheCommand {
    #[command(subcommand)]
    command: CacheSubcommand,
}

#[derive(Subcommand, Debug)]
enum CacheSubcommand {
    /// Clear every cache: analytics, decrypted images, and the in-memory Moments / group caches
    ClearAll,
}

#[derive(Args, Debug)]
struct ConfigCommand {
    #[command(subcommand)]
    command: ConfigSubcommand,
}

#[derive(Subcommand, Debug)]
enum ConfigSubcommand {
    /// Show the path of the config file
    Path,
    /// Show every key of the active profile
    List,
    /// Show one key (or all when omitted)
    Get {
        /// Key name (default: show every key)
        key: Option<String>,
    },
    /// Set a key
    Set {
        #[arg(
            help = "Key name; `<profile>.<key>` (for example `work.wxid`) sets a key of another profile\n  db_path                  WeChat data directory (`db detect` finds it)\n  wxid                     Account wxid (`db wxid` prints it)\n  decrypt_key              Database key in hex (`key db` prints it)\n  image_xor_key            Image XOR key, a number (`key image` prints it)\n  image_aes_key            Image AES key (`key image` prints it)\n  cache_path               Folder for cached images, voices, stickers and Moments (default: `cache` next to the config file)\n  http_api_token           Access token of the HTTP API (`serve`)\n  http_api_host            Listen address of `serve` (default 127.0.0.1)\n  http_api_port            Listen port of `serve` (default 5031)\n  ai_model_api_base_url    Base URL of the AI model API used by `insight`\n  ai_model_api_key         API key of the AI model API\n  ai_model_api_model       Model name\n  ai_model_api_max_tokens  Most tokens in one AI reply\n  ai_insight_enabled       true or false: turn AI insight on or off\n  log_enabled              true or false; kept for the desktop app, the CLI does not read it\n  no_progress              true or false: true hides the automatic progress bar\n  progress_delay_seconds   Seconds a command runs before the progress bar appears (default 5; 0 = always)\nSettings of the config file itself (stored in that file, not in a profile; each config file has its own):\n  lang                     Output language, en or zh (the same as `weflow lang`)\n  current_profile          Active profile name, created when it does not exist (default: default)\nWhich config file is used (kept next to the default location, not in any config file):\n  config_path              Config file used from now on; `config unset config_path` or the default path switches back"
        )]
        key: String,
        /// Value to store
        value: String,
    },
    /// Remove a key
    Unset {
        /// Key name
        key: String,
    },
    /// Remove every key of the active profile
    Clear,
    /// Import settings from the desktop app's config.json
    Import {
        /// Desktop app config.json (default: found in the usual locations)
        path: Option<PathBuf>,
    },
}

#[derive(Args, Debug)]
struct DbCommand {
    #[command(subcommand)]
    command: DbSubcommand,
}

#[derive(Subcommand, Debug)]
enum DbSubcommand {
    /// Find WeChat data directories on this computer
    Detect,
    /// Scan a folder for WeChat account directories
    Scan {
        /// Folder to scan
        root: String,
    },
    /// Test that the configured database can be opened with the key
    Test,
    /// Open the configured database and show a short summary
    Open,
    /// Show the wxid of the account(s) in the WeChat data directory (the value for `config set wxid`)
    Wxid {
        /// WeChat data directory (default: `db_path` from the config, else the usual locations)
        root: Option<String>,
    },
}

#[derive(Args, Debug)]
struct KeyCommand {
    #[command(subcommand)]
    command: KeySubcommand,
}

#[derive(Subcommand, Debug)]
enum KeySubcommand {
    /// Hook WeChat and wait for the database key (Windows: keep the command running and log in to WeChat)
    Db {
        /// WeChat process id (default: the first Weixin.exe / WeChat.exe found)
        #[arg(long)]
        pid: Option<u32>,
        /// How long to wait for the key, in seconds
        #[arg(long, default_value_t = 180)]
        timeout: u64,
    },
    /// Derive the image keys from WeChat's kvcomm cache (verified against a .dat template)
    Image {
        /// Account directory to search for templates (default: the configured account directory)
        #[arg(long)]
        user_dir: Option<String>,
    },
    /// Scan WeChat's memory for the image AES key (macOS)
    ScanImage {
        /// Account directory (user directory) to scan for the image key
        user_dir: String,
    },
}

#[derive(Args, Debug)]
struct ChatCommand {
    #[command(subcommand)]
    command: ChatSubcommand,
}

#[derive(Subcommand, Debug)]
enum ChatSubcommand {
    /// Remove WeFlow's data of the current account: its caches (and the account settings of this profile) with
    /// --cache, and entries named after the account in the given export folders. WeChat's own files are never touched.
    ClearAccountData {
        /// Remove the cached images, voices, stickers, Moments and analytics of the account, then reset db_path,
        /// wxid, decrypt_key and the image keys of the profile
        #[arg(long)]
        cache: bool,
        /// Folder of exports; files and folders named after the account are removed (repeatable)
        #[arg(long = "exports-dir")]
        exports_dir: Vec<PathBuf>,
        /// Confirm the removal
        #[arg(long)]
        yes: bool,
    },
    /// List conversations, newest first
    Sessions {
        /// Maximum number of sessions (0 = all)
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },
    /// A page of a conversation's messages
    Messages(PageArgs),
    /// The latest messages of a conversation
    Latest {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Number of messages
        #[arg(long, default_value_t = 20)]
        limit: i32,
    },
    /// Search the messages of one conversation or of all of them
    Search {
        /// Text to search for
        keyword: String,
        /// Only search this conversation (default: all conversations)
        #[arg(long)]
        session_id: Option<String>,
        /// Maximum number of results
        #[arg(long, default_value_t = 50)]
        limit: i32,
        /// Number of results to skip
        #[arg(long, default_value_t = 0)]
        offset: i32,
        /// Only messages from this time on (Unix seconds, 0 = no limit)
        #[arg(long, default_value_t = 0)]
        start: i32,
        /// Only messages up to this time (Unix seconds, 0 = no limit)
        #[arg(long, default_value_t = 0)]
        end: i32,
    },
    /// List all contacts
    Contacts,
    /// Show one contact
    Contact {
        /// Contact username (wxid)
        username: String,
    },
    /// Edit a message (always refused: the database is read-only)
    UpdateMessage {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        local_id: i64,
        /// Message create time (seconds)
        create_time: i32,
        /// New message content
        content: String,
    },
    /// Delete a message (always refused: the database is read-only)
    DeleteMessage {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        local_id: i64,
        /// Message create time (seconds)
        create_time: i32,
        /// Database file the message is stored in
        #[arg(long)]
        db_path_hint: Option<String>,
    },
    /// Anti-revoke triggers (installing and removing are refused: the database is read-only)
    AntiRevoke {
        #[command(subcommand)]
        command: AntiRevokeSubcommand,
    },
    /// Look up one message by local id or server id
    Message {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        #[arg(long)]
        local_id: Option<i32>,
        /// Server message id
        #[arg(long)]
        server_id: Option<String>,
    },
    /// Dates that have messages in a session (YYYY-MM-DD)
    Dates {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
    },
    /// Message count per day for a session
    DateCounts {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
    },
    /// Total message counts for several sessions
    Counts {
        /// Conversation ids
        sessions: Vec<String>,
    },
    /// Folded / muted state of sessions
    Statuses {
        /// Conversation usernames
        usernames: Vec<String>,
    },
    /// Session details (contact info, message count, message tables, first/latest time)
    Detail {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Only the fast part (contact info + message count)
        #[arg(long)]
        fast: bool,
        /// Only the extra part (tables, first/latest time)
        #[arg(long)]
        extra: bool,
    },
    /// Mark all sessions as read
    MarkRead,
    /// Contact counts per tab (friends, groups, official accounts, former friends)
    TabCounts,
    /// Per-session statistics used by the export page
    ExportStats {
        /// Conversation ids
        sessions: Vec<String>,
        /// Start date, local time (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
        /// Skip mutual group / friend relations
        #[arg(long)]
        no_relations: bool,
    },
    /// Read or set the cached "my message count" of a group chat
    GroupHint {
        /// Group chat id (xxx@chatroom)
        chatroom_id: String,
        /// Store this value as the "my message count"
        #[arg(long)]
        set: Option<i64>,
    },
    /// List image / video / voice / file messages across sessions
    Resources {
        /// Only this conversation (default: all conversations)
        #[arg(long)]
        session: Option<String>,
        /// image, video, voice, file (repeatable)
        #[arg(long = "type")]
        types: Vec<String>,
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
        /// Maximum number of results
        #[arg(long, default_value_t = 300)]
        limit: usize,
        /// Number of results to skip
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// All image identifiers of a session (md5 / dat name), newest first
    Images {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
    },
    /// All voice messages of a session
    VoiceMessages {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
    },
    /// Page through image/video messages with the native media scanner
    MediaStream {
        /// Only this conversation (default: all conversations)
        #[arg(long)]
        session: Option<String>,
        /// image, video or all
        #[arg(long, default_value = "all")]
        media_type: String,
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
        /// Maximum number of results
        #[arg(long, default_value_t = 200)]
        limit: i32,
        /// Number of results to skip
        #[arg(long, default_value_t = 0)]
        offset: i32,
    },
    /// Resolve payer / receiver display names of a transfer message
    TransferNames {
        /// Group chat id (xxx@chatroom)
        chatroom_id: String,
        /// Payer username (wxid)
        payer: String,
        /// Receiver username (wxid)
        receiver: String,
    },
    /// Export all voice messages of a conversation as WAV files
    Voice {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Output directory (default: the current directory)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Decode one voice message (SILK) into a 24 kHz WAV file
    VoiceData {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        msg_id: String,
        /// Message create time (seconds)
        #[arg(long)]
        create_time: Option<i64>,
        /// Server message id
        #[arg(long)]
        server_id: Option<String>,
        /// Sender wxid (important in group chats)
        #[arg(long)]
        sender: Option<String>,
        /// Output WAV path (default: print base64 in the JSON result)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Decrypt the image of one message and write it to a file
    ImageData {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        msg_id: String,
        /// Output file
        #[arg(long)]
        out: PathBuf,
    },
    /// Check whether a decoded voice WAV is already cached for a message id
    VoiceCache {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        msg_id: String,
    },
    /// Decode and cache many voice messages; takes a JSON array of {localId, createTime, serverId?, senderWxid?}
    VoicePreload {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// JSON array of {localId, createTime, serverId?, senderWxid?}
        messages_json: String,
    },
    /// Download the stickers of a conversation
    Emoji {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Output directory
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Args, Debug)]
struct PageArgs {
    /// Conversation id: the other party's wxid, or xxx@chatroom for a group
    session_id: String,
    /// Maximum number of results
    #[arg(long, default_value_t = 50)]
    limit: i32,
    /// Number of results to skip
    #[arg(long, default_value_t = 0)]
    offset: i32,
}

#[derive(Subcommand, Debug)]
enum AntiRevokeSubcommand {
    /// Sessions that anti-revoke can be installed for
    Sessions,
    /// Check whether the anti-revoke trigger is installed for sessions
    Check {
        /// Conversation ids
        sessions: Vec<String>,
    },
    /// Install the anti-revoke trigger for sessions (refused: the database is read-only)
    Install {
        /// Conversation ids
        sessions: Vec<String>,
    },
    /// Remove the anti-revoke trigger from sessions (refused: the database is read-only)
    Uninstall {
        /// Conversation ids
        sessions: Vec<String>,
    },
}

#[derive(Args, Debug)]
struct ExportCommand {
    #[command(subcommand)]
    command: ExportSubcommand,
}

#[derive(Subcommand, Debug)]
enum ExportSubcommand {
    /// Export the session list
    Sessions {
        /// Only these conversations (repeatable; default: all)
        #[arg(long = "session")]
        sessions: Vec<String>,
        /// Export format (default: json)
        #[arg(long)]
        format: Option<String>,
        /// Output file
        #[arg(long)]
        out: PathBuf,
    },
    /// Export the contact list
    Contacts {
        /// Export format (default: json)
        #[arg(long)]
        format: Option<String>,
        /// Output file
        #[arg(long)]
        out: PathBuf,
    },
    /// Export the footprint statistics
    Footprint {
        /// Export format (default: json)
        #[arg(long)]
        format: Option<String>,
        /// Output file
        #[arg(long)]
        out: PathBuf,
    },
    /// Export the images, voice (WAV), videos and stickers of messages
    Media {
        /// Output directory
        #[arg(long)]
        out: PathBuf,
        /// Only media of this conversation id (default: all conversations)
        #[arg(long)]
        session: Option<String>,
        /// What to export: image, voice, video, emoji or all
        #[arg(long, default_value = "all")]
        r#type: String,
        /// Only messages from this date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// Only messages up to this date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// Export the messages of one conversation
    Messages {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
        /// Output file
        #[arg(long)]
        out: PathBuf,
        /// txt (default), json, arkme-json, chatlab, chatlab-jsonl, excel, weclone, html, sql
        #[arg(long, default_value = "txt")]
        format: String,
        /// Only export messages sent by this wxid
        #[arg(long)]
        sender: Option<String>,
        /// How senders are named: group-nickname, remark or nickname (default: group-nickname, i.e. group nickname, then remark, nickname, wxid)
        #[arg(long)]
        display_name: Option<String>,
        /// Excel: compact columns (time, sender, type, content)
        #[arg(long)]
        excel_compact: bool,
        /// Copy media next to the export and point the messages at the copies: any of image, voice, video, emoji
        /// (comma separated) or all. Files go to `media/<output name>/` beside the output file.
        #[arg(long, value_delimiter = ',')]
        media: Vec<String>,
    },
}

#[derive(Args, Debug)]
struct AnalyticsCommand {
    #[command(subcommand)]
    command: AnalyticsSubcommand,
}

#[derive(Subcommand, Debug)]
enum AnalyticsSubcommand {
    /// Overall chat statistics (private chats, honouring the exclusion list)
    Overall {
        /// Recompute instead of using the cached aggregate
        #[arg(long)]
        force: bool,
    },
    /// Contacts ranked by message count
    Rankings {
        /// Number of contacts
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// First day (YYYY-MM-DD, local time)
        #[arg(long)]
        start: Option<String>,
        /// Last day, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// Hour / weekday / month distribution
    Time,
    /// Show the exclusion list, or replace it with --set
    Excluded {
        /// Comma-separated usernames that replace the list (pass "" to clear it)
        #[arg(long = "set", value_delimiter = ',', num_args = 0..)]
        set: Option<Vec<String>>,
    },
    /// Private chats that can be excluded from analytics
    ExcludeCandidates,
    /// Drop the cached aggregate
    ClearCache,
}

#[derive(Args, Debug)]
struct GroupCommand {
    #[command(subcommand)]
    command: GroupSubcommand,
}

#[derive(Subcommand, Debug)]
enum GroupSubcommand {
    /// Group chats with member counts
    List,
    /// Members panel (friend flag, owner, group nickname, optional message counts)
    Members {
        /// Group chat id (xxx@chatroom)
        chatroom_id: String,
        /// Include per-member message counts
        #[arg(long)]
        counts: bool,
        /// Bypass the 10-minute panel cache
        #[arg(long)]
        refresh: bool,
    },
    /// Most active members
    Ranking {
        /// Group chat id (xxx@chatroom)
        chatroom_id: String,
        /// Number of members
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// Messages per hour of day
    Hours {
        /// Group chat id (xxx@chatroom)
        chatroom_id: String,
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// Message type mix
    Media {
        /// Group chat id (xxx@chatroom)
        chatroom_id: String,
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// One member's statistics (types, hours, common phrases and emoji)
    Member {
        /// Group chat id (xxx@chatroom)
        chatroom_id: String,
        /// Member username (wxid)
        username: String,
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// A page of one member's messages (newest first)
    MemberMessages {
        /// Group chat id (xxx@chatroom)
        chatroom_id: String,
        /// Member username (wxid)
        username: String,
        /// Number of messages
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Cursor returned as `nextCursor` by the previous page
        #[arg(long, default_value_t = 0)]
        cursor: usize,
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// Export a member's messages (.csv or .xlsx)
    ExportMemberMessages {
        /// Group chat id (xxx@chatroom)
        chatroom_id: String,
        /// Member username (wxid)
        username: String,
        /// Output file
        out: PathBuf,
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// Export the member list (.csv or .xlsx)
    ExportMembers {
        /// Group chat id (xxx@chatroom)
        chatroom_id: String,
        /// Output file
        out: PathBuf,
    },
}

#[derive(Args, Debug)]
struct ReportCommand {
    #[command(subcommand)]
    command: ReportSubcommand,
}

#[derive(Subcommand, Debug)]
enum ReportSubcommand {
    /// Annual report
    Annual {
        #[command(subcommand)]
        command: AnnualSubcommand,
    },
    /// Dual-person report
    Dual {
        #[command(subcommand)]
        command: DualSubcommand,
    },
}

#[derive(Subcommand, Debug)]
enum AnnualSubcommand {
    /// Years that have data for the annual report
    Years,
    /// Generate the annual report
    Generate {
        /// Report year; 0 (default) covers all years.
        #[arg(long, default_value_t = 0)]
        year: i32,
    },
}

#[derive(Subcommand, Debug)]
enum DualSubcommand {
    /// Generate the report for you and one friend
    Generate {
        /// Friend's username (wxid)
        #[arg(long)]
        friend: String,
        /// Report year; 0 (default) covers all years.
        #[arg(long, default_value_t = 0)]
        year: i32,
        /// Words to exclude from the phrase rankings (repeatable).
        #[arg(long = "exclude-word")]
        exclude_words: Vec<String>,
    },
}

#[derive(Args, Debug)]
struct SnsCommand {
    #[command(subcommand)]
    command: SnsSubcommand,
}

#[derive(Subcommand, Debug)]
enum SnsSubcommand {
    /// Moments timeline (newest first)
    Timeline {
        /// Number of posts
        #[arg(long, default_value_t = 20)]
        limit: i32,
        /// Number of posts to skip
        #[arg(long, default_value_t = 0)]
        offset: i32,
        /// Only posts of these users (repeatable)
        #[arg(long = "user")]
        users: Vec<String>,
        /// Only posts containing this text
        #[arg(long)]
        keyword: Option<String>,
        /// Only posts from this time on (Unix seconds, 0 = no limit)
        #[arg(long, default_value_t = 0)]
        start: i64,
        /// Only posts up to this time (Unix seconds, 0 = no limit)
        #[arg(long, default_value_t = 0)]
        end: i64,
        /// Download and decrypt images / videos into the cache and inline them as data URLs
        #[arg(long)]
        with_media: bool,
    },
    /// Users that have posted
    Users,
    /// Ask the server for the first bytes of a Moments resource and show the status and decryption headers
    DebugResource {
        /// Resource URL
        url: String,
    },
    /// Export statistics (total posts / friends / mine); --fast reads the cached counts only
    Stats {
        /// Only read the cached counts
        #[arg(long)]
        fast: bool,
    },
    /// Post counts per user, or the statistics of one user
    PostCounts {
        /// Statistics of this user only
        #[arg(long)]
        user: Option<String>,
        /// Prefer the cached counts
        #[arg(long)]
        prefer_cache: bool,
    },
    /// Export the timeline as json, html or arkmejson
    Export {
        /// Output file
        #[arg(long)]
        out: PathBuf,
        /// Export format: json, html or arkmejson
        #[arg(long, default_value = "json")]
        format: String,
        /// Only posts of these users (repeatable)
        #[arg(long = "user")]
        users: Vec<String>,
        /// Only posts containing this text
        #[arg(long)]
        keyword: Option<String>,
        /// Only posts from this time on (Unix seconds, 0 = no limit)
        #[arg(long, default_value_t = 0)]
        start: i64,
        /// Only posts up to this time (Unix seconds, 0 = no limit)
        #[arg(long, default_value_t = 0)]
        end: i64,
        /// Also save images / live photos / videos next to the export
        #[arg(long)]
        media: bool,
    },
    /// Fetch (and decrypt, with --key) a Moments image or video
    Media {
        /// Resource URL
        url: String,
        /// Decryption key
        #[arg(long)]
        key: Option<String>,
        /// Output file (default: print base64 in the result)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Download a Moments sticker (plain or AES-GCM encrypted)
    DownloadEmoji {
        /// Sticker URL
        url: String,
        /// URL of the encrypted sticker
        #[arg(long)]
        encrypt_url: Option<String>,
        /// AES key of the encrypted sticker
        #[arg(long)]
        aes_key: Option<String>,
    },
    /// Download an arbitrary image URL and decrypt it if it is a .dat payload
    DownloadImage {
        /// Image URL
        url: String,
        /// Output file (default: print base64 in the result)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Moments block-delete trigger (installing and removing are refused: the database is read-only)
    BlockDelete {
        /// check, install or uninstall
        action: TriggerAction,
    },
    /// Delete a Moments post (always refused: the database is read-only)
    Delete {
        /// Post id
        post_id: String,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TriggerAction {
    Check,
    Install,
    Uninstall,
}

#[derive(Args, Debug)]
struct BizCommand {
    #[command(subcommand)]
    command: BizSubcommand,
}

#[derive(Subcommand, Debug)]
enum BizSubcommand {
    /// List official accounts
    Accounts,
    /// Messages of one official account
    Messages {
        /// Official account username
        username: String,
        /// Maximum number of results
        #[arg(long, default_value_t = 50)]
        limit: i32,
        /// Number of results to skip
        #[arg(long, default_value_t = 0)]
        offset: i32,
    },
    /// WeChat Pay records
    PayRecords {
        /// Maximum number of results
        #[arg(long, default_value_t = 50)]
        limit: i32,
        /// Number of results to skip
        #[arg(long, default_value_t = 0)]
        offset: i32,
    },
}

#[derive(Args, Debug)]
struct ImageCommand {
    #[command(subcommand)]
    command: ImageSubcommand,
}

#[derive(Args, Debug, Clone)]
struct ImageTarget {
    /// Conversation the image belongs to (locates msg/attach/<md5(session)>/…)
    #[arg(long)]
    session: Option<String>,
    /// Image md5 from the message
    #[arg(long)]
    md5: Option<String>,
    /// Image .dat file name
    #[arg(long)]
    dat_name: Option<String>,
    /// Message create time (seconds); selects the year-month folder
    #[arg(long)]
    create_time: Option<i64>,
    /// Return a file path instead of a base64 data URL
    #[arg(long)]
    prefer_file_path: bool,
    /// Only look the file up in WeChat's hardlink index
    #[arg(long)]
    hardlink_only: bool,
    /// Do not fall back to scanning by .dat name
    #[arg(long)]
    no_cache_index: bool,
}

impl ImageTarget {
    fn payload(&self, force: bool) -> weflow_core::services::ImagePayload {
        weflow_core::services::ImagePayload {
            session_id: self.session.clone(),
            image_md5: self.md5.clone(),
            image_dat_name: self.dat_name.clone(),
            create_time: self.create_time,
            prefer_file_path: self.prefer_file_path,
            hardlink_only: self.hardlink_only,
            allow_cache_index: if self.no_cache_index {
                Some(false)
            } else {
                None
            },
            force,
        }
    }
}

#[derive(Subcommand, Debug)]
enum ImageSubcommand {
    /// Decrypt an image into the image cache. Without --force a cached thumbnail may be returned; with --force the HD original is used when it is on disk (`export media` always behaves like --force)
    Decrypt {
        #[command(flatten)]
        target: ImageTarget,
        /// Prefer the HD rendition
        #[arg(long)]
        force: bool,
    },
    /// Look an image up in the cache without decrypting
    ResolveCache {
        #[command(flatten)]
        target: ImageTarget,
    },
    /// Resolve many images; takes a JSON array of payloads (sessionId, imageMd5, imageDatName, createTime, …)
    ResolveBatch {
        /// JSON array of image payloads
        payloads_json: String,
    },
    /// Delete every decrypted image from the cache
    ClearCache,
    /// Windows only: make WeChat download original-size images (img_helper.dll hook)
    AutoDownload {
        #[command(subcommand)]
        command: AutoDownloadSubcommand,
    },
}

#[derive(Subcommand, Debug)]
enum AutoDownloadSubcommand {
    /// Hook WeChat and keep the hook alive until interrupted (re-hooks after a WeChat restart)
    Start {
        /// Conversation ids to download originals for (default: `autoDownloadWhitelist` from the config)
        #[arg(long, value_delimiter = ',')]
        whitelist: Vec<String>,
    },
    /// Show whether the image auto-download hook is running
    Status,
}

#[derive(Args, Debug)]
struct VideoCommand {
    #[command(subcommand)]
    command: VideoSubcommand,
}

#[derive(Subcommand, Debug)]
enum VideoSubcommand {
    /// Look up the on-disk video (and optional cover/thumbnail) for a message md5
    Info {
        /// Video md5 from the message
        md5: String,
        /// Skip cover / thumbnail images
        #[arg(long)]
        no_poster: bool,
        /// Return `file://` URLs instead of base64 data URLs for the posters
        #[arg(long)]
        file_url: bool,
    },
    /// Extract the video md5 from a message XML payload
    ParseMd5 {
        /// Message XML
        content: String,
    },
}

#[derive(Args, Debug)]
struct InsightCommand {
    #[command(subcommand)]
    command: InsightSubcommand,
}

#[derive(Subcommand, Debug)]
enum InsightSubcommand {
    /// Test the AI model connection
    Test,
    /// Generate a test insight for the first eligible private chat (or for --session)
    Trigger {
        /// Generate for this conversation instead of the first eligible one
        session_id: Option<String>,
    },
    /// List insight records (newest first)
    Records {
        /// Only records containing this text
        #[arg(long)]
        keyword: Option<String>,
        /// Only this conversation
        #[arg(long)]
        session: Option<String>,
        /// Only records from this time on (Unix seconds)
        #[arg(long)]
        start: Option<i64>,
        /// Only records up to this time (Unix seconds)
        #[arg(long)]
        end: Option<i64>,
        /// Maximum number of records
        #[arg(long)]
        limit: Option<i64>,
        /// Number of records to skip
        #[arg(long)]
        offset: Option<i64>,
    },
    /// Show one insight record
    Get {
        /// Record id
        id: String,
    },
    /// Mark an insight record as read
    MarkRead {
        /// Record id
        id: String,
    },
    /// Delete insight records (all, or filtered)
    Clear {
        /// Only this conversation
        #[arg(long)]
        session: Option<String>,
        /// Only records from this time on (Unix seconds)
        #[arg(long)]
        start: Option<i64>,
        /// Only records up to this time (Unix seconds)
        #[arg(long)]
        end: Option<i64>,
    },
    /// Today's trigger counts (only meaningful inside a running `serve --insight`)
    TodayStats,
    /// Run the silence scan once
    Scan,
    /// Footprint statistics from the database
    Footprint,
    /// AI footprint summary; takes the JSON payload {rangeLabel, summary, privateSegments, mentionGroups}
    FootprintSummary {
        /// JSON payload
        payload_json: String,
    },
}

#[derive(Args, Debug)]
struct ServeCommand {
    /// Serve the HTTP API
    #[arg(long)]
    http: bool,
    /// Push new messages to HTTP clients (SSE)
    #[arg(long)]
    message_push: bool,
    /// Run the AI insight engine
    #[arg(long)]
    insight: bool,
    /// Run the image auto-download hook (Windows only)
    #[arg(long)]
    image_auto_download: bool,
    /// Listen address (default: `http_api_host` from the config, else 127.0.0.1).
    #[arg(long)]
    host: Option<String>,
    /// Listen port (default: `http_api_port` from the config, else 5031).
    #[arg(long)]
    port: Option<u16>,
    /// Access token for the HTTP API (default: `http_api_token` from the config).
    #[arg(long, env = "WEFLOW_HTTP_TOKEN")]
    api_token: Option<String>,
}

#[derive(Args, Debug)]
struct RuntimeCommand {
    #[command(subcommand)]
    command: RuntimeSubcommand,
}

#[derive(Subcommand, Debug)]
enum RuntimeSubcommand {
    /// Show where the runtime is unpacked and its version
    Info,
    /// List the files of the embedded runtime with sizes and hashes
    Manifest,
}

#[derive(Args, Debug)]
struct BackupCommand {
    #[command(subcommand)]
    command: BackupSubcommand,
}

#[derive(Subcommand, Debug)]
enum BackupSubcommand {
    /// Create a backup of the current account
    Create {
        /// Output file
        #[arg(long)]
        out: PathBuf,
        /// Leave out the cached images
        #[arg(long)]
        no_images: bool,
        /// Leave out the cached voice files
        #[arg(long)]
        no_voice: bool,
        /// Leave out the cached stickers
        #[arg(long)]
        no_emojis: bool,
    },
    /// Show what a backup contains
    Inspect {
        /// Backup file
        path: PathBuf,
    },
    /// Restore a backup into a folder
    Restore {
        /// Backup file
        path: PathBuf,
        /// Folder to restore into
        #[arg(long)]
        target: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    #[cfg(windows)]
    reserve_allocator_arena();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli: Cli = i18n::parse();
    weflow_core::output::set_json_output(cli.json);
    weflow_core::output::set_progress_mode(if cli.progress {
        weflow_core::output::ProgressMode::Ndjson
    } else {
        weflow_core::output::ProgressMode::Auto
    });
    let outcome = run(&cli).await;
    weflow_core::output::finish_progress();
    match outcome {
        Ok(value) => {
            print_response(&success(value), &cli);
            ExitCode::SUCCESS
        }
        Err(err) => {
            print_failure(&err, &cli);
            ExitCode::from(err.exit_code as u8)
        }
    }
}

async fn run(cli: &Cli) -> AppResult<Value> {
    let ctx = AppContext::new(None, VERSION)?;
    let mut config =
        ConfigStore::load(&ctx.config_path).map_err(|err| AppError::config(err.to_string()))?;

    apply_progress_settings(&config, cli);
    weflow_core::ffmpeg::set_home(&ctx.home_dir);

    match &cli.command {
        Commands::Config(command) => return handle_config(command, &ctx, &mut config),
        Commands::Lang { lang } => {
            let code = match lang {
                LangArg::En => "en",
                LangArg::Zh => "zh",
            };
            config.lang = Some(code.to_string());
            config
                .save(&ctx.config_path)
                .map_err(|err| AppError::config(err.to_string()))?;
            return Ok(json!({ "lang": code }));
        }
        Commands::Runtime(command) => return handle_runtime(command, &ctx),
        Commands::Ffmpeg(command) => return handle_ffmpeg(command, &ctx, &mut config).await,
        Commands::Chat(ChatCommand {
            command:
                ChatSubcommand::ClearAccountData {
                    cache,
                    exports_dir,
                    yes,
                },
        }) => {
            if !*yes {
                return Err(AppError::usage("this removes files; add --yes to confirm"));
            }
            let hub = ServiceHub::new(ctx.clone(), config.clone(), None, None, None, None);
            let result = hub.clear_current_account_data(*cache, exports_dir)?;
            if *cache {
                // like the desktop app: the account is signed out of this profile
                for key in [
                    "db_path",
                    "wxid",
                    "decrypt_key",
                    "image_xor_key",
                    "image_aes_key",
                ] {
                    config.unset_key(None, key);
                }
                config
                    .save(&ctx.config_path)
                    .map_err(|err| AppError::config(err.to_string()))?;
            }
            return Ok(result);
        }
        _ => {}
    }

    let hub = ServiceHub::new(ctx, config, None, None, None, None);

    match &cli.command {
        Commands::Db(command) => handle_db(command, &hub),
        Commands::Chat(command) => handle_chat(command, &hub).await,
        Commands::Key(command) => handle_key(command, &hub),
        Commands::Export(command) => handle_export(command, &hub).await,
        Commands::Analytics(command) => handle_analytics(command, &hub),
        Commands::Group(command) => handle_group(command, &hub),
        Commands::Report(command) => handle_report(command, &hub),
        Commands::Sns(command) => handle_sns(command, &hub).await,
        Commands::Biz(command) => handle_biz(command, &hub),
        Commands::Insight(command) => handle_insight(command, &hub).await,
        Commands::Image(ImageCommand {
            command: ImageSubcommand::AutoDownload { command },
        }) => match command {
            AutoDownloadSubcommand::Start { whitelist } => {
                let svc = weflow_core::image_download::ImageAutoDownload::new(hub.runtime_dir());
                let list = if whitelist.is_empty() {
                    config_whitelist(&hub)
                } else {
                    whitelist.clone()
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
                Ok(json!({ "stopped": true }))
            }
            AutoDownloadSubcommand::Status => Ok(
                json!({ "isHooked": false, "pid": null, "supported": weflow_core::image_download::supported() }),
            ),
        },
        Commands::Image(command) => Ok(match &command.command {
            ImageSubcommand::AutoDownload { .. } => unreachable!("handled above"),
            ImageSubcommand::Decrypt { target, force } => {
                hub.image_decrypt(&target.payload(*force)).to_json()
            }
            ImageSubcommand::ResolveCache { target } => {
                hub.image_resolve_cache(&target.payload(false)).to_json()
            }
            ImageSubcommand::ResolveBatch { payloads_json } => {
                let list: Vec<Value> = serde_json::from_str(payloads_json).map_err(|e| {
                    AppError::usage(format!("payloads_json must be a JSON array: {e}"))
                })?;
                let payloads: Vec<_> = list
                    .iter()
                    .map(weflow_core::services::ImagePayload::from_json)
                    .collect();
                hub.image_resolve_cache_batch(&payloads)
            }
            ImageSubcommand::ClearCache => hub.image_clear_cache(),
        }),
        Commands::Video(command) => match &command.command {
            VideoSubcommand::Info {
                md5,
                no_poster,
                file_url,
            } => hub.video_info(
                md5,
                !*no_poster,
                if *file_url {
                    weflow_core::video::PosterFormat::FileUrl
                } else {
                    weflow_core::video::PosterFormat::DataUrl
                },
            ),
            VideoSubcommand::ParseMd5 { content } => {
                Ok(serde_json::json!({ "md5": weflow_core::video::parse_video_md5(content) }))
            }
        },
        Commands::Serve(command) => handle_serve(command, &hub).await,
        Commands::Backup(command) => handle_backup(command, &hub),
        Commands::Cache(CacheCommand {
            command: CacheSubcommand::ClearAll,
        }) => {
            let r = hub.cache_clear_all();
            if r["success"] != true {
                return Err(AppError::runtime(
                    r["error"]
                        .as_str()
                        .unwrap_or("clearing the caches failed")
                        .to_string(),
                ));
            }
            Ok(r)
        }
        Commands::Runtime(_)
        | Commands::Ffmpeg(_)
        | Commands::Config(_)
        | Commands::Lang { .. } => unreachable!(),
    }
}

/// Progress bar settings from the config (`no_progress`, `progress_delay_seconds`) and `WEFLOW_PROGRESS_DELAY`;
/// `--progress` keeps NDJSON events whatever the config says.
fn apply_progress_settings(config: &ConfigStore, cli: &Cli) {
    if !cli.progress {
        let off = config.get_key(None, "no_progress");
        if off
            .as_bool()
            .unwrap_or_else(|| off.as_str() == Some("true"))
        {
            weflow_core::output::set_progress_mode(weflow_core::output::ProgressMode::Off);
        }
    }
    let from_env = std::env::var("WEFLOW_PROGRESS_DELAY")
        .ok()
        .and_then(|v| weflow_core::output::parse_delay(&v));
    let v = config.get_key(None, "progress_delay_seconds");
    let delay = from_env.or_else(|| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(weflow_core::output::parse_delay))
    });
    if let Some(d) = delay {
        weflow_core::output::set_progress_delay(d);
    }
}

fn handle_config(
    command: &ConfigCommand,
    ctx: &AppContext,
    config: &mut ConfigStore,
) -> AppResult<Value> {
    match &command.command {
        ConfigSubcommand::Path => Ok(json!(ctx.config_path.to_string_lossy())),
        ConfigSubcommand::List => Ok(serde_json::to_value(config).unwrap()),
        ConfigSubcommand::Get { key } => {
            if matches!(key.as_deref(), Some("config_path" | "configPath")) {
                return Ok(json!(ctx.config_path.to_string_lossy()));
            }
            if let Some(key) = key {
                Ok(config.get_key(None, key))
            } else {
                Ok(serde_json::to_value(config.profile(None)).unwrap())
            }
        }
        ConfigSubcommand::Set { key, value } => {
            if matches!(key.as_str(), "config_path" | "configPath") {
                let path = std::path::absolute(value)
                    .map_err(|err| AppError::usage(format!("invalid path {value}: {err}")))?;
                if path.is_dir() {
                    return Err(AppError::usage(format!(
                        "{} is a folder; give the config file path",
                        path.display()
                    )));
                }
                weflow_core::config::set_saved_config_path(&path)?;
                return Ok(json!({ "configPath": path }));
            }
            if matches!(
                key.as_str(),
                "progress_delay_seconds" | "progressDelaySeconds"
            ) && weflow_core::output::parse_delay(value).is_none()
            {
                return Err(AppError::usage(format!(
                    "invalid value '{value}' for {key}; use a whole number of seconds (0 or more)"
                )));
            }
            let value = parse_config_value(value);
            config.set_key(None, key, value)?;
            config
                .save(&ctx.config_path)
                .map_err(|err| AppError::config(err.to_string()))?;
            Ok(json!({ "configPath": ctx.config_path }))
        }
        ConfigSubcommand::Unset { key } => {
            if matches!(key.as_str(), "config_path" | "configPath") {
                weflow_core::config::clear_saved_config_path()?;
                return Ok(json!({ "configPath": weflow_core::config::default_config_path() }));
            }
            config.unset_key(None, key);
            config
                .save(&ctx.config_path)
                .map_err(|err| AppError::config(err.to_string()))?;
            Ok(json!({ "configPath": ctx.config_path }))
        }
        ConfigSubcommand::Clear => {
            *config = ConfigStore::default();
            config
                .save(&ctx.config_path)
                .map_err(|err| AppError::config(err.to_string()))?;
            Ok(json!({ "configPath": ctx.config_path }))
        }
        ConfigSubcommand::Import { path } => {
            let path = path.clone().or_else(|| {
                old_electron_config_candidates()
                    .into_iter()
                    .find(|candidate| candidate.exists())
            });
            let path = path.ok_or_else(|| {
                AppError::config("old Electron config not found; pass an explicit path")
            })?;
            let skipped = config
                .import_electron_config(&path, None)
                .map_err(|err| AppError::config(err.to_string()))?;
            config
                .save(&ctx.config_path)
                .map_err(|err| AppError::config(err.to_string()))?;
            Ok(
                json!({ "importedFrom": path, "configPath": ctx.config_path, "skippedEncryptedKeys": skipped }),
            )
        }
    }
}

async fn handle_ffmpeg(
    command: &FfmpegCommand,
    ctx: &AppContext,
    config: &mut ConfigStore,
) -> AppResult<Value> {
    let save = |config: &ConfigStore| {
        config
            .save(&ctx.config_path)
            .map_err(|err| AppError::config(err.to_string()))
    };
    match &command.command {
        FfmpegSubcommand::Install { force } => {
            weflow_core::ffmpeg::install(&ctx.home_dir, config.ffmpeg_base_url.as_deref(), *force)
                .await
        }
        FfmpegSubcommand::Path => {
            let mut found = weflow_core::ffmpeg::status();
            found["baseUrl"] = json!(config
                .ffmpeg_base_url
                .as_deref()
                .unwrap_or(weflow_core::ffmpeg::DEFAULT_BASE_URL));
            Ok(found)
        }
        FfmpegSubcommand::Set {
            setting: FfmpegSetting::Baseurl { url },
        } => {
            let url = weflow_core::ffmpeg::check_base_url(url)?;
            config.ffmpeg_base_url = Some(url.clone());
            save(config)?;
            Ok(json!({ "baseUrl": url }))
        }
        FfmpegSubcommand::Unset {
            setting: FfmpegUnsetting::Baseurl,
        } => {
            config.ffmpeg_base_url = None;
            save(config)?;
            Ok(json!({ "baseUrl": weflow_core::ffmpeg::DEFAULT_BASE_URL }))
        }
    }
}

fn handle_runtime(command: &RuntimeCommand, ctx: &AppContext) -> AppResult<Value> {
    match command.command {
        RuntimeSubcommand::Info => Ok(json!({
            "homeDir": ctx.home_dir,
            "configPath": ctx.config_path,
            "runtimeDir": ctx.runtime_dir,
            "version": ctx.version,
            "target": weflow_assets::target_triple()
        })),
        RuntimeSubcommand::Manifest => Ok(serde_json::to_value(weflow_assets::manifest()).unwrap()),
    }
}

fn handle_db(command: &DbCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        DbSubcommand::Detect => Ok(hub.db_detect()),
        DbSubcommand::Scan { root } => Ok(hub.db_scan(root)),
        DbSubcommand::Test | DbSubcommand::Open => hub.db_test(),
        DbSubcommand::Wxid { root } => hub.db_wxid(root.as_deref()),
    }
}

async fn handle_chat(command: &ChatCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        ChatSubcommand::ClearAccountData { .. } => unreachable!("handled in run()"),
        ChatSubcommand::Sessions { limit } => {
            let mut sessions = hub.chat_sessions_list()?;
            if *limit > 0 {
                sessions.truncate(*limit);
            }
            Ok(Value::Array(sessions))
        }
        ChatSubcommand::Messages(args) => hub.messages(&args.session_id, args.limit, args.offset),
        ChatSubcommand::Latest { session_id, limit } => hub.latest(session_id, *limit),
        ChatSubcommand::Search {
            keyword,
            session_id,
            limit,
            offset,
            start,
            end,
        } => hub.search(
            keyword,
            session_id.as_deref(),
            *limit,
            *offset,
            *start,
            *end,
        ),
        ChatSubcommand::Contacts => hub.contacts(),
        ChatSubcommand::Contact { username } => hub.contact(username),
        ChatSubcommand::UpdateMessage {
            session_id,
            local_id,
            create_time,
            content,
        } => hub.update_message(session_id, *local_id, *create_time, content),
        ChatSubcommand::DeleteMessage {
            session_id,
            local_id,
            create_time,
            db_path_hint,
        } => hub.delete_message(session_id, *local_id, *create_time, db_path_hint.as_deref()),
        ChatSubcommand::AntiRevoke { command } => match command {
            AntiRevokeSubcommand::Sessions => hub.chat_anti_revoke_sessions(),
            AntiRevokeSubcommand::Check { sessions } => hub.anti_revoke("check", sessions),
            AntiRevokeSubcommand::Install { sessions } => hub.anti_revoke("install", sessions),
            AntiRevokeSubcommand::Uninstall { sessions } => hub.anti_revoke("uninstall", sessions),
        },
        ChatSubcommand::Message {
            session_id,
            local_id,
            server_id,
        } => match (local_id, server_id) {
            (Some(id), None) => hub.chat_message_by_id(session_id, *id),
            (None, Some(svr)) => hub.chat_message_by_server_id(session_id, svr),
            _ => Err(AppError::usage(
                "pass exactly one of --local-id or --server-id",
            )),
        },
        ChatSubcommand::Dates { session_id } => hub.chat_dates(session_id),
        ChatSubcommand::DateCounts { session_id } => hub.chat_date_counts(session_id),
        ChatSubcommand::Counts { sessions } => hub.chat_counts(sessions),
        ChatSubcommand::Statuses { usernames } => hub.chat_statuses(usernames),
        ChatSubcommand::Detail {
            session_id,
            fast,
            extra,
        } => match (fast, extra) {
            (true, false) => hub.chat_detail_fast(session_id),
            (false, true) => hub.chat_detail_extra(session_id),
            _ => hub.chat_detail(session_id),
        },
        ChatSubcommand::MarkRead => hub.chat_mark_all_read(),
        ChatSubcommand::TabCounts => hub.chat_tab_counts(),
        ChatSubcommand::ExportStats {
            sessions,
            start,
            end,
            no_relations,
        } => {
            let (b, e) = date_range_args(start.as_deref(), end.as_deref())?;
            hub.chat_export_stats(
                sessions,
                b.unwrap_or(0),
                e.map(|x| x - 1).unwrap_or(0),
                !no_relations,
            )
        }
        ChatSubcommand::GroupHint { chatroom_id, set } => match set {
            Some(count) => hub.set_group_hint(chatroom_id, *count),
            None => hub.get_group_hint(chatroom_id),
        },
        ChatSubcommand::Resources {
            session,
            types,
            start,
            end,
            limit,
            offset,
        } => {
            let (b, e) = date_range_args(start.as_deref(), end.as_deref())?;
            hub.chat_resources(&weflow_core::services::ResourceQuery {
                session_id: session.clone(),
                types: types.clone(),
                begin: b.unwrap_or(0),
                end: e.map(|x| x - 1).unwrap_or(0),
                limit: *limit,
                offset: *offset,
            })
        }
        ChatSubcommand::Images { session_id } => hub.chat_all_images(session_id),
        ChatSubcommand::VoiceMessages { session_id } => hub.chat_all_voices(session_id),
        ChatSubcommand::MediaStream {
            session,
            media_type,
            start,
            end,
            limit,
            offset,
        } => {
            let (b, e) = date_range_args(start.as_deref(), end.as_deref())?;
            hub.chat_media_stream(
                session.as_deref(),
                media_type,
                b.unwrap_or(0),
                e.map(|x| x - 1).unwrap_or(0),
                *limit,
                *offset,
            )
        }
        ChatSubcommand::TransferNames {
            chatroom_id,
            payer,
            receiver,
        } => hub.chat_transfer_names(chatroom_id, payer, receiver),
        ChatSubcommand::Voice { session_id, out } => {
            let out_path = out.as_deref().unwrap_or(Path::new("."));
            hub.export_media(Some(session_id), out_path, "voice", None, None)
                .await
        }
        ChatSubcommand::VoiceData {
            session_id,
            msg_id,
            create_time,
            server_id,
            sender,
            out,
        } => {
            let wav = hub.voice_data(
                session_id,
                msg_id,
                *create_time,
                server_id.as_deref(),
                sender.as_deref(),
            )?;
            match out {
                Some(path) => {
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            AppError::runtime(format!("failed to create {}: {e}", parent.display()))
                        })?;
                    }
                    std::fs::write(path, &wav).map_err(|e| {
                        AppError::runtime(format!("failed to write {}: {e}", path.display()))
                    })?;
                    Ok(
                        serde_json::json!({ "success": true, "path": path.to_string_lossy(), "bytes": wav.len() }),
                    )
                }
                None => {
                    use base64::Engine;
                    Ok(
                        serde_json::json!({ "success": true, "data": base64::engine::general_purpose::STANDARD.encode(&wav) }),
                    )
                }
            }
        }
        ChatSubcommand::ImageData {
            session_id,
            msg_id,
            out,
        } => {
            let bytes = hub.image_data_for_message(session_id, msg_id)?;
            if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).map_err(|e| {
                    AppError::runtime(format!("failed to create {}: {e}", parent.display()))
                })?;
            }
            std::fs::write(out, &bytes).map_err(|e| {
                AppError::runtime(format!("failed to write {}: {e}", out.display()))
            })?;
            Ok(
                serde_json::json!({ "success": true, "path": out.to_string_lossy(), "bytes": bytes.len() }),
            )
        }
        ChatSubcommand::VoiceCache { session_id, msg_id } => {
            Ok(hub.voice_resolve_cache(session_id, msg_id))
        }
        ChatSubcommand::VoicePreload {
            session_id,
            messages_json,
        } => {
            let messages: Vec<Value> = serde_json::from_str(messages_json)
                .map_err(|e| AppError::usage(format!("messages_json must be a JSON array: {e}")))?;
            hub.voice_preload(session_id, &messages)
        }
        ChatSubcommand::Emoji { session_id, out } => hub.emoji_download(session_id, out).await,
    }
}

fn handle_key(command: &KeyCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        KeySubcommand::Db { pid, timeout } => hub.key_db(*pid, *timeout),
        KeySubcommand::Image { user_dir } => hub.key_image(user_dir.as_deref()),
        KeySubcommand::ScanImage { user_dir } => hub.key_scan_image(user_dir),
    }
}

async fn handle_export(command: &ExportCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        ExportSubcommand::Sessions {
            sessions,
            format,
            out,
        } => {
            let mut data = hub.sessions()?;
            if !sessions.is_empty() {
                data = filter_named_items(data, sessions);
            }
            write_export("sessions", format.as_deref(), out, &data)
        }
        ExportSubcommand::Contacts { format, out } => {
            let data = hub.contacts()?;
            write_export("contacts", format.as_deref(), out, &data)
        }
        ExportSubcommand::Footprint { format, out } => {
            let data = hub.footprint()?;
            write_export("footprint", format.as_deref(), out, &data)
        }
        ExportSubcommand::Media {
            out,
            session,
            r#type,
            start,
            end,
        } => {
            let (start_ts, end_ts) = date_range_args(start.as_deref(), end.as_deref())?;
            hub.export_media(session.as_deref(), out, r#type, start_ts, end_ts)
                .await
        }
        ExportSubcommand::Messages {
            session_id,
            start,
            end,
            out,
            format,
            sender,
            display_name,
            excel_compact,
            media,
        } => {
            let (start_ts, end_ts) = date_range_args(start.as_deref(), end.as_deref())?;
            let fmt = format.to_ascii_lowercase();
            let media_opts = parse_media_selection(media)?;
            let parse_display = |s: &str| {
                weflow_core::export_msg::DisplayPref::parse(s).ok_or_else(|| {
                    AppError::usage("--display-name must be group-nickname, remark or nickname")
                })
            };
            let ext = match fmt.as_str() {
                "txt" => "txt",
                "json" | "arkme-json" => "json",
                "chatlab" => "chatlab.json",
                "chatlab-jsonl" => "jsonl",
                "excel" | "xlsx" => "xlsx",
                "weclone" => "csv",
                "html" => "html",
                "sql" => "sql",
                other => {
                    return Err(AppError::usage(format!(
                        "unsupported message export format: {other}; supported: {}",
                        weflow_core::services::MESSAGE_EXPORT_FORMATS
                    )))
                }
            };
            let display_pref = parse_display(display_name.as_deref().unwrap_or("group-nickname"))?;
            let safe_name: String = session_id
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '@' || c == '.' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let target = export_path(out, &format!("{safe_name}.{ext}"));
            let request = weflow_core::services::MessageExportRequest {
                session_id: session_id.clone(),
                format: fmt,
                // the end is inclusive seconds
                start: start_ts,
                end: end_ts.map(|e| e - 1),
                sender: sender.clone(),
                display_pref,
                excel_compact: *excel_compact,
            };
            if media_opts.enabled {
                hub.export_messages_with_media(&request, &target, &media_opts)
                    .await
            } else {
                hub.export_messages(&request, &target)
            }
        }
    }
}

/// `--media image,voice` / `--media all` → the media kinds to copy next to a message export.
fn parse_media_selection(values: &[String]) -> Result<weflow_core::api::ApiMediaOptions, AppError> {
    let mut opts = weflow_core::api::ApiMediaOptions::default();
    for v in values
        .iter()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
    {
        match v.as_str() {
            "all" => {
                (opts.images, opts.voices, opts.videos, opts.emojis) = (true, true, true, true)
            }
            "image" | "images" => opts.images = true,
            "voice" | "voices" => opts.voices = true,
            "video" | "videos" => opts.videos = true,
            "emoji" | "emojis" | "sticker" | "stickers" => opts.emojis = true,
            other => {
                return Err(AppError::usage(format!(
                    "unknown media kind: {other}; use image, voice, video, emoji or all"
                )))
            }
        }
    }
    opts.enabled = opts.images || opts.voices || opts.videos || opts.emojis;
    Ok(opts)
}

/// `YYYY-MM-DD` → (year, month, day), validated.
fn parse_date_parts(s: &str) -> Result<(i32, u32, u32), String> {
    let parts: Vec<&str> = s.splitn(3, '-').collect();
    if parts.len() != 3 {
        return Err(format!("invalid date '{s}'; use YYYY-MM-DD"));
    }
    let y: i32 = parts[0]
        .parse()
        .map_err(|_| format!("invalid year in '{s}'"))?;
    let m: u32 = parts[1]
        .parse()
        .map_err(|_| format!("invalid month in '{s}'"))?;
    let d: u32 = parts[2]
        .parse()
        .map_err(|_| format!("invalid day in '{s}'"))?;
    if !(1970..=2100).contains(&y) {
        return Err(format!("invalid year in '{s}'; use 1970-2100"));
    }
    if !(1..=12).contains(&m) {
        return Err(format!("invalid month in '{s}'; use 01-12"));
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dim = match m {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=dim).contains(&d) {
        return Err(format!("invalid day in '{s}'; {y}-{m:02} has {dim} days"));
    }
    Ok((y, m, d))
}

/// Unix time of 00:00 on that date in the machine's local time zone (the zone `chat dates` and the statistics use).
fn parse_date_local(s: &str) -> Result<i64, String> {
    let (y, m, d) = parse_date_parts(s)?;
    weflow_core::message::local_midnight(y, m, d)
        .ok_or_else(|| format!("'{s}' does not exist in the local time zone"))
}

/// Unix time of 00:00 on the day after that date (a 23- or 25-hour day is handled by the zone rules).
fn parse_next_date_local(s: &str) -> Result<i64, String> {
    let (y, m, d) = parse_date_parts(s)?;
    weflow_core::message::local_midnight_after(y, m, d)
        .ok_or_else(|| format!("'{s}' does not exist in the local time zone"))
}

/// Parses an optional `--start` / `--end` pair (`end` becomes the exclusive next-midnight bound) and
/// rejects a start that is after the end.
/// `--start` / `--end` dates (local time). The end is returned exclusive (next midnight).
fn date_range_args(
    start: Option<&str>,
    end: Option<&str>,
) -> AppResult<(Option<i64>, Option<i64>)> {
    let b = start
        .map(parse_date_local)
        .transpose()
        .map_err(AppError::usage)?;
    let e = end
        .map(parse_next_date_local)
        .transpose()
        .map_err(AppError::usage)?;
    if let (Some(b), Some(e)) = (b, e) {
        if b >= e {
            return Err(AppError::usage(format!(
                "--start ({}) is after --end ({})",
                start.unwrap_or(""),
                end.unwrap_or("")
            )));
        }
    }
    Ok((b, e))
}

fn handle_analytics(command: &AnalyticsCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        AnalyticsSubcommand::Overall { force } => hub.analytics_overall_statistics(*force),
        AnalyticsSubcommand::Rankings { limit, start, end } => {
            let (b, e) = date_range_args(start.as_deref(), end.as_deref())?;
            Ok(json!(hub.analytics_contact_rankings(
                *limit,
                b.unwrap_or(0),
                e.map(|e| e - 1).unwrap_or(0)
            )?))
        }
        AnalyticsSubcommand::Time => hub.analytics_time_distribution(),
        AnalyticsSubcommand::Excluded { set } => match set {
            Some(list) => Ok(json!(hub.analytics_set_excluded_usernames(
                &list
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
            )?)),
            None => Ok(json!(hub.analytics_excluded_usernames()?)),
        },
        AnalyticsSubcommand::ExcludeCandidates => Ok(json!(hub.analytics_exclude_candidates()?)),
        AnalyticsSubcommand::ClearCache => {
            hub.analytics_clear_cache()?;
            Ok(json!({ "cleared": true }))
        }
    }
}

fn handle_group(command: &GroupCommand, hub: &ServiceHub) -> AppResult<Value> {
    fn range(start: &Option<String>, end: &Option<String>) -> AppResult<(i64, i64)> {
        let (b, e) = date_range_args(start.as_deref(), end.as_deref())?;
        Ok((b.unwrap_or(0), e.map(|e| e - 1).unwrap_or(0)))
    }
    match &command.command {
        GroupSubcommand::List => Ok(json!(hub.group_chats()?)),
        GroupSubcommand::Members {
            chatroom_id,
            counts,
            refresh,
        } => {
            let (members, from_cache, updated_at) =
                hub.group_members_panel(chatroom_id, *refresh, *counts)?;
            Ok(
                json!({ "chatroomId": chatroom_id, "count": members.len(), "fromCache": from_cache, "updatedAt": updated_at, "members": members }),
            )
        }
        GroupSubcommand::Ranking {
            chatroom_id,
            limit,
            start,
            end,
        } => {
            let (b, e) = range(start, end)?;
            Ok(json!(hub.group_message_ranking(
                chatroom_id,
                *limit,
                b,
                e
            )?))
        }
        GroupSubcommand::Hours {
            chatroom_id,
            start,
            end,
        } => {
            let (b, e) = range(start, end)?;
            hub.group_active_hours(chatroom_id, b, e)
        }
        GroupSubcommand::Media {
            chatroom_id,
            start,
            end,
        } => {
            let (b, e) = range(start, end)?;
            hub.group_media_stats(chatroom_id, b, e)
        }
        GroupSubcommand::Member {
            chatroom_id,
            username,
            start,
            end,
        } => {
            let (b, e) = range(start, end)?;
            hub.group_member_analytics(chatroom_id, username, b, e)
        }
        GroupSubcommand::MemberMessages {
            chatroom_id,
            username,
            limit,
            cursor,
            start,
            end,
        } => {
            let (b, e) = range(start, end)?;
            hub.group_member_messages(chatroom_id, username, b, e, *limit, *cursor)
        }
        GroupSubcommand::ExportMemberMessages {
            chatroom_id,
            username,
            out,
            start,
            end,
        } => {
            let (b, e) = range(start, end)?;
            hub.group_export_member_messages(chatroom_id, username, out, b, e)
        }
        GroupSubcommand::ExportMembers { chatroom_id, out } => {
            hub.group_export_members(chatroom_id, out)
        }
    }
}

fn handle_report(command: &ReportCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        ReportSubcommand::Annual { command } => match command {
            AnnualSubcommand::Years => hub.report_available_years(),
            AnnualSubcommand::Generate { year } => hub.report_annual(*year),
        },
        ReportSubcommand::Dual { command } => match command {
            DualSubcommand::Generate {
                friend,
                year,
                exclude_words,
            } => hub.report_dual(friend, *year, exclude_words),
        },
    }
}

async fn handle_sns(command: &SnsCommand, hub: &ServiceHub) -> AppResult<Value> {
    use weflow_core::services::{SnsExportOptions, SnsTimelineQuery};
    match &command.command {
        SnsSubcommand::DebugResource { url } => {
            let r = hub.sns_debug_resource(url).await;
            if r["success"] != true {
                return Err(AppError::runtime(
                    r["error"].as_str().unwrap_or("request failed").to_string(),
                ));
            }
            Ok(r)
        }
        SnsSubcommand::Timeline {
            limit,
            offset,
            users,
            keyword,
            start,
            end,
            with_media,
        } => {
            let posts = hub.sns_timeline_query(&SnsTimelineQuery {
                limit: *limit,
                offset: *offset,
                usernames: users.clone(),
                keyword: keyword.clone(),
                start: *start,
                end: *end,
            })?;
            let posts = if *with_media {
                hub.sns_enrich_timeline_media(posts, "", true, true).await
            } else {
                posts
            };
            Ok(json!({ "timeline": posts }))
        }
        SnsSubcommand::Users => Ok(json!({ "usernames": hub.sns_usernames_list()? })),
        SnsSubcommand::Stats { fast } => hub.sns_export_stats(*fast),
        SnsSubcommand::PostCounts { user, prefer_cache } => match user {
            Some(u) => hub.sns_user_post_stats(u),
            None => Ok(json!(hub.sns_user_post_counts(*prefer_cache)?)),
        },
        SnsSubcommand::Export {
            out,
            format,
            users,
            keyword,
            start,
            end,
            media,
        } => {
            hub.sns_export_timeline(&SnsExportOptions {
                output_dir: out.clone(),
                format: format.clone(),
                usernames: users.clone(),
                keyword: keyword.clone(),
                export_media: *media,
                start: *start,
                end: *end,
                ..Default::default()
            })
            .await
        }
        SnsSubcommand::Media { url, key, out } => {
            let fetched = hub.sns_fetch_media(url, key.as_deref()).await?;
            let data = fetched.data.clone().unwrap_or_default();
            match out {
                Some(path) => {
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            AppError::runtime(format!("failed to create {}: {e}", parent.display()))
                        })?;
                    }
                    std::fs::write(path, &data).map_err(|e| {
                        AppError::runtime(format!("failed to write {}: {e}", path.display()))
                    })?;
                    Ok(
                        json!({ "out": path.to_string_lossy(), "contentType": fetched.content_type, "bytes": data.len(), "cachePath": fetched.cache_path }),
                    )
                }
                None => Ok(
                    json!({ "contentType": fetched.content_type, "bytes": data.len(), "cachePath": fetched.cache_path }),
                ),
            }
        }
        SnsSubcommand::DownloadEmoji {
            url,
            encrypt_url,
            aes_key,
        } => {
            hub.sns_download_emoji(url, encrypt_url.as_deref(), aes_key.as_deref())
                .await
        }
        SnsSubcommand::DownloadImage { url, out } => {
            hub.sns_download_image(url, out.as_deref()).await
        }
        SnsSubcommand::BlockDelete { action } => match action {
            TriggerAction::Check => hub.sns_block_delete_status(),
            TriggerAction::Install => hub.sns_block_delete_install(),
            TriggerAction::Uninstall => hub.sns_block_delete_uninstall(),
        },
        SnsSubcommand::Delete { post_id } => hub.sns_delete_post(post_id),
    }
}

fn handle_biz(command: &BizCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        BizSubcommand::Accounts => hub.biz_accounts(),
        BizSubcommand::Messages {
            username,
            limit,
            offset,
        } => hub.biz_messages(username, *limit, *offset),
        BizSubcommand::PayRecords { limit, offset } => hub.biz_pay_records(*limit, *offset),
    }
}

fn insight_filters(
    session: &Option<String>,
    keyword: &Option<String>,
    start: &Option<i64>,
    end: &Option<i64>,
    limit: &Option<i64>,
    offset: &Option<i64>,
) -> weflow_core::insight::RecordFilters {
    weflow_core::insight::RecordFilters {
        keyword: keyword.clone().unwrap_or_default(),
        session_id: session.clone().unwrap_or_default(),
        start_time: start.unwrap_or(0),
        end_time: end.unwrap_or(0),
        limit: *limit,
        offset: *offset,
    }
}

fn print_insight(r: &weflow_core::insight::InsightRecord) {
    weflow_core::output::event(json!({ "type": "insight", "record": r.summary() }));
}

async fn handle_insight(command: &InsightCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        InsightSubcommand::Test => Ok(hub.insight_test_connection().await),
        InsightSubcommand::Trigger { session_id } => match session_id {
            None => Ok(hub.insight_trigger_test().await),
            Some(id) => {
                let name = hub
                    .chat_contact_avatar(id)
                    .map(|(_, n)| n)
                    .unwrap_or_else(|| id.clone());
                match hub
                    .insight_generate(id, &name, weflow_core::services::InsightTrigger::Test, None)
                    .await
                {
                    Some(r) => Ok(json!({ "success": true, "record": r.summary() })),
                    None => Ok(
                        json!({ "success": false, "message": "no insight was generated (AI not configured, request failed, or the model answered SKIP)" }),
                    ),
                }
            }
        },
        InsightSubcommand::Records {
            keyword,
            session,
            start,
            end,
            limit,
            offset,
        } => Ok(hub.insight_list_records(&insight_filters(
            session, keyword, start, end, limit, offset,
        ))),
        InsightSubcommand::Get { id } => Ok(hub.insight_get_record(id)),
        InsightSubcommand::MarkRead { id } => Ok(hub.insight_mark_record_read(id)),
        InsightSubcommand::Clear {
            session,
            start,
            end,
        } => {
            Ok(hub
                .insight_clear_records(&insight_filters(session, &None, start, end, &None, &None)))
        }
        InsightSubcommand::TodayStats => Ok(hub.insight_today_stats()),
        InsightSubcommand::Scan => {
            let mut found = Vec::new();
            let n = hub
                .insight_silence_scan(&mut |r| {
                    print_insight(r);
                    found.push(r.summary());
                })
                .await;
            Ok(json!({ "generated": n, "records": found }))
        }
        InsightSubcommand::Footprint => hub.insight_footprint(),
        InsightSubcommand::FootprintSummary { payload_json } => {
            let payload: Value = serde_json::from_str(payload_json)
                .map_err(|e| AppError::usage(format!("payload_json must be JSON: {e}")))?;
            Ok(hub.insight_footprint_summary(&payload).await)
        }
    }
}

/// The background engine of `serve --insight`: polls for new messages (the desktop app reacts to
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

async fn handle_serve(command: &ServeCommand, hub: &ServiceHub) -> AppResult<Value> {
    if !command.http && !command.message_push && !command.insight && !command.image_auto_download {
        return Err(AppError::usage(
            "serve needs at least one of --http, --message-push, --insight, --image-auto-download",
        ));
    }

    let cfg_str = |key: &str| {
        hub.config_value(key)
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let host = command
        .host
        .clone()
        .or_else(|| cfg_str("httpApiHost"))
        .unwrap_or_else(|| "127.0.0.1".into());
    let port = command
        .port
        .or_else(|| hub.config_value("httpApiPort").as_u64().map(|p| p as u16))
        .unwrap_or(5031);
    let token = command
        .api_token
        .clone()
        .or_else(|| cfg_str("httpApiToken"));
    let addr = format!("{host}:{port}")
        .parse::<std::net::SocketAddr>()
        .map_err(|err| AppError::usage(format!("invalid listen address: {err}")))?;

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

    let broker = weflow_core::push::PushBroker::new();
    if command.message_push {
        let hub = hub.clone();
        let broker = broker.clone();
        std::thread::spawn(move || {
            let channel = weflow_core::push::EventChannel::new(256);
            let mut rx = channel.subscribe();
            let sender = channel.sender();
            let forward = {
                let broker = broker.clone();
                std::thread::spawn(move || {
                    while let Ok(payload) = rx.blocking_recv() {
                        broker.broadcast(&payload);
                    }
                })
            };
            weflow_core::push::message_push_loop(hub, sender, 5);
            let _ = forward.join();
        });
    }

    weflow_core::output::event(json!({
        "type": "server_started",
        "url": format!("http://{addr}"),
        "http": command.http,
        "tokenConfigured": token.is_some(),
        "messagePush": command.message_push,
        "insight": command.insight,
        "imageAutoDownload": command.image_auto_download
    }));
    if command.http && token.is_none() {
        eprintln!(
            "{}",
            weflow_core::locale::tr(
                "warning: no HTTP API token configured; every request except /health will be refused (set http_api_token or pass --api-token)",
                "警告：未配置 HTTP API 令牌；除 /health 外的所有请求都会被拒绝（请设置 http_api_token 或传入 --api-token）"
            )
        );
    }
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|err| AppError::runtime(format!("failed to bind HTTP server: {err}")))?;
    let cfg = weflow_core::http_server::HttpConfig {
        host,
        port: listener.local_addr().map(|a| a.port()).unwrap_or(port),
        token,
        push_enabled: command.message_push
            || hub.config_value("messagePushEnabled").as_bool() == Some(true),
    };
    let server = async {
        if command.http {
            weflow_core::http_server::serve(hub.clone(), cfg, broker, listener)
                .await
                .map_err(|err| AppError::runtime(format!("HTTP server stopped: {err}")))
        } else {
            std::future::pending::<AppResult<()>>().await
        }
    };
    let outcome = tokio::select! {
        result = server => result.map(|_| ()),
        _ = tokio::signal::ctrl_c() => Err(AppError::new("user_interrupt", "interrupted by user", 130)),
    };
    insight_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Some(svc) = &auto_download {
        svc.stop();
    }
    outcome?;
    Ok(json!({ "stopped": true }))
}

fn write_json_file(path: &Path, value: &Value) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            AppError::runtime(format!("failed to create {}: {err}", parent.display()))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| AppError::runtime(format!("failed to serialize JSON: {err}")))?;
    std::fs::write(path, bytes)
        .map_err(|err| AppError::runtime(format!("failed to write {}: {err}", path.display())))
}

fn write_export(name: &str, format: Option<&str>, out: &Path, value: &Value) -> AppResult<Value> {
    let format = format.unwrap_or("json").to_ascii_lowercase();
    let extension = match format.as_str() {
        "json" => "json",
        "csv" => "csv",
        "txt" => "txt",
        "html" => "html",
        "excel" | "xlsx" => "xlsx",
        "sql" => "sql",
        "chatlab" => "chatlab.json",
        "weclone" => "weclone.csv",
        other => {
            return Err(AppError::usage(format!(
                "unsupported export format: {other}; supported: json, csv, txt, html, excel, sql, chatlab, weclone"
            )));
        }
    };
    let path = export_path(out, &format!("{name}.{extension}"));
    match format.as_str() {
        "json" => write_json_file(&path, value)?,
        "csv" => write_csv_file(&path, value)?,
        "txt" => write_text_file(&path, &serde_json::to_string_pretty(value).unwrap())?,
        "html" => weflow_core::export::export_html(name, value, &json!([]), &path)
            .map_err(|err| AppError::runtime(err.to_string()))?,
        "excel" | "xlsx" => weflow_core::export::export_excel(value, &path)
            .map_err(|err| AppError::runtime(err.to_string()))?,
        "sql" => weflow_core::export::export_sql(name, value, &path)
            .map_err(|err| AppError::runtime(err.to_string()))?,
        "chatlab" => weflow_core::export::export_chatlab(name, value, &json!([]), &path)
            .map_err(|err| AppError::runtime(err.to_string()))?,
        "weclone" => weflow_core::export::export_weclone("", value, &json!([]), &path)
            .map_err(|err| AppError::runtime(err.to_string()))?,
        _ => unreachable!(),
    }
    Ok(json!({ "out": path, "format": format }))
}

fn export_path(out: &Path, default_name: &str) -> PathBuf {
    if out.extension().is_some() {
        out.to_path_buf()
    } else {
        out.join(default_name)
    }
}

fn write_text_file(path: &Path, text: &str) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            AppError::runtime(format!("failed to create {}: {err}", parent.display()))
        })?;
    }
    std::fs::write(path, text)
        .map_err(|err| AppError::runtime(format!("failed to write {}: {err}", path.display())))
}

fn write_csv_file(path: &Path, value: &Value) -> AppResult<()> {
    let rows = value.as_array().ok_or_else(|| {
        AppError::runtime("CSV export expects an array; use --format json for nested data")
    })?;
    let mut headers = Vec::<String>::new();
    for row in rows {
        if let Some(object) = row.as_object() {
            for key in object.keys() {
                if !headers.contains(key) {
                    headers.push(key.clone());
                }
            }
        }
    }

    let mut out = String::new();
    if headers.is_empty() {
        out.push_str("index,value\n");
        for (idx, row) in rows.iter().enumerate() {
            out.push_str(&format!("{idx},{}\n", csv_escape(&row.to_string())));
        }
    } else {
        out.push_str(
            &headers
                .iter()
                .map(|header| csv_escape(header))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
        for row in rows {
            let Some(object) = row.as_object() else {
                continue;
            };
            out.push_str(
                &headers
                    .iter()
                    .map(|header| {
                        object
                            .get(header)
                            .map(|value| {
                                value
                                    .as_str()
                                    .map(ToString::to_string)
                                    .unwrap_or_else(|| value.to_string())
                            })
                            .map(|value| csv_escape(&value))
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            );
            out.push('\n');
        }
    }
    write_text_file(path, &out)
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn filter_named_items(value: Value, needles: &[String]) -> Value {
    let Some(items) = value.as_array() else {
        return value;
    };
    Value::Array(
        items
            .iter()
            .filter(|item| {
                let text = item.to_string();
                needles.iter().any(|needle| text.contains(needle))
            })
            .cloned()
            .collect(),
    )
}

fn handle_backup(command: &BackupCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        BackupSubcommand::Create {
            out,
            no_images,
            no_voice,
            no_emojis,
        } => hub.backup_create(out, !no_images, !no_voice, !no_emojis),
        BackupSubcommand::Inspect { path } => hub.backup_inspect(path),
        BackupSubcommand::Restore { path, target } => {
            let target_dir = target.as_deref().unwrap_or_else(|| Path::new("."));
            hub.backup_restore(path, target_dir)
        }
    }
}

fn parse_config_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn print_response<T: serde::Serialize>(response: &T, cli: &Cli) {
    let mut value = serde_json::to_value(response).unwrap();
    weflow_core::locale::localize_json(&mut value);
    if cli.json {
        println!("{}", serde_json::to_string(&value).unwrap());
    } else {
        // Human-readable: the payload itself, without the {success, data} envelope.
        let data = value.get("data").cloned().unwrap_or(Value::Null);
        match key_summary(&cli.command, &data) {
            Some(text) => print!("{text}"),
            None => print!("{}", weflow_core::render::render(&data)),
        }
        if let Some(meta) = value.get("meta") {
            print!("\n{}", weflow_core::render::render(meta));
        }
    }
}

/// `key db` / `key image` print the keys under the names `config set` expects, so they can be copied over.
fn key_summary(command: &Commands, data: &Value) -> Option<String> {
    use weflow_core::locale::tr;
    if let Commands::Config(ConfigCommand { command }) = command {
        return match command {
            // saving a setting succeeds silently; the config file path is not news
            ConfigSubcommand::Set { .. }
            | ConfigSubcommand::Unset { .. }
            | ConfigSubcommand::Clear => Some(String::new()),
            ConfigSubcommand::Import { .. } => {
                let mut shown = data.clone();
                shown.as_object_mut()?.remove("configPath");
                Some(weflow_core::render::render(&shown))
            }
            _ => None,
        };
    }
    if let Commands::Db(DbCommand {
        command: DbSubcommand::Detect,
    }) = command
    {
        // only the directories that exist, named like the setting that takes them
        let found: Vec<&str> = data["candidates"]
            .as_array()?
            .iter()
            .filter(|c| c["exists"] == true)
            .filter_map(|c| c["path"].as_str())
            .collect();
        return Some(if found.is_empty() {
            format!(
                "{}\n",
                tr("No WeChat data directory found", "未找到微信数据目录")
            )
        } else {
            found.iter().map(|p| format!("db_path: {p}\n")).collect()
        });
    }
    if let Commands::Db(DbCommand {
        command: DbSubcommand::Wxid { .. },
    }) = command
    {
        let accounts = data["accounts"].as_array()?;
        return Some(match accounts.as_slice() {
            [one] => format!("wxid: {}\n", one["wxid"].as_str()?),
            many => many
                .iter()
                .map(|a| {
                    format!(
                        "wxid: {}  {}\n",
                        a["wxid"].as_str().unwrap_or(""),
                        a["path"].as_str().unwrap_or("")
                    )
                })
                .collect(),
        });
    }
    let Commands::Key(KeyCommand { command }) = command else {
        return None;
    };
    match command {
        KeySubcommand::Db { .. } => {
            let key = data["decrypt_key"].as_str()?;
            Some(format!(
                "{}\ndecrypt_key: {key}\n",
                tr("Database key obtained", "已获取数据库密钥")
            ))
        }
        KeySubcommand::Image { .. } => {
            let xor = &data["image_xor_key"];
            let aes = data["image_aes_key"].as_str()?;
            let mut text = format!(
                "{}\nimage_xor_key: {}\nimage_aes_key: {aes}\n",
                tr("Image keys obtained", "已获取图片密钥"),
                xor.as_i64()
                    .map_or_else(|| xor.to_string(), |n| n.to_string())
            );
            if data["verified"] == false {
                text.push_str(tr(
                    "(not verified: no .dat template was found, the keys may be wrong)\n",
                    "（未验证：没有找到 .dat 模板，密钥可能不正确）\n",
                ));
            }
            Some(text)
        }
        KeySubcommand::ScanImage { .. } => None,
    }
}

fn print_failure(err: &AppError, cli: &Cli) {
    if cli.json {
        print_response(&failure(err.payload()), cli);
    } else {
        let details = err.details.as_ref();
        eprint!(
            "{}",
            weflow_core::render::render_error(&err.message, details)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_validated() {
        assert!(parse_date_local("2026-09-24").is_ok());
        assert!(parse_date_local("2024-02-29").is_ok());
        for bad in [
            "2026-13-40",
            "2026-02-29",
            "2026-04-31",
            "2026-00-10",
            "1969-01-01",
            "abc",
            "2026-09",
        ] {
            assert!(parse_date_local(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn range_ends_are_the_next_local_midnight() {
        let (b, e) = date_range_args(Some("2026-09-30"), Some("2026-09-30")).unwrap();
        let day = e.unwrap() - b.unwrap();
        assert!(
            (23 * 3600..=25 * 3600).contains(&day),
            "one local day: {day}"
        );
        assert_eq!(date_range_args(None, None).unwrap(), (None, None));
    }

    #[test]
    fn ranges_must_be_ordered() {
        assert!(date_range_args(Some("2026-09-01"), Some("2026-09-30")).is_ok());
        assert!(
            date_range_args(Some("2026-09-30"), Some("2026-09-30")).is_ok(),
            "the end day is inclusive"
        );
        assert!(date_range_args(Some("2026-09-30"), Some("2026-09-01")).is_err());
        assert!(date_range_args(None, Some("2026-09-01")).is_ok());
    }
}
