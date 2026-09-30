use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};
use tracing_subscriber::EnvFilter;
use weflow_core::config::{old_electron_config_candidates, AppContext, ConfigStore};
use weflow_core::error::{AppError, AppResult};
use weflow_core::output::{failure, success};
use weflow_core::services::ServiceHub;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(name = "weflow", version, about = "Native CLI for WeFlow")]
struct Cli {
    /// Path to the config file
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Config profile name
    #[arg(long, global = true)]
    profile: Option<String>,
    /// WeChat data directory (overrides config)
    #[arg(long, global = true)]
    db_path: Option<String>,
    /// Database decrypt key in hex (overrides config)
    #[arg(long, global = true)]
    decrypt_key: Option<String>,
    /// Account wxid (overrides config)
    #[arg(long, global = true)]
    wxid: Option<String>,
    /// Output language for generated text (default: from WEFLOW_LANG/LC_ALL/LC_MESSAGES/LANG, else en)
    #[arg(long, global = true, value_enum)]
    lang: Option<LangArg>,
    /// Print JSON (default)
    #[arg(long, global = true)]
    json: bool,
    /// Print human-readable pretty JSON
    #[arg(long, global = true)]
    pretty: bool,
    /// Emit NDJSON progress events on stderr
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
    /// Read and write configuration (list, get, set, unset, clear, import)
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
}

#[derive(Args, Debug)]
struct ConfigCommand {
    #[command(subcommand)]
    command: ConfigSubcommand,
}

#[derive(Subcommand, Debug)]
enum ConfigSubcommand {
    List,
    Get { key: Option<String> },
    Set { key: String, value: String },
    Unset { key: String },
    Clear,
    Import { path: Option<PathBuf> },
}

#[derive(Args, Debug)]
struct DbCommand {
    #[command(subcommand)]
    command: DbSubcommand,
}

#[derive(Subcommand, Debug)]
enum DbSubcommand {
    Detect,
    Scan { root: String },
    Test,
    Open,
}

#[derive(Args, Debug)]
struct KeyCommand {
    #[command(subcommand)]
    command: KeySubcommand,
}

#[derive(Subcommand, Debug)]
enum KeySubcommand {
    Db,
    Image,
    ScanImage { user_dir: String },
}

#[derive(Args, Debug)]
struct ChatCommand {
    #[command(subcommand)]
    command: ChatSubcommand,
}

#[derive(Subcommand, Debug)]
enum ChatSubcommand {
    Sessions {
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },
    Messages(PageArgs),
    Latest {
        session_id: String,
        #[arg(long, default_value_t = 20)]
        limit: i32,
    },
    Search {
        keyword: String,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: i32,
        #[arg(long, default_value_t = 0)]
        offset: i32,
        #[arg(long, default_value_t = 0)]
        start: i32,
        #[arg(long, default_value_t = 0)]
        end: i32,
    },
    Contacts,
    Contact {
        username: String,
    },
    UpdateMessage {
        session_id: String,
        local_id: i64,
        create_time: i32,
        content: String,
    },
    DeleteMessage {
        session_id: String,
        local_id: i64,
        create_time: i32,
        #[arg(long)]
        db_path_hint: Option<String>,
    },
    AntiRevoke {
        #[command(subcommand)]
        command: AntiRevokeSubcommand,
    },
    /// Look up one message by local id or server id
    Message {
        session_id: String,
        #[arg(long)]
        local_id: Option<i32>,
        #[arg(long)]
        server_id: Option<String>,
    },
    /// Dates that have messages in a session (YYYY-MM-DD)
    Dates { session_id: String },
    /// Message count per day for a session
    DateCounts { session_id: String },
    /// Total message counts for several sessions
    Counts { sessions: Vec<String> },
    /// Folded / muted state of sessions
    Statuses { usernames: Vec<String> },
    /// Session details (contact info, message count, message tables, first/latest time)
    Detail {
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
        sessions: Vec<String>,
        /// Start date in Beijing time (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date in Beijing time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
        /// Skip mutual group / friend relations
        #[arg(long)]
        no_relations: bool,
    },
    /// Read or set the cached "my message count" of a group chat
    GroupHint {
        chatroom_id: String,
        #[arg(long)]
        set: Option<i64>,
    },
    /// List image / video / voice / file messages across sessions
    Resources {
        #[arg(long)]
        session: Option<String>,
        /// image, video, voice, file (repeatable)
        #[arg(long = "type")]
        types: Vec<String>,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
        #[arg(long, default_value_t = 300)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// All image identifiers of a session (md5 / dat name), newest first
    Images { session_id: String },
    /// All voice messages of a session
    VoiceMessages { session_id: String },
    /// Page through image/video messages with the native media scanner
    MediaStream {
        #[arg(long)]
        session: Option<String>,
        /// image, video or all
        #[arg(long, default_value = "all")]
        media_type: String,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
        #[arg(long, default_value_t = 200)]
        limit: i32,
        #[arg(long, default_value_t = 0)]
        offset: i32,
    },
    /// Resolve payer / receiver display names of a transfer message
    TransferNames { chatroom_id: String, payer: String, receiver: String },
    Voice {
        session_id: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Decode one voice message (SILK) into a 24 kHz WAV file
    VoiceData {
        session_id: String,
        /// Local message id
        msg_id: String,
        #[arg(long)]
        create_time: Option<i64>,
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
        session_id: String,
        msg_id: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Check whether a decoded voice WAV is already cached for a message id
    VoiceCache { session_id: String, msg_id: String },
    /// Decode and cache many voice messages; takes a JSON array of {localId, createTime, serverId?, senderWxid?}
    VoicePreload { session_id: String, messages_json: String },
    Emoji {
        session_id: String,
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Args, Debug)]
struct PageArgs {
    session_id: String,
    #[arg(long, default_value_t = 50)]
    limit: i32,
    #[arg(long, default_value_t = 0)]
    offset: i32,
}

#[derive(Subcommand, Debug)]
enum AntiRevokeSubcommand {
    /// Sessions that anti-revoke can be installed for
    Sessions,
    Check { sessions: Vec<String> },
    Install { sessions: Vec<String> },
    Uninstall { sessions: Vec<String> },
}

#[derive(Args, Debug)]
struct ExportCommand {
    #[command(subcommand)]
    command: ExportSubcommand,
}

#[derive(Subcommand, Debug)]
enum ExportSubcommand {
    Sessions {
        #[arg(long = "session")]
        sessions: Vec<String>,
        #[arg(long)]
        format: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    Contacts {
        #[arg(long)]
        format: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    Footprint {
        #[arg(long)]
        format: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    Media {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value = "all")]
        r#type: String,
    },
    Messages {
        session_id: String,
        /// Start date in Beijing time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        start: Option<String>,
        /// End date in Beijing time, inclusive (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
        #[arg(long)]
        out: PathBuf,
        /// txt (default), json, arkme-json, chatlab, chatlab-jsonl, excel, weclone, html, sql
        #[arg(long, default_value = "txt")]
        format: String,
        /// Only export messages sent by this wxid
        #[arg(long)]
        sender: Option<String>,
        /// How senders are named: group-nickname, remark (default) or nickname
        #[arg(long, default_value = "remark")]
        display_name: String,
        /// Excel: compact columns (time, sender, type, content)
        #[arg(long)]
        excel_compact: bool,
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
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// First day (YYYY-MM-DD, Beijing time)
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
        chatroom_id: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
    },
    /// Messages per hour of day
    Hours {
        chatroom_id: String,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
    },
    /// Message type mix
    Media {
        chatroom_id: String,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
    },
    /// One member's statistics (types, hours, common phrases and emoji)
    Member {
        chatroom_id: String,
        username: String,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
    },
    /// A page of one member's messages (newest first)
    MemberMessages {
        chatroom_id: String,
        username: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Cursor returned as `nextCursor` by the previous page
        #[arg(long, default_value_t = 0)]
        cursor: usize,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
    },
    /// Export a member's messages (.csv or .xlsx)
    ExportMemberMessages {
        chatroom_id: String,
        username: String,
        out: PathBuf,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
    },
    /// Export the member list (.csv or .xlsx)
    ExportMembers {
        chatroom_id: String,
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
    Annual {
        #[command(subcommand)]
        command: AnnualSubcommand,
    },
    Dual {
        #[command(subcommand)]
        command: DualSubcommand,
    },
}

#[derive(Subcommand, Debug)]
enum AnnualSubcommand {
    Years,
    Generate {
        /// Report year; 0 (default) covers all years.
        #[arg(long, default_value_t = 0)]
        year: i32,
    },
}

#[derive(Subcommand, Debug)]
enum DualSubcommand {
    Generate {
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
        #[arg(long, default_value_t = 20)]
        limit: i32,
        #[arg(long, default_value_t = 0)]
        offset: i32,
        /// Only posts of these users (repeatable)
        #[arg(long = "user")]
        users: Vec<String>,
        #[arg(long)]
        keyword: Option<String>,
        #[arg(long, default_value_t = 0)]
        start: i64,
        #[arg(long, default_value_t = 0)]
        end: i64,
        /// Download and decrypt images / videos into the cache and inline them as data URLs
        #[arg(long)]
        with_media: bool,
    },
    /// Users that have posted
    Users,
    /// Export statistics (total posts / friends / mine); --fast reads the cached counts only
    Stats {
        #[arg(long)]
        fast: bool,
    },
    /// Post counts per user, or the statistics of one user
    PostCounts {
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        prefer_cache: bool,
    },
    /// Export the timeline as json, html or arkmejson
    Export {
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "json")]
        format: String,
        #[arg(long = "user")]
        users: Vec<String>,
        #[arg(long)]
        keyword: Option<String>,
        #[arg(long, default_value_t = 0)]
        start: i64,
        #[arg(long, default_value_t = 0)]
        end: i64,
        /// Also save images / live photos / videos next to the export
        #[arg(long)]
        media: bool,
    },
    /// Fetch (and decrypt, with --key) a Moments image or video
    Media {
        url: String,
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Download a Moments sticker (plain or AES-GCM encrypted)
    DownloadEmoji {
        url: String,
        #[arg(long)]
        encrypt_url: Option<String>,
        #[arg(long)]
        aes_key: Option<String>,
    },
    /// Download an arbitrary image URL and decrypt it if it is a .dat payload
    DownloadImage {
        url: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    BlockDelete { action: TriggerAction },
    Delete { post_id: String },
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
    Accounts,
    Messages {
        username: String,
        #[arg(long, default_value_t = 50)]
        limit: i32,
        #[arg(long, default_value_t = 0)]
        offset: i32,
    },
    PayRecords {
        #[arg(long, default_value_t = 50)]
        limit: i32,
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
    #[arg(long)]
    md5: Option<String>,
    #[arg(long)]
    dat_name: Option<String>,
    /// Message create time (seconds); selects the year-month folder
    #[arg(long)]
    create_time: Option<i64>,
    /// Return a file path instead of a base64 data URL
    #[arg(long)]
    prefer_file_path: bool,
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
            allow_cache_index: if self.no_cache_index { Some(false) } else { None },
            force,
        }
    }
}

#[derive(Subcommand, Debug)]
enum ImageSubcommand {
    /// Decrypt an image (HD preferred with --force) into the image cache
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
    ResolveBatch { payloads_json: String },
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
        md5: String,
        /// Skip cover / thumbnail images
        #[arg(long)]
        no_poster: bool,
        /// Return `file://` URLs instead of base64 data URLs for the posters
        #[arg(long)]
        file_url: bool,
    },
    /// Extract the video md5 from a message XML payload
    ParseMd5 { content: String },
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
        #[arg(long)]
        keyword: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        start: Option<i64>,
        #[arg(long)]
        end: Option<i64>,
        #[arg(long)]
        limit: Option<i64>,
        #[arg(long)]
        offset: Option<i64>,
    },
    Get { id: String },
    MarkRead { id: String },
    /// Delete insight records (all, or filtered)
    Clear {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        start: Option<i64>,
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
    FootprintSummary { payload_json: String },
}

#[derive(Args, Debug)]
struct ServeCommand {
    #[arg(long)]
    http: bool,
    #[arg(long)]
    message_push: bool,
    #[arg(long)]
    insight: bool,
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
    Info,
    Manifest,
}

#[derive(Args, Debug)]
struct BackupCommand {
    #[command(subcommand)]
    command: BackupSubcommand,
}

#[derive(Subcommand, Debug)]
enum BackupSubcommand {
    Create {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        no_images: bool,
        #[arg(long)]
        no_voice: bool,
        #[arg(long)]
        no_emojis: bool,
    },
    Inspect {
        path: PathBuf,
    },
    Restore {
        path: PathBuf,
        #[arg(long)]
        target: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    if let Some(lang) = cli.lang {
        weflow_core::locale::set(match lang {
            LangArg::En => weflow_core::locale::Lang::En,
            LangArg::Zh => weflow_core::locale::Lang::Zh,
        });
    }
    match run(&cli).await {
        Ok(value) => {
            print_response(&success(value), cli.pretty);
            ExitCode::SUCCESS
        }
        Err(err) => {
            print_response(&failure(err.payload()), cli.pretty);
            ExitCode::from(err.exit_code as u8)
        }
    }
}

async fn run(cli: &Cli) -> AppResult<Value> {
    let ctx = AppContext::new(cli.config.clone(), VERSION)?;
    let mut config =
        ConfigStore::load(&ctx.config_path).map_err(|err| AppError::config(err.to_string()))?;

    match &cli.command {
        Commands::Config(command) => return handle_config(command, &ctx, &mut config, cli),
        Commands::Runtime(command) => return handle_runtime(command, &ctx),
        _ => {}
    }

    let hub = ServiceHub::new(
        ctx,
        config,
        cli.profile.clone(),
        cli.db_path.clone(),
        cli.decrypt_key.clone(),
        cli.wxid.clone(),
    );
    let hub = {
        let mut h = hub;
        h.progress_enabled = cli.progress;
        h
    };

    match &cli.command {
        Commands::Db(command) => handle_db(command, &hub),
        Commands::Chat(command) => handle_chat(command, &hub).await,
        Commands::Key(command) => handle_key(command, &hub),
        Commands::Export(command) => handle_export(command, &hub),
        Commands::Analytics(command) => handle_analytics(command, &hub),
        Commands::Group(command) => handle_group(command, &hub),
        Commands::Report(command) => handle_report(command, &hub),
        Commands::Sns(command) => handle_sns(command, &hub).await,
        Commands::Biz(command) => handle_biz(command, &hub),
        Commands::Insight(command) => handle_insight(command, &hub).await,
        Commands::Image(ImageCommand { command: ImageSubcommand::AutoDownload { command } }) => match command {
            AutoDownloadSubcommand::Start { whitelist } => {
                let svc = weflow_core::image_download::ImageAutoDownload::new(hub.runtime_dir());
                let list = if whitelist.is_empty() { config_whitelist(&hub) } else { whitelist.clone() };
                let started = svc.start(list);
                if started["success"] != true {
                    return Err(AppError::runtime(started["error"].as_str().unwrap_or("auto download failed to start").to_string()));
                }
                eprintln!("{}", serde_json::to_string(&json!({ "type": "auto_download_started", "status": svc.status() })).unwrap());
                let _ = tokio::signal::ctrl_c().await;
                svc.stop();
                Ok(json!({ "stopped": true }))
            }
            AutoDownloadSubcommand::Status => Ok(json!({ "isHooked": false, "pid": null, "supported": weflow_core::image_download::supported() })),
        },
        Commands::Image(command) => Ok(match &command.command {
            ImageSubcommand::AutoDownload { .. } => unreachable!("handled above"),
            ImageSubcommand::Decrypt { target, force } => hub.image_decrypt(&target.payload(*force)).to_json(),
            ImageSubcommand::ResolveCache { target } => hub.image_resolve_cache(&target.payload(false)).to_json(),
            ImageSubcommand::ResolveBatch { payloads_json } => {
                let list: Vec<Value> = serde_json::from_str(payloads_json).map_err(|e| AppError::usage(format!("payloads_json must be a JSON array: {e}")))?;
                let payloads: Vec<_> = list.iter().map(weflow_core::services::ImagePayload::from_json).collect();
                hub.image_resolve_cache_batch(&payloads)
            }
            ImageSubcommand::ClearCache => hub.image_clear_cache(),
        }),
        Commands::Video(command) => match &command.command {
            VideoSubcommand::Info { md5, no_poster, file_url } => hub.video_info(md5, !*no_poster, if *file_url { weflow_core::video::PosterFormat::FileUrl } else { weflow_core::video::PosterFormat::DataUrl }),
            VideoSubcommand::ParseMd5 { content } => Ok(serde_json::json!({ "md5": weflow_core::video::parse_video_md5(content) })),
        },
        Commands::Serve(command) => handle_serve(command, &hub).await,
        Commands::Backup(command) => handle_backup(command, &hub),
        Commands::Runtime(_) | Commands::Config(_) => unreachable!(),
    }
}

fn handle_config(
    command: &ConfigCommand,
    ctx: &AppContext,
    config: &mut ConfigStore,
    cli: &Cli,
) -> AppResult<Value> {
    match &command.command {
        ConfigSubcommand::List => Ok(serde_json::to_value(config).unwrap()),
        ConfigSubcommand::Get { key } => {
            if let Some(key) = key {
                Ok(config.get_key(cli.profile.as_deref(), key))
            } else {
                Ok(serde_json::to_value(config.profile(cli.profile.as_deref())).unwrap())
            }
        }
        ConfigSubcommand::Set { key, value } => {
            let value = parse_config_value(value);
            config.set_key(cli.profile.as_deref(), key, value)?;
            config
                .save(&ctx.config_path)
                .map_err(|err| AppError::config(err.to_string()))?;
            Ok(json!({ "configPath": ctx.config_path }))
        }
        ConfigSubcommand::Unset { key } => {
            config.unset_key(cli.profile.as_deref(), key);
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
                .import_electron_config(&path, cli.profile.as_deref())
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
    }
}

async fn handle_chat(command: &ChatCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        ChatSubcommand::Sessions { limit } => {
            let mut sessions = hub.sessions()?;
            if *limit > 0 {
                if let Some(arr) = sessions.as_array_mut() {
                    arr.truncate(*limit);
                }
            }
            Ok(sessions)
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
        ChatSubcommand::Message { session_id, local_id, server_id } => match (local_id, server_id) {
            (Some(id), None) => hub.chat_message_by_id(session_id, *id),
            (None, Some(svr)) => hub.chat_message_by_server_id(session_id, svr),
            _ => Err(AppError::usage("pass exactly one of --local-id or --server-id")),
        },
        ChatSubcommand::Dates { session_id } => hub.chat_dates(session_id),
        ChatSubcommand::DateCounts { session_id } => hub.chat_date_counts(session_id),
        ChatSubcommand::Counts { sessions } => hub.chat_counts(sessions),
        ChatSubcommand::Statuses { usernames } => hub.chat_statuses(usernames),
        ChatSubcommand::Detail { session_id, fast, extra } => match (fast, extra) {
            (true, false) => hub.chat_detail_fast(session_id),
            (false, true) => hub.chat_detail_extra(session_id),
            _ => hub.chat_detail(session_id),
        },
        ChatSubcommand::MarkRead => hub.chat_mark_all_read(),
        ChatSubcommand::TabCounts => hub.chat_tab_counts(),
        ChatSubcommand::ExportStats { sessions, start, end, no_relations } => {
            let (b, e) = date_range_args(start.as_deref(), end.as_deref())?;
            hub.chat_export_stats(sessions, b.unwrap_or(0), e.map(|x| x - 1).unwrap_or(0), !no_relations)
        }
        ChatSubcommand::GroupHint { chatroom_id, set } => match set {
            Some(count) => hub.set_group_hint(chatroom_id, *count),
            None => hub.get_group_hint(chatroom_id),
        },
        ChatSubcommand::Resources { session, types, start, end, limit, offset } => {
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
        ChatSubcommand::MediaStream { session, media_type, start, end, limit, offset } => {
            let (b, e) = date_range_args(start.as_deref(), end.as_deref())?;
            hub.chat_media_stream(session.as_deref(), media_type, b.unwrap_or(0), e.map(|x| x - 1).unwrap_or(0), *limit, *offset)
        }
        ChatSubcommand::TransferNames { chatroom_id, payer, receiver } => hub.chat_transfer_names(chatroom_id, payer, receiver),
        ChatSubcommand::Voice { session_id, out } => {
            let out_path = out.as_deref().unwrap_or(Path::new("."));
            hub.export_media_images(Some(session_id), out_path, "voice")
        }
        ChatSubcommand::VoiceData { session_id, msg_id, create_time, server_id, sender, out } => {
            let wav = hub.voice_data(session_id, msg_id, *create_time, server_id.as_deref(), sender.as_deref())?;
            match out {
                Some(path) => {
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        std::fs::create_dir_all(parent).map_err(|e| AppError::runtime(format!("failed to create {}: {e}", parent.display())))?;
                    }
                    std::fs::write(path, &wav).map_err(|e| AppError::runtime(format!("failed to write {}: {e}", path.display())))?;
                    Ok(serde_json::json!({ "success": true, "path": path.to_string_lossy(), "bytes": wav.len() }))
                }
                None => {
                    use base64::Engine;
                    Ok(serde_json::json!({ "success": true, "data": base64::engine::general_purpose::STANDARD.encode(&wav) }))
                }
            }
        }
        ChatSubcommand::ImageData { session_id, msg_id, out } => {
            let bytes = hub.image_data_for_message(session_id, msg_id)?;
            if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).map_err(|e| AppError::runtime(format!("failed to create {}: {e}", parent.display())))?;
            }
            std::fs::write(out, &bytes).map_err(|e| AppError::runtime(format!("failed to write {}: {e}", out.display())))?;
            Ok(serde_json::json!({ "success": true, "path": out.to_string_lossy(), "bytes": bytes.len() }))
        }
        ChatSubcommand::VoiceCache { session_id, msg_id } => Ok(hub.voice_resolve_cache(session_id, msg_id)),
        ChatSubcommand::VoicePreload { session_id, messages_json } => {
            let messages: Vec<Value> = serde_json::from_str(messages_json).map_err(|e| AppError::usage(format!("messages_json must be a JSON array: {e}")))?;
            hub.voice_preload(session_id, &messages)
        }
        ChatSubcommand::Emoji { session_id, out } => hub.emoji_download(session_id, out).await,
    }
}

fn handle_key(command: &KeyCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        KeySubcommand::Db => hub.key_db(),
        KeySubcommand::Image => hub.key_image(),
        KeySubcommand::ScanImage { user_dir } => hub.key_scan_image(user_dir),
    }
}

fn handle_export(command: &ExportCommand, hub: &ServiceHub) -> AppResult<Value> {
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
        ExportSubcommand::Media { out, session, r#type } => {
            hub.export_media_images(session.as_deref(), out, r#type)
        }
        ExportSubcommand::Messages { session_id, start, end, out, format, sender, display_name, excel_compact } => {
            let start_ts = start
                .as_deref()
                .map(parse_date_beijing)
                .transpose()
                .map_err(AppError::usage)?;
            let end_ts = end
                .as_deref()
                .map(|d| parse_date_beijing(d).map(|ts| ts + 86400))
                .transpose()
                .map_err(AppError::usage)?;
            let fmt = format.to_ascii_lowercase();
            if fmt == "txt" {
                return hub.export_messages_txt(session_id, start_ts, end_ts, out);
            }
            let ext = match fmt.as_str() {
                "json" | "arkme-json" => "json",
                "chatlab" => "chatlab.json",
                "chatlab-jsonl" => "jsonl",
                "excel" | "xlsx" => "xlsx",
                "weclone" => "csv",
                "html" => "html",
                "sql" => "sql",
                other => {
                    return Err(AppError::usage(format!(
                        "unsupported message export format: {other}; supported: txt, {}",
                        weflow_core::services::MESSAGE_EXPORT_FORMATS
                    )))
                }
            };
            let display_pref = weflow_core::export_msg::DisplayPref::parse(display_name)
                .ok_or_else(|| AppError::usage("--display-name must be group-nickname, remark or nickname"))?;
            let safe_name: String = session_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '@' || c == '.' { c } else { '_' }).collect();
            let target = export_path(out, &format!("{safe_name}.{ext}"));
            let request = weflow_core::services::MessageExportRequest {
                session_id: session_id.clone(),
                format: fmt,
                // the desktop formats take the end as inclusive seconds, TXT as exclusive
                start: start_ts,
                end: end_ts.map(|e| e - 1),
                sender: sender.clone(),
                display_pref,
                excel_compact: *excel_compact,
            };
            hub.export_messages(&request, &target)
        }
    }
}

/// `--start` / `--end` dates (Beijing time). The end is returned exclusive (next midnight).
fn date_range_args(start: Option<&str>, end: Option<&str>) -> AppResult<(Option<i64>, Option<i64>)> {
    let b = start.map(parse_date_beijing).transpose().map_err(AppError::usage)?;
    let e = end.map(|d| parse_date_beijing(d).map(|ts| ts + 86400)).transpose().map_err(AppError::usage)?;
    Ok((b, e))
}

fn parse_date_beijing(s: &str) -> Result<i64, String> {
    let parts: Vec<&str> = s.splitn(3, '-').collect();
    if parts.len() != 3 {
        return Err(format!("invalid date '{s}'; use YYYY-MM-DD"));
    }
    let y: i64 = parts[0].parse().map_err(|_| format!("invalid year in '{s}'"))?;
    let m: i64 = parts[1].parse().map_err(|_| format!("invalid month in '{s}'"))?;
    let d: i64 = parts[2].parse().map_err(|_| format!("invalid day in '{s}'"))?;
    // Days since Unix epoch for this calendar date
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let days = 365 * y2 + y2 / 4 - y2 / 100 + y2 / 400 + (153 * m2 + 2) / 5 + d - 719_469;
    // UTC midnight of that date minus 8 h = Beijing midnight
    Ok(days * 86400 - 8 * 3600)
}

fn handle_analytics(command: &AnalyticsCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        AnalyticsSubcommand::Overall { force } => hub.analytics_overall_statistics(*force),
        AnalyticsSubcommand::Rankings { limit, start, end } => {
            let (b, e) = date_range_args(start.as_deref(), end.as_deref())?;
            Ok(json!(hub.analytics_contact_rankings(*limit, b.unwrap_or(0), e.map(|e| e - 1).unwrap_or(0))?))
        }
        AnalyticsSubcommand::Time => hub.analytics_time_distribution(),
        AnalyticsSubcommand::Excluded { set } => match set {
            Some(list) => Ok(json!(hub.analytics_set_excluded_usernames(&list.iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>())?)),
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
        GroupSubcommand::Members { chatroom_id, counts, refresh } => {
            let (members, from_cache, updated_at) = hub.group_members_panel(chatroom_id, *refresh, *counts)?;
            Ok(json!({ "chatroomId": chatroom_id, "count": members.len(), "fromCache": from_cache, "updatedAt": updated_at, "members": members }))
        }
        GroupSubcommand::Ranking { chatroom_id, limit, start, end } => {
            let (b, e) = range(start, end)?;
            Ok(json!(hub.group_message_ranking(chatroom_id, *limit, b, e)?))
        }
        GroupSubcommand::Hours { chatroom_id, start, end } => {
            let (b, e) = range(start, end)?;
            hub.group_active_hours(chatroom_id, b, e)
        }
        GroupSubcommand::Media { chatroom_id, start, end } => {
            let (b, e) = range(start, end)?;
            hub.group_media_stats(chatroom_id, b, e)
        }
        GroupSubcommand::Member { chatroom_id, username, start, end } => {
            let (b, e) = range(start, end)?;
            hub.group_member_analytics(chatroom_id, username, b, e)
        }
        GroupSubcommand::MemberMessages { chatroom_id, username, limit, cursor, start, end } => {
            let (b, e) = range(start, end)?;
            hub.group_member_messages(chatroom_id, username, b, e, *limit, *cursor)
        }
        GroupSubcommand::ExportMemberMessages { chatroom_id, username, out, start, end } => {
            let (b, e) = range(start, end)?;
            hub.group_export_member_messages(chatroom_id, username, out, b, e)
        }
        GroupSubcommand::ExportMembers { chatroom_id, out } => hub.group_export_members(chatroom_id, out),
    }
}

fn handle_report(command: &ReportCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        ReportSubcommand::Annual { command } => match command {
            AnnualSubcommand::Years => hub.report_available_years(),
            AnnualSubcommand::Generate { year } => hub.report_annual(*year),
        },
        ReportSubcommand::Dual { command } => match command {
            DualSubcommand::Generate { friend, year, exclude_words } => hub.report_dual(friend, *year, exclude_words),
        },
    }
}

async fn handle_sns(command: &SnsCommand, hub: &ServiceHub) -> AppResult<Value> {
    use weflow_core::services::{SnsExportOptions, SnsTimelineQuery};
    match &command.command {
        SnsSubcommand::Timeline { limit, offset, users, keyword, start, end, with_media } => {
            let posts = hub.sns_timeline_query(&SnsTimelineQuery { limit: *limit, offset: *offset, usernames: users.clone(), keyword: keyword.clone(), start: *start, end: *end })?;
            let posts = if *with_media { hub.sns_enrich_timeline_media(posts, "", true, true).await } else { posts };
            Ok(json!({ "timeline": posts }))
        }
        SnsSubcommand::Users => Ok(json!({ "usernames": hub.sns_usernames_list()? })),
        SnsSubcommand::Stats { fast } => hub.sns_export_stats(*fast),
        SnsSubcommand::PostCounts { user, prefer_cache } => match user {
            Some(u) => hub.sns_user_post_stats(u),
            None => Ok(json!(hub.sns_user_post_counts(*prefer_cache)?)),
        },
        SnsSubcommand::Export { out, format, users, keyword, start, end, media } => {
            hub.sns_export_timeline(&SnsExportOptions { output_dir: out.clone(), format: format.clone(), usernames: users.clone(), keyword: keyword.clone(), export_media: *media, start: *start, end: *end, ..Default::default() }).await
        }
        SnsSubcommand::Media { url, key, out } => {
            let fetched = hub.sns_fetch_media(url, key.as_deref()).await?;
            let data = fetched.data.clone().unwrap_or_default();
            match out {
                Some(path) => {
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        std::fs::create_dir_all(parent).map_err(|e| AppError::runtime(format!("failed to create {}: {e}", parent.display())))?;
                    }
                    std::fs::write(path, &data).map_err(|e| AppError::runtime(format!("failed to write {}: {e}", path.display())))?;
                    Ok(json!({ "out": path.to_string_lossy(), "contentType": fetched.content_type, "bytes": data.len(), "cachePath": fetched.cache_path }))
                }
                None => Ok(json!({ "contentType": fetched.content_type, "bytes": data.len(), "cachePath": fetched.cache_path })),
            }
        }
        SnsSubcommand::DownloadEmoji { url, encrypt_url, aes_key } => hub.sns_download_emoji(url, encrypt_url.as_deref(), aes_key.as_deref()).await,
        SnsSubcommand::DownloadImage { url, out } => hub.sns_download_image(url, out.as_deref()).await,
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
        BizSubcommand::Messages { username, limit, offset } => hub.biz_messages(username, *limit, *offset),
        BizSubcommand::PayRecords { limit, offset } => hub.biz_pay_records(*limit, *offset),
    }
}

fn insight_filters(session: &Option<String>, keyword: &Option<String>, start: &Option<i64>, end: &Option<i64>, limit: &Option<i64>, offset: &Option<i64>) -> weflow_core::insight::RecordFilters {
    weflow_core::insight::RecordFilters { keyword: keyword.clone().unwrap_or_default(), session_id: session.clone().unwrap_or_default(), start_time: start.unwrap_or(0), end_time: end.unwrap_or(0), limit: *limit, offset: *offset }
}

fn print_insight(r: &weflow_core::insight::InsightRecord) {
    eprintln!("{}", serde_json::to_string(&json!({ "type": "insight", "record": r.summary() })).unwrap());
}

async fn handle_insight(command: &InsightCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        InsightSubcommand::Test => Ok(hub.insight_test_connection().await),
        InsightSubcommand::Trigger { session_id } => match session_id {
            None => Ok(hub.insight_trigger_test().await),
            Some(id) => {
                let name = hub.chat_contact_avatar(id).map(|(_, n)| n).unwrap_or_else(|| id.clone());
                match hub.insight_generate(id, &name, weflow_core::services::InsightTrigger::Test, None).await {
                    Some(r) => Ok(json!({ "success": true, "record": r.summary() })),
                    None => Ok(json!({ "success": false, "message": "no insight was generated (AI not configured, request failed, or the model answered SKIP)" })),
                }
            }
        },
        InsightSubcommand::Records { keyword, session, start, end, limit, offset } => Ok(hub.insight_list_records(&insight_filters(session, keyword, start, end, limit, offset))),
        InsightSubcommand::Get { id } => Ok(hub.insight_get_record(id)),
        InsightSubcommand::MarkRead { id } => Ok(hub.insight_mark_record_read(id)),
        InsightSubcommand::Clear { session, start, end } => Ok(hub.insight_clear_records(&insight_filters(session, &None, start, end, &None, &None))),
        InsightSubcommand::TodayStats => Ok(hub.insight_today_stats()),
        InsightSubcommand::Scan => {
            let mut found = Vec::new();
            let n = hub.insight_silence_scan(&mut |r| {
                print_insight(r);
                found.push(r.summary());
            })
            .await;
            Ok(json!({ "generated": n, "records": found }))
        }
        InsightSubcommand::Footprint => hub.insight_footprint(),
        InsightSubcommand::FootprintSummary { payload_json } => {
            let payload: Value = serde_json::from_str(payload_json).map_err(|e| AppError::usage(format!("payload_json must be JSON: {e}")))?;
            Ok(hub.insight_footprint_summary(&payload).await)
        }
    }
}

/// The background engine of `serve --insight`: polls for new messages (the desktop app reacts to
/// database-change events) and runs the silence scan on its own schedule.
fn spawn_insight_engine(hub: ServiceHub, stop: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
        rt.block_on(async move {
            let mut next_scan = std::time::Instant::now() + std::time::Duration::from_secs(3 * 60);
            while !stop.load(Ordering::Relaxed) {
                if hub.insight_enabled() {
                    let notify = hub.config_value("aiInsightNotificationEnabled") != Value::Bool(false);
                    let mut emit = |r: &weflow_core::insight::InsightRecord| {
                        if notify {
                            print_insight(r);
                        }
                    };
                    if std::time::Instant::now() >= next_scan {
                        hub.insight_silence_scan(&mut emit).await;
                        let hours = hub.config_value("aiInsightScanIntervalHours").as_f64().filter(|h| *h != 0.0).unwrap_or(4.0).max(0.1);
                        next_scan = std::time::Instant::now() + std::time::Duration::from_secs_f64(hours * 3600.0);
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
        .map(|a| a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

async fn handle_serve(command: &ServeCommand, hub: &ServiceHub) -> AppResult<Value> {
    if !command.http && !command.message_push && !command.insight && !command.image_auto_download {
        return Err(AppError::usage(
            "serve needs at least one of --http, --message-push, --insight, --image-auto-download",
        ));
    }

    let cfg_str = |key: &str| hub.config_value(key).as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let host = command.host.clone().or_else(|| cfg_str("httpApiHost")).unwrap_or_else(|| "127.0.0.1".into());
    let port = command
        .port
        .or_else(|| hub.config_value("httpApiPort").as_u64().map(|p| p as u16))
        .unwrap_or(5031);
    let token = command.api_token.clone().or_else(|| cfg_str("httpApiToken"));
    let addr = format!("{host}:{port}")
        .parse::<std::net::SocketAddr>()
        .map_err(|err| AppError::usage(format!("invalid listen address: {err}")))?;

    let auto_download = if command.image_auto_download {
        let svc = weflow_core::image_download::ImageAutoDownload::new(hub.runtime_dir());
        let started = svc.start(config_whitelist(hub));
        if started["success"] != true {
            eprintln!("warning: image auto download not started: {}", started["error"].as_str().unwrap_or("unknown error"));
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

    eprintln!(
        "{}",
        serde_json::to_string(&json!({
            "type": "server_started",
            "url": format!("http://{addr}"),
            "http": command.http,
            "tokenConfigured": token.is_some(),
            "messagePush": command.message_push,
            "insight": command.insight,
            "imageAutoDownload": command.image_auto_download
        }))
        .unwrap()
    );
    if command.http && token.is_none() {
        eprintln!("warning: no HTTP API token configured; every request except /health will be refused (set http_api_token or pass --api-token)");
    }
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|err| AppError::runtime(format!("failed to bind HTTP server: {err}")))?;
    let cfg = weflow_core::http_server::HttpConfig {
        host,
        port: listener.local_addr().map(|a| a.port()).unwrap_or(port),
        token,
        push_enabled: command.message_push || hub.config_value("messagePushEnabled").as_bool() == Some(true),
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
        BackupSubcommand::Create { out, no_images, no_voice, no_emojis } => {
            hub.backup_create(out, !no_images, !no_voice, !no_emojis)
        }
        BackupSubcommand::Inspect { path } => hub.backup_inspect(path),
        BackupSubcommand::Restore { path, target } => {
            let target_dir = target
                .as_deref()
                .unwrap_or_else(|| Path::new("."));
            hub.backup_restore(path, target_dir)
        }
    }
}

fn parse_config_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn print_response<T: serde::Serialize>(response: &T, pretty: bool) {
    if pretty {
        println!("{}", serde_json::to_string_pretty(response).unwrap());
    } else {
        println!("{}", serde_json::to_string(response).unwrap());
    }
}
