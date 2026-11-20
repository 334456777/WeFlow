#!/usr/bin/env bash
# Waits until a pull request is merged or closed, or one of its required checks fails, then prints its state.
#
# Usage: wait-pr.sh <PR number> [owner/repo]
# Exits 0 once merged, 1 when closed unmerged or a required check failed or was cancelled, 2 after 90 minutes.
# It does not trust `gh pr checks --watch --required`: a required summary check only appears once its job starts,
# and --watch returns as soon as the checks that have appeared pass.
set -u

n=${1:?usage: wait-pr.sh <PR number> [owner/repo]}
repo=${2:-334456777/WeFlow}
state=OPEN
failed=

for _ in $(seq 180); do
  state=$(gh pr view "$n" -R "$repo" --json state -q .state 2>/dev/null) || state=UNKNOWN
  if [ "$state" = MERGED ] || [ "$state" = CLOSED ]; then
    break
  fi
  failed=$(gh pr checks "$n" -R "$repo" --required --json name,bucket \
    --jq '[.[] | select(.bucket == "fail" or .bucket == "cancel") | .name] | join(", ")' 2>/dev/null) || failed=
  if [ -n "$failed" ]; then
    break
  fi
  sleep 30
done

echo "PR #$n: $state${failed:+, failed: $failed}"
case "$state" in
  MERGED) exit 0 ;;
  CLOSED) exit 1 ;;
esac
[ -n "$failed" ] && exit 1
exit 2
