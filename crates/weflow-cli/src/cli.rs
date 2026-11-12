//! The top of the command line: the global options and the list of subcommands (`Commands`), whose definitions are
//! in `commands/`.

use clap::{Parser, Subcommand, ValueEnum};

use crate::commands::analytics::AnalyticsCommand;
use crate::commands::backup::BackupCommand;
use crate::commands::biz::BizCommand;
use crate::commands::cache::CacheCommand;
use crate::commands::chat::ChatCommand;
use crate::commands::config::ConfigCommand;
use crate::commands::db::DbCommand;
use crate::commands::export::ExportCommand;
use crate::commands::group::GroupCommand;
use crate::commands::image::ImageCommand;
use crate::commands::insight::InsightCommand;
use crate::commands::key::KeyCommand;
use crate::commands::report::ReportCommand;
use crate::commands::runtime::RuntimeCommand;
use crate::commands::sns::SnsCommand;
use crate::commands::video::VideoCommand;

use crate::commands::serve::ServeCommand;
use crate::commands::web::WebCommand;

#[derive(Parser, Debug)]
#[command(name = "weflow", version = concat!(env!("WEFLOW_VERSION"), " (", env!("WEFLOW_BUILD_INFO"), ")"), about = "Native CLI for WeFlow")]
pub(crate) struct Cli {
    /// Print JSON (for scripts) instead of the human-readable output
    #[arg(long, global = true)]
    pub(crate) json: bool,
    /// Emit NDJSON progress events on stderr (machine-readable)
    #[arg(long, global = true)]
    pub(crate) progress: bool,
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum LangArg {
    En,
    Zh,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Commands {
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
    /// Download, pick and remove versions of the WebUI that `serve` shows
    Web(WebCommand),
    /// Save the output language
    Lang {
        /// Output language
        #[arg(value_enum)]
        lang: LangArg,
    },
}
