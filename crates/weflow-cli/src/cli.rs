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
use crate::commands::sns::SnsCommand;
use crate::commands::video::VideoCommand;

use crate::commands::serve::ServeCommand;
use crate::commands::web::WebCommand;

/// Where the global options stand in every command's help: after the command's own options, whose places clap counts
/// from 0 in the order they are defined.
const GLOBAL_OPTIONS_ORDER: usize = 1000;

#[derive(Parser, Debug)]
#[command(name = "weflow", version = concat!(env!("WEFLOW_VERSION"), " (", env!("WEFLOW_BUILD_INFO"), ")"), about = "Native CLI for WeFlow")]
pub(crate) struct Cli {
    /// Print JSON for scripts instead of the human-readable output
    #[arg(long, global = true, display_order = GLOBAL_OPTIONS_ORDER)]
    pub(crate) json: bool,
    /// Emit machine-readable NDJSON progress events on stderr
    #[arg(long, global = true, display_order = GLOBAL_OPTIONS_ORDER + 1)]
    pub(crate) progress: bool,
    /// Show every field of the answer, as --json has them, instead of the short human-readable view
    #[arg(long, global = true, display_order = GLOBAL_OPTIONS_ORDER + 2)]
    pub(crate) verbose: bool,
    /// Draw bars and rules in the human-readable output with ASCII characters only
    #[arg(long, global = true, display_order = GLOBAL_OPTIONS_ORDER + 3)]
    pub(crate) ascii: bool,
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
    /// Read sessions, messages, contacts and media
    Chat(ChatCommand),
    /// Export sessions, contacts, messages, footprint and media
    Export(ExportCommand),
    /// Overall statistics, rankings and time distribution
    Analytics(AnalyticsCommand),
    /// Group chat members and statistics
    Group(GroupCommand),
    /// Annual and dual-person reports
    Report(ReportCommand),
    /// Moments, WeChat's SNS: timeline, export and media
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
    /// Create, inspect and restore backups
    Backup(BackupCommand),
    /// List and clear WeFlow's caches
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
