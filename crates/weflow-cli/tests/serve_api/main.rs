//! End-to-end tests of the command API (`weflow serve`): a real server process, real HTTP requests, and the
//! same commands run directly for comparison, over a synthetic encrypted account.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers fail the test by panicking"
)]
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "test values are known to fit, or are meant to wrap"
)]

mod biz;
mod commands;
mod common;
mod contract;
mod events;
mod exports;
mod in_process;
mod jobs;
mod media;
mod server;
mod usage;
