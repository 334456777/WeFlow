use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};

mod i18n;
mod serve_api;
use serde_json::{json, Value};
use tracing_subscriber::EnvFilter;
use weflow_core::config::{old_electron_config_candidates, AppContext, ConfigStore};
use weflow_core::dates::date_range_args;
use weflow_core::error::{AppError, AppResult};
use weflow_core::output::{failure, success};
use weflow_core::reads::{image_variant, MediaBody, MediaFile, MediaRead, MessagePage, Read};
use weflow_core::services::{CachePart, ServiceHub};

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

const VERSION: &str = env!("WEFLOW_VERSION");

#[derive(Parser, Debug)]
#[command(name = "weflow", version = concat!(env!("WEFLOW_VERSION"), " (", env!("WEFLOW_BUILD_INFO"), ")"), about = "Native CLI for WeFlow")]
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
    /// Install and locate the ffmpeg for the few WXGF images the built-in decoder cannot read
    Ffmpeg(FfmpegCommand),
    /// Download, pick and remove versions of the WebUI that `serve` shows
    Web(WebCommand),
    /// Save the output language
    Lang {
        /// Output language
        #[arg(value_enum)]
        lang: LangArg,
    },
}

#[derive(Args, Debug)]
struct WebCommand {
    #[command(subcommand)]
    command: WebSubcommand,
}

#[derive(Subcommand, Debug)]
enum WebSubcommand {
    /// The WebUI `serve` shows: the version in use, the one picked, the default one of this weflow, and where they are kept
    Info,
    /// WebUI versions kept on this machine, or published ones (--remote) and whether they suit this weflow
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
    /// Pick the WebUI version `serve` shows (downloaded first when needed)
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
    /// Show the ffmpeg those WXGF images use and where it was found (FFMPEG_PATH, PATH, installed or missing)
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
    /// Show each part of the cache: the option of `cache clear` that removes it, its size and where it is
    List,
    /// Remove the chosen parts of the cache; shows what goes and asks [y/N] first
    Clear(CacheClearArgs),
    /// Remove WeFlow's data of the current account and take the account out of this profile: its cached images,
    /// voices, stickers, Moments, analytics and key fingerprints, then db_path, wxid, decrypt_key and the image keys
    /// of the profile. Shows what goes and asks [y/N] first. WeChat's own files are never touched.
    ClearAccount {
        /// Folder of exports; files and folders named after the account are removed too (repeatable)
        #[arg(long = "exports-dir")]
        exports_dir: Vec<PathBuf>,
        /// Do not ask before removing
        #[arg(short = 'y', long)]
        yes: bool,
        /// Only list what would be removed, with its size; remove nothing
        #[arg(long, conflicts_with = "yes")]
        dry_run: bool,
    },
}

#[derive(Args, Debug)]
struct CacheClearArgs {
    /// Decrypted images
    #[arg(long)]
    images: bool,
    /// Decoded voice messages
    #[arg(long)]
    voices: bool,
    /// Stickers
    #[arg(long)]
    emojis: bool,
    /// Images and videos of Moments
    #[arg(long)]
    sns: bool,
    /// Analytics aggregate
    #[arg(long)]
    analytics: bool,
    /// Media exported and avatars pushed by the HTTP API, and files returned by `serve` (`?return=file`)
    #[arg(long)]
    api: bool,
    /// Fingerprints of keys that worked (the next command checks the key again)
    #[arg(long)]
    keys: bool,
    /// Runtimes unpacked by other WeFlow versions (the running version's stays)
    #[arg(long)]
    runtime: bool,
    /// WebUI versions downloaded by `weflow web` and `serve` (the one `serve` needs is downloaded again)
    #[arg(long)]
    web: bool,
    /// Every part above
    #[arg(long)]
    all: bool,
    /// Do not ask before removing
    #[arg(short = 'y', long)]
    yes: bool,
    /// Only list what would be removed, with its size; remove nothing
    #[arg(long, conflicts_with = "yes")]
    dry_run: bool,
}

impl CacheClearArgs {
    fn parts(&self) -> Vec<CachePart> {
        // in the order of `CachePart::ALL`
        let chosen = [
            self.images,
            self.voices,
            self.emojis,
            self.sns,
            self.analytics,
            self.api,
            self.keys,
            self.runtime,
            self.web,
        ];
        CachePart::ALL
            .into_iter()
            .zip(chosen)
            .filter(|&(_, on)| on || self.all)
            .map(|(part, _)| part)
            .collect()
    }
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
    /// List the profiles: account, data directory and whether the keys are set (never the keys themselves)
    Profiles,
    /// Show one key (or all when omitted)
    Get {
        /// Key name (default: show every key)
        key: Option<String>,
    },
    /// Set a key
    Set {
        #[arg(
            help = "Key name; `<profile>.<key>` (for example `work.wxid`) sets a key of another profile\n  db_path                  WeChat data directory (`db detect` finds it)\n  wxid                     Account wxid (`db wxid` prints it)\n  decrypt_key              Database key in hex (`key db` prints it)\n  image_xor_key            Image XOR key, a number (`key image` prints it)\n  image_aes_key            Image AES key (`key image` prints it)\n  cache_path               Folder for cached images, voices, stickers and Moments (default: `cache` next to the config file)\n  http_api_token           Access token of the HTTP API (`serve`)\n  http_api_host            Listen address of `serve --desktop` (default 127.0.0.1)\n  http_api_port            Listen port of `serve --desktop` (default 5031)\n  ai_model_api_base_url    Base URL of the AI model API used by `insight`\n  ai_model_api_key         API key of the AI model API\n  ai_model_api_model       Model name\n  ai_model_api_max_tokens  Most tokens in one AI reply\n  ai_insight_enabled       true or false: turn AI insight on or off\n  log_enabled              true or false; kept for the desktop app, the CLI does not read it\n  no_progress              true or false: true hides the automatic progress bar\n  progress_delay_seconds   Seconds a command runs before the progress bar appears (default 5; 0 = always)\nSettings of the config file itself (stored in that file, not in a profile; each config file has its own):\n  lang                     Output language, en or zh (the same as `weflow lang`)\n  current_profile          Active profile name, created when it does not exist (default: default)\nWhich config file is used (kept next to the default location, not in any config file):\n  config_path              Config file used from now on; `config unset config_path` or the default path switches back"
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
        /// Save the key to the active profile instead of printing it
        #[arg(long)]
        save: bool,
    },
    /// Derive the image keys from WeChat's kvcomm cache (verified against a .dat template)
    Image {
        /// Account directory to search for templates (default: the configured account directory)
        #[arg(long)]
        user_dir: Option<String>,
        /// kvcomm directories (repeat for net, net_1, etc.; default on Windows: the existing xwechat net*/kvcomm directories)
        #[arg(long, action = clap::ArgAction::Append)]
        kvcomm_dir: Vec<PathBuf>,
        /// Maximum filesystem entries scanned for templates in the selected account (default: 10000)
        #[arg(long)]
        scan_budget: Option<usize>,
        /// Save the keys to the active profile instead of printing them
        #[arg(long)]
        save: bool,
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
        /// Each hit parsed as the desktop app shows it (type, text, quote, media, link, file, …) instead of the database row
        #[arg(long)]
        parsed: bool,
    },
    /// List all contacts
    Contacts {
        /// The contact list as the desktop app shows it (friends, groups, official accounts, former friends, with names, labels, region, signature and avatar) instead of the database rows
        #[arg(long)]
        parsed: bool,
    },
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
        local_id: Option<i64>,
        /// Server message id
        #[arg(long)]
        server_id: Option<String>,
        /// Create time (seconds) of the message, with --local-id: tells apart messages of different message databases that share the local id
        #[arg(long, requires = "local_id")]
        create_time: Option<i64>,
        /// The message alone, parsed as `chat messages --parsed` gives it, with its `_cursor`
        #[arg(long)]
        parsed: bool,
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
        /// Cursor returned as `nextCursor` by the previous page
        #[arg(long)]
        before: Option<String>,
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
        /// Message create time (seconds): tells apart messages of different message databases that share the local id
        #[arg(long)]
        create_time: Option<i64>,
        /// WeChat's own thumbnail of the image (a few KB, for lists), or the image when there is none
        #[arg(long, conflicts_with = "variant")]
        thumb: bool,
        /// Which file of the image: best (the best on this computer), thumb (the thumbnail of a chat), normal (the image as opened), hd (the original WeChat downloads on View original; only when it has)
        #[arg(long, value_parser = ["best", "thumb", "normal", "hd"], default_value = "best")]
        variant: String,
        /// Output file
        #[arg(long)]
        out: PathBuf,
    },
    /// Which files of a message's image are on this computer (thumbnail, image, HD original) and their sizes
    ImageInfo {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        msg_id: String,
        /// Message create time (seconds): tells apart messages of different message databases that share the local id
        #[arg(long)]
        create_time: Option<i64>,
    },
    /// Copy the video of one message (as WeChat keeps it, MP4) to a file
    VideoData {
        /// Conversation id: the other party's wxid, or xxx@chatroom for a group
        session_id: String,
        /// Local message id
        msg_id: String,
        /// Message create time (seconds): tells apart messages of different message databases that share the local id
        #[arg(long)]
        create_time: Option<i64>,
        /// WeChat's own thumbnail of the video, or its cover when there is none, instead of the video
        #[arg(long)]
        thumb: bool,
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
    /// Only messages older than this one: the `_cursor` of the last message of the previous page (as fast however far back)
    #[arg(long, conflicts_with_all = ["offset", "after"])]
    before: Option<String>,
    /// Only messages newer than this one (the `_cursor` of a message): the closest ones, still newest first
    #[arg(long, conflicts_with = "offset")]
    after: Option<String>,
    /// The messages from where this time (Unix seconds) falls in the chat's order: the earliest ones, still newest first, as when a chat window jumps to a day
    #[arg(long, conflicts_with_all = ["offset", "before", "after"])]
    since: Option<i64>,
    /// Each message parsed as the desktop app shows it (type, text, quote, media, link, file, …) instead of the database row
    #[arg(long)]
    parsed: bool,
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
        /// The contacts as `chat contacts --parsed` lists them instead of the database rows
        #[arg(long)]
        parsed: bool,
        /// Only these kinds of contact (repeat for several; with --parsed)
        #[arg(long, requires = "parsed", value_parser = ["friend", "group", "official", "former_friend"], action = clap::ArgAction::Append)]
        kind: Vec<String>,
    },
    /// Export the footprint statistics
    Footprint {
        /// Export format: json (default), or csv or excel for the timeline (a row for each private chat and each @ of you)
        #[arg(long)]
        format: Option<String>,
        /// Output file
        #[arg(long)]
        out: PathBuf,
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
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
        /// Output file; with several formats a folder, or with --zip the zip (a folder: <conversation>.zip in it)
        #[arg(long)]
        out: PathBuf,
        /// txt (default), json, arkme-json, chatlab, chatlab-jsonl, excel, weclone, html, sql; several comma separated
        #[arg(long, value_delimiter = ',', default_value = "txt")]
        format: Vec<String>,
        /// Pack the export (every format, and the media folder) into one zip
        #[arg(long)]
        zip: bool,
        /// Only export messages sent by this person: their bare wxid, as `chat contacts` lists it (not an account folder name with a `_xxxx` suffix)
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
        /// Number of the WeChat Pay account's messages to read (0 = all)
        #[arg(long, default_value_t = 50)]
        limit: i32,
        /// Number of the account's messages to skip (`hasMore` says whether more follow)
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
    Footprint {
        /// Start date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date, local time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
    },
    /// AI footprint summary; takes the JSON payload {rangeLabel, summary, privateSegments, mentionGroups}
    FootprintSummary {
        /// JSON payload
        payload_json: String,
    },
}

#[derive(Args, Debug)]
struct ServeCommand {
    /// Serve the desktop app's HTTP API (`/api/v1/*`) instead of the command API
    #[arg(long)]
    desktop: bool,
    /// Push new messages to HTTP clients (SSE; needs --desktop)
    #[arg(long)]
    message_push: bool,
    /// Run the AI insight engine
    #[arg(long)]
    insight: bool,
    /// Run the image auto-download hook (Windows only)
    #[arg(long)]
    image_auto_download: bool,
    /// Listen address (default: 127.0.0.1; with --desktop: `http_api_host` from the config, else 127.0.0.1). On a loopback address the command API only answers requests to localhost, 127.0.0.1 or [::1].
    #[arg(long)]
    host: Option<String>,
    /// Listen port (default: 18108; with --desktop: `http_api_port` from the config, else 5031).
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
    ui_dir: Option<PathBuf>,
    /// Do not open the WebUI in the browser when the server starts
    #[arg(long)]
    no_browser: bool,
    /// Show this WebUI version for this run (downloaded first when needed), instead of the one picked with `weflow web use`
    #[arg(long, value_name = "VERSION", conflicts_with = "ui_dir")]
    web_version: Option<String>,
    /// Serve the API alone, without the WebUI
    #[arg(long, conflicts_with_all = ["ui_dir", "web_version", "no_browser"])]
    api_only: bool,
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
    // a command run by `serve` gets the database keys the server already derived on stdin (see `serve_api::spawn`)
    if std::env::var_os(serve_api::KEYS_ON_STDIN).is_some() {
        let mut stdin = std::io::stdin();
        let mut keys = Vec::new();
        if !std::io::IsTerminal::is_terminal(&stdin)
            && std::io::Read::read_to_end(&mut stdin, &mut keys).is_ok()
        {
            weflow_core::page_keys::import_keys(&keys);
        }
    }
    weflow_core::output::set_json_output(cli.json);
    weflow_core::output::set_progress_mode(if cli.progress {
        weflow_core::output::ProgressMode::Ndjson
    } else {
        weflow_core::output::ProgressMode::Auto
    });
    let outcome = run(&cli).await;
    weflow_core::output::finish_progress();
    if cli.json {
        let (doc, exit) = json_document(outcome);
        println!("{}", serde_json::to_string(&doc).unwrap());
        return ExitCode::from(exit);
    }
    match checked(outcome) {
        Ok(value) => {
            print_response(&success(value), &cli);
            ExitCode::SUCCESS
        }
        Err(err) => {
            print_failure(&err);
            ExitCode::from(err.exit_code as u8)
        }
    }
}

/// The hub `run` builds, and the config file it was built from: the command API's in-process routes keep one
/// open and build it again when that file changes.
fn open_hub() -> AppResult<(ServiceHub, PathBuf)> {
    let ctx = AppContext::new(None, VERSION)?;
    let config =
        ConfigStore::load(&ctx.config_path).map_err(|err| AppError::config(err.to_string()))?;
    let path = ctx.config_path.clone();
    Ok((ServiceHub::new(ctx, config, None, None, None, None), path))
}

/// The commands that are a [`Read`] of `weflow_core::reads` (#397): the CLI runs them as such, and the command API's
/// in-process routes (`/api/…`, issue #140) answer the same on a hub that stays open.
fn read_of(command: &Commands) -> Option<Read> {
    Some(match command {
        Commands::Chat(c) => match &c.command {
            ChatSubcommand::Sessions { limit } => Read::ChatSessions { limit: *limit },
            ChatSubcommand::Messages(a) => Read::ChatMessages(MessagePage {
                session_id: a.session_id.clone(),
                limit: a.limit,
                offset: a.offset,
                before: a.before.clone(),
                after: a.after.clone(),
                since: a.since,
                parsed: a.parsed,
            }),
            ChatSubcommand::Latest { session_id, limit } => Read::ChatLatest {
                session_id: session_id.clone(),
                limit: *limit,
            },
            ChatSubcommand::Message {
                session_id,
                local_id,
                server_id,
                create_time,
                parsed,
            } => Read::ChatMessage {
                session_id: session_id.clone(),
                local_id: *local_id,
                server_id: server_id.clone(),
                create_time: *create_time,
                parsed: *parsed,
            },
            ChatSubcommand::Contact { username } => Read::ChatContact {
                username: username.clone(),
            },
            ChatSubcommand::Contacts { parsed } => Read::ChatContacts { parsed: *parsed },
            ChatSubcommand::Counts { sessions } => Read::ChatCounts {
                sessions: sessions.clone(),
            },
            ChatSubcommand::Search {
                keyword,
                session_id,
                limit,
                offset,
                start,
                end,
                parsed,
            } => Read::ChatSearch {
                keyword: keyword.clone(),
                session_id: session_id.clone(),
                limit: *limit,
                offset: *offset,
                start: *start,
                end: *end,
                parsed: *parsed,
            },
            ChatSubcommand::Dates { session_id } => Read::ChatDates {
                session_id: session_id.clone(),
            },
            ChatSubcommand::DateCounts { session_id } => Read::ChatDateCounts {
                session_id: session_id.clone(),
            },
            ChatSubcommand::Detail {
                session_id,
                fast: true,
                extra: false,
            } => Read::ChatDetailFast {
                session_id: session_id.clone(),
            },
            ChatSubcommand::Resources {
                session,
                types,
                start,
                end,
                limit,
                before,
            } => Read::ChatResources {
                session: session.clone(),
                types: types.clone(),
                start: start.clone(),
                end: end.clone(),
                limit: *limit,
                before: before.clone(),
            },
            ChatSubcommand::ImageInfo {
                session_id,
                msg_id,
                create_time,
            } => Read::ChatImageInfo {
                session_id: session_id.clone(),
                msg_id: msg_id.clone(),
                create_time: *create_time,
            },
            ChatSubcommand::Statuses { usernames } => Read::ChatStatuses {
                usernames: usernames.clone(),
            },
            ChatSubcommand::TabCounts => Read::ChatTabCounts,
            _ => return None,
        },
        Commands::Group(c) => match &c.command {
            GroupSubcommand::List => Read::GroupList,
            GroupSubcommand::Members {
                chatroom_id,
                counts,
                refresh,
            } => Read::GroupMembers {
                chatroom_id: chatroom_id.clone(),
                counts: *counts,
                refresh: *refresh,
            },
            GroupSubcommand::Ranking {
                chatroom_id,
                limit,
                start,
                end,
            } => Read::GroupRanking {
                chatroom_id: chatroom_id.clone(),
                limit: *limit,
                start: start.clone(),
                end: end.clone(),
            },
            GroupSubcommand::Hours {
                chatroom_id,
                start,
                end,
            } => Read::GroupHours {
                chatroom_id: chatroom_id.clone(),
                start: start.clone(),
                end: end.clone(),
            },
            GroupSubcommand::Media {
                chatroom_id,
                start,
                end,
            } => Read::GroupMedia {
                chatroom_id: chatroom_id.clone(),
                start: start.clone(),
                end: end.clone(),
            },
            GroupSubcommand::Member {
                chatroom_id,
                username,
                start,
                end,
            } => Read::GroupMember {
                chatroom_id: chatroom_id.clone(),
                username: username.clone(),
                start: start.clone(),
                end: end.clone(),
            },
            GroupSubcommand::MemberMessages {
                chatroom_id,
                username,
                limit,
                cursor,
                start,
                end,
            } => Read::GroupMemberMessages {
                chatroom_id: chatroom_id.clone(),
                username: username.clone(),
                limit: *limit,
                cursor: *cursor,
                start: start.clone(),
                end: end.clone(),
            },
            _ => return None,
        },
        Commands::Analytics(c) => match &c.command {
            AnalyticsSubcommand::Overall { force } => Read::AnalyticsOverall { force: *force },
            AnalyticsSubcommand::Rankings { limit, start, end } => Read::AnalyticsRankings {
                limit: *limit,
                start: start.clone(),
                end: end.clone(),
            },
            AnalyticsSubcommand::Time => Read::AnalyticsTime,
            // `--set` saves the list
            AnalyticsSubcommand::Excluded { set: None } => Read::AnalyticsExcluded,
            AnalyticsSubcommand::ExcludeCandidates => Read::AnalyticsExcludeCandidates,
            _ => return None,
        },
        Commands::Biz(c) => match &c.command {
            BizSubcommand::Accounts => Read::BizAccounts,
            BizSubcommand::Messages {
                username,
                limit,
                offset,
            } => Read::BizMessages {
                username: username.clone(),
                limit: *limit,
                offset: *offset,
            },
            BizSubcommand::PayRecords { limit, offset } => Read::BizPayRecords {
                limit: *limit,
                offset: *offset,
            },
        },
        Commands::Insight(c) => match &c.command {
            InsightSubcommand::Footprint { start, end } => Read::InsightFootprint {
                start: start.clone(),
                end: end.clone(),
            },
            InsightSubcommand::Get { id } => Read::InsightGet { id: id.clone() },
            _ => return None,
        },
        Commands::Report(c) => match &c.command {
            ReportSubcommand::Annual {
                command: AnnualSubcommand::Years,
            } => Read::ReportAnnualYears,
            _ => return None,
        },
        Commands::Cache(c) => match &c.command {
            CacheSubcommand::List => Read::CacheList,
            _ => return None,
        },
        Commands::Config(c) => match &c.command {
            ConfigSubcommand::Get { key } => Read::ConfigGet { key: key.clone() },
            ConfigSubcommand::Profiles => Read::ConfigProfiles,
            _ => return None,
        },
        Commands::Runtime(c) => match &c.command {
            RuntimeSubcommand::Info => Read::RuntimeInfo,
            _ => return None,
        },
        Commands::Db(c) => match &c.command {
            DbSubcommand::Detect => Read::DbDetect,
            DbSubcommand::Test | DbSubcommand::Open => Read::DbTest,
            DbSubcommand::Wxid { root } => Read::DbWxid { root: root.clone() },
            _ => return None,
        },
        Commands::Sns(c) => match &c.command {
            // the timeline without its media, which are downloaded
            SnsSubcommand::Timeline {
                limit,
                offset,
                users,
                keyword,
                start,
                end,
                with_media: false,
            } => Read::SnsTimeline {
                limit: *limit,
                offset: *offset,
                users: users.clone(),
                keyword: keyword.clone(),
                start: *start,
                end: *end,
            },
            SnsSubcommand::Users => Read::SnsUsers,
            SnsSubcommand::Stats { fast } => Read::SnsStats { fast: *fast },
            SnsSubcommand::PostCounts { user, prefer_cache } => Read::SnsPostCounts {
                user: user.clone(),
                prefer_cache: *prefer_cache,
            },
            _ => return None,
        },
        _ => return None,
    })
}

/// The commands that write a message's file, as a [`MediaRead`]: the `/api/` media routes answer the file itself.
fn media_of(command: &ChatSubcommand) -> Option<MediaRead> {
    Some(match command {
        ChatSubcommand::ImageData {
            session_id,
            msg_id,
            create_time,
            thumb,
            variant,
            ..
        } => MediaRead::ChatImage {
            session_id: session_id.clone(),
            msg_id: msg_id.clone(),
            create_time: *create_time,
            variant: image_variant(*thumb, variant),
        },
        ChatSubcommand::VoiceData {
            session_id,
            msg_id,
            create_time,
            server_id,
            sender,
            ..
        } => MediaRead::ChatVoice {
            session_id: session_id.clone(),
            msg_id: msg_id.clone(),
            create_time: *create_time,
            server_id: server_id.clone(),
            sender: sender.clone(),
        },
        ChatSubcommand::VideoData {
            session_id,
            msg_id,
            create_time,
            thumb,
            ..
        } => MediaRead::ChatVideo {
            session_id: session_id.clone(),
            msg_id: msg_id.clone(),
            create_time: *create_time,
            thumb: *thumb,
        },
        _ => return None,
    })
}

/// Writes a media file to `out`, making its folder first; the number of bytes written.
fn write_media(file: MediaFile, out: &Path) -> AppResult<u64> {
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::runtime(format!("failed to create {}: {e}", parent.display()))
        })?;
    }
    let written = match file.body {
        MediaBody::Bytes(bytes) => std::fs::write(out, &bytes).map(|()| bytes.len() as u64),
        MediaBody::File(from) => std::fs::copy(from, out),
    };
    written.map_err(|e| AppError::runtime(format!("failed to write {}: {e}", out.display())))
}

async fn run(cli: &Cli) -> AppResult<Value> {
    let ctx = AppContext::new(None, VERSION)?;
    let mut config =
        ConfigStore::load(&ctx.config_path).map_err(|err| AppError::config(err.to_string()))?;

    apply_progress_settings(&config, cli);
    weflow_core::ffmpeg::set_home(&ctx.home_dir);

    if let Some(read) = read_of(&cli.command) {
        return read.run(&ServiceHub::new(ctx, config, None, None, None, None));
    }
    match &cli.command {
        Commands::Config(command) => return handle_config(command, &ctx, &mut config),
        Commands::Lang { lang } => {
            let code = match lang {
                LangArg::En => "en",
                LangArg::Zh => "zh",
            };
            ConfigStore::update(&ctx.config_path, |config| {
                config.lang = Some(code.to_string());
                Ok(())
            })?;
            return Ok(json!({ "lang": code }));
        }
        Commands::Runtime(command) => return handle_runtime(command, &ctx),
        Commands::Ffmpeg(command) => return handle_ffmpeg(command, &ctx, &mut config).await,
        Commands::Cache(command) => return handle_cache(command, &ctx, &mut config),
        _ => {}
    }

    let config_path = ctx.config_path.clone();
    let hub = ServiceHub::new(ctx, config, None, None, None, None);
    match &cli.command {
        Commands::Db(command) => handle_db(command, &hub),
        Commands::Chat(command) => handle_chat(command, &hub).await,
        Commands::Key(command) => handle_key(command, &hub, &config_path),
        Commands::Export(command) => handle_export(command, &hub).await,
        Commands::Analytics(command) => handle_analytics(command, &hub),
        Commands::Group(command) => handle_group(command, &hub),
        Commands::Report(command) => handle_report(command, &hub),
        Commands::Sns(command) => handle_sns(command, &hub).await,
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
        Commands::Web(command) => handle_web(command, &hub).await,
        Commands::Backup(command) => handle_backup(command, &hub),
        Commands::Biz(_) => unreachable!("a read"),
        Commands::Runtime(_)
        | Commands::Ffmpeg(_)
        | Commands::Cache(_)
        | Commands::Config(_)
        | Commands::Lang { .. } => unreachable!(),
    }
}

/// `weflow cache …`; `clear-account` also changes the profile, so this runs before the shared hub is built.
fn handle_cache(
    command: &CacheCommand,
    ctx: &AppContext,
    config: &mut ConfigStore,
) -> AppResult<Value> {
    use weflow_core::locale::tr;
    use weflow_core::services::disk_usage;

    let hub = ServiceHub::new(ctx.clone(), config.clone(), None, None, None, None);
    match &command.command {
        CacheSubcommand::List => unreachable!("a read"),
        CacheSubcommand::Clear(args) => {
            let parts = args.parts();
            if parts.is_empty() {
                let options: Vec<String> = CachePart::ALL
                    .iter()
                    .map(|p| format!("--{}", p.name()))
                    .chain(["--all".to_string()])
                    .collect();
                return Err(AppError::usage(format!(
                    "choose what to clear: {} (`weflow cache list` shows each part)",
                    options.join(", ")
                )));
            }
            let targets: Vec<RemovalTarget> = parts
                .iter()
                .flat_map(|&part| {
                    hub.cache_part_paths(part)
                        .into_iter()
                        .filter(|p| p.exists())
                        .map(move |path| RemovalTarget {
                            label: cache_part_label(part),
                            bytes: disk_usage(&path).0,
                            path,
                        })
                })
                .collect();
            if args.dry_run {
                return Ok(removal_plan(&targets, &[]));
            }
            if targets.is_empty() {
                return Ok(json!({ "removedPaths": [], "freedBytes": 0 }));
            }
            if !confirm_removal(tr("About to remove:", "将要清理："), &targets, args.yes)? {
                return Ok(json!({ "cancelled": true }));
            }
            Ok(hub.cache_clear(&parts))
        }
        CacheSubcommand::ClearAccount {
            exports_dir,
            yes,
            dry_run,
        } => {
            let targets: Vec<RemovalTarget> = hub
                .account_data_paths(true, exports_dir)?
                .into_iter()
                .map(|path| RemovalTarget {
                    label: "",
                    bytes: disk_usage(&path).0,
                    path,
                })
                .collect();
            if *dry_run {
                return Ok(removal_plan(&targets, &ACCOUNT_KEYS));
            }
            let heading = tr(
                "About to remove these files and take the account out of the profile (db_path, wxid, decrypt_key and the image keys):",
                "将要删除以下文件，并把当前账号移出配置档案（db_path、wxid、decrypt_key 和图片密钥）：",
            );
            if !confirm_removal(heading, &targets, *yes)? {
                return Ok(json!({ "cancelled": true }));
            }
            let result = hub.clear_current_account_data(true, exports_dir)?;
            // like the desktop app: the account is signed out of this profile
            ConfigStore::update(&ctx.config_path, |config| {
                for key in ACCOUNT_KEYS {
                    config.unset_key(None, key);
                }
                Ok(())
            })?;
            Ok(result)
        }
    }
}

/// The keys `cache clear-account` takes out of the profile.
const ACCOUNT_KEYS: [&str; 5] = [
    "db_path",
    "wxid",
    "decrypt_key",
    "image_xor_key",
    "image_aes_key",
];

/// `--dry-run` (#215): what `cache clear` / `cache clear-account` would remove, and the profile keys it would unset,
/// for a page that asks before it removes anything.
fn removal_plan(targets: &[RemovalTarget], keys: &[&str]) -> Value {
    json!({
        "dryRun": true,
        "paths": targets.iter().map(|t| json!({ "path": t.path, "bytes": t.bytes })).collect::<Vec<_>>(),
        "bytes": targets.iter().map(|t| t.bytes).sum::<u64>(),
        "profileKeys": keys,
    })
}

/// A file or folder `cache clear` / `cache clear-account` is about to remove.
struct RemovalTarget {
    label: &'static str,
    bytes: u64,
    path: PathBuf,
}

fn cache_part_label(part: CachePart) -> &'static str {
    use weflow_core::locale::tr;
    match part {
        CachePart::Images => tr("images", "图片"),
        CachePart::Voices => tr("voices", "语音"),
        CachePart::Emojis => tr("stickers", "表情"),
        CachePart::Sns => tr("Moments", "朋友圈"),
        CachePart::Analytics => tr("analytics", "统计"),
        CachePart::Api => tr("HTTP API", "API 导出"),
        CachePart::Keys => tr("key fingerprints", "密钥指纹"),
        CachePart::Runtime => tr("older runtimes", "旧运行时"),
        CachePart::Web => tr("WebUI versions", "网页版本"),
    }
}

/// Lists `targets` on stderr under `heading` and asks [y/N]; only `y` or `yes` goes ahead, anything else (Enter
/// included) is a no. `yes` skips the question; without a terminal to ask on, it is required.
fn confirm_removal(heading: &str, targets: &[RemovalTarget], yes: bool) -> AppResult<bool> {
    use std::io::{IsTerminal, Write};
    use weflow_core::locale::tr;

    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(AppError::usage("this removes files; add --yes to confirm"));
    }
    let mut text = format!("{heading}\n");
    if targets.is_empty() {
        text.push_str(&format!("  {}\n", tr("(no files)", "（没有文件）")));
    }
    let rows: Vec<Vec<String>> = targets
        .iter()
        .map(|t| {
            vec![
                t.label.to_string(),
                format_bytes(t.bytes),
                t.path.display().to_string(),
            ]
        })
        .collect();
    for line in align_columns(&rows) {
        text.push_str(&format!("  {line}\n"));
    }
    text.push_str(tr("Remove them? [y/N] ", "确认清理？[y/N] "));
    let mut err = std::io::stderr();
    let _ = err.write_all(text.as_bytes());
    let _ = err.flush();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|e| AppError::runtime(format!("failed to read the answer: {e}")))?;
    Ok(is_yes(&answer))
}

fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// `1536` → `1.5 KB`: binary units, one decimal below 10.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// Pads the cells of `rows` into columns two spaces apart (CJK counts double); empty columns are left out. A cell
/// of several values (one per line, `\n` between them) takes as many lines, each starting under its column's
/// header (issue #258).
fn align_columns(rows: &[Vec<String>]) -> Vec<String> {
    use weflow_core::render::display_width;
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|i| {
            rows.iter()
                .filter_map(|r| r.get(i))
                .flat_map(|c| c.lines())
                .map(display_width)
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = Vec::new();
    for row in rows {
        let cells: Vec<Vec<&str>> = row.iter().map(|c| c.lines().collect()).collect();
        let height = cells.iter().map(Vec::len).max().unwrap_or(0).max(1);
        for line in 0..height {
            let text = cells
                .iter()
                .zip(&widths)
                .filter(|&(_, &w)| w > 0)
                .map(|(cell, &w)| {
                    let part = cell.get(line).copied().unwrap_or("");
                    format!("{part}{}", " ".repeat(w - display_width(part)))
                })
                .collect::<Vec<_>>()
                .join("  ");
            out.push(text.trim_end().to_string());
        }
    }
    out
}

/// `text` in green on a terminal; plain when stdout is not one or NO_COLOR is set (https://no-color.org).
fn green(text: &str) -> String {
    use std::io::IsTerminal;
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
        && std::io::stdout().is_terminal()
        && enable_ansi();
    if color {
        format!("\x1b[32m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// The Windows console shows ANSI colours only once asked to.
#[cfg(windows)]
fn enable_ansi() -> bool {
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        STD_OUTPUT_HANDLE,
    };
    // SAFETY: console calls on this process's own stdout handle; `mode` outlives the call that writes it
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        GetConsoleMode(handle, &mut mode) != 0
            && (mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
                || SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0)
    }
}

#[cfg(not(windows))]
fn enable_ansi() -> bool {
    true
}

/// `cache list` as a table under a green header; `cache clear` / `clear-account`: what was removed.
fn cache_summary(command: &CacheSubcommand, data: &Value) -> Option<String> {
    use weflow_core::locale::tr;
    match command {
        CacheSubcommand::List => {
            let header = [
                tr("Part", "部分"),
                tr("Option", "选项"),
                tr("Size", "大小"),
                tr("Path", "路径"),
            ];
            let mut rows: Vec<Vec<String>> = vec![header.iter().map(|h| h.to_string()).collect()];
            for (part, item) in CachePart::ALL.iter().zip(data.as_array()?) {
                let paths: Vec<&str> = item["paths"]
                    .as_array()?
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                rows.push(vec![
                    cache_part_label(*part).to_string(),
                    format!("--{}", part.name()),
                    format_bytes(item["bytes"].as_u64().unwrap_or(0)),
                    if paths.is_empty() {
                        tr("(none)", "（无）").to_string()
                    } else {
                        // one path a line, under the header
                        paths.join("\n")
                    },
                ]);
            }
            let mut lines = align_columns(&rows).into_iter();
            let mut out = format!("{}\n", green(&lines.next()?));
            lines.for_each(|l| out.push_str(&format!("{l}\n")));
            Some(out)
        }
        CacheSubcommand::Clear(_) | CacheSubcommand::ClearAccount { .. } => {
            if data["dryRun"] == true {
                let paths = data["paths"].as_array()?;
                let mut out = String::from(if paths.is_empty() {
                    tr(
                        "Nothing would be removed.
",
                        "没有需要清理的文件。
",
                    )
                } else {
                    tr(
                        "Would remove:
",
                        "将会清理：
",
                    )
                });
                let rows: Vec<Vec<String>> = paths
                    .iter()
                    .map(|p| {
                        vec![
                            format_bytes(p["bytes"].as_u64().unwrap_or(0)),
                            p["path"].as_str().unwrap_or("").to_string(),
                        ]
                    })
                    .collect();
                for line in align_columns(&rows) {
                    out.push_str(&format!(
                        "  {line}
"
                    ));
                }
                return Some(out);
            }
            if data["cancelled"] == true {
                return Some(format!(
                    "{}\n",
                    tr(
                        "Cancelled; nothing was removed.",
                        "已取消，没有删除任何文件。"
                    )
                ));
            }
            let removed: Vec<&str> = data["removedPaths"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect();
            let mut out = String::new();
            if removed.is_empty() {
                out.push_str(tr("Nothing to remove.\n", "没有需要清理的文件。\n"));
            } else {
                out.push_str(tr("Removed:\n", "已清理：\n"));
                removed
                    .iter()
                    .for_each(|p| out.push_str(&format!("  {p}\n")));
            }
            if let Some(freed) = data["freedBytes"].as_u64().filter(|&b| b > 0) {
                out.push_str(&format!(
                    "{}{}\n",
                    tr("Freed ", "共释放 "),
                    format_bytes(freed)
                ));
            }
            if data["profileReset"] == true {
                out.push_str(tr(
                    "The account was taken out of the profile.\n",
                    "已把当前账号移出配置档案。\n",
                ));
            }
            for warning in data["warnings"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "{}{}\n",
                    tr("warning: ", "警告："),
                    warning.as_str().unwrap_or_default()
                ));
            }
            Some(out)
        }
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
        ConfigSubcommand::Profiles | ConfigSubcommand::Get { .. } => unreachable!("a read"),
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
            ConfigStore::update(&ctx.config_path, |config| config.set_key(None, key, value))?;
            Ok(json!({ "configPath": ctx.config_path }))
        }
        ConfigSubcommand::Unset { key } => {
            if matches!(key.as_str(), "config_path" | "configPath") {
                weflow_core::config::clear_saved_config_path()?;
                return Ok(json!({ "configPath": weflow_core::config::default_config_path() }));
            }
            ConfigStore::update(&ctx.config_path, |config| {
                config.unset_key(None, key);
                Ok(())
            })?;
            Ok(json!({ "configPath": ctx.config_path }))
        }
        ConfigSubcommand::Clear => {
            ConfigStore::update(&ctx.config_path, |config| {
                *config = ConfigStore::default();
                Ok(())
            })?;
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
            let skipped = ConfigStore::update(&ctx.config_path, |config| {
                config
                    .import_electron_config(&path, None)
                    .map_err(|err| AppError::config(err.to_string()))
            })?;
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
            ConfigStore::update(&ctx.config_path, |config| {
                config.ffmpeg_base_url = Some(url.clone());
                Ok(())
            })?;
            Ok(json!({ "baseUrl": url }))
        }
        FfmpegSubcommand::Unset {
            setting: FfmpegUnsetting::Baseurl,
        } => {
            ConfigStore::update(&ctx.config_path, |config| {
                config.ffmpeg_base_url = None;
                Ok(())
            })?;
            Ok(json!({ "baseUrl": weflow_core::ffmpeg::DEFAULT_BASE_URL }))
        }
    }
}

fn handle_runtime(command: &RuntimeCommand, ctx: &AppContext) -> AppResult<Value> {
    match command.command {
        RuntimeSubcommand::Info => unreachable!("a read"),
        RuntimeSubcommand::Manifest => {
            Ok(serde_json::to_value(weflow_assets::manifest(&ctx.version)).unwrap())
        }
    }
}

fn handle_db(command: &DbCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        DbSubcommand::Scan { root } => Ok(hub.db_scan(root)),
        DbSubcommand::Detect
        | DbSubcommand::Test
        | DbSubcommand::Open
        | DbSubcommand::Wxid { .. } => unreachable!("a read"),
    }
}

async fn handle_chat(command: &ChatCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        // run as a `Read` by `run`
        ChatSubcommand::Sessions { .. }
        | ChatSubcommand::Messages(_)
        | ChatSubcommand::Latest { .. }
        | ChatSubcommand::Search { .. }
        | ChatSubcommand::Contacts { .. }
        | ChatSubcommand::Contact { .. }
        | ChatSubcommand::Message { .. }
        | ChatSubcommand::Dates { .. }
        | ChatSubcommand::DateCounts { .. }
        | ChatSubcommand::Counts { .. }
        | ChatSubcommand::Resources { .. }
        | ChatSubcommand::ImageInfo { .. }
        | ChatSubcommand::Statuses { .. }
        | ChatSubcommand::TabCounts => unreachable!("a read"),
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
        ChatSubcommand::Detail {
            session_id,
            fast,
            extra,
        } => match (fast, extra) {
            (false, true) => hub.chat_detail_extra(session_id).map(|t| json!(t)),
            _ => hub.chat_detail(session_id).map(|d| json!(d)),
        },
        ChatSubcommand::MarkRead => hub.chat_mark_all_read(),
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
        ChatSubcommand::VoiceData { out, .. } => {
            let read = media_of(&command.command).expect("a media command");
            let file = read.fetch(hub.clone()).await?;
            match out {
                Some(path) => {
                    let bytes = write_media(file, path)?;
                    Ok(
                        serde_json::json!({ "success": true, "path": path.to_string_lossy(), "bytes": bytes }),
                    )
                }
                None => {
                    use base64::Engine;
                    let MediaBody::Bytes(wav) = file.body else {
                        unreachable!("a voice is decoded in memory")
                    };
                    Ok(
                        serde_json::json!({ "success": true, "data": base64::engine::general_purpose::STANDARD.encode(&wav) }),
                    )
                }
            }
        }
        ChatSubcommand::ImageData { out, .. } | ChatSubcommand::VideoData { out, .. } => {
            let read = media_of(&command.command).expect("a media command");
            let bytes = write_media(read.fetch(hub.clone()).await?, out)?;
            Ok(
                serde_json::json!({ "success": true, "path": out.to_string_lossy(), "bytes": bytes }),
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

/// `--save` (#216): writes the keys a `key` command found to the active profile and leaves them out of its result, so
/// they go from WeChat to the config file without being printed, nor reaching a WebUI through `serve`.
fn save_keys(config_path: &Path, mut data: Value, names: &[&str]) -> AppResult<Value> {
    let fields = data
        .as_object_mut()
        .ok_or_else(|| AppError::runtime("no keys to save"))?;
    let keys = names
        .iter()
        .map(|name| {
            fields
                .remove(*name)
                .filter(|v| !v.is_null())
                .map(|v| (*name, v))
                .ok_or_else(|| AppError::runtime(format!("no {name} to save")))
        })
        .collect::<AppResult<Vec<_>>>()?;
    ConfigStore::update(config_path, |config| {
        keys.into_iter()
            .try_for_each(|(name, value)| config.set_key(None, name, value))
    })?;
    fields.insert("saved".into(), json!(names));
    Ok(data)
}

/// `key db` run by `weflow serve`, which cannot send it Ctrl+C: the end of stdin (the job cancelled, the request or
/// the server gone) interrupts it as Ctrl+C would, so a hook it set is taken out of WeChat before it ends (#287).
fn interrupt_when_stdin_ends() {
    std::thread::spawn(|| {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
        // a hook may be about to be set: interrupt it too, until the command has ended
        loop {
            weflow_core::services::interrupt_key_db();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

fn handle_key(command: &KeyCommand, hub: &ServiceHub, config_path: &Path) -> AppResult<Value> {
    match &command.command {
        KeySubcommand::Db { pid, timeout, save } => {
            if std::env::var_os(serve_api::CANCEL_ON_STDIN).is_some() {
                interrupt_when_stdin_ends();
            }
            let data = hub.key_db(*pid, *timeout)?;
            if *save {
                save_keys(config_path, data, &["decrypt_key"])
            } else {
                Ok(data)
            }
        }
        KeySubcommand::Image {
            user_dir,
            kvcomm_dir,
            scan_budget,
            save,
        } => {
            if !cfg!(windows) {
                return Err(AppError::native(
                    "`key image` is not supported on this platform yet",
                ));
            }
            let mut directories: Vec<_> = kvcomm_dir
                .iter()
                .map(|p| weflow_core::config::expand_home(&p.to_string_lossy()))
                .collect();
            if directories.is_empty() {
                directories = weflow_core::image_keys::default_kvcomm_dirs();
            }
            let data = hub.key_image(
                user_dir.as_deref(),
                &directories,
                scan_budget.unwrap_or(weflow_core::image_keys::DEFAULT_SCAN_BUDGET),
            )?;
            if *save {
                save_keys(config_path, data, &["image_xor_key", "image_aes_key"])
            } else {
                Ok(data)
            }
        }
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
        ExportSubcommand::Contacts {
            format,
            out,
            parsed,
            kind,
        } => {
            let data = if *parsed {
                let mut contacts = hub.chat_contacts_parsed()?;
                if !kind.is_empty() {
                    contacts.retain(|c| kind.iter().any(|k| k == c.kind.name()));
                }
                if contacts.is_empty() {
                    return Err(AppError::runtime("no contacts of the kinds asked for"));
                }
                json!(contacts)
            } else {
                hub.contacts()?
            };
            write_export("contacts", format.as_deref(), out, &data)
        }
        ExportSubcommand::Footprint {
            format,
            out,
            start,
            end,
        } => {
            // the footprint is a summary with lists in it: JSON as it is, CSV and Excel as its timeline; the other
            // formats are for messages, and are refused before anything is read (#315)
            let format = format.as_deref().unwrap_or("json").to_ascii_lowercase();
            let extension = match format.as_str() {
                "json" => "json",
                "csv" => "csv",
                "excel" | "xlsx" => "xlsx",
                other => {
                    return Err(AppError::usage(format!(
                        "export footprint writes json, csv or excel, not {other}"
                    )))
                }
            };
            let (b, e) = day_span(start, end)?;
            let data = hub.footprint(b, e)?;
            if extension == "json" {
                return write_export("footprint", Some("json"), out, &data);
            }
            let (headers, rows) = hub.footprint_timeline(&data)?;
            let path = export_path(out, &format!("footprint.{extension}"));
            make_parent(&path)?;
            if extension == "csv" {
                let cell = |v: &Value| {
                    csv_escape(&v.as_str().map_or_else(|| v.to_string(), str::to_string))
                };
                let mut csv = headers
                    .iter()
                    .map(|h| csv_escape(h))
                    .collect::<Vec<_>>()
                    .join(",")
                    + "\n";
                for row in &rows {
                    csv += &(row.iter().map(cell).collect::<Vec<_>>().join(",") + "\n");
                }
                write_text_file(&path, &csv)?;
            } else {
                weflow_core::export::export_table_excel(&headers, &rows, &path)
                    .map_err(|err| AppError::runtime(err.to_string()))?;
            }
            Ok(json!({ "out": path, "format": format }))
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
            zip,
        } => {
            let (start_ts, end_ts) = date_range_args(start.as_deref(), end.as_deref())?;
            let mut formats: Vec<String> = Vec::new();
            for f in format
                .iter()
                .map(|f| f.trim().to_ascii_lowercase())
                .filter(|f| !f.is_empty())
            {
                if !formats.contains(&f) {
                    formats.push(f);
                }
            }
            let fmt = formats.first().cloned().unwrap_or_else(|| "txt".into());
            let media_opts = parse_media_selection(media)?;
            let parse_display = |s: &str| {
                weflow_core::export_msg::DisplayPref::parse(s).ok_or_else(|| {
                    AppError::usage("--display-name must be group-nickname, remark or nickname")
                })
            };
            let ext_of = |fmt: &str| -> Result<&'static str, AppError> {
                Ok(match fmt {
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
                })
            };
            // json and arkme-json write the same extension: two of them would take one file
            let mut exts: Vec<&str> = Vec::new();
            for f in &formats {
                let e = ext_of(f)?;
                if exts.contains(&e) {
                    return Err(AppError::usage(format!(
                        "--format {f} writes .{e} as another format given does; give one of them"
                    )));
                }
                exts.push(e);
            }
            let ext = exts[0];
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
            if formats.len() == 1 && !*zip {
                return if media_opts.enabled {
                    hub.export_messages_with_media(&request, &target, &media_opts)
                        .await
                } else {
                    hub.export_messages(&request, &target)
                };
            }
            export_messages_bundle(
                hub,
                &request,
                &formats,
                &exts,
                &safe_name,
                out,
                *zip,
                &media_opts,
            )
            .await
        }
    }
}

/// `export messages` in several formats or packed (#342): every format into one folder, the media in its `media/`
/// folder; with `zip`, the folder (beside the zip, taken away after) packed into the zip, whose path is the result's
/// `out`, the formats' files named inside it.
#[allow(clippy::too_many_arguments)]
async fn export_messages_bundle(
    hub: &ServiceHub,
    request: &weflow_core::services::MessageExportRequest,
    formats: &[String],
    exts: &[&str],
    safe_name: &str,
    out: &Path,
    zip: bool,
    media: &weflow_core::api::ApiMediaOptions,
) -> AppResult<Value> {
    let zip_path = zip.then(|| export_path(out, &format!("{safe_name}.zip")));
    let folder = match &zip_path {
        Some(z) => z.with_file_name(format!(
            ".{}.parts",
            z.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        )),
        None if out.extension().is_some() => return Err(AppError::usage(
            "several formats are written to a folder: give --out a folder, or pack them with --zip",
        )),
        None => out.to_path_buf(),
    };
    if zip_path.is_some() {
        let _ = std::fs::remove_dir_all(&folder);
    }
    std::fs::create_dir_all(&folder).map_err(|err| {
        AppError::runtime(format!("failed to create {}: {err}", folder.display()))
    })?;
    let outs: Vec<(String, PathBuf)> = formats
        .iter()
        .zip(exts)
        .map(|(f, e)| (f.clone(), folder.join(format!("{safe_name}.{e}"))))
        .collect();
    let written = hub.export_messages_formats(request, &outs, media).await;
    let Some(zip_path) = zip_path else {
        let files = written?;
        return Ok(json!({ "out": folder, "formats": formats, "files": files }));
    };
    let packed = written.and_then(|mut files| {
        weflow_core::export::zip_folder(&folder, &zip_path).map_err(|err| {
            AppError::runtime(format!("failed to write {}: {err:#}", zip_path.display()))
        })?;
        // the files are inside the zip now: their names there
        for (file, (_, path)) in files.iter_mut().zip(&outs) {
            file["out"] = json!(path.file_name().map(|n| n.to_string_lossy().to_string()));
        }
        Ok(files)
    });
    let _ = std::fs::remove_dir_all(&folder);
    let files = packed?;
    let media = files
        .first()
        .map(|f| f["media"].clone())
        .unwrap_or(Value::Null);
    Ok(json!({ "out": zip_path, "formats": formats, "files": files, "media": media }))
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

fn handle_analytics(command: &AnalyticsCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        AnalyticsSubcommand::Excluded { set: Some(list) } => Ok(json!(hub
            .analytics_set_excluded_usernames(
                &list
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
            )?)),
        _ => unreachable!("a read"),
    }
}

fn day_span(start: &Option<String>, end: &Option<String>) -> AppResult<(i64, i64)> {
    weflow_core::reads::day_span(start.as_deref(), end.as_deref())
}

fn handle_group(command: &GroupCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        GroupSubcommand::List
        | GroupSubcommand::Members { .. }
        | GroupSubcommand::Ranking { .. }
        | GroupSubcommand::Hours { .. }
        | GroupSubcommand::Media { .. }
        | GroupSubcommand::Member { .. }
        | GroupSubcommand::MemberMessages { .. } => unreachable!("a read"),
        GroupSubcommand::ExportMemberMessages {
            chatroom_id,
            username,
            out,
            start,
            end,
        } => {
            let (b, e) = day_span(start, end)?;
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
            AnnualSubcommand::Years => unreachable!("a read"),
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
        // with its media: without them the timeline is a read
        SnsSubcommand::Timeline {
            limit,
            offset,
            users,
            keyword,
            start,
            end,
            with_media: _,
        } => {
            let posts = hub.sns_timeline_query(&SnsTimelineQuery {
                limit: *limit,
                offset: *offset,
                usernames: users.clone(),
                keyword: keyword.clone(),
                start: *start,
                end: *end,
            })?;
            let posts = hub.sns_enrich_timeline_media(posts, "", true, true).await;
            Ok(json!({ "timeline": posts }))
        }
        SnsSubcommand::Users | SnsSubcommand::Stats { .. } | SnsSubcommand::PostCounts { .. } => {
            unreachable!("a read")
        }
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
        InsightSubcommand::Get { .. } | InsightSubcommand::Footprint { .. } => {
            unreachable!("a read")
        }
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

/// The WebUI version this build defaults to (web/package.json at build time): only the version number is in the
/// binary, the page itself is downloaded.
const WEB_VERSION: &str = env!("WEFLOW_WEB_VERSION");

/// What a WebUI has to suit: this command API and this weflow.
fn web_backend() -> weflow_core::webui::Backend<'static> {
    weflow_core::webui::Backend {
        api_version: serve_api::API_VERSION,
        version: env!("WEFLOW_VERSION"),
    }
}

/// The version `serve` shows without `--ui-dir` and `--web-version`: the one picked with `weflow web use`, else the
/// default version of this weflow.
fn web_wanted(store: &weflow_core::webui::WebStore) -> Option<String> {
    store
        .chosen()
        .or_else(|| (!WEB_VERSION.is_empty()).then(|| WEB_VERSION.to_string()))
}

/// The WebUI `serve` starts with, and the version to download beside it (issue #148): none with `--api-only`; the
/// folder of `--ui-dir`; `--web-version` (downloaded first); else the version wanted, or while it is downloaded,
/// the newest suitable one kept here.
async fn serve_webui(
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
                "--ui-dir {}: no index.html there; pass the folder of a built WebUI (such as web/dist)",
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
                done as usize,
                total as usize,
            )
        };
        let meta =
            weflow_core::webui::install(&store, &registry, version, web_backend(), &progress)
                .await?;
        return Ok((UiDir::open_version(&store, &meta.version), None));
    }
    let Some(wanted) = web_wanted(&store) else {
        return Ok((None, None));
    };
    let backend = web_backend();
    // the newest suitable version kept here: shown while the one wanted downloads, or instead of one kept that
    // does not suit this weflow
    let fallback = || {
        store
            .installed()
            .into_iter()
            .find(|m| {
                backend
                    .refusal(&m.api_version, &m.min_backend_version)
                    .is_none()
            })
            .and_then(|m| UiDir::open_version(&store, &m.version))
    };
    match weflow_core::webui::kept(&store, &wanted, backend) {
        Ok(Some(meta)) => Ok((UiDir::open_version(&store, &meta.version), None)),
        Ok(None) => Ok((fallback(), Some(wanted))),
        // kept by another weflow: downloading it again would not help
        Err(refused) => {
            eprintln!(
                "{}{}",
                weflow_core::locale::tr("warning: ", "警告："),
                weflow_core::locale::localize(refused.message.clone())
            );
            Ok((fallback(), None))
        }
    }
}

/// `weflow web …`: the WebUI versions kept here and published (issue #148).
async fn handle_web(command: &WebCommand, hub: &ServiceHub) -> AppResult<Value> {
    use weflow_core::webui::{self, Registry, WebMeta};
    let store = hub.web_store();
    let backend = web_backend();
    let chosen = store.chosen();
    let wanted = web_wanted(&store);
    let entry = |m: &WebMeta| {
        json!({
            "version": m.version,
            "apiVersion": m.api_version,
            "minBackendVersion": m.min_backend_version,
            "path": store.dir(&m.version),
            "inUse": wanted.as_deref() == Some(m.version.as_str()),
            "picked": chosen.as_deref() == Some(m.version.as_str()),
            "suits": backend.refusal(&m.api_version, &m.min_backend_version).is_none(),
        })
    };
    let progress = |done: u64, total: u64| {
        weflow_core::output::progress(
            "web",
            "downloading the WebUI",
            done as usize,
            total as usize,
        )
    };
    match &command.command {
        WebSubcommand::Info => {
            let in_use = wanted.as_deref().and_then(|v| store.meta(v));
            Ok(json!({
                "inUse": in_use.as_ref().map(|m| m.version.clone()),
                "path": in_use.as_ref().map(|m| store.dir(&m.version)),
                "picked": chosen,
                "builtIn": WEB_VERSION,
                "wanted": wanted,
                "downloaded": in_use.is_some(),
                "store": store.root(),
                "installed": store.installed().iter().map(|m| m.version.clone()).collect::<Vec<_>>(),
            }))
        }
        WebSubcommand::List { remote: false } => {
            Ok(Value::Array(store.installed().iter().map(entry).collect()))
        }
        WebSubcommand::List { remote: true } => {
            let registry = Registry::default_registry()?;
            let mut out = Vec::new();
            for version in registry.versions().await?.into_iter().take(30) {
                let published = registry.published(&version).await?;
                let refusal = match (&published.api_version, &published.min_backend_version) {
                    (Some(api), Some(min)) => backend.refusal(api, min),
                    _ => None,
                };
                out.push(json!({
                    "version": version,
                    "size": published.size,
                    "apiVersion": published.api_version,
                    "minBackendVersion": published.min_backend_version,
                    "suits": refusal.is_none(),
                    "why": refusal,
                    "downloaded": store.meta(&version).is_some(),
                }));
            }
            Ok(Value::Array(out))
        }
        WebSubcommand::Install { version } => {
            let registry = Registry::default_registry()?;
            let meta = webui::install(&store, &registry, version, backend, &progress).await?;
            Ok(entry(&meta))
        }
        WebSubcommand::Use { version } => {
            let registry = Registry::default_registry()?;
            let meta = webui::install(&store, &registry, version, backend, &progress).await?;
            store.choose(&meta.version)?;
            Ok(json!({ "version": meta.version, "path": store.dir(&meta.version) }))
        }
        WebSubcommand::Update => {
            let registry = Registry::default_registry()?;
            let mut suitable = Vec::new();
            for version in registry.versions().await?.into_iter().take(30) {
                let published = registry.published(&version).await?;
                if let (Some(api), Some(min)) =
                    (&published.api_version, &published.min_backend_version)
                {
                    if backend.refusal(api, min).is_none() {
                        suitable.push(version);
                    }
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
            let before = wanted.clone();
            let meta = webui::install(&store, &registry, &newest, backend, &progress).await?;
            store.choose(&meta.version)?;
            Ok(
                json!({ "version": meta.version, "before": before, "path": store.dir(&meta.version) }),
            )
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
            Ok(json!({ "removed": removed, "kept": kept, "shown": shown }))
        }
    }
}

/// `value` as a URL query value: every byte but the unreserved ones (RFC 3986) percent-encoded, so a token with `+`,
/// `&`, `#` or `=` reaches the page as it is.
fn query_escape(value: &str) -> String {
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
fn open_in_browser(url: &str) {
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
    let _ = opener
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// `serve` runs the command API (every CLI command over HTTP, see `serve_api`) or, with `--desktop`, the desktop
/// app's HTTP API, next to the optional insight and image auto-download services.
async fn handle_serve(command: &ServeCommand, hub: &ServiceHub) -> AppResult<Value> {
    if command.message_push && !command.desktop {
        return Err(AppError::usage("--message-push needs --desktop"));
    }
    if command.desktop
        && (command.max_tasks.is_some()
            || command.max_reads.is_some()
            || command.max_media.is_some()
            || command.log_requests
            || command.ui_dir.is_some()
            || command.no_browser
            || command.web_version.is_some()
            || command.api_only)
    {
        return Err(AppError::usage(
            "--max-tasks, --max-reads, --max-media, --log-requests, --ui-dir, --no-browser, --web-version and --api-only are for the command API, not --desktop",
        ));
    }
    let (ui, fetch) = if command.desktop {
        (None, None)
    } else {
        serve_webui(command, hub).await?
    };
    let ui = std::sync::Arc::new(std::sync::RwLock::new(ui));

    let cfg_str = |key: &str| {
        hub.config_value(key)
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let host = command.host.clone().unwrap_or_else(|| {
        if command.desktop {
            cfg_str("httpApiHost").unwrap_or_else(|| "127.0.0.1".into())
        } else {
            "127.0.0.1".into()
        }
    });
    let port = command.port.unwrap_or_else(|| {
        if command.desktop {
            hub.config_value("httpApiPort")
                .as_u64()
                .map(|p| p as u16)
                .unwrap_or(5031)
        } else {
            serve_api::DEFAULT_PORT
        }
    });
    let token = command
        .api_token
        .clone()
        .or_else(|| cfg_str("httpApiToken"));
    // without a configured token the command API makes one up; the desktop API refuses every request instead
    let generated_token = (!command.desktop && token.is_none()).then(serve_api::generate_token);
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

    // files returned by an earlier run of the command API are not needed any more
    let files_dir = hub.serve_files_dir();
    if !command.desktop {
        if let Err(err) = std::fs::remove_dir_all(&files_dir) {
            if err.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "{}{}: {err}",
                    weflow_core::locale::tr("warning: cannot clear ", "警告：无法清空 "),
                    files_dir.display()
                );
            }
        }
    }

    let mut started = json!({
        "type": "server_started",
        "url": format!("http://{bound}"),
        "api": !command.desktop,
        "desktop": command.desktop,
        "tokenConfigured": token.is_some(),
        "messagePush": command.message_push,
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
        .read()
        .unwrap()
        .as_ref()
        .map(|ui| ui.version().map(str::to_string));
    if !command.desktop {
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
                    *ui.write().unwrap() = Some(dir);
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
    if command.desktop && token.is_none() {
        eprintln!(
            "{}",
            weflow_core::locale::tr(
                "warning: no HTTP API token configured; every request except /health will be refused (set http_api_token or pass --api-token)",
                "警告：未配置 HTTP API 令牌；除 /health 外的所有请求都会被拒绝（请设置 http_api_token 或传入 --api-token）"
            )
        );
    }
    if !command.desktop && !bound.ip().is_loopback() {
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
        let stopped = if command.desktop {
            let cfg = weflow_core::http_server::HttpConfig {
                host,
                port: bound.port(),
                token,
                push_enabled: command.message_push
                    || hub.config_value("messagePushEnabled").as_bool() == Some(true),
            };
            weflow_core::http_server::serve(hub.clone(), cfg, broker, listener).await
        } else {
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
    make_parent(&path)?;
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

/// The folder `path` goes into, made when it is not there: the writers of xlsx, HTML and SQL do not make it.
fn make_parent(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            AppError::runtime(format!("failed to create {}: {err}", parent.display()))
        })?;
    }
    Ok(())
}

fn write_text_file(path: &Path, text: &str) -> AppResult<()> {
    make_parent(path)?;
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

/// A result that reports `success: false` itself is a failure.
fn checked(outcome: AppResult<Value>) -> AppResult<Value> {
    outcome.and_then(|value| match reported_failure(&value) {
        Some(err) => Err(err),
        None => Ok(value),
    })
}

/// The `--json` document of a command's outcome, and its exit code. The command API's in-process routes answer
/// with it too, so they match the child process byte for byte.
fn json_document(outcome: AppResult<Value>) -> (Value, u8) {
    let (mut doc, exit) = match checked(outcome) {
        Ok(value) => (serde_json::to_value(success(value)).unwrap(), 0),
        Err(err) => (
            serde_json::to_value(failure(err.payload())).unwrap(),
            err.exit_code as u8,
        ),
    };
    weflow_core::locale::localize_json(&mut doc);
    (doc, exit)
}

/// The human-readable output: the payload itself, without the {success, data} envelope.
fn print_response<T: serde::Serialize>(response: &T, cli: &Cli) {
    let mut value = serde_json::to_value(response).unwrap();
    weflow_core::locale::localize_json(&mut value);
    let data = value.get("data").cloned().unwrap_or(Value::Null);
    match key_summary(&cli.command, &data) {
        Some(text) => print!("{text}"),
        None => print!("{}", weflow_core::render::render(&data)),
    }
    if let Some(meta) = value.get("meta") {
        print!("\n{}", weflow_core::render::render(meta));
    }
}

/// `key db` / `key image` print the keys under the names `config set` expects, so they can be copied over.
fn key_summary(command: &Commands, data: &Value) -> Option<String> {
    use weflow_core::locale::tr;
    if let Commands::Cache(CacheCommand { command }) = command {
        return cache_summary(command, data);
    }
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
    if data.get("saved").is_some() {
        let what = match command {
            KeySubcommand::Db { .. } => tr("Database key", "数据库密钥"),
            _ => tr("Image keys", "图片密钥"),
        };
        let mut text = format!(
            "{}
",
            tr(
                "{what} obtained and saved to the active profile",
                "已获取{what}，已保存到当前档案"
            )
            .replace("{what}", what)
        );
        if data["verified"] == false {
            text.push_str(tr(
                "(AES header verified; XOR and the complete pair are not verified)
",
                "（AES 首块已验证；XOR 与整组密钥尚未验证）
",
            ));
        }
        return Some(text);
    }
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
                    "(AES header verified; XOR and the complete pair are not verified)\n",
                    "（AES 首块已验证；XOR 与整组密钥尚未验证）\n",
                ));
            }
            Some(text)
        }
        KeySubcommand::ScanImage { .. } => None,
    }
}

/// Some services report a failure in their result (`{"success": false, "error" | "message": …}`), the shape the
/// HTTP API and the desktop app expect. For the CLI that is a failed command: an error envelope and a non-zero exit
/// code, with the other fields (such as `failureKind`) kept as details.
fn reported_failure(value: &Value) -> Option<AppError> {
    let fields = value.as_object()?;
    if fields.get("success") != Some(&Value::Bool(false)) {
        return None;
    }
    let message = ["error", "message"]
        .iter()
        .find_map(|key| fields.get(*key).and_then(Value::as_str))
        .filter(|text| !text.is_empty())
        .unwrap_or("the operation failed");
    let mut err = AppError::runtime(message);
    let details: serde_json::Map<String, Value> = fields
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "success" | "error" | "message"))
        .map(|(key, v)| (key.clone(), v.clone()))
        .collect();
    if !details.is_empty() {
        err.details = Some(Value::Object(details));
    }
    Some(err)
}

fn print_failure(err: &AppError) {
    let details = err.details.as_ref();
    eprint!(
        "{}",
        weflow_core::render::render_error(&err.message, details)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reported_failures_become_errors() {
        let err = reported_failure(
            &json!({ "success": false, "error": "not found", "failureKind": "not_found" }),
        )
        .unwrap();
        assert_eq!((err.code.as_str(), err.exit_code), ("runtime_error", 1));
        assert_eq!(err.message, "not found");
        assert_eq!(err.details, Some(json!({ "failureKind": "not_found" })));

        let err = reported_failure(&json!({ "success": false, "message": "no key" })).unwrap();
        assert_eq!(
            (err.message.as_str(), err.details.is_none()),
            ("no key", true)
        );
        assert_eq!(
            reported_failure(&json!({ "success": false }))
                .unwrap()
                .message,
            weflow_core::locale::localize("the operation failed".to_string())
        );

        for ok in [
            json!({ "success": true, "path": "x" }),
            json!({ "out": "x" }),
            json!([{ "success": false }]),
            json!({ "results": [{ "success": false }] }),
        ] {
            assert!(reported_failure(&ok).is_none(), "{ok}");
        }
    }

    #[test]
    fn saved_keys_go_to_the_profile_and_not_to_the_result() {
        let dir = std::env::temp_dir().join(format!("weflow-key-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let found =
            json!({ "image_xor_key": 83, "image_aes_key": "0123456789abcdef", "verified": true });
        let shown = save_keys(&path, found, &["image_xor_key", "image_aes_key"]).unwrap();
        assert_eq!(
            shown,
            json!({ "verified": true, "saved": ["image_xor_key", "image_aes_key"] })
        );
        let config = ConfigStore::load(&path).unwrap();
        let profile = config.profile(None).unwrap();
        assert_eq!(profile.image_xor_key, Some(83));
        assert_eq!(profile.image_aes_key.as_deref(), Some("0123456789abcdef"));

        let err = save_keys(&path, json!({ "method": "wx_key" }), &["decrypt_key"]).unwrap_err();
        assert!(err.message.contains("decrypt_key"), "{}", err.message);
        assert_eq!(
            ConfigStore::load(&path)
                .unwrap()
                .profile(None)
                .unwrap()
                .decrypt_key,
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_key_arguments() {
        let parse =
            |args: &[&str]| Cli::try_parse_from(["weflow", "key", "image"].iter().chain(args));
        let Commands::Key(KeyCommand {
            command:
                KeySubcommand::Image {
                    kvcomm_dir,
                    scan_budget,
                    ..
                },
        }) = parse(&[]).unwrap().command
        else {
            panic!("image command")
        };
        assert!(kvcomm_dir.is_empty());
        assert_eq!(scan_budget, None);
        let Commands::Key(KeyCommand {
            command:
                KeySubcommand::Image {
                    kvcomm_dir,
                    scan_budget,
                    ..
                },
        }) = parse(&[
            "--kvcomm-dir",
            "net/kvcomm",
            "--kvcomm-dir",
            "net_1/kvcomm",
            "--scan-budget",
            "10",
        ])
        .unwrap()
        .command
        else {
            panic!("image command")
        };
        assert_eq!(
            kvcomm_dir,
            [PathBuf::from("net/kvcomm"), PathBuf::from("net_1/kvcomm")]
        );
        assert_eq!(scan_budget, Some(10));
    }

    #[test]
    fn partial_image_key_summary_does_not_claim_missing_templates() {
        let command = Cli::try_parse_from(["weflow", "key", "image"])
            .unwrap()
            .command;
        let summary = key_summary(
            &command,
            &json!({
                "image_xor_key": 123, "image_aes_key": "0123456789abcdef",
                "verified": false, "scan": {"truncated": true},
            }),
        )
        .unwrap();
        assert!(summary.contains("XOR"));
        assert!(!summary.contains("no .dat template"));
        assert!(!summary.contains("--json"));
    }

    #[test]
    fn only_y_or_yes_confirms() {
        for answer in ["y\n", "Y\n", "yes\r\n", " YES "] {
            assert!(is_yes(answer), "{answer:?}");
        }
        for answer in ["\n", "", "n\n", "N", "no", "ye", "yy", "是"] {
            assert!(!is_yes(answer), "{answer:?}");
        }
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(85 << 20), "85 MB");
        assert_eq!(format_bytes((12 << 30) / 10), "1.2 GB");
    }

    #[test]
    fn cache_clear_options_pick_their_parts() {
        let parse = |args: &[&str]| {
            let cli = Cli::try_parse_from(["weflow", "cache", "clear"].iter().chain(args)).unwrap();
            match cli.command {
                Commands::Cache(CacheCommand {
                    command: CacheSubcommand::Clear(args),
                }) => args.parts(),
                other => panic!("{other:?}"),
            }
        };
        assert!(parse(&[]).is_empty());
        assert_eq!(
            parse(&["--voices", "--images", "-y"]),
            [CachePart::Images, CachePart::Voices]
        );
        assert_eq!(parse(&["--keys"]), [CachePart::Keys]);
        assert_eq!(parse(&["--runtime"]), [CachePart::Runtime]);
        assert_eq!(parse(&["--all"]), CachePart::ALL);
        assert_eq!(parse(&["--all", "--images"]), CachePart::ALL);
    }

    #[test]
    fn columns_line_up_and_skip_empty_ones() {
        let rows = vec![
            vec!["".to_string(), "图片".to_string(), "1 KB".to_string()],
            vec!["".to_string(), "keys".to_string(), "20 MB".to_string()],
        ];
        assert_eq!(align_columns(&rows), ["图片  1 KB", "keys  20 MB"]);
    }

    #[test]
    fn a_cell_of_several_values_takes_a_line_each_under_its_column() {
        let row = |cells: &[&str]| cells.iter().map(|c| c.to_string()).collect::<Vec<_>>();
        let rows = vec![
            row(&["部分", "大小", "路径"]),
            row(&["旧运行时", "3 MB", "/a/runtime/1\n/a/runtime/longer/2"]),
            row(&["图片", "1 KB", "/a/images"]),
        ];
        assert_eq!(
            align_columns(&rows),
            [
                "部分      大小  路径",
                "旧运行时  3 MB  /a/runtime/1",
                "                /a/runtime/longer/2",
                "图片      1 KB  /a/images",
            ]
        );
    }
}
