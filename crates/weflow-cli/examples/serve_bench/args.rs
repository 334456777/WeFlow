//! The command line of the benchmark.

use std::path::PathBuf;

use anyhow::{anyhow, bail, ensure, Result};

use super::{PAGE, SHARDS};

pub(crate) struct Args {
    pub(crate) work: PathBuf,
    pub(crate) weflow: Option<PathBuf>,
    /// The WeFlow home of the real account.
    pub(crate) home: PathBuf,
    pub(crate) runs: usize,
    pub(crate) messages: i64,
    pub(crate) sessions: usize,
    pub(crate) contacts: usize,
    pub(crate) keyword: String,
    pub(crate) save: Option<PathBuf>,
    pub(crate) compare: Option<PathBuf>,
    pub(crate) tolerance: f64,
    pub(crate) serve: Vec<String>,
}

pub(crate) fn parse_args() -> Result<Args> {
    let mut it = std::env::args().skip(1);
    const USAGE: &str = "usage: serve_bench <work dir> --home <WEFLOW_HOME> [--weflow PATH] [--runs N] [--messages N] [--sessions N] [--contacts N] [--keyword K] [--save FILE] [--compare FILE] [--tolerance X] [-- <serve arguments>]";
    let work = it
        .next()
        .filter(|a| !a.starts_with('-'))
        .ok_or_else(|| anyhow!(USAGE))?;
    let mut home = None;
    let mut args = Args {
        work: work.into(),
        weflow: None,
        home: PathBuf::new(),
        runs: 10,
        messages: 200_000,
        sessions: 500,
        contacts: 3000,
        keyword: "好".into(),
        save: None,
        compare: None,
        tolerance: 2.0,
        serve: Vec::new(),
    };
    while let Some(flag) = it.next() {
        if flag == "--" {
            args.serve = it.by_ref().collect();
            break;
        }
        let value = it.next().ok_or_else(|| anyhow!("{flag} needs a value"))?;
        match flag.as_str() {
            "--weflow" => args.weflow = Some(value.into()),
            "--home" => home = Some(PathBuf::from(value)),
            "--runs" => args.runs = value.parse()?,
            "--messages" => args.messages = value.parse()?,
            "--sessions" => args.sessions = value.parse()?,
            "--contacts" => args.contacts = value.parse()?,
            "--keyword" => args.keyword = value,
            "--save" => args.save = Some(value.into()),
            "--compare" => args.compare = Some(value.into()),
            "--tolerance" => args.tolerance = value.parse()?,
            _ => bail!("unknown option {flag}"),
        }
    }
    args.home = home.ok_or_else(|| {
        anyhow!("--home is required: the real account measures the speed, the synthetic one only checks the answers\n{USAGE}")
    })?;
    ensure!(args.runs > 0, "--runs must be at least 1");
    ensure!(
        args.messages >= SHARDS * PAGE * 4,
        "--messages must be at least {}",
        SHARDS * PAGE * 4
    );
    ensure!(args.sessions > 0, "--sessions must be at least 1");
    Ok(args)
}

pub(crate) fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
