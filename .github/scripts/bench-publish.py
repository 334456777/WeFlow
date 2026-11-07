"""Put the results of a benchmark run into the gh-pages branch (#601, #603).

usage: python bench-publish.py --pages DIR --results DIR --viewer DIR

`--pages` is a checkout of the gh-pages branch, `--results` the `--out` folder of bench-commits.py (its run.json and
`<sha>/result.json`), `--viewer` the folder of the viewer's files (`.github/bench`). Writes, under `<pages>/bench/`:
  - `data/<env key>/<sha>.json`: each result measured, replacing an older one of the same commit
  - `data/index.json`: every environment (what it is, its anchor) and the commits it holds, with the names they
    were asked by (a tag or a branch; a commit asked again by another name gains that name) and when they were
    measured; the viewer lists them from here
  - `index.html` and `compare.mjs`, the viewer, as they are in `--viewer`
Prints the environment key and the full SHAs of the commits asked for, one per line, for the steps after it, and
appends what reached gh-pages to $GITHUB_STEP_SUMMARY: how many results, the viewer and, for each commit, the
`BENCH-<anchor>-<commit>` reference to its comparison with the anchor.
"""

import argparse
import json
import os
import shutil


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--pages", required=True)
    p.add_argument("--results", required=True)
    p.add_argument("--viewer", required=True)
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
    held = index["envs"].setdefault(key, {k: v for k, v in env.items() if k != "key"})
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
        # a name that is the commit itself (a SHA, or its start) or HEAD says nothing more
        if c["ref"] not in entry["refs"] and c["ref"] != "HEAD" and not sha.startswith(c["ref"]):
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
    lines = ["## 发布", "", f"- {len(published)} 份结果推到 gh-pages，环境 `{key}`；查看页：{viewer}"]
    refs = [f"  - {c['ref']}：`BENCH-{anchor[:10]}-{c['sha'][:10]}`" for c in published if c["sha"] != anchor]
    if refs:
        lines += ["- 和锚点的对比（在 issue、PR 和提交说明里写这个引用，会自动链接到查看页）："] + refs
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
    print(key)
    for c in run["commits"]:
        print(c["sha"])


if __name__ == "__main__":
    main()
