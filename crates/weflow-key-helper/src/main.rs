#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use anyhow::Context;
use anyhow::{bail, Result};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help") {
        println!("xkey_helper_rust --db-key [--pid PID] [--timeout SECONDS]");
        return Ok(());
    }
    run(&args)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn run(args: &[String]) -> Result<()> {
    use weflow_key_helper::{linux, scan};
    let mut pid = None;
    let mut timeout = 180u64;
    let mut args = args.iter();
    if args.next().map(String::as_str) != Some("--db-key") {
        bail!("expected --db-key; see --help");
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pid" => {
                pid = Some(
                    args.next()
                        .context("--pid requires a value")?
                        .parse::<i32>()?,
                )
            }
            "--timeout" => timeout = args.next().context("--timeout requires a value")?.parse()?,
            _ => bail!("unknown helper argument"),
        }
    }
    let pid = match pid {
        Some(pid) if pid > 0 => pid,
        Some(_) => bail!("pid must be positive"),
        None => find_pid()?,
    };
    if timeout == 0 || timeout > 3600 {
        bail!("timeout must be between 1 and 3600 seconds");
    }
    let proc = linux::proc_directory(pid)?;
    let executable = std::fs::read_link(proc.join("exe"))?;
    let executable = executable.to_str().context("non-UTF-8 executable path")?;
    let data = std::fs::read(proc.join("exe"))?;
    let target = scan::locate_elf(&data)?;
    let maps = std::fs::read_to_string(proc.join("maps"))?;
    let target = scan::runtime_address(&data, &maps, target, executable)?;
    linux::install_signal_handlers()?;
    eprintln!("Rust helper: unique candidate located; waiting for a 32-byte database key");
    let key = linux::capture(pid, target, std::time::Duration::from_secs(timeout))?;
    let text: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    println!("{text}");
    Ok(())
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn find_pid() -> Result<i32> {
    let mut pids = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Ok(_proc_pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) else {
            continue;
        };
        if matches!(comm.trim(), "wechat" | "WeChat" | "weixin" | "Weixin") {
            let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
                continue;
            };
            if let Some(pid) = weflow_key_helper::linux::namespace_pid(&status) {
                if weflow_key_helper::linux::proc_directory(pid).ok().as_ref()
                    == Some(&entry.path())
                {
                    pids.push(pid);
                }
            }
        }
    }
    match pids.as_slice() {
        [pid] => Ok(*pid),
        [] => bail!("WeChat process not found; start it and use --pid"),
        _ => bail!("multiple WeChat processes found; select one with --pid"),
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn run(_args: &[String]) -> Result<()> {
    bail!("the Rust database-key helper currently supports Linux x86-64 only")
}
