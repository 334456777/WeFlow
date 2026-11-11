"""Measure the commits of a list of requests one by one on the benchmark runner (#601, #603, #688), skipping those
already measured.

usage: python bench-commits.py --requests FILE --bench-dir DIR --tools DIR --index FILE --out DIR [--night]
                               [--backfill]

Run from a checkout of the commit whose tools measure (the workflow's), with the tools already built in `--tools`
(`weflow.exe`, `bench_account.exe`, `serve_bench.exe`). `--bench-dir` is the runner's own folder that outlives a
job, so nothing is installed, downloaded or written again that a run before already has: the accounts
(`accounts/`), the weflow binaries of the commits measured (`bin/<sha>/`), a clone to build them in (`src/`), the
build cache (`target/`), Rust and the Python packages (set up by bench-runner.sh); `profile/profile.json`, when
there, is the account's profile calibrated on this machine (#605), which the account is built from instead of the
default one. `--index` is `bench/data/index.json` of the gh-pages branch (missing: nothing measured yet), which says
which commits each environment already holds and which is its anchor.

`--requests` is a JSON object whose `requests` are what bench_queue.py makes of a run of bench.yml: its commits and
its options (`runs`, `seed`, `group_messages`, `conversations`, `load_seconds`, `only`, `force`), measured in turn.
A commit is a ref (a SHA, a tag, a branch, `abc123^`), or `NAME=REF` to give it a name the viewer shows (such as
`#424前=c054035f^`). For each: resolved to its full SHA; skipped when the environment holds a full result of it (a
result of `only` some parts is measured again by a full run), unless `force`; else built (unless its binary is kept)
and measured by bench.py next to the anchor. An environment with no anchor yet takes the first commit measured. A
commit asked for twice (by two names) is measured once and keeps both names.

Each result is also kept in `<bench dir>/pending/<env key>/<sha>/` as soon as it is measured, until gh-pages holds it:
a run whose results never reached gh-pages (its upload or its publishing failed) loses nothing, and the next run of the
same environment publishes them along with its own, without measuring them again; a later request of the same run
takes them from there too. A commit that does not build or whose measuring stops is left out with a warning (an old
commit may not build with today's tools), and the others go on; only an anchor's failing stops the run.

With `--backfill` (the run at 23:30), a new environment of the default options, which holds no result yet (a new
profile from the calibration of #605, a new machine, measuring method or generator layout), is measured first on the
releases and the PRs labeled `bench` (bench_queue.py's `backfill_commits`, through the API with GITHUB_TOKEN), so it
holds results to compare at once.

With `--night` (the queue taken at night), a commit is built and measured only during bench_queue.py's NIGHT: once it
is over, no further commit starts (the one under way runs to its end). The commits a request has left are postponed:
its run.json says what to ask next (`next`), which the queue keeps for the next night, and the requests after it are
not begun.

Writes, for the i-th request begun, `<out>/<i>/<sha>/result.json` (and its logs) for each commit measured,
`<out>/<i>/run.json`: the request, the environment, the anchor, every commit asked for with its SHA, its name, whether
it was measured, and why not (it failed, it is the same as another, it was postponed), and `<out>/<i>/summary.md`, how
it went (each commit's outcome, how long it took to build and to measure, how busy the machine was, what the version
lacked or failed); `<out>/summary.md` says how many requests were begun. The summaries are also printed to the log;
the publish job puts them on the run's Summary page, whose network is steadier than the runner's (an upload from the
runner that fails is not tried again).
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
from bench_queue import NIGHT, OPTIONS, at_night, backfill_commits, resolve_commits, spec  # noqa: E402

TARGET = "x86_64-pc-windows-msvc"
# the account a run uses when it is given no size, kept whatever the run used
DEFAULT_SIZE = "s1-g200000-c300"


def git(*args, cwd=None):
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


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


def request_summary(run, out, elapsed):
    """How a request went, in Chinese for the one who asked: the environment, then a row per commit."""
    env, anchor, commits, request = run["env"], run["anchor"], run["commits"], run["request"]
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
        # a commit asked for by its SHA (HEAD is queued as one) has no name to show but the start of it
        name = c["ref"][:10] if c["sha"].startswith(c["ref"]) else c["ref"]
        rows.append(f"| {name} | `{c['sha'][:10]}` | {outcome[kind]} | {minutes(c.get('build_s'))} | "
                    f"{minutes(c.get('measure_s'))} | {quiet} | {cmds} | {unsupported} | {failed_runs} | {load} |")
    tally = "，".join(f"{short[k]} {counts[k]}" for k in short)
    postponed = counts["postponed"]
    lines = [
        f"### 请求：{request['run']}" if request.get("run") else "### 请求",
        "",
        f"- 环境 `{env['key']}`：{m['cpu']}，{m['cpus']} 个逻辑 CPU，{m['memory_gib']} GiB 内存；测法版本 {env['method']}",
        f"- 基准账号：种子 {acc['seed']}，{acc['messages']:,} 条消息，{acc['conversations']} 个会话，"
        + (f"profile `{profile['hash']}`（测于 {profile['measured']}）" if profile else "默认 profile"),
        (f"- 锚点 `{anchor[:10]}`" + ("（这个环境的第一个提交，这次新成为锚点）" if run["new_anchor"]
                                       else "（沿用这个环境原有的锚点）")) if anchor
        else "- 锚点：这个环境还没有，由之后第一个测量的提交担任",
        f"- {len(commits)} 个提交：{tally}；用时 {minutes(elapsed)}",
        *([f"- 夜间时段已过，{postponed} 个提交留在队列里，下一个夜间时段再测"] if postponed else []),
        "",
        "| 名字 | 提交 | 结果 | 编译 | 测量 | 其他进程 CPU（p50 / p95） | 命令 | 还没有 | 失败的运行 | 持续负载 |",
        "|---|---|---|---|---|---|---|---|---|---|",
        *rows,
        "",
    ]
    text = "\n".join(lines)
    with open(os.path.join(out, "summary.md"), "w", encoding="utf-8") as f:
        f.write(text)
    print(text, flush=True)


def clean_up(bench_dir, anchors, accounts):
    """What the next run will not use: the binaries but the anchors' (a commit measured is not measured again), and
    the accounts but this run's and the one of the default size."""
    root = os.path.join(bench_dir, "bin")
    for d in os.listdir(root) if os.path.isdir(root) else []:
        if d not in anchors:
            shutil.rmtree(os.path.join(root, d), ignore_errors=True)
    root = os.path.join(bench_dir, "accounts")
    for d in os.listdir(root) if os.path.isdir(root) else []:
        if d not in accounts | {DEFAULT_SIZE}:
            shutil.rmtree(os.path.join(root, d), ignore_errors=True)


def load_pending(pending_dir, index):
    """{env key: {sha: (folder, result)}} of the results measured before but not published yet; those gh-pages holds
    now are removed. A result's own environment says where it belongs, whatever folder it sits in (the folders of an
    older layout, `pending/<sha>/`, are read too)."""
    pending = {}
    os.makedirs(pending_dir, exist_ok=True)
    folders = []
    for name in os.listdir(pending_dir):
        top = os.path.join(pending_dir, name)
        if os.path.exists(os.path.join(top, "result.json")):
            folders.append(top)
        elif os.path.isdir(top):
            folders += [os.path.join(top, sha) for sha in os.listdir(top)]
            if not os.listdir(top):
                os.rmdir(top)
    for folder in folders:
        file = os.path.join(folder, "result.json")
        if not os.path.exists(file):
            shutil.rmtree(folder, ignore_errors=True)
            continue
        with open(file, encoding="utf-8") as f:
            kept = json.load(f)
        key, sha = kept["env"]["key"], kept["commit"]
        if sha in index.get("envs", {}).get(key, {}).get("results", {}):
            shutil.rmtree(folder, ignore_errors=True)
            continue
        pending.setdefault(key, {})[sha] = (folder, kept)
    return pending


class Run:
    """What the requests of one run share: the night being over, the results not published yet, and what to keep."""

    def __init__(self, args, index):
        self.args, self.index = args, index
        self.bench_dir, self.tools = os.path.abspath(args.bench_dir), os.path.abspath(args.tools)
        self.pending_dir = os.path.join(self.bench_dir, "pending")
        self.pending = load_pending(self.pending_dir, index)
        # the pending results some request of this run already carries, which the others do not carry again
        self.carried = set()
        self.late = False
        self.accounts, self.anchors = set(), set()

    def account(self, req):
        """The benchmark account of a request's size, built when needed, and the environment it measures in."""
        seed, group_messages, conversations = (int(req[k]) for k in ("seed", "group_messages", "conversations"))
        # an account per size, kept between runs (about 1.5 GB to write): bench_account builds it again only when its
        # generator's layout, its size or its profile changed
        size = f"s{seed}-g{group_messages}-c{conversations}"
        account = os.path.join(self.bench_dir, "accounts", size)
        self.accounts.add(size)
        # a profile calibrated on this machine (#605), else the default one
        profile = os.path.join(self.bench_dir, "profile", "profile.json")
        subprocess.run([os.path.join(self.tools, "bench_account.exe"), account, "--weflow",
                        os.path.join(self.tools, "weflow.exe"), "--seed", str(seed), "--group-messages",
                        str(group_messages), "--conversations", str(conversations),
                        *(["--profile", profile] if os.path.exists(profile) else [])],
                       check=True, stdout=subprocess.DEVNULL)
        return account, bench.environment_of(account, int(req["load_seconds"]), self.args.load_workers)

    def backfill(self):
        """The request a new environment of the default options starts with (bench_queue.backfill_commits), or None
        when the environment holds results already, here or in pending/. A new profile from the calibration of #605
        makes one; so do a new machine, measuring method or generator layout."""
        req = {**OPTIONS, "force": False, "release_tag": ""}
        key = self.account(req)[1]["key"]
        if self.index.get("envs", {}).get(key, {}).get("results") or self.pending.get(key):
            return None
        # the API out of reach costs the backfill, not the night: the queue is measured, and the backfill is looked for
        # again by the next run of the queue (bench-runner.sh keeps profile-seen as it was)
        try:
            commits = backfill_commits()
        except (OSError, ValueError, KeyError) as e:
            print(f"::warning::environment {key} holds no result yet, but its backfill could not be listed: {e}",
                  flush=True)
            with open(os.path.join(self.args.out, "backfill-unlisted"), "w", encoding="utf-8") as f:
                f.write(f"{e}\n")
            return None
        print(f"environment {key} holds no result yet: measured first on {commits}", flush=True)
        server, repo = os.environ.get("GITHUB_SERVER_URL", "https://github.com"), os.environ.get("GITHUB_REPOSITORY")
        run_id = os.environ.get("GITHUB_RUN_ID", "0")
        return {**req, "id": f"backfill-{key}", "backfill": True,
                "asked_at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
                "run": f"{server}/{repo}/actions/runs/{run_id}" if repo else "",
                "commits": ",".join(spec(c) for c in resolve_commits(commits))}

    def night_over(self):
        if self.args.night and not self.late and not at_night():
            print(f"the night ({NIGHT[0]:%H:%M}-{NIGHT[1]:%H:%M} JST) is over: what is left is postponed", flush=True)
            self.late = True
        return self.late

    def request(self, req, out):
        """Measures one request into `out`: what its run.json holds."""
        started = time.time()
        os.makedirs(out, exist_ok=True)
        args, bench_dir, tools = self.args, self.bench_dir, self.tools
        runs, load_seconds, only, force = int(req["runs"]), int(req["load_seconds"]), req["only"], bool(req["force"])
        account, env = self.account(req)
        key = env["key"]
        print(f"environment {key}: {json.dumps(env, ensure_ascii=False)}", flush=True)
        held = self.index.get("envs", {}).get(key, {})
        anchor = held.get("anchor")
        results = held.get("results", {})
        waiting = self.pending.setdefault(key, {})
        if waiting:
            anchor = anchor or next(iter(waiting.values()))[1]["anchor"]["commit"]

        commits = resolve_commits(req["commits"])
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
            c["measured"] = force or not have or (bool(have.get("only")) and not only)
            if not c["measured"]:
                print(f"== {c['ref']} ({c['sha'][:10]}): already measured in {key}", flush=True)
            elif c["sha"] in waiting and not force:
                c["reused"] = True
        # the pending results nobody asked for go along with the first request of their environment
        asked_shas = {c["sha"] for c in commits}
        extra = [{"ref": r.get("ref") or sha[:10], "sha": sha, "measured": True, "reused": True}
                 for sha, (_, r) in waiting.items() if sha not in asked_shas and (key, sha) not in self.carried]
        if extra:
            print(f"{len(extra)} results measured before but not published: published with this request", flush=True)
        commits += extra
        for c in commits:
            if c.get("reused"):
                self.carried.add((key, c["sha"]))
                os.makedirs(os.path.join(out, c["sha"]), exist_ok=True)
                shutil.copyfile(os.path.join(waiting[c["sha"]][0], "result.json"),
                                os.path.join(out, c["sha"], "result.json"))
        todo = [c for c in commits if c["measured"] and not c.get("reused")]
        new_anchor = bool(todo) and not anchor
        if new_anchor:
            anchor = todo[0]["sha"]
            print(f"environment {key} has no anchor yet: {anchor[:10]} becomes it", flush=True)

        anchor_exe = anchor_alloc = None
        if todo and any(c["sha"] != anchor for c in todo) and not self.night_over():
            anchor_exe, anchor_alloc = build(anchor, bench_dir)
        for c in todo:
            if self.night_over():
                c.update(measured=False, postponed=True)
                continue
            begin = time.time()
            try:
                exe, alloc = build(c["sha"], bench_dir)
            except subprocess.CalledProcessError as e:
                if c["sha"] == anchor:
                    raise
                print(f"::warning::{c['ref']} ({c['sha'][:10]}) does not build ({e.returncode}): left out",
                      flush=True)
                c.update(measured=False, failed="build")
                continue
            c["build_s"] = time.time() - begin
            argv = [sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "bench.py"), "measure",
                    "--exe", exe, "--commit", c["sha"], "--ref", c["ref"], "--account", account,
                    "--out", os.path.join(out, c["sha"]), "--runs", str(runs),
                    "--load-seconds", str(load_seconds), "--load-workers", str(args.load_workers),
                    "--serve-bench", os.path.join(tools, "serve_bench.exe")]
            if alloc:
                argv += ["--alloc", alloc]
            if c["sha"] != anchor:
                argv += ["--anchor", anchor_exe, "--anchor-commit", anchor]
                if anchor_alloc:
                    argv += ["--anchor-alloc", anchor_alloc]
            if only:
                argv += ["--only", only]
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
            keep = os.path.join(self.pending_dir, key, c["sha"])
            os.makedirs(keep, exist_ok=True)
            shutil.copyfile(os.path.join(out, c["sha"], "result.json"), os.path.join(keep, "result.json"))
            with open(os.path.join(keep, "result.json"), encoding="utf-8") as f:
                waiting[c["sha"]] = (keep, json.load(f))
            self.carried.add((key, c["sha"]))
        postponed = [c for c in commits if c.get("postponed")]
        # a new anchor that was postponed is no anchor yet: a later run picks it again, as the first commit it measures
        if new_anchor and any(c["sha"] == anchor for c in postponed):
            anchor, new_anchor = None, False
        if anchor:
            self.anchors.add(anchor)
        # What the queue asks next: every commit asked for (those measured now are held by then, and two commits stay a
        # base and a head); with `force`, the postponed ones alone, which are measured again whatever is held.
        following = None
        if postponed:
            following = {"commits": ",".join(spec(c) for c in postponed) if force else ",".join(asked_specs),
                         "force": force}
        run = {"request": req, "env": env, "anchor": anchor, "new_anchor": new_anchor, "commits": commits,
               "next": following}
        with open(os.path.join(out, "run.json"), "w", encoding="utf-8") as f:
            json.dump(run, f, ensure_ascii=False, indent=1)
        request_summary(run, out, time.time() - started)


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--requests", required=True)
    p.add_argument("--bench-dir", required=True)
    p.add_argument("--tools", required=True)
    p.add_argument("--index", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--load-workers", type=int, default=4)
    p.add_argument("--night", action="store_true", help="start a commit only during NIGHT, postpone the rest")
    p.add_argument("--backfill", action="store_true",
                   help="a new environment of the default options is measured first on the releases and bench PRs")
    args = p.parse_args()
    sys.stdout.reconfigure(encoding="utf-8")
    # The machine must not go to sleep on its own while it measures: a commit timed across a sleep has its wall times
    # off by the time asleep. Held until this process ends.
    if sys.platform == "win32":
        import ctypes
        es_continuous, es_system_required = 0x80000000, 0x00000001
        ctypes.windll.kernel32.SetThreadExecutionState(es_continuous | es_system_required)
    out = os.path.abspath(args.out)
    os.makedirs(out, exist_ok=True)
    with open(args.requests, encoding="utf-8") as f:
        requests = json.load(f)["requests"]
    index = {}
    if os.path.exists(args.index):
        with open(args.index, encoding="utf-8") as f:
            index = json.load(f)

    run = Run(args, index)
    if args.backfill and not run.night_over():
        first = run.backfill()
        if first:
            requests.insert(0, first)
    begun = 0
    for i, req in enumerate(requests):
        # a request is begun only during the night; one cut short by its end leaves those after it as they are
        if run.night_over():
            break
        run.request(req, os.path.join(out, str(i)))
        begun += 1
    run.anchors |= {e.get("anchor") for e in index.get("envs", {}).values()}
    clean_up(run.bench_dir, run.anchors, run.accounts)
    text = (f"## 运行情况\n\n- {len(requests)} 个请求，开始了 {begun} 个"
            + (f"；夜间时段已过，其余 {len(requests) - begun} 个留在队列里" if begun < len(requests) else "")
            + "\n- 「还没有」是这个版本还没有的命令（命令行不认识它的子命令、选项或取值）；「失败的运行」是各命令运行失败的次数之和；"
            "持续负载「还没有」是这个版本的 `weflow serve` 还不认识测量用的参数，「没跑成」是服务没有启动或中途出错。\n\n")
    with open(os.path.join(out, "summary.md"), "w", encoding="utf-8") as f:
        f.write(text)
    print(text, flush=True)


if __name__ == "__main__":
    main()
