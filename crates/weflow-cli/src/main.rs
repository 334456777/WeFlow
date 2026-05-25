use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tracing_subscriber::EnvFilter;
use weflow_core::config::{old_electron_config_candidates, AppContext, ConfigStore};
use weflow_core::error::{AppError, AppResult};
use weflow_core::output::{failure, success};
use weflow_core::services::ServiceHub;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(name = "weflow", version, about = "Native CLI for WeFlow")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    profile: Option<String>,
    #[arg(long, global = true)]
    db_path: Option<String>,
    #[arg(long, global = true)]
    decrypt_key: Option<String>,
    #[arg(long, global = true)]
    wxid: Option<String>,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    pretty: bool,
    #[arg(long, global = true)]
    progress: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Config(ConfigCommand),
    Db(DbCommand),
    Key(KeyCommand),
    Chat(ChatCommand),
    Export(ExportCommand),
    Analytics(AnalyticsCommand),
    Group(GroupCommand),
    Report(ReportCommand),
    Sns(SnsCommand),
    Biz(BizCommand),
    Insight(InsightCommand),
    Serve(ServeCommand),
    Runtime(RuntimeCommand),
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
    Voice {
        session_id: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
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
    },
}

#[derive(Args, Debug)]
struct AnalyticsCommand {
    #[command(subcommand)]
    command: AnalyticsSubcommand,
}

#[derive(Subcommand, Debug)]
enum AnalyticsSubcommand {
    Overall,
    Rankings,
    Time,
    Excluded,
}

#[derive(Args, Debug)]
struct GroupCommand {
    #[command(subcommand)]
    command: GroupSubcommand,
}

#[derive(Subcommand, Debug)]
enum GroupSubcommand {
    List,
    Members {
        chatroom_id: String,
    },
    Ranking {
        chatroom_id: String,
    },
    Hours {
        chatroom_id: String,
    },
    Media {
        chatroom_id: String,
    },
    Member {
        chatroom_id: String,
        username: String,
    },
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
        #[arg(long)]
        year: i32,
    },
}

#[derive(Subcommand, Debug)]
enum DualSubcommand {
    Generate {
        #[arg(long)]
        friend: String,
        #[arg(long)]
        year: i32,
    },
}

#[derive(Args, Debug)]
struct SnsCommand {
    #[command(subcommand)]
    command: SnsSubcommand,
}

#[derive(Subcommand, Debug)]
enum SnsSubcommand {
    Timeline,
    Users,
    Stats,
    Export { out: PathBuf },
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
struct InsightCommand {
    #[command(subcommand)]
    command: InsightSubcommand,
}

#[derive(Subcommand, Debug)]
enum InsightSubcommand {
    Test,
    Records,
    Get { id: String },
    MarkRead { id: String },
    Clear,
    Trigger {
        session_id: String,
    },
    Footprint,
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
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 5031)]
    port: u16,
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
            AntiRevokeSubcommand::Check { sessions } => hub.anti_revoke("check", sessions),
            AntiRevokeSubcommand::Install { sessions } => hub.anti_revoke("install", sessions),
            AntiRevokeSubcommand::Uninstall { sessions } => hub.anti_revoke("uninstall", sessions),
        },
        ChatSubcommand::Voice { session_id, out } => {
            let out_path = out.as_deref().unwrap_or(Path::new("."));
            hub.export_media_images(Some(session_id), out_path, "voice")
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
        ExportSubcommand::Messages { session_id, start, end, out } => {
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
            hub.export_messages_txt(session_id, start_ts, end_ts, out)
        }
    }
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
    match command.command {
        AnalyticsSubcommand::Overall => hub.analytics_overall(),
        AnalyticsSubcommand::Rankings => hub.analytics_rankings(),
        AnalyticsSubcommand::Time => hub.analytics_time(),
        AnalyticsSubcommand::Excluded => hub.analytics_excluded(),
    }
}

fn handle_group(command: &GroupCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        GroupSubcommand::List => hub.group_list(),
        GroupSubcommand::Members { chatroom_id } => hub.group_members(chatroom_id),
        GroupSubcommand::Ranking { chatroom_id } => hub.group_stats(chatroom_id, "ranking"),
        GroupSubcommand::Hours { chatroom_id } => hub.group_stats(chatroom_id, "hours"),
        GroupSubcommand::Media { chatroom_id } => hub.group_stats(chatroom_id, "media"),
        GroupSubcommand::Member {
            chatroom_id,
            username,
        } => hub.group_member(chatroom_id, username),
        GroupSubcommand::ExportMembers { chatroom_id, out } => {
            let data = hub.group_members(chatroom_id)?;
            write_json_file(out, &data)?;
            Ok(json!({ "out": out, "data": data }))
        }
    }
}

fn handle_report(command: &ReportCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        ReportSubcommand::Annual { command } => match command {
            AnnualSubcommand::Years => hub.report_annual_years(),
            AnnualSubcommand::Generate { year } => hub.report_annual_generate(*year),
        },
        ReportSubcommand::Dual { command } => match command {
            DualSubcommand::Generate { friend, year } => hub.report_dual_generate(friend, *year),
        },
    }
}

async fn handle_sns(command: &SnsCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        SnsSubcommand::Timeline => hub.sns_timeline(),
        SnsSubcommand::Users => hub.sns_users(),
        SnsSubcommand::Stats => hub.sns_stats(),
        SnsSubcommand::Export { out } => {
            let data = hub.sns_timeline()?;
            write_json_file(out, &data)?;
            Ok(json!({ "out": out, "data": data }))
        }
        SnsSubcommand::DownloadImage { url, out } => {
            hub.sns_download_image(url, out.as_deref()).await
        }
        SnsSubcommand::BlockDelete { action } => match action {
            TriggerAction::Check => hub.sns_block_delete("check"),
            TriggerAction::Install => hub.sns_block_delete("install"),
            TriggerAction::Uninstall => hub.sns_block_delete("uninstall"),
        },
        SnsSubcommand::Delete { post_id } => hub.sns_delete(post_id),
    }
}

fn handle_biz(command: &BizCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        BizSubcommand::Accounts => hub.biz_accounts(),
        BizSubcommand::Messages { username, limit, offset } => hub.biz_messages(username, *limit, *offset),
        BizSubcommand::PayRecords { limit, offset } => hub.biz_pay_records(*limit, *offset),
    }
}

async fn handle_insight(command: &InsightCommand, hub: &ServiceHub) -> AppResult<Value> {
    match &command.command {
        InsightSubcommand::Test => hub.insight_test().await,
        InsightSubcommand::Trigger { session_id } => hub.insight_trigger(session_id).await,
        InsightSubcommand::Records => hub.insight_records(),
        InsightSubcommand::Get { id } => hub.insight_get(id),
        InsightSubcommand::MarkRead { id } => hub.insight_mark_read(id),
        InsightSubcommand::Clear => hub.insight_clear(),
        InsightSubcommand::Footprint => hub.insight_footprint(),
    }
}

async fn handle_serve(command: &ServeCommand, hub: &ServiceHub) -> AppResult<Value> {
    if !command.http && !command.message_push && !command.insight && !command.image_auto_download {
        return Err(AppError::usage(
            "serve needs at least one of --http, --message-push, --insight, --image-auto-download",
        ));
    }

    let addr = format!("{}:{}", command.host, command.port)
        .parse::<std::net::SocketAddr>()
        .map_err(|err| AppError::usage(format!("invalid listen address: {err}")))?;

    let event_channel = weflow_core::push::EventChannel::new(256);
    let event_sender = event_channel.sender();

    if command.message_push {
        let hub = hub.clone();
        let sender = event_sender.clone();
        tokio::spawn(async move {
            weflow_core::push::message_push_loop(hub, sender, 5).await;
        });
    }

    let state = HttpState {
        hub: Arc::new(hub.clone()),
        event_sender: event_sender.clone(),
    };
    let app = Router::new()
        .route("/health", get(http_health))
        .route("/api/v1/runtime", get(http_runtime))
        .route("/api/v1/sessions", get(http_sessions))
        .route("/api/v1/messages", get(http_messages))
        .route(
            "/api/v1/sessions/{session_id}/messages",
            get(http_session_messages),
        )
        .route("/api/v1/contacts", get(http_contacts))
        .route("/api/v1/analytics/overall", get(http_analytics_overall))
        .route("/api/v1/groups", get(http_groups))
        .route(
            "/api/v1/groups/{chatroom_id}/members",
            get(http_group_members),
        )
        .route("/api/v1/sns/timeline", get(http_sns_timeline))
        .route("/api/v1/sns/usernames", get(http_sns_users))
        .route("/api/v1/sns/export/stats", get(http_sns_stats))
        .route("/api/v1/events", get(http_events_sse))
        .with_state(state);

    eprintln!(
        "{}",
        serde_json::to_string(&json!({
            "type": "server_started",
            "url": format!("http://{addr}"),
            "messagePush": command.message_push,
            "insight": command.insight,
            "imageAutoDownload": command.image_auto_download
        }))
        .unwrap()
    );
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|err| AppError::runtime(format!("failed to bind HTTP server: {err}")))?;
    tokio::select! {
        result = axum::serve(listener, app) => {
            result.map_err(|err| AppError::runtime(format!("HTTP server stopped: {err}")))?;
        }
        _ = tokio::signal::ctrl_c() => {
            return Err(AppError::new("user_interrupt", "interrupted by user", 130));
        }
    }
    Ok(json!({ "stopped": true }))
}

#[derive(Clone)]
struct HttpState {
    hub: Arc<ServiceHub>,
    event_sender: tokio::sync::broadcast::Sender<Value>,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    keyword: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct MessageQuery {
    talker: Option<String>,
    session_id: Option<String>,
    keyword: Option<String>,
    limit: Option<i32>,
    offset: Option<i32>,
    start: Option<i32>,
    end: Option<i32>,
}

type HttpJson = (StatusCode, Json<Value>);

async fn http_health() -> HttpJson {
    http_ok(json!({ "ok": true }))
}

async fn http_runtime(State(state): State<HttpState>) -> HttpJson {
    http_ok(state.hub.runtime_info())
}

async fn http_sessions(State(state): State<HttpState>, Query(query): Query<ListQuery>) -> HttpJson {
    match state.hub.sessions() {
        Ok(value) => http_ok(filter_and_limit_array(
            value,
            query.keyword.as_deref(),
            query.limit,
        )),
        Err(err) => http_error(err),
    }
}

async fn http_contacts(State(state): State<HttpState>, Query(query): Query<ListQuery>) -> HttpJson {
    match state.hub.contacts() {
        Ok(value) => http_ok(filter_and_limit_array(
            value,
            query.keyword.as_deref(),
            query.limit,
        )),
        Err(err) => http_error(err),
    }
}

async fn http_messages(
    State(state): State<HttpState>,
    Query(query): Query<MessageQuery>,
) -> HttpJson {
    let limit = query.limit.unwrap_or(50);
    let offset = query.offset.unwrap_or(0);
    if let Some(keyword) = query.keyword.as_deref() {
        return match state.hub.search(
            keyword,
            query.session_id.as_deref().or(query.talker.as_deref()),
            limit,
            offset,
            query.start.unwrap_or(0),
            query.end.unwrap_or(0),
        ) {
            Ok(value) => http_ok(value),
            Err(err) => http_error(err),
        };
    }

    let Some(session_id) = query.session_id.as_deref().or(query.talker.as_deref()) else {
        return http_error(AppError::usage(
            "messages endpoint needs talker, session_id, or keyword",
        ));
    };
    match state.hub.messages(session_id, limit, offset) {
        Ok(value) => http_ok(value),
        Err(err) => http_error(err),
    }
}

async fn http_session_messages(
    State(state): State<HttpState>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<MessageQuery>,
) -> HttpJson {
    let limit = query.limit.unwrap_or(50);
    let offset = query.offset.unwrap_or(0);
    match state.hub.messages(&session_id, limit, offset) {
        Ok(value) => http_ok(value),
        Err(err) => http_error(err),
    }
}

async fn http_analytics_overall(State(state): State<HttpState>) -> HttpJson {
    match state.hub.analytics_overall() {
        Ok(value) => http_ok(value),
        Err(err) => http_error(err),
    }
}

async fn http_groups(State(state): State<HttpState>) -> HttpJson {
    match state.hub.group_list() {
        Ok(value) => http_ok(value),
        Err(err) => http_error(err),
    }
}

async fn http_group_members(
    State(state): State<HttpState>,
    AxumPath(chatroom_id): AxumPath<String>,
) -> HttpJson {
    match state.hub.group_members(&chatroom_id) {
        Ok(value) => http_ok(value),
        Err(err) => http_error(err),
    }
}

async fn http_sns_timeline(State(state): State<HttpState>) -> HttpJson {
    match state.hub.sns_timeline() {
        Ok(value) => http_ok(value),
        Err(err) => http_error(err),
    }
}

async fn http_sns_users(State(state): State<HttpState>) -> HttpJson {
    match state.hub.sns_users() {
        Ok(value) => http_ok(value),
        Err(err) => http_error(err),
    }
}

async fn http_sns_stats(State(state): State<HttpState>) -> HttpJson {
    match state.hub.sns_stats() {
        Ok(value) => http_ok(value),
        Err(err) => http_error(err),
    }
}

async fn http_events_sse(
    State(state): State<HttpState>,
) -> axum::response::Sse<impl tokio_stream::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    let mut receiver = state.event_sender.subscribe();
    let stream = async_stream::stream! {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let data = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok(axum::response::sse::Event::default().data(data));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let data = serde_json::to_string(&json!({"type": "lagged", "missed": n})).unwrap_or_default();
                    yield Ok(axum::response::sse::Event::default().data(data));
                }
                Err(_) => break,
            }
        }
    };
    axum::response::Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("ping"),
    )
}

fn filter_and_limit_array(value: Value, keyword: Option<&str>, limit: Option<usize>) -> Value {
    let Some(items) = value.as_array() else {
        return value;
    };
    let keyword = keyword.map(str::to_lowercase);
    let mut filtered = items
        .iter()
        .filter(|item| {
            let Some(keyword) = keyword.as_deref() else {
                return true;
            };
            item.to_string().to_lowercase().contains(keyword)
        })
        .cloned()
        .collect::<Vec<_>>();
    if let Some(limit) = limit {
        filtered.truncate(limit);
    }
    Value::Array(filtered)
}

fn http_ok(data: Value) -> HttpJson {
    (
        StatusCode::OK,
        Json(serde_json::to_value(success(data)).unwrap()),
    )
}

fn http_error(err: AppError) -> HttpJson {
    let status = match err.exit_code {
        2 => StatusCode::BAD_REQUEST,
        3 => StatusCode::PRECONDITION_FAILED,
        4 => StatusCode::FAILED_DEPENDENCY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(serde_json::to_value(failure(err.payload())).unwrap()),
    )
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
