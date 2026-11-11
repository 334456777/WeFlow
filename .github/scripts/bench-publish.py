"""Put the results of a benchmark run into the gh-pages branch (#601, #603).

usage: python bench-publish.py --pages DIR --results DIR --viewer DIR [--more FILE]

`--pages` is a checkout of the gh-pages branch, `--results` the `--out` folder of bench-commits.py (its run.json and
`<env key>/<sha>/result.json`), `--viewer` the folder of the viewer's files (`.github/bench`). Writes, under
`<pages>/bench/`:
  - `data/<env key>/<sha>.json`: each result measured, replacing an older one of the same commit
  - `data/index.json`: every environment (what it is, the shape of its account (#610), its anchor) and the commits it
    holds, with the names they were asked by (a tag or a branch; a commit asked again by another name gains that
    name) and when they were measured; the viewer lists them from here
  - `index.html` and `compare.mjs`, the viewer, as they are in `--viewer`
Prints the environment keys, for the commit message, and appends what reached gh-pages to $GITHUB_STEP_SUMMARY, for
each shape the run measured: how many results, the viewer and, for each commit, the `BENCH-<anchor>-<commit>`
reference to its comparison with the anchor. When exactly two commits were asked for and an environment holds both,
they are a base and a head: the summary adds the reference and the viewer's address of their comparison. Those of
the first shape are given to the steps after it as the outputs `key`, `url`, `base` and `head` (full SHAs), for the
release notes; `--more` gets a line for each other shape, with its name and the address of its comparison.
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


def publish(run, index, data, results):
    """One shape's results into `data/<env key>/` and its environment in the index: the environment's entry."""
    env, key = run["env"], run["env"]["key"]
    os.makedirs(os.path.join(data, key), exist_ok=True)
    held = index["envs"].setdefault(key, {k: v for k, v in env.items() if k != "key"})
    # not in the key: an environment measured before shapes were named gains its name here
    held["shape"] = {"name": run["shape"], "label": run["label"]}
    held.setdefault("results", {})
    if run.get("anchor"):
        held.setdefault("anchor", run["anchor"])
    for c in run["commits"]:
        sha = c["sha"]
        entry = held["results"].get(sha)
        if c["measured"]:
            file = os.path.join(results, key, sha, "result.json")
            with open(file, encoding="utf-8") as f:
                result = json.load(f)
            if result["env"]["key"] != key:
                raise SystemExit(f"{sha}: measured in {result['env']['key']}, not {key}")
            shutil.copyfile(file, os.path.join(data, key, f"{sha}.json"))
            entry = {"refs": (entry or {}).get("refs", []), "version": result["version"],
                     "measured_at": result["measured_at"], "only": result["only"], "quiet": result["quiet"]}
        if entry is None:
            continue
        if says_more(c) and c["ref"] not in entry["refs"]:
            entry["refs"].append(c["ref"])
        held["results"][sha] = entry
    return held


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--pages", required=True)
    p.add_argument("--results", required=True)
    p.add_argument("--viewer", required=True)
    p.add_argument("--more", help="a file for a line per shape past the first: its name and its comparison")
    args = p.parse_args()

    with open(os.path.join(args.results, "run.json"), encoding="utf-8") as f:
        runs = json.load(f)["runs"]
    root = os.path.join(args.pages, "bench")
    data = os.path.join(root, "data")
    os.makedirs(data, exist_ok=True)
    index_path = os.path.join(data, "index.json")
    index = {"format": 1, "envs": {}}
    if os.path.exists(index_path):
        with open(index_path, encoding="utf-8") as f:
            index = json.load(f)
    helds = [publish(run, index, data, args.results) for run in runs]
    with open(index_path, "w", encoding="utf-8") as f:
        json.dump(index, f, ensure_ascii=False, indent=1)
    for name in ("index.html", "compare.mjs"):
        shutil.copyfile(os.path.join(args.viewer, name), os.path.join(root, name))

    owner = os.environ.get("GITHUB_REPOSITORY_OWNER", "334456777").lower()
    repo = os.environ.get("GITHUB_REPOSITORY", "334456777/WeFlow").split("/")[-1]
    lines, more, output = ["## 发布", ""], [], ""
    for i, (run, held) in enumerate(zip(runs, helds)):
        key, label = run["env"]["key"], run["label"]
        published = [c for c in run["commits"] if c["measured"]]
        anchor = held.get("anchor", "")
        viewer = f"https://{owner}.github.io/{repo}/bench/?env={key}"
        lines.append(f"- {label}：{len(published)} 份结果推到 gh-pages，环境 `{key}`；查看页：{viewer}")
        refs = [f"    - {c['ref']}：`BENCH-{anchor[:10]}-{c['sha'][:10]}`" for c in published if c["sha"] != anchor]
        if refs:
            lines += ["  - 和锚点的对比（在 issue、PR 和提交说明里写这个引用，会自动链接到查看页）："] + refs
        # two commits asked for are a base and a head, compared when the environment holds both
        pair = run["commits"] if len(run["commits"]) == 2 else []
        if pair and all(c["sha"] in held["results"] for c in pair):
            base, head = pair
            url = f"{viewer}&base={link_name(base)}&head={link_name(head)}"
            lines.append(f"  - base 对 head：`BENCH-{base['sha'][:10]}-{head['sha'][:10]}`，查看页：{url}")
            if i == 0:
                output = f"key={key}\nurl={url}\nbase={base['sha']}\nhead={head['sha']}\n"
            else:
                more.append(f"- 「{label}」形状的基准账号上的对比：[在基准测试页查看]({url})")
    if len(runs) > 1:
        lines.append("- `BENCH-` 引用不带形状：查看页打开同时测过这两个提交的第一种形状（`group` 在先），可在页面上切换形状")
    if args.more:
        with open(args.more, "w", encoding="utf-8") as f:
            f.write("".join(line + "\n" for line in more))
    if output and os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as f:
            f.write(output)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
    print(", ".join(run["env"]["key"] for run in runs))


if __name__ == "__main__":
    main()
