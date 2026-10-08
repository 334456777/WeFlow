#!/usr/bin/env bash
# Prints the generated part of the release notes' "更新日志" section (Markdown) for release.yml.
# usage: release-changelog.sh <tag> <commit>
# Run inside a clone that has the history and tags; needs gh (with GH_TOKEN) and jq.
#
# It compares <commit> with the previous release: a stable tag (vX.Y.Z) with the previous stable release, a
# pre-release with the previous release of any kind. Pull requests come from the merge commits in between
# ("Merge pull request #N" or a squash merge's "(#N)"). It lists:
# - the issues those pull requests closed, grouped by label (增强/优化, bug/反常, everything else);
# - the pull requests that closed no issue;
# - the issues opened since the previous release that are still open.
# Anything labelled 文档 without 增强/优化/bug/反常 is left out.
set -euo pipefail

TAG="$1"
SHA="$2"
REPO="${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"

if [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  KIND='select(.isPrerelease | not)'
else
  KIND='.'
fi
PREV_JSON="$(gh release list --repo "$REPO" --exclude-drafts --limit 100 --json tagName,isPrerelease,publishedAt \
  --jq "[.[] | select(.tagName != \"$TAG\") | $KIND] | sort_by(.publishedAt) | last // empty")"
if [ -z "$PREV_JSON" ]; then
  echo "首个版本。"
  exit 0
fi
PREV="$(jq -r .tagName <<< "$PREV_JSON")"
PREV_DATE="$(jq -r .publishedAt <<< "$PREV_JSON")"

PRS="$(git log --format=%s "$PREV..$SHA" |
  sed -nE 's/^Merge pull request #([0-9]+).*/\1/p; s/.*\(#([0-9]+)\)$/\1/p' | sort -un)"

PR_JSON="$(for n in $PRS; do
  gh api graphql -f owner="${REPO%/*}" -f name="${REPO#*/}" -F number="$n" -f query='
    query($owner: String!, $name: String!, $number: Int!) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          number title merged
          labels(first: 20) { nodes { name } }
          closingIssuesReferences(first: 20) { nodes { number title labels(first: 20) { nodes { name } } } }
        }
      }
    }' --jq '.data.repository.pullRequest | select(.merged) | {
      number, title,
      labels: [.labels.nodes[].name],
      issues: [.closingIssuesReferences.nodes[] | {number, title, labels: [.labels.nodes[].name]}]
    }'
done | jq -s .)"

NEW_JSON="$(gh issue list --repo "$REPO" --state open --search "created:>=$PREV_DATE" --limit 100 \
  --json number,title,labels --jq '[.[] | {number, title, labels: [.labels[].name]}]')"

jq -rn --argjson prs "$PR_JSON" --argjson new "$NEW_JSON" \
  --arg repo "$REPO" --arg prev "$PREV" --arg tag "$TAG" '
  def group:
    if any(.[]; . == "增强" or . == "优化") then "新功能和优化"
    elif any(.[]; . == "bug" or . == "反常") then "修复"
    elif any(.[]; . == "文档") then null
    else "其他" end;
  def line: "- #\(.number) \(.title)";

  ([$prs[] | .number as $pr | .issues[] | . + {pr: $pr}]
    | group_by(.number) | map(.[0] + {prs: map(.pr)})
    | map(. + {group: (.labels | group)}) | map(select(.group))) as $issues
  | [$prs[] | select((.issues | length) == 0) | select(.labels | group)] | sort_by(.number) as $others
  | [$new[] | select(.labels | group)] | sort_by(.number) as $new

  | [
      (if ($issues | length) > 0 then
        "### 完成的 issue\n" + ([
          ("新功能和优化", "修复", "其他") as $g
          | [$issues[] | select(.group == $g)]
          | select(length > 0)
          | "**\($g)**\n" + (map(line + "（" + (.prs | map("#\(.)") | join("、")) + "）") | join("\n"))
        ] | join("\n\n"))
      else empty end),
      (if ($others | length) > 0 then "### 其他改动\n" + ($others | map(line) | join("\n")) else empty end),
      (if ($new | length) > 0 then "### 新登记的 issue\n" + ($new | map(line) | join("\n")) else empty end),
      "**完整对比**：[\($prev)...\($tag)](https://github.com/\($repo)/compare/\($prev)...\($tag))"
    ]
  | join("\n\n")'
