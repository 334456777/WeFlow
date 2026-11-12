//! `weflow video`: the files of a video message.

use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub(crate) struct VideoCommand {
    #[command(subcommand)]
    pub(crate) command: VideoSubcommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum VideoSubcommand {
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
