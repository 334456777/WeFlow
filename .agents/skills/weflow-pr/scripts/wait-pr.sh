#!/usr/bin/env bash
# Waits until every required check of a pull request has passed or one has failed, or until it is merged or closed,
# then prints the outcome.
#
# Usage: wait-pr.sh <PR number> [owner/repo]
# Exits 0 once merged or once every required check passed, 1 when closed unmerged or a required check failed or was
# cancelled, 2 after 90 minutes. The required checks are read from the rules of the base branch: a summary check only
# appears once its job starts, so `gh pr checks --watch --required`, which watches the checks that have appeared,
# returns before they all ran.
set -u

n=${1:?usage: wait-pr.sh <PR number> [owner/repo]}
repo=${2:-334456777/WeFlow}
base=$(gh pr view "$n" -R "$repo" --json baseRefName -q .baseRefName)
required=$(gh api "repos/$repo/rules/branches/$base" \
  --jq '[.[] | select(.type == "required_status_checks") | .parameters.required_status_checks[].context] | unique | join(",")')
state=OPEN
failed=
passed=

for _ in $(seq 180); do
  state=$(gh pr view "$n" -R "$repo" --json state -q .state 2>/dev/null) || state=UNKNOWN
  if [ "$state" = MERGED ] || [ "$state" = CLOSED ]; then
    break
  fi
  # Two lines: the failed or cancelled required checks, then the passed ones.
  checks=$(gh pr checks "$n" -R "$repo" --required --json name,bucket --jq '
    ([.[] | select(.bucket == "fail" or .bucket == "cancel") | .name] | join(", ")),
    ([.[] | select(.bucket == "pass") | .name] | unique | join(","))' 2>/dev/null) || checks=
  { read -r failed; read -r passed; } <<< "$checks"
  if [ -n "$failed" ]; then
    break
  fi
  if [ -n "$required" ] && [ "$passed" = "$required" ]; then
    break
  fi
  sleep 30
done

if [ "$state" = MERGED ]; then
  echo "PR #$n: merged"
  exit 0
fi
if [ "$state" = CLOSED ]; then
  echo "PR #$n: closed without merging"
  exit 1
fi
if [ -n "$failed" ]; then
  echo "PR #$n: failed: $failed"
  exit 1
fi
if [ -n "$required" ] && [ "$passed" = "$required" ]; then
  echo "PR #$n: every required check passed ($required)"
  exit 0
fi
echo "PR #$n: still waiting after 90 minutes"
exit 2
