"""Put the results of one request of a benchmark run into the gh-pages branch (#601, #603, #688).

usage: python bench-publish.py --pages DIR --results DIR --viewer DIR --outputs FILE --summary FILE

`--pages` is a checkout of the gh-pages branch, `--results` a request's folder of bench-commits.py (its run.json and
`<sha>/result.json`), `--viewer` the folder of the viewer's files (`.github/bench`). Writes, under `<pages>/bench/`:
  - `data/<env key>/<sha>.json`: each result measured, replacing an older one of the same commit
  - `data/index.json`: every environment (what it is, its anchor) and the commits it holds, with the names they
    were asked by (a tag or a branch; a commit asked again by another name gains that name) and when they were
    measured; the viewer lists them from here. An environment new to it is added only once it holds a result.
  - `index.html` and `compare.mjs`, the viewer, as they are in `--viewer`
Prints the environment key, for the commit message, and writes to `--summary` what reached gh-pages: how many results,
the viewer and, for each commit, the `BENCH-<anchor>-<commit>` reference to its comparison with the anchor. When the
request asked for exactly two commits and the environment holds both, they are a base and a head: the summary adds the
reference and the viewer's address of their comparison, and `--outputs` gets `key`, `url`, `base` and `head` (full
SHAs), `base_name` and the request's `release_tag`, for the release notes. Both files are replaced on every call: the
workflow calls it again when its push lost a race with another one.
"""

import argparse
import json
import os
import shutil
from urllib.parse import quote


def says_more(c):
    """Whether the name a commit was asked by says more than the commit itself (a SHA, or its start, or HEAD)."""
    return c["ref"] != "HEAD" and not c["sha"].startswith(c["ref"])


def link_name(c):
    """How the viewer's address names a commit: by its name when that says more, else by its SHA."""
    return quote(c["ref"] if says_more(c) else c["sha"][:12], safe="")


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--pages", required=True)
    p.add_argument("--results", required=True)
    p.add_argument("--viewer", required=True)
    p.add_argument("--outputs", required=True)
    p.add_argument("--summary", required=True)
    args = p.parse_args()

    with open(os.path.join(args.results, "run.json"), encoding="utf-8") as f:
        run = json.load(f)
    env, key = run["env"], run["env"]["key"]
    root = os.path.join(args.pages, "bench")
    data = os.path.join(root, "data")
    os.makedirs(os.path.join(data, key), exist_ok=True)
    index_path = os.path.join(data, "index.json")
    index = {"format": 1, "envs": {}}
    if os.path.exists(index_path):
        with open(index_path, encoding="utf-8") as f:
            index = json.load(f)
    # an environment nothing was measured in yet (every commit postponed) is not listed until it holds a result
    if key in index["envs"] or any(c["measured"] for c in run["commits"]):
        held = index["envs"].setdefault(key, {k: v for k, v in env.items() if k != "key"})
    else:
        held = {}
    held.setdefault("results", {})
    if run.get("anchor"):
        held.setdefault("anchor", run["anchor"])

    for c in run["commits"]:
        sha = c["sha"]
        entry = held["results"].get(sha)
        if c["measured"]:
            with open(os.path.join(args.results, sha, "result.json"), encoding="utf-8") as f:
                result = json.load(f)
            if result["env"]["key"] != key:
                raise SystemExit(f"{sha}: measured in {result['env']['key']}, not {key}")
            shutil.copyfile(os.path.join(args.results, sha, "result.json"), os.path.join(data, key, f"{sha}.json"))
            entry = {"refs": (entry or {}).get("refs", []), "version": result["version"],
                     "measured_at": result["measured_at"], "only": result["only"], "quiet": result["quiet"]}
        if entry is None:
            continue
        if says_more(c) and c["ref"] not in entry["refs"]:
            entry["refs"].append(c["ref"])
        held["results"][sha] = entry

    with open(index_path, "w", encoding="utf-8") as f:
        json.dump(index, f, ensure_ascii=False, indent=1)
    for name in ("index.html", "compare.mjs"):
        shutil.copyfile(os.path.join(args.viewer, name), os.path.join(root, name))
    published = [c for c in run["commits"] if c["measured"]]
    anchor = held.get("anchor", "")
    owner = os.environ.get("GITHUB_REPOSITORY_OWNER", "334456777").lower()
    repo = os.environ.get("GITHUB_REPOSITORY", "334456777/WeFlow").split("/")[-1]
    viewer = f"https://{owner}.github.io/{repo}/bench/?env={key}"
    lines = [f"- {len(published)} 份结果推到 gh-pages，环境 `{key}`；查看页：{viewer}"]
    refs = [f"  - {c['ref']}：`BENCH-{anchor[:10]}-{c['sha'][:10]}`" for c in published if c["sha"] != anchor]
    if refs:
        lines += ["- 和锚点的对比（在 issue、PR 和提交说明里写这个引用，会自动链接到查看页）："] + refs
    outputs = {}
    # a request that asked for exactly two commits names them as its pair (bench_queue.py), a base and a head, compared
    # once the environment holds both, whatever part of them this run measured
    pair = []
    for item in run["request"].get("pair") or []:
        name, _, sha = item.rpartition("=")
        pair.append({"ref": name or sha, "sha": sha})
    if len(pair) == 2 and all(c["sha"] in held["results"] for c in pair):
        base, head = pair
        url = f"{viewer}&base={link_name(base)}&head={link_name(head)}"
        lines.append(f"- base 对 head：`BENCH-{base['sha'][:10]}-{head['sha'][:10]}`，查看页：{url}")
        outputs = {"key": key, "url": url, "base": base["sha"], "head": head["sha"], "base_name": base["ref"],
                   "release_tag": run["request"].get("release_tag", "")}
    if run.get("next"):
        waiting = sum(1 for c in run["commits"] if c.get("postponed"))
        lines.append(f"- {waiting} 个提交留在队列里，下一个夜间时段再测")
    with open(args.outputs, "w", encoding="utf-8") as f:
        f.write("".join(f"{k}={v}\n" for k, v in outputs.items()))
    with open(args.summary, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n\n")
    print(key)


if __name__ == "__main__":
    main()
