"""The benchmark queue (#688): what bench.yml was asked to measure, kept on the gh-pages branch until it is measured.

usage: python bench_queue.py plan --pages DIR --event EVENT [--night] --outputs FILE --summary FILE
                                  (env: COMMITS, RUNS, SEED, GROUP_MESSAGES, CONVERSATIONS, LOAD_SECONDS, ONLY,
                                   FORCE, RELEASE_TAG as bench.yml passes them; GITHUB_RUN_ID and the like)
       python bench_queue.py done --pages DIR --results DIR

The benchmark runner is a machine used by day, so commits are measured at night in Japan ([`NIGHT`]). A run asked for
during the day does not hold a job open until then: its request waits in `<pages>/bench/queue.json`, and the run
bench.yml starts every day at 23:30 (`schedule`) measures the requests there in order (bench-commits.py), until the
night is over. Nothing in the queue is ever cancelled or replaced, as runs waiting in a concurrency group would be.

`plan`, in the queue job, run from a checkout of the repository with its history and tags (to resolve the commits):
  - a run asked for by hand or by release.yml with `night` off goes to the measure job at once (`mode=now`,
    `request=<JSON>`), and the queue is left as it is; so does one started on another branch than the default one
    (`OTHER_BRANCH` names it): the queue is measured and published by the default branch's workflow, while a trial on
    a branch may have turned its own publishing off
  - with `night` on, the request goes to the end of the queue, its commits resolved to full SHAs (a branch or HEAD
    stays the commit it was when asked for); when it is night already, the measure job takes the queue at once
    (`mode=drain`), else the run at 23:30 does (`mode=none`)
  - the run at 23:30: `mode=drain` when the queue holds a request, else `mode=none`
It writes the outputs to `--outputs` and what it did, in Chinese, to `--summary`, both replaced on every call: the
workflow calls it again when its push of the queue lost a race with another one.

`done`, in the publish job: each request the measure job went through (`<results>/<i>/run.json`) leaves the queue when
it was measured through; one the night was over for stays, with what bench-commits.py says to ask next (`next`);
those it did not reach stay as they were.
"""

import argparse
import datetime
import json
import os
import subprocess

# When commits are measured: from 23:30 to 05:00 in Japan, which keeps no daylight saving time. bench.yml's schedule
# (14:30 UTC) and bench-runner.sh hold the same hours.
JST = datetime.timezone(datetime.timedelta(hours=9))
NIGHT = (datetime.time(23, 30), datetime.time(5, 0))
# the options of a request, as bench.yml names its inputs, with their defaults
OPTIONS = {"runs": "5", "seed": "1", "group_messages": "200000", "conversations": "300", "load_seconds": "90",
           "only": ""}


def at_night(now=None):
    """Whether it is NIGHT in Japan."""
    t = (now or datetime.datetime.now(JST)).astimezone(JST).time()
    return t >= NIGHT[0] or t < NIGHT[1]


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


def resolve_commits(text):
    """[{ref, sha}] of a comma separated list of `[NAME=]REF`: a commit is named by NAME, else by REF; a branch moves
    on, so its name alone would later stand for another commit, and the commit goes with it (`branch@sha`)."""
    commits = []
    for item in [r.strip() for r in text.split(",") if r.strip()]:
        name, _, rev = item.rpartition("=")
        name, rev = name.strip(), rev.strip()
        sha = resolve(rev)
        if not name and is_branch(rev):
            name = f"{rev}@{sha[:10]}"
        commits.append({"ref": name or rev, "sha": sha})
    return commits


def spec(c):
    """How a commit is asked for again: its name with its full SHA, so a branch or HEAD stays the commit it was."""
    says_more = c["ref"] != "HEAD" and not c["sha"].startswith(c["ref"])
    return f"{c['ref']}={c['sha']}" if says_more else c["sha"]


def queue_path(pages):
    return os.path.join(pages, "bench", "queue.json")


def load_queue(pages):
    try:
        with open(queue_path(pages), encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        return {"format": 1, "requests": []}


def save_queue(pages, queue):
    os.makedirs(os.path.dirname(queue_path(pages)), exist_ok=True)
    with open(queue_path(pages), "w", encoding="utf-8") as f:
        json.dump(queue, f, ensure_ascii=False, indent=1)


def request_from_env():
    """The request of this run, from what bench.yml passes in the environment."""
    server = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
    run_id = os.environ.get("GITHUB_RUN_ID", "0")
    commits = resolve_commits(os.environ.get("COMMITS", ""))
    if not commits:
        raise SystemExit("no commits asked for")
    return {
        "id": f"{run_id}-{os.environ.get('GITHUB_RUN_ATTEMPT', '1')}",
        "asked_at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "run": f"{server}/{os.environ.get('GITHUB_REPOSITORY', '')}/actions/runs/{run_id}",
        "commits": ",".join(spec(c) for c in commits),
        **{k: (os.environ.get(k.upper()) or default) for k, default in OPTIONS.items()},
        "force": os.environ.get("FORCE") == "1",
        "release_tag": os.environ.get("RELEASE_TAG", ""),
    }


def names(request):
    """The names of a request's commits, for people."""
    return "，".join(item.rpartition("=")[0] or item[:10] for item in request["commits"].split(","))


def plan(args):
    out, said = {"mode": "none"}, []
    night = at_night()
    queue = load_queue(args.pages)
    if args.event == "schedule":
        if queue["requests"]:
            out["mode"] = "drain"
            said.append(f"- 夜间运行：队列里有 {len(queue['requests'])} 个请求，按顺序测到日本时间 {NIGHT[1]:%H:%M}")
        else:
            said.append("- 夜间运行：队列为空，不测量")
    else:
        request = request_from_env()
        branch = os.environ.get("OTHER_BRANCH", "")
        if not args.night or branch:
            out.update(mode="now", request=json.dumps(request, ensure_ascii=False))
            why = f"从分支 {branch} 发起，不进夜间队列" if branch else "关闭了 night"
            said.append(f"- 立即测量（{why}）：{names(request)}")
        else:
            queue["requests"].append(request)
            save_queue(args.pages, queue)
            place = f"排入夜间队列，第 {len(queue['requests'])} 位：{names(request)}"
            if night:
                out["mode"] = "drain"
                said.append(f"- {place}；现在是夜间时段，立即按顺序测量队列")
            else:
                said.append(f"- {place}；日本时间 {NIGHT[0]:%H:%M} 的夜间运行开始测量")
    with open(args.outputs, "w", encoding="utf-8") as f:
        f.write("".join(f"{k}={v}\n" for k, v in out.items()))
    with open(args.summary, "w", encoding="utf-8") as f:
        f.write("## 排队\n\n" + "\n".join(said) + "\n\n")
    print(out["mode"])


def done(args):
    queue = load_queue(args.pages)
    by_id = {r["id"]: r for r in queue["requests"]}
    for name in sorted(os.listdir(args.results)):
        file = os.path.join(args.results, name, "run.json")
        if not os.path.exists(file):
            continue
        with open(file, encoding="utf-8") as f:
            run = json.load(f)
        request = by_id.get(run["request"]["id"])
        # a run asked to measure at once is not in the queue, and another run may have taken a request out already
        if request is None:
            continue
        if run.get("next"):
            request.update(commits=run["next"]["commits"], force=run["next"]["force"])
        else:
            queue["requests"].remove(request)
    save_queue(args.pages, queue)


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    modes = p.add_subparsers(dest="mode", required=True)
    pl = modes.add_parser("plan", help="queue the request of this run, or say to measure it now")
    pl.add_argument("--pages", required=True)
    pl.add_argument("--event", required=True)
    pl.add_argument("--night", action="store_true")
    pl.add_argument("--outputs", required=True)
    pl.add_argument("--summary", required=True)
    d = modes.add_parser("done", help="take the measured requests out of the queue")
    d.add_argument("--pages", required=True)
    d.add_argument("--results", required=True)
    args = p.parse_args()
    plan(args) if args.mode == "plan" else done(args)


if __name__ == "__main__":
    main()
