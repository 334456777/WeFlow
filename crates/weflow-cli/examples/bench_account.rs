//! The synthetic account of the benchmark workflow (#259, #488): as big and as varied as a real account, drawn from a
//! seed (see `weflow_native::fixture::bench`).
//!
//! usage: bench_account <work dir> --weflow <weflow binary> [--seed N] [--group-messages N] [--conversations N]
//!                      [--shards N] [--no-media]
//!
//! `<work dir>/data` holds the account, `<work dir>/home` the WeFlow home pointing at it and `<work dir>/manifest.json`
//! what is in it (conversations, messages by type, files written). They are built again only when an option changes;
//! the home is printed on stdout. The defaults: seed 1, a group of 200000 messages, 300 other conversations, 2 message
//! databases, and the files of pictures, videos and files (about 1.5 GB; `--no-media` leaves them out).
#![expect(
    clippy::cast_possible_truncation,
    reason = "probe and benchmark data are small values that fit"
)]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{anyhow, ensure, Context, Result};
use serde_json::{json, Value};
use weflow_native::fixture::bench::{self, BenchSpec};
use weflow_native::sqlcipher::testutil::KEY;

const USAGE: &str =
    "usage: bench_account <work dir> --weflow <weflow binary> [--seed N] [--group-messages N] \
                     [--conversations N] [--shards N] [--no-media]";
/// Bump on any change to what `bench::build` makes, so an account built before is built again.
const LAYOUT: u32 = 3;

fn number(args: &mut impl Iterator<Item = String>, name: &str) -> Result<u64> {
    args.next()
        .ok_or_else(|| anyhow!("{name} needs a value"))?
        .parse()
        .with_context(|| format!("{name} takes a number"))
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let work = PathBuf::from(args.next().ok_or_else(|| anyhow!(USAGE))?);
    std::fs::create_dir_all(&work)?;
    // the home keeps the account's path: from anywhere
    let work = std::path::absolute(&work)?;
    let mut spec = BenchSpec::default();
    let mut exe = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--weflow" => exe = args.next().map(PathBuf::from),
            "--seed" => spec.seed = number(&mut args, &arg)?,
            "--group-messages" => spec.group_messages = number(&mut args, &arg)? as usize,
            "--conversations" => spec.conversations = number(&mut args, &arg)? as usize,
            "--shards" => spec.shards = number(&mut args, &arg)? as usize,
            "--no-media" => spec.media = false,
            other => return Err(anyhow!("unknown argument {other}\n{USAGE}")),
        }
    }
    let exe = exe.ok_or_else(|| anyhow!(USAGE))?;
    let (data, home) = (work.join("data"), work.join("home"));
    let (stamp, manifest) = (work.join("spec.json"), work.join("manifest.json"));
    let wanted = json!({
        "layout": LAYOUT,
        "seed": spec.seed,
        "group_messages": spec.group_messages,
        "conversations": spec.conversations,
        "shards": spec.shards,
        "media": spec.media,
    });
    let built = std::fs::read(&stamp)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    if built.as_ref() != Some(&wanted) || !data.exists() || !home.exists() {
        let started = Instant::now();
        let _ = std::fs::remove_file(&stamp);
        let made = bench::build(&data, &spec).context("build the account")?;
        std::fs::write(&manifest, serde_json::to_vec_pretty(&made)?)?;
        let _ = std::fs::remove_dir_all(&home);
        for (key, value) in [
            (
                "db_path",
                bench::account_dir(&data).to_string_lossy().to_string(),
            ),
            ("wxid", bench::ACCOUNT_DIR.to_string()),
            ("decrypt_key", weflow_native::native_db::hex(&KEY)),
            ("image_xor_key", bench::IMAGE_XOR_KEY.to_string()),
            ("image_aes_key", bench::IMAGE_AES_KEY.to_string()),
        ] {
            let out = Command::new(&exe)
                .env("WEFLOW_HOME", &home)
                .env("WEFLOW_LANG", "en")
                .stdin(Stdio::null())
                .args(["config", "set", key, &value])
                .output()?;
            ensure!(
                out.status.success(),
                "config set {key} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::fs::write(&stamp, serde_json::to_vec(&wanted)?)?;
        eprintln!(
            "built the benchmark account in {:.1} s: {} messages",
            started.elapsed().as_secs_f64(),
            made["messages"]
        );
    }
    println!("{}", home.display());
    Ok(())
}
