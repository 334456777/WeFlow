"""The benchmark queue (#688): what bench.yml was asked to measure, kept on the gh-pages branch until it is measured.

usage: python bench_queue.py plan --pages DIR --event EVENT [--night] --outputs FILE --summary FILE
                                  (env: COMMITS, RUNS, SEED, GROUP_MESSAGES, CONVERSATIONS, LOAD_SECONDS, ONLY,
                                   FORCE, RELEASE_TAG, PR as bench.yml passes them; GITHUB_RUN_ID and the like)
       python bench_queue.py done --pages DIR --results DIR

Commits are measured at night in Japan ([`NIGHT`]). A run asked for
during the day does not hold a job open until then: its request waits in `<pages>/bench/queue.json`, and a night's run
of bench.yml measures the requests there in order (bench-commits.py), from 23:30 until the night is over. A night's run
(`--event night`) is one started by hand on the default branch with no commits, which the runner's computer starts at
23:25 (`.github/bench/night.ps1`, #1001) and which waits for 23:30 in the queue job, on GitHub's machine. Nothing in
the queue is ever cancelled or replaced, as runs waiting in a concurrency group would be.

`plan`, in the queue job, run from a checkout of the repository with its history and tags (to resolve the commits):
  - a run asked for by hand or by release.yml with `night` off goes to the measure job at once (`mode=now`,
    `request=<JSON>`), and the queue is left as it is; so does one started on another branch than the default one
    (`OTHER_BRANCH` names it): the queue is measured and published by the default branch's workflow, while a trial on
    a branch may have turned its own publishing off
  - with `night` on, the request goes to the end of the queue, its commits resolved to full SHAs (a branch or HEAD
    stays the commit it was when asked for); when it is night already, the measure job takes the queue at once
    (`mode=drain`), else a night's run does (`mode=none`)
  - a night's run first puts into the queue the PRs labeled `bench` merged since it last looked, each measured before
    and after it ([`queue_bench_prs`]), with the PR's number (`pr`), under which the publish job comments the
    comparison; the queue's `labeled` holds the PRs it looked at already, so that each is queued once. One started
    on another branch than the default one leaves the queue alone (`mode=none`): the queue is the default branch's
  - a night's run during the night: `mode=drain` when the queue holds a request; with an empty queue, only on
    Friday nights ([`calibration_night`]), so that the runner looks whether the calibration of #605 changed the
    profile: a new profile is a new environment, and bench-commits.py `--backfill` measures it first on
    [`backfill_requests`], from 23:30, the night's whole time. Other nights with an empty queue do not use the runner
    (`mode=none`)
  - one measure job a night takes the queue: the queue's `taken` notes the night ([`night_began`]) a run sent one for
    it, and a later night's run that night does not use the runner, as one that comes after the night does
    not; a request asked for during the night still goes to the runner at once, since the job that took the queue
    before does not see it
It writes the outputs to `--outputs` and what it did, in Chinese, to `--summary`, both replaced on every call: the
workflow calls it again when its push of the queue lost a race with another one.

`done`, in the publish job: each request the measure job went through (`<results>/<i>/run.json`) leaves the queue when
it was measured through; one the night was over for stays, with what bench-commits.py says to ask next (`next`);
those it did not reach stay as they were. A backfill the night was over for, which was in no queue, goes to its front.
A request measured through outside the queue (at once, or a backfill) takes out the queue's requests that ask for the
same ([`asks_alike`]): a PR measured by hand with its number, or a release's notes written, is not measured again,
and the PR joins the queue's `labeled`, so that a night's run does not queue it either.
"""

import argparse
import datetime
import json
import os
import re
import subprocess
import urllib.request

# When commits are measured: from 23:30 to 05:00 in Japan, which keeps no daylight saving time. bench.yml's wait for
# the night and bench-runner.sh hold the same hours.
JST = datetime.timezone(datetime.timedelta(hours=9))
NIGHT = (datetime.time(23, 30), datetime.time(5, 0))
# the options of a request, as bench.yml names its inputs, with their defaults
OPTIONS = {"runs": "5", "seed": "1", "group_messages": "200000", "conversations": "300", "load_seconds": "90",
           "only": ""}


def at_night(now=None):
    """Whether it is NIGHT in Japan."""
    t = (now or datetime.datetime.now(JST)).astimezone(JST).time()
    return t >= NIGHT[0] or t < NIGHT[1]


def night_began(now=None):
    """The day in Japan the night of `now` begins on at 23:30: the day before until the night is over (a run that comes
    after midnight still belongs to the night before), else the day itself."""
    now = (now or datetime.datetime.now(JST)).astimezone(JST)
    return now.date() - datetime.timedelta(days=1) if now.time() < NIGHT[1] else now.date()


def calibration_night(now=None):
    """Whether this is the night after the calibration of #605, which runs on Fridays at 02:40 in Japan: the night that
    begins on Friday at 23:30."""
    return night_began(now).weekday() == 4


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout.strip()


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


def merged_here(sha, into="HEAD"):
    """Whether the commit `sha` is in the history of `into`, by default the checkout's HEAD, which for a night's run is
    the default branch."""
    return subprocess.run(["git", "merge-base", "--is-ancestor", sha, into], capture_output=True).returncode == 0


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


def api(path):
    """A GET of the repository's REST API, with the job's token when it has one."""
    headers = {"Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"}
    if os.environ.get("GITHUB_TOKEN"):
        headers["Authorization"] = f"Bearer {os.environ['GITHUB_TOKEN']}"
    url = f"https://api.github.com/repos/{os.environ.get('GITHUB_REPOSITORY', '334456777/WeFlow')}{path}"
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=60) as reply:
        return json.load(reply)


def merged_bench_prs():
    """The PRs labeled `bench` that were merged, the oldest merged first, as the issues API lists them (`number`,
    `pull_request.merged_at`; no merge commit): of the hundred updated last, which hold every PR merged or labeled
    lately."""
    items = api("/issues?labels=bench&state=closed&sort=updated&direction=desc&per_page=100")
    merged = [i for i in items if (i.get("pull_request") or {}).get("merged_at")]
    return sorted(merged, key=lambda i: i["pull_request"]["merged_at"])


def merge_commit(number):
    """The commit a merged PR made on its base branch; the PRs of a stack merged together share one."""
    return api(f"/pulls/{number}")["merge_commit_sha"]


def pr_commits(number, sha):
    """What a merged PR is measured on: before it (`#N前`, its merge commit's first parent) and after it (`#N后`)."""
    return f"#{number}前={sha}^,#{number}后={sha}"


def last_merges(count):
    """[(PR number, merge commit)] of the `count` merge commits of PRs labeled `bench` that came into the checkout's
    HEAD last, the oldest first. The PRs of a stack merged together share a merge commit, which counts once, named by
    the first of them as in [`queue_bench_prs`]; a PR merged into another branch counts once its merge commit is in
    the default branch."""
    picked = {}
    for pr in reversed(merged_bench_prs()):
        sha = merge_commit(pr["number"])
        if not sha or not merged_here(sha):
            continue
        if sha not in picked and len(picked) == count:
            break
        # newest first, so an older PR of the same merge commit, the first of its stack, names it
        picked[sha] = pr["number"]
    return [(number, sha) for sha, number in reversed(picked.items())]


STABLE = re.compile(r"v(\d+)\.(\d+)\.(\d+)")
# a PR an environment holds under its own name, before (`#N前`) or after (`#N后`) it
PR_SIDE = re.compile(r"#(\d+)(前|后)")
# the PRs an environment measured again from the environment before it, and when there is none, the merge commits of
# PRs labeled `bench` it is measured on
AGAIN = 5


def when(stamp):
    """The time of an ISO 8601 stamp of the API (`...Z`) or of a result (`...+00:00`)."""
    return datetime.datetime.fromisoformat(stamp.replace("Z", "+00:00"))


def stable_releases():
    """[(tag, published_at)] of the published releases that are no release candidate (`vX.Y.Z`), the lowest version
    first."""
    releases = [r for r in api("/releases?per_page=100")
                if not r["draft"] and not r["prerelease"] and STABLE.fullmatch(r["tag_name"])]
    return sorted(((r["tag_name"], r["published_at"]) for r in releases),
                  key=lambda t: tuple(int(x) for x in STABLE.fullmatch(t[0]).groups()))


def last_measured(env):
    """The SHA of the commit an environment measured last."""
    return max(env["results"], key=lambda sha: when(env["results"][sha]["measured_at"]))


def previous_env(index, key):
    """The environment measured in last before `key` (its newest result the newest of all), or None."""
    envs = [e for k, e in index.get("envs", {}).items() if k != key and e.get("results")]
    return max(envs, key=lambda e: when(e["results"][last_measured(e)]["measured_at"]), default=None)


def names_held(env):
    """Every name an environment holds a commit by: a tag, `#N前`, the parts of `#564–#568后，#569前`."""
    return {name for r in env["results"].values() for ref in r.get("refs") or [] for name in ref.split("，")}


def prs_held(env):
    """{PR number: (before, after)} of the PRs an environment holds before and after under their own names; a stack
    measured under one name (`#531–#536前`) names none of its PRs alone."""
    sides = {}
    for sha, r in env["results"].items():
        for ref in r.get("refs") or []:
            for name in ref.split("，"):
                if m := PR_SIDE.fullmatch(name):
                    sides.setdefault(int(m[1]), {})[m[2]] = sha
    return {n: (s["前"], s["后"]) for n, s in sides.items() if len(s) == 2}


def backfill_requests(index, key, queued=()):
    """What a new environment `key` is measured on first (#1254), so that it holds results to compare at once, and
    with the environment measured in before it ([`previous_env`]): [{commits, release_tag, pr}], in this order.
      - the releases: the newest stable release of each major version, and the stable releases that came after the
        commit the environment before measured last (not in its history) and that it does not hold, with the stable
        release before each of them, which its release notes compare with. The environment before's anchor goes first, so that the two
        environments share their anchor; without it, the newest release does, as the anchor is the first commit
        measured. The others follow, the lowest version first.
      - the release notes of those releases, but the ones only measured as the release before another: each compared
        with the stable release before it (`release_tag`), which rewrites the 性能基准 section of its notes
      - the AGAIN newest PRs the environment before holds before and after under their own names, measured again
        under those names, then each PR labeled `bench` whose merge commit came after the commit it measured last and
        that it does not hold and no request of the queue measures (`queued`, their numbers), before and after it
        (`#N前`, `#N后`), the oldest merged first; with the PR's number (`pr`), under which the publish job comments
        the comparison. The PRs of a stack merged together share a merge commit, measured once, for the first of them.
        Those merged before that commit are left out: the environment before measured them, or the queue does.
    With no environment before, the releases are the newest of each major version and the PRs the AGAIN merge commits
    of PRs labeled `bench` that came into the default branch last ([`last_merges`]). Run from a checkout of the
    default branch, with its tags."""
    releases = stable_releases()
    tags = [t for t, _ in releases]
    before = previous_env(index, key)
    newest = {}
    for t in tags:
        newest[STABLE.fullmatch(t)[1]] = t
    noted = set(newest.values())
    if before:
        last, held = last_measured(before), names_held(before)
        noted |= {t for t in tags if t not in held and not merged_here(resolve(t), last)}
    noted = [t for t in tags if t in noted]
    bases = {t: tags[tags.index(t) - 1] for t in noted if tags.index(t) > 0}
    measured = [t for t in tags if t in noted or t in bases.values()]
    anchor = next((t for t in measured if before and resolve(t) == before.get("anchor")), measured[-1])
    items = [{"commits": ",".join([anchor] + [t for t in measured if t != anchor])}]
    items += [{"commits": f"{bases[t]},{t}", "release_tag": t} for t in noted if t in bases]
    if not before:
        return items + [{"commits": pr_commits(n, sha), "pr": n} for n, sha in last_merges(AGAIN)]
    prs = prs_held(before)
    items += [{"commits": f"#{n}前={prs[n][0]},#{n}后={prs[n][1]}", "pr": n} for n in sorted(prs)[-AGAIN:]]
    seen = set()
    for pr in merged_bench_prs():
        n = pr["number"]
        if n in prs or n in queued:
            continue
        sha = merge_commit(n)
        if sha and merged_here(sha) and not merged_here(sha, last) and sha not in seen:
            seen.add(sha)
            items.append({"commits": pr_commits(n, sha), "pr": n})
    return items


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


def new_request(request_id, commits, options, force=False, release_tag="", pr=None):
    """A request of the queue, of `commits` as resolve_commits gives them, asked for by this run of the workflow; `pr`
    is the number of the PR it measures before and after (one labeled `bench`, or named by the run's `pr` input), under
    which bench.yml comments the comparison."""
    server = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
    run_id = os.environ.get("GITHUB_RUN_ID", "0")
    return {
        "id": request_id,
        "asked_at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "run": f"{server}/{os.environ.get('GITHUB_REPOSITORY', '')}/actions/runs/{run_id}",
        "commits": ",".join(spec(c) for c in commits),
        # two commits asked for are a base and a head: kept apart, since `commits` shrinks to what is left when a
        # forced request is cut short by the end of the night
        "pair": [spec(c) for c in commits] if len(commits) == 2 else None,
        **options,
        "force": force,
        "release_tag": release_tag,
        "pr": pr,
    }


def request_from_env():
    """The request of this run, from what bench.yml passes in the environment."""
    commits = resolve_commits(os.environ.get("COMMITS", ""))
    if not commits:
        raise SystemExit("no commits asked for")
    run = f"{os.environ.get('GITHUB_RUN_ID', '0')}-{os.environ.get('GITHUB_RUN_ATTEMPT', '1')}"
    options = {k: (os.environ.get(k.upper()) or default) for k, default in OPTIONS.items()}
    pr = os.environ.get("PR", "").strip().lstrip("#")
    if pr and not pr.isdigit():
        raise SystemExit(f"pr {pr!r} is no PR number")
    return new_request(run, commits, options, os.environ.get("FORCE") == "1", os.environ.get("RELEASE_TAG", ""),
                       int(pr) if pr else None)


def names(request):
    """The names of a request's commits, for people."""
    return "，".join(item.rpartition("=")[0] or item[:10] for item in request["commits"].split(","))


def queue_bench_prs(queue):
    """Puts into the queue each PR labeled `bench` merged since the last look, before and after it, the oldest merged
    first, and says what it did. The queue's `labeled` holds the PRs looked at already, so that each is queued once,
    also after its request was measured and left the queue; the first look, with no `labeled` yet, records the PRs
    merged until then without queuing them. The PRs of a stack merged together share a merge commit, queued once, for
    the first of them. A PR the API did not answer for, or whose merge commit the checkout of the default branch does
    not hold yet, is not recorded, and the next night looks at it again."""
    try:
        merged = merged_bench_prs()
    except (OSError, ValueError, KeyError) as e:
        return [f"- 没能列出带 `bench` 标签的 PR（{e}），下一晚再查"]
    if "labeled" not in queue:
        queue["labeled"] = sorted(pr["number"] for pr in merged)
        return [f"- 第一次检查带 `bench` 标签的 PR：已合并的 {len(merged)} 个只记下，不排队"]
    said, queued = [], {}
    for number in (pr["number"] for pr in merged if pr["number"] not in queue["labeled"]):
        try:
            sha = merge_commit(number)
        except (OSError, ValueError, KeyError) as e:
            said.append(f"- 没能取到 #{number} 的合并提交（{e}），下一晚再查")
            continue
        if not sha or not merged_here(sha):
            said.append(f"- #{number} 的合并提交还不在默认分支上，下一晚再查")
            continue
        if sha in queued:
            said.append(f"- #{number} 与 #{queued[sha]} 是同一个合并提交，不另外排队")
        else:
            request = new_request(f"pr-{number}", resolve_commits(pr_commits(number, sha)), dict(OPTIONS), pr=number)
            queue["requests"].append(request)
            queued[sha] = number
            said.append(f"- 新合并的 `bench` 标签 PR 排入夜间队列，第 {len(queue['requests'])} 位：{names(request)}")
        queue["labeled"].append(number)
    queue["labeled"].sort()
    return said


def plan(args):
    out, said = {"mode": "none"}, []
    night, tonight = at_night(), night_began().isoformat()
    queue = load_queue(args.pages)
    branch = os.environ.get("OTHER_BRANCH", "")
    if args.event == "night":
        if branch:
            # the queue and its PRs are the default branch's
            said.append(f"- 夜间运行：从分支 {branch} 发起，不检查 PR，也不测量队列")
        else:
            said += queue_bench_prs(queue)
            if not night:
                # started by hand after the night, or earlier than the queue job waits (bench.yml)
                said.append("- 夜间运行：开始时不在夜间时段，只检查带 `bench` 标签的 PR，不用基准机器")
            elif queue.get("taken") == tonight:
                said.append("- 夜间运行：今晚已有一次运行在测量队列，这次不用基准机器")
            elif queue["requests"]:
                out["mode"], queue["taken"] = "drain", tonight
                said.append(f"- 夜间运行：队列里有 {len(queue['requests'])} 个请求，按顺序测到日本时间 {NIGHT[1]:%H:%M}")
            elif calibration_night():
                # the runner looks whether the calibration changed the profile (bench-runner.sh)
                out["mode"], queue["taken"] = "drain", tonight
                said.append("- 夜间运行：队列为空；今天是校准日（周五），在 runner 上检查 profile 是否更换")
            else:
                said.append("- 夜间运行：队列为空，不用基准机器")
            save_queue(args.pages, queue)
    else:
        request = request_from_env()
        if not args.night or branch:
            out.update(mode="now", request=json.dumps(request, ensure_ascii=False))
            why = f"从分支 {branch} 发起，不进夜间队列" if branch else "关闭了 night"
            said.append(f"- 立即测量（{why}）：{names(request)}")
        else:
            queue["requests"].append(request)
            if night:
                out["mode"], queue["taken"] = "drain", tonight
            save_queue(args.pages, queue)
            place = f"排入夜间队列，第 {len(queue['requests'])} 位：{names(request)}"
            if night:
                said.append(f"- {place}；现在是夜间时段，立即按顺序测量队列")
            else:
                said.append(f"- {place}；日本时间 {NIGHT[0]:%H:%M} 的夜间运行开始测量")
    with open(args.outputs, "w", encoding="utf-8") as f:
        f.write("".join(f"{k}={v}\n" for k, v in out.items()))
    with open(args.summary, "w", encoding="utf-8") as f:
        f.write("## 排队\n\n" + "\n".join(said) + "\n\n")
    print(out["mode"])


def shas(request):
    """The commits a request asks for, as full SHAs (its `commits` are resolved)."""
    return {item.rpartition("=")[2] for item in request["commits"].split(",")}


def asks_alike(a, b):
    """Whether two requests ask for the same, with the same options: a comparison commented under the same PR, or
    written into the same release's notes, or else the same commits."""
    if any(str(a.get(k, "")) != str(b.get(k, "")) for k in OPTIONS):
        return False
    if a.get("pr") or b.get("pr"):
        return a.get("pr") == b.get("pr")
    if a.get("release_tag") or b.get("release_tag"):
        return a.get("release_tag") == b.get("release_tag")
    return shas(a) == shas(b)


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
        # A run asked to measure at once is not in the queue, nor is a backfill, and another run may have taken a
        # request out already. A backfill the night was over for goes first, so the next night carries it on before
        # anything else; one measured through, as one measured at once, answers the requests of the queue that ask for
        # the same, which leave it.
        if request is None:
            if run["request"].get("backfill") and run.get("next"):
                queue["requests"].insert(0, {**run["request"], "commits": run["next"]["commits"],
                                             "force": run["next"]["force"]})
            elif not run.get("next"):
                queue["requests"] = [r for r in queue["requests"] if not asks_alike(r, run["request"])]
                # a PR measured with its number is looked at: a night's run does not queue it again
                pr = run["request"].get("pr")
                if pr and "labeled" in queue and pr not in queue["labeled"]:
                    queue["labeled"] = sorted(queue["labeled"] + [pr])
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
    pl.add_argument("--event", required=True, help="what started the run: `night` for a night's run, else its event")
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
