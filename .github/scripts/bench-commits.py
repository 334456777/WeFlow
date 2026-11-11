"""Measure a list of commits one by one on the benchmark runner (#601, #603), skipping those already measured.

usage: python bench-commits.py --commits [NAME=]REF[,...] --bench-dir DIR --tools DIR --index FILE --out DIR
                               [--runs N] [--load-seconds S] [--only NAMES] [--seed N] [--group-messages N]
                               [--conversations N] [--force] [--night]

Run from a checkout of the commit whose tools measure (the workflow's), with the tools already built in `--tools`
(`weflow.exe`, `bench_account.exe`, `serve_bench.exe`). `--bench-dir` is the runner's own folder that outlives a
job, so nothing is installed, downloaded or written again that a run before already has: the accounts
(`accounts/`), the weflow binaries of the commits measured (`bin/<sha>/`), a clone to build them in (`src/`), the
build cache (`target/`), Rust and the Python packages (set up by bench-runner.sh); `profile/profile.json`, when
there, is the account's profile calibrated on this machine (#605), which the account is built from instead of the
default one. `--index` is `bench/data/index.json` of the gh-pages branch (missing: nothing measured yet), which says
which commits the environment already holds and which is its anchor.

A commit is a ref (a SHA, a tag, a branch, `abc123^`), or `NAME=REF` to give it a name the viewer shows (such as
`#424前=c054035f^`). For each: resolved to its full SHA; skipped when the environment holds a full result of it (a
result of `--only` some parts is measured again by a full run), unless `--force`; else built (unless its binary is
kept) and measured by bench.py next to the anchor. An environment with no anchor yet takes the first commit measured.
Each result is also kept in `<bench dir>/pending/<sha>/` as soon as it is measured, until gh-pages holds it: a run
whose results never reached gh-pages (its upload or its publishing failed) loses nothing, and the next run of the same
environment publishes them along with its own, without measuring them again.
A commit that does not build or whose measuring stops is left out with a warning (an old commit may not build with
today's tools), and the others go on; only the anchor's failing stops the run. A commit asked for twice (by two
names) is measured once and keeps both names.

With `--night`, a commit is built and measured only while it is night in Japan ([`NIGHT`], 23:30 to 05:00), when
nobody uses the machine: bench.yml waits for the night before it starts this, and once the night is over no further
commit starts (the one under way runs to its end). The commits left are postponed: run.json says how to ask for them
again, and the publish job starts another run with them, which waits for the next night.

Writes `<out>/<sha>/result.json` (and its logs) for each commit measured, `<out>/run.json`: the environment, the
anchor, every commit asked for with its SHA, its name, whether it was measured, and why not (it failed, it is the same
as another, it was postponed), and what to ask the next run when some were postponed (`next`), and `<out>/summary.md`,
how the run went (each commit's outcome, how long it took to build and to measure, how busy the machine was, what the
version lacked or failed), also printed to the log; the publish job puts it on the run's Summary page, whose network is
steadier than the runner's (an upload from the runner that fails is not tried again).
"""

import argparse
import datetime
import json
import os
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench  # noqa: E402

TARGET = "x86_64-pc-windows-msvc"
# the account a run uses when it is given no size
DEFAULT_SIZE = "s1-g200000-c300"
# When a run with --night may start a commit: from 23:30 to 05:00 in Japan, which keeps no daylight saving time.
# bench-runner.sh waits for the same start.
JST = datetime.timezone(datetime.timedelta(hours=9))
NIGHT = (datetime.time(23, 30), datetime.time(5, 0))


def at_night(now=None):
    """Whether it is NIGHT in Japan."""
    t = (now or datetime.datetime.now(JST)).astimezone(JST).time()
    return t >= NIGHT[0] or t < NIGHT[1]


def spec(c):
    """How a commit is asked for again: its name with its full SHA, so a branch or HEAD stays the commit it was."""
    says_more = c["ref"] != "HEAD" and not c["sha"].startswith(c["ref"])
    return f"{c['ref']}={c['sha']}" if says_more else c["sha"]


def git(*args, cwd=None):
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def resolve(ref):
    for name in (ref, f"origin/{ref}"):
        try:
            return git("rev-parse", "--verify", "-q", f"{name}^{{commit}}")
        except subprocess.CalledProcessError:
            continue
    raise SystemExit(f"no commit named {ref!r}")


def is_branch(ref):
    """Whether `ref` names a branch (here or on origin) and no tag; HEAD is no branch's name."""
    def has(full):
        return subprocess.run(["git", "show-ref", "--verify", "-q", full]).returncode == 0
    return ref != "HEAD" and not has(f"refs/tags/{ref}") and (has(f"refs/heads/{ref}")
                                                             or has(f"refs/remotes/origin/{ref}"))


def build(sha, bench_dir):
    """The weflow binary of `sha` (and its alloc-stats build when it has the feature), built once and kept."""
    kept = os.path.join(bench_dir, "bin", sha)
    exe, alloc = os.path.join(kept, "weflow.exe"), os.path.join(kept, "weflow-alloc.exe")
    if os.path.exists(exe):
        os.utime(kept)
        return exe, alloc if os.path.exists(alloc) else None
    # a clone of its own, so it outlives the job's checkout; the repository is public, so no token goes in it
    src = os.path.join(bench_dir, "src")
    if not os.path.isdir(os.path.join(src, ".git")):
        shutil.rmtree(src, ignore_errors=True)
        git("init", "-q", src)
        git("remote", "add", "origin", git("remote", "get-url", "origin"), cwd=src)
    git("fetch", "-q", "--depth=1", "origin", sha, cwd=src)
    git("checkout", "-q", "--detach", "--force", sha, cwd=src)
    git("clean", "-q", "-fdx", cwd=src)
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(bench_dir, "target"))
    # a commit older than rust-toolchain.toml builds with the tools' own toolchain (rustup has no default here)
    if not os.path.exists(os.path.join(src, "rust-toolchain.toml")):
        env["RUSTUP_TOOLCHAIN"] = channel_of(os.getcwd())
    note_toolchain(bench_dir, env.get("RUSTUP_TOOLCHAIN") or channel_of(src))
    cargo = ["cargo", "build", "--release", "-p", "weflow-cli", "--bin", "weflow", "--target", TARGET]
    print(f"== build {sha[:10]}", flush=True)
    subprocess.run(cargo, cwd=src, env=env, check=True)
    built = os.path.join(bench_dir, "target", TARGET, "release", "weflow.exe")
    os.makedirs(kept, exist_ok=True)
    shutil.copy2(built, exe)
    with open(os.path.join(src, "crates", "weflow-cli", "Cargo.toml"), encoding="utf-8") as f:
        has_alloc = any(line.startswith("alloc-stats") for line in f)
    # the same commit counting its allocations (#513), when it has the feature; only weflow-cli is rebuilt
    if has_alloc:
        subprocess.run([*cargo, "--features", "alloc-stats"], cwd=src, env=env, check=True)
        shutil.copy2(built, alloc)
    return exe, alloc if has_alloc else None


def channel_of(root):
    """The Rust channel the checkout at `root` names in its rust-toolchain.toml, or None."""
    try:
        with open(os.path.join(root, "rust-toolchain.toml"), encoding="utf-8") as f:
            return next(line.split("=", 1)[1].strip().strip('"') for line in f if line.strip().startswith("channel"))
    except (OSError, StopIteration):
        return None


def note_toolchain(bench_dir, channel):
    """Notes when the Rust toolchain `channel` was last used, in `toolchains-used.txt`, so that the clean-up of
    bench-runner.sh keeps the ones used lately: a backfill spread over several runs downloads each once."""
    if not channel:
        return
    with open(os.path.join(bench_dir, "toolchains-used.txt"), "a", encoding="utf-8") as f:
        f.write(f"{channel} {int(time.time())}\n")


def minutes(seconds):
    if seconds is None:
        return "-"
    if seconds < 60:
        return "不到 1 分钟"
    hours, mins = divmod(round(seconds / 60), 60)
    return f"{hours} 小时 {mins} 分钟" if hours else f"{mins} 分钟"


def load_outcome(result):
    """Whether the sustained load of a result ran: 有, 还没有 (the version's serve refuses what the load asks of it)
    or 没跑成."""
    if result.get("load"):
        return "有"
    said = result.get("load_failed") or ""
    return "还没有" if any(s in said for s in bench.UNSUPPORTED) else "没跑成"


def run_summary(env, anchor, new_anchor, commits, out, elapsed, postponed):
    """How the run went, in Chinese for the one who started it: the environment, then a row per commit."""
    m, acc = env["machine"], env["account"]
    profile = acc.get("profile") or {}
    outcome = {"measured": "测完", "reused": "沿用之前测完、没发布的结果", "held": "已测过，跳过",
               "build": "编译失败，跳过", "measure": "测量中断，跳过", "same": "与前面同名提交相同，一并记名",
               "postponed": "夜间时段已过，留到下一次"}
    short = {"measured": "测完", "held": "已测过", "reused": "沿用", "build": "编译失败", "measure": "测量中断",
             "same": "重复", "postponed": "延后"}
    counts = dict.fromkeys(short, 0)
    rows = []
    for c in commits:
        kind = c.get("failed") or ("same" if c.get("same_as") else "postponed" if c.get("postponed")
                                   else "reused" if c.get("reused") else "measured" if c["measured"] else "held")
        counts[kind] += 1
        quiet = cmds = unsupported = failed_runs = load = "-"
        file = os.path.join(out, c["sha"], "result.json")
        if c["measured"] and os.path.exists(file):
            with open(file, encoding="utf-8") as f:
                r = json.load(f)
            q = r.get("quiet") or {}
            quiet = f"{q.get('p50_pct', '-')}% / {q.get('p95_pct', '-')}%"
            entries = (r.get("commands") or {}).values()
            cmds = str(len(entries))
            unsupported = str(sum(1 for e in entries if e.get("unsupported")))
            failed_runs = str(sum(e.get("failed_runs", 0) for e in entries))
            load = load_outcome(r)
        rows.append(f"| {c['ref']} | `{c['sha'][:10]}` | {outcome[kind]} | {minutes(c.get('build_s'))} | "
                    f"{minutes(c.get('measure_s'))} | {quiet} | {cmds} | {unsupported} | {failed_runs} | {load} |")
    tally = "，".join(f"{short[k]} {counts[k]}" for k in short)
    lines = [
        "## 运行情况",
        "",
        f"- 环境 `{env['key']}`：{m['cpu']}，{m['cpus']} 个逻辑 CPU，{m['memory_gib']} GiB 内存；测法版本 {env['method']}",
        f"- 基准账号：种子 {acc['seed']}，{acc['messages']:,} 条消息，{acc['conversations']} 个会话，"
        + (f"profile `{profile['hash']}`（测于 {profile['measured']}）" if profile else "默认 profile"),
        (f"- 锚点 `{anchor[:10]}`" + ("（这个环境的第一个提交，这次新成为锚点）" if new_anchor
                                       else "（沿用这个环境原有的锚点）")) if anchor
        else "- 锚点：这个环境还没有，由下一次运行第一个测量的提交担任",
        f"- {len(commits)} 个提交：{tally}；用时 {minutes(elapsed)}",
        *([(f"- 日本时间 {NIGHT[0]:%H:%M}–{NIGHT[1]:%H:%M} 的夜间时段已过，{postponed} 个提交延后：publish 另起一次运行，"
            "等到下一个夜间时段再测")] if postponed else []),
        "",
        "| 名字 | 提交 | 结果 | 编译 | 测量 | 其他进程 CPU（p50 / p95） | 命令 | 还没有 | 失败的运行 | 持续负载 |",
        "|---|---|---|---|---|---|---|---|---|---|",
        *rows,
        "",
        "「还没有」是这个版本还没有的命令（命令行不认识它的子命令、选项或取值）；「失败的运行」是各命令运行失败的次数之和；"
        "持续负载「还没有」是这个版本的 `weflow serve` 还不认识测量用的参数，「没跑成」是服务没有启动或中途出错。",
        "",
    ]
    text = "\n".join(lines)
    with open(os.path.join(out, "summary.md"), "w", encoding="utf-8") as f:
        f.write(text)
    print(text, flush=True)


def clean_up(bench_dir, anchors, account):
    """What the next run will not use: the binaries but the anchors' (a commit measured is not measured again), and
    the accounts but this run's and the one of the default size."""
    root = os.path.join(bench_dir, "bin")
    for d in os.listdir(root) if os.path.isdir(root) else []:
        if d not in anchors:
            shutil.rmtree(os.path.join(root, d), ignore_errors=True)
    root, used = os.path.split(account)
    for d in os.listdir(root):
        if d not in (used, DEFAULT_SIZE):
            shutil.rmtree(os.path.join(root, d), ignore_errors=True)


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--commits", required=True)
    p.add_argument("--bench-dir", required=True)
    p.add_argument("--tools", required=True)
    p.add_argument("--index", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--runs", type=int, default=5)
    p.add_argument("--load-seconds", type=int, default=90)
    p.add_argument("--load-workers", type=int, default=4)
    p.add_argument("--only", default="")
    p.add_argument("--seed", type=int, default=1)
    p.add_argument("--group-messages", type=int, default=200000)
    p.add_argument("--conversations", type=int, default=300)
    p.add_argument("--force", action="store_true")
    p.add_argument("--night", action="store_true", help="start a commit only during NIGHT, postpone the rest")
    args = p.parse_args()
    started = time.time()
    sys.stdout.reconfigure(encoding="utf-8")

    bench_dir, out, tools = (os.path.abspath(x) for x in (args.bench_dir, args.out, args.tools))
    os.makedirs(out, exist_ok=True)
    # an account per size, kept between runs (about 1.5 GB to write): bench_account builds it again only when its
    # generator's layout, its size or its profile changed
    account = os.path.join(bench_dir, "accounts", f"s{args.seed}-g{args.group_messages}-c{args.conversations}")
    # a profile calibrated on this machine (#605), else the default one
    profile = os.path.join(bench_dir, "profile", "profile.json")
    subprocess.run([os.path.join(tools, "bench_account.exe"), account, "--weflow", os.path.join(tools, "weflow.exe"),
                    "--seed", str(args.seed), "--group-messages", str(args.group_messages),
                    "--conversations", str(args.conversations),
                    *(["--profile", profile] if os.path.exists(profile) else [])],
                   check=True, stdout=subprocess.DEVNULL)
    env = bench.environment_of(account, args.load_seconds, args.load_workers)
    key = env["key"]
    print(f"environment {key}: {json.dumps(env, ensure_ascii=False)}", flush=True)

    index = {}
    if os.path.exists(args.index):
        with open(args.index, encoding="utf-8") as f:
            index = json.load(f)
    held = index.get("envs", {}).get(key, {})
    anchor = held.get("anchor")
    results = held.get("results", {})

    # results measured but not published yet; those gh-pages holds now are dropped
    pending_dir = os.path.join(bench_dir, "pending")
    os.makedirs(pending_dir, exist_ok=True)
    pending = {}
    for sha in os.listdir(pending_dir):
        file = os.path.join(pending_dir, sha, "result.json")
        if sha in results or not os.path.exists(file):
            shutil.rmtree(os.path.join(pending_dir, sha), ignore_errors=True)
            continue
        with open(file, encoding="utf-8") as f:
            kept = json.load(f)
        if kept["env"]["key"] == key:
            pending[sha] = kept
    if pending:
        print(f"{len(pending)} results measured before but not published: published with this run", flush=True)
        anchor = anchor or next(iter(pending.values()))["anchor"]["commit"]

    asked = [r.strip() for r in args.commits.split(",") if r.strip()]
    commits = []
    for item in asked:
        name, _, rev = item.rpartition("=")
        name, rev = name.strip(), rev.strip()
        sha = resolve(rev)
        # a branch moves on: its name alone would later stand for another commit, so the commit goes with it
        if not name and is_branch(rev):
            name = f"{rev}@{sha[:10]}"
        commits.append({"ref": name or rev, "sha": sha})
    print("commits: " + ", ".join(f"{c['ref']} = {c['sha'][:10]}" for c in commits), flush=True)
    asked_specs = [spec(c) for c in commits]
    first_name = {}
    for c in commits:
        # the same commit by another name: measured once, the publish gives it both names
        if c["sha"] in first_name:
            c.update(measured=False, same_as=first_name[c["sha"]])
            print(f"== {c['ref']} ({c['sha'][:10]}): the same commit as {c['same_as']}", flush=True)
            continue
        first_name[c["sha"]] = c["ref"]
        have = results.get(c["sha"])
        c["measured"] = args.force or not have or (bool(have.get("only")) and not args.only)
        if not c["measured"]:
            print(f"== {c['ref']} ({c['sha'][:10]}): already measured in {key}", flush=True)
        elif c["sha"] in pending and not args.force:
            c["reused"] = True
    # the pending results nobody asked for this time go along too
    asked_shas = {c["sha"] for c in commits}
    commits += [{"ref": r.get("ref") or sha[:10], "sha": sha, "measured": True, "reused": True}
                for sha, r in pending.items() if sha not in asked_shas]
    for c in commits:
        if c.get("reused"):
            os.makedirs(os.path.join(out, c["sha"]), exist_ok=True)
            shutil.copyfile(os.path.join(pending_dir, c["sha"], "result.json"), os.path.join(out, c["sha"], "result.json"))
    todo = [c for c in commits if c["measured"] and not c.get("reused")]
    new_anchor = bool(todo) and not anchor
    if new_anchor:
        anchor = todo[0]["sha"]
        print(f"environment {key} has no anchor yet: {anchor[:10]} becomes it", flush=True)

    def night_over():
        if args.night and not at_night():
            print(f"the night ({NIGHT[0]:%H:%M}-{NIGHT[1]:%H:%M} JST) is over: the commits left are postponed",
                  flush=True)
            return True
        return False

    anchor_exe = anchor_alloc = None
    late = False
    if todo and any(c["sha"] != anchor for c in todo):
        late = night_over()
        if not late:
            anchor_exe, anchor_alloc = build(anchor, bench_dir)
    for c in todo:
        late = late or night_over()
        if late:
            c.update(measured=False, postponed=True)
            continue
        begin = time.time()
        try:
            exe, alloc = build(c["sha"], bench_dir)
        except subprocess.CalledProcessError as e:
            if c["sha"] == anchor:
                raise
            print(f"::warning::{c['ref']} ({c['sha'][:10]}) does not build ({e.returncode}): left out", flush=True)
            c.update(measured=False, failed="build")
            continue
        c["build_s"] = time.time() - begin
        argv = [sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "bench.py"), "measure",
                "--exe", exe, "--commit", c["sha"], "--ref", c["ref"], "--account", account,
                "--out", os.path.join(out, c["sha"]), "--runs", str(args.runs),
                "--load-seconds", str(args.load_seconds), "--load-workers", str(args.load_workers),
                "--serve-bench", os.path.join(tools, "serve_bench.exe")]
        if alloc:
            argv += ["--alloc", alloc]
        if c["sha"] != anchor:
            argv += ["--anchor", anchor_exe, "--anchor-commit", anchor]
            if anchor_alloc:
                argv += ["--anchor-alloc", anchor_alloc]
        if args.only:
            argv += ["--only", args.only]
        print(f"== measure {c['ref']} ({c['sha'][:10]})", flush=True)
        begin = time.time()
        code = subprocess.run(argv).returncode
        c["measure_s"] = time.time() - begin
        if code:
            if c["sha"] == anchor:
                raise SystemExit(f"the anchor {anchor[:10]} could not be measured ({code})")
            print(f"::warning::measuring {c['ref']} ({c['sha'][:10]}) stopped ({code}): left out", flush=True)
            c.update(measured=False, failed="measure")
            continue
        os.makedirs(os.path.join(pending_dir, c["sha"]), exist_ok=True)
        shutil.copyfile(os.path.join(out, c["sha"], "result.json"), os.path.join(pending_dir, c["sha"], "result.json"))
    postponed = [c for c in commits if c.get("postponed")]
    # a new anchor that was postponed is no anchor yet: the next run picks it again, as the first commit it measures
    if new_anchor and any(c["sha"] == anchor for c in postponed):
        anchor, new_anchor = None, False
    # The next run gets every commit asked for (those measured now are held by then, and two commits stay a base and a
    # head); with --force, the postponed ones alone, which it measures again whatever the environment holds.
    following = None
    if postponed:
        following = {"commits": ",".join(spec(c) for c in postponed) if args.force else ",".join(asked_specs),
                     "force": args.force}
    clean_up(bench_dir, {anchor} | {e.get("anchor") for e in index.get("envs", {}).values()}, account)
    with open(os.path.join(out, "run.json"), "w", encoding="utf-8") as f:
        json.dump({"env": env, "anchor": anchor, "commits": commits, "next": following}, f, ensure_ascii=False,
                  indent=1)
    run_summary(env, anchor, new_anchor, commits, out, time.time() - started, len(postponed))


if __name__ == "__main__":
    main()
