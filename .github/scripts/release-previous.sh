#!/usr/bin/env bash
# Prints the release before <tag> as JSON ({tagName, publishedAt}), or nothing when there is none: the highest version
# below it, in SemVer order. A stable tag (vX.Y.Z) looks among stable releases, a pre-release among all of them.
# usage: release-previous.sh <tag>
# Needs gh (with GH_TOKEN).
set -euo pipefail

TAG="$1"
REPO="${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"

if [[ "$TAG" =~ ^v[0-9]+[.][0-9]+[.][0-9]+$ ]]; then
  KIND='select(.isPrerelease | not)'
else
  KIND='.'
fi
# The sort key: major, minor and patch, then 1 for a stable version or 0 and the pre-release's parts, numeric ones as
# numbers, so that rc.2 comes before rc.10 and both before the version itself
KEY='ltrimstr("v") | split("+")[0]
  | capture("^(?<core>[0-9]+[.][0-9]+[.][0-9]+)[-.]?(?<pre>.*)$")
  | (.core | split(".") | map(tonumber))
    + (if .pre == "" then [1] else [0] + (.pre | split(".") | map(if test("^[0-9]+$") then tonumber else . end)) end)'
gh release list --repo "$REPO" --exclude-drafts --limit 100 --json tagName,isPrerelease,publishedAt --jq "
  def key: $KEY;
  (\"$TAG\" | key) as \$tag
  | [.[] | select(.tagName | test(\"^v[0-9]+[.][0-9]+[.][0-9]+\")) | $KIND | select((.tagName | key) < \$tag)]
  | sort_by(.tagName | key) | last // empty | {tagName, publishedAt}"
