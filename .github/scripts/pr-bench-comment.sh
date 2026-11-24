#!/usr/bin/env bash
# The comment under a merged PR labeled bench once a night's run measured it before and after (bench_queue.py): the
# BENCH-<base>-<head> reference, which the repository's autolink turns into the comparison in the benchmark viewer,
# and the lines of its highlights.md (`.github/bench/cli.mjs`). The comment starts with a marker, so running it again
# (the publish job run again, or the PR measured again) edits that comment instead of adding another. Used by
# bench.yml's publish job.
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
  echo "合并前后在同一台自托管 Windows x64 机器上、以合成的基准账号分别测得：BENCH-${base:0:10}-${head:0:10}"
  echo
  cat "$highlights"
} > "$work/comment.md"
id=$(gh api --paginate "repos/$GITHUB_REPOSITORY/issues/$pr/comments" \
  --jq ".[] | select(.user.login == \"github-actions[bot]\" and (.body | startswith(\"$marker\"))) | .id" | tail -n 1)
if [ -n "$id" ]; then
  gh api -X PATCH "repos/$GITHUB_REPOSITORY/issues/comments/$id" -F "body=@$work/comment.md" > /dev/null
else
  gh api "repos/$GITHUB_REPOSITORY/issues/$pr/comments" -F "body=@$work/comment.md" > /dev/null
fi
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  printf -- '- 对比已评论到 PR：%s/%s/pull/%s\n' "${GITHUB_SERVER_URL:-https://github.com}" "$GITHUB_REPOSITORY" "$pr" \
    >> "$GITHUB_STEP_SUMMARY"
fi
