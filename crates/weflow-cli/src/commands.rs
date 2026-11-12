//! The subcommands, one file each: a command's clap definition (`XxxCommand`, `XxxSubcommand` and their
//! arguments) beside the function that runs it.

pub(crate) mod backup;
pub(crate) mod cache;
pub(crate) mod config;
pub(crate) mod db;
pub(crate) mod key;
pub(crate) mod runtime;
