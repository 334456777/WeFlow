"""Measure a list of commits one by one on the benchmark runner (#601, #603), skipping those already measured.

usage: python bench-commits.py --commits REF[,REF...] --bench-dir DIR --tools DIR --index FILE --out DIR
                               [--runs N] [--load-seconds S] [--only NAMES] [--seed N] [--group-messages N]
                               [--conversations N] [--force]

Run from a checkout of the commit whose tools measure (the workflow's), with the tools already built in `--tools`
(`weflow.exe`, `bench_account.exe`, `serve_bench.exe`). `--bench-dir` is the runner's own folder that outlives a
job, so nothing is installed, downloaded or written again that a run before already has: the accounts
(`accounts/`), the weflow binaries of the commits measured (`bin/<sha>/`), a clone to build them in (`src/`), the
build cache (`target/`), Rust and the Python packages (set up by bench.yml). `--index` is `bench/data/index.json` of the gh-pages branch (missing:
nothing measured yet), which says which commits the environment already holds and which is its anchor.

For each commit: resolved to its full SHA; skipped when the environment holds a full result of it (a result of
`--only` some parts is measured again by a full run), unless `--force`; else built (unless its binary is kept) and
measured by bench.py next to the anchor. An environment with no anchor yet takes the first commit measured. Writes
`<out>/<sha>/result.json` (and its logs) for each commit measured and `<out>/run.json`: the environment, the anchor
and every commit asked for with its SHA, whether it was measured, and the name it was asked by.
"""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench  # noqa: E402

TARGET = "x86_64-pc-windows-msvc"
# what builds the account: an account is built again only when one of these changes
ACCOUNT_PATHS = ("crates/weflow-native/src/fixture", "crates/weflow-native/src/fixture.rs",
                 "crates/weflow-native/src/sqlcipher", "crates/weflow-cli/examples/bench_account.rs")
# what measures, besides: their versions together name the tools of an environment
TOOL_PATHS = (*ACCOUNT_PATHS, ".github/scripts/bench.py", "crates/weflow-cli/examples/serve_bench.rs")
# commit binaries kept besides the anchor's, the newest first
KEEP_BINARIES = 30


def git(*args, cwd=None):
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def version_of(paths):
    trees = [f"{p}={git('rev-parse', f'HEAD:{p}')}" for p in paths]
    return hashlib.sha256("\n".join(trees).encode()).hexdigest()[:12]


def resolve(ref):
    for name in (ref, f"origin/{ref}"):
        try:
            return git("rev-parse", "--verify", "-q", f"{name}^{{commit}}")
        except subprocess.CalledProcessError:
            continue
    raise SystemExit(f"no commit named {ref!r}")


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


def prune_binaries(bench_dir, anchor):
    root = os.path.join(bench_dir, "bin")
    if not os.path.isdir(root):
        return
    kept = sorted((d for d in os.listdir(root) if d != anchor), key=lambda d: -os.path.getmtime(os.path.join(root, d)))
    for d in kept[KEEP_BINARIES:]:
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
    args = p.parse_args()
    sys.stdout.reconfigure(encoding="utf-8")

    bench_dir, out, tools = (os.path.abspath(x) for x in (args.bench_dir, args.out, args.tools))
    os.makedirs(out, exist_ok=True)
    tid, generator = version_of(TOOL_PATHS), version_of(ACCOUNT_PATHS)
    # an account per version of its generator and size, kept between runs (about 1.5 GB to write): a changed
    # generator never reuses an older account, a change elsewhere in the tools reuses it
    account = os.path.join(bench_dir, "accounts",
                           f"{generator}-s{args.seed}-g{args.group_messages}-c{args.conversations}")
    subprocess.run([os.path.join(tools, "bench_account.exe"), account, "--weflow", os.path.join(tools, "weflow.exe"),
                    "--seed", str(args.seed), "--group-messages", str(args.group_messages),
                    "--conversations", str(args.conversations)], check=True, stdout=subprocess.DEVNULL)
    accounts = os.path.join(bench_dir, "accounts")
    for old in os.listdir(accounts):
        if not old.startswith(generator):
            shutil.rmtree(os.path.join(accounts, old), ignore_errors=True)
    env = bench.environment_of(account, tid, args.load_seconds, args.load_workers)
    key = env["key"]
    print(f"environment {key}: {json.dumps(env, ensure_ascii=False)}", flush=True)

    index = {}
    if os.path.exists(args.index):
        with open(args.index, encoding="utf-8") as f:
            index = json.load(f)
    held = index.get("envs", {}).get(key, {})
    anchor = held.get("anchor")
    results = held.get("results", {})

    asked = [r.strip() for r in args.commits.split(",") if r.strip()]
    commits = [{"ref": r, "sha": resolve(r)} for r in asked]
    print("commits: " + ", ".join(f"{c['ref']} = {c['sha'][:10]}" for c in commits), flush=True)
    for c in commits:
        have = results.get(c["sha"])
        c["measured"] = args.force or not have or (bool(have.get("only")) and not args.only)
        if not c["measured"]:
            print(f"== {c['ref']} ({c['sha'][:10]}): already measured in {key}", flush=True)
    todo = [c for c in commits if c["measured"]]
    if todo and not anchor:
        anchor = todo[0]["sha"]
        print(f"environment {key} has no anchor yet: {anchor[:10]} becomes it", flush=True)

    anchor_exe = anchor_alloc = None
    if todo and any(c["sha"] != anchor for c in todo):
        anchor_exe, anchor_alloc = build(anchor, bench_dir)
    for c in todo:
        exe, alloc = build(c["sha"], bench_dir)
        argv = [sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "bench.py"), "measure",
                "--exe", exe, "--commit", c["sha"], "--ref", c["ref"], "--account", account, "--tools-id", tid,
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
        subprocess.run(argv, check=True)
    prune_binaries(bench_dir, anchor)
    with open(os.path.join(out, "run.json"), "w", encoding="utf-8") as f:
        json.dump({"env": env, "anchor": anchor, "commits": commits}, f, ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()
