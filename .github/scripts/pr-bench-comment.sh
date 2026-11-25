#!/usr/bin/env bash
# The comment under a merged PR once a run measured it before and after (a PR labeled bench from the queue or the
# backfill of a new environment, bench_queue.py, or one named by the run's `pr` input): the BENCH-<base>-<head>
# reference, which the repository's autolink turns into the comparison in the benchmark viewer, and the lines of its
# highlights.md (`.github/bench/cli.mjs`). Each comparison is a comment of its own (#1254): a PR measured again, as in
# a new environment, gains another, and those before it stay as they were; the same comment there already (the
# publish job run again) is not added twice. The comment starts with a marker. Used by bench.yml's publish job.
# usage: pr-bench-comment.sh <pr> <base sha> <head sha> <highlights.md>
# env: GH_TOKEN (pull-requests: write), GITHUB_REPOSITORY; with GITHUB_STEP_SUMMARY the run's page links the PR.
set -euo pipefail

pr=$1 base=$2 head=$3 highlights=$4
marker="<!-- bench -->"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

{
  echo "$marker"
  echo "## 性能基准"
  echo
  # outside a code span, where the autolink does not reach
  echo "BENCH-${base:0:10}-${head:0:10}"
  echo
  cat "$highlights"
} > "$work/comment.md"
gh api --paginate "repos/$GITHUB_REPOSITORY/issues/$pr/comments" \
  --jq ".[] | select(.user.login == \"github-actions[bot]\" and (.body | startswith(\"$marker\"))) | .body | @json" \
  > "$work/before.jsonl"
said=posted
if python3 - "$work/comment.md" "$work/before.jsonl" <<'EOF'
import json
import sys

body = open(sys.argv[1], encoding="utf-8").read().strip()
with open(sys.argv[2], encoding="utf-8") as f:
    sys.exit(0 if any(json.loads(line).strip() == body for line in f if line.strip()) else 1)
EOF
then
  said=held
else
  gh api "repos/$GITHUB_REPOSITORY/issues/$pr/comments" -F "body=@$work/comment.md" > /dev/null
fi
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  url="${GITHUB_SERVER_URL:-https://github.com}/$GITHUB_REPOSITORY/pull/$pr"
  if [ "$said" = held ]; then
    printf -- '- PR 下已有同样的对比评论，不再重复：%s\n' "$url" >> "$GITHUB_STEP_SUMMARY"
  else
    printf -- '- 对比已评论到 PR：%s\n' "$url" >> "$GITHUB_STEP_SUMMARY"
  fi
fi
