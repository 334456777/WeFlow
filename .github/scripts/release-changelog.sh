#!/usr/bin/env bash
# Prints the generated part of the release notes' "更新日志" section (Markdown) for release.yml.
# usage: release-changelog.sh <tag> <commit>
# Run inside a clone that has the history and tags; needs gh (with GH_TOKEN) and jq.
#
# It compares <commit> with the previous release, as release-previous.sh picks it. Pull requests come from the merge commits in between
# ("Merge pull request #N" or a squash merge's "(#N)"), with the layers below each that came in by the same merge:
# `gh stack merge` leaves main one merge commit, the top's, and every layer of the stack has it as its merge commit;
# a layer's base is the head branch of the one below (#382). It lists:
# - the issues those pull requests closed, grouped by label (增强/优化, bug/反常, everything else);
# - the pull requests that closed no issue;
# - the issues opened since the previous release that are still open.
# Anything labelled 文档 without 增强/优化/bug/反常 is left out.
#
# A number that is no pull request (a commit pushed to main may end in the (#N) of an issue) is skipped. Any other failed
# read is tried again and then stops the script with an ::error:: line: no pull request is left out of the notes silently.
set -euo pipefail

TAG="$1"
SHA="$2"
REPO="${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"

PREV_JSON="$(bash "$(dirname "$0")/release-previous.sh" "$TAG")"
if [ -z "$PREV_JSON" ]; then
  echo "首个版本。"
  exit 0
fi
PREV="$(jq -r .tagName <<< "$PREV_JSON")"
PREV_DATE="$(jq -r .publishedAt <<< "$PREV_JSON")"

# Only the first parents: a commit inside a merged pull request may end in the (#N) of an issue
MERGES="$(git log --first-parent --format=%s "$PREV..$SHA" |
  sed -nE 's/^Merge pull request #([0-9]+).*/\1/p; s/.*\(#([0-9]+)\)$/\1/p')"

DEFAULT="$(gh repo view "$REPO" --json defaultBranchRef --jq .defaultBranchRef.name)"

# gh_retry <what> <answer> <gh argument>...: prints what `gh <argument>...` prints. A failure (network, rate limit, 5xx)
# is tried again, twice at most, and then reported on stderr as an ::error:: naming <what>, with return 1. A message
# containing the text <answer> is gh's answer, not a failure: nothing is printed. Errexit is off inside a command
# substitution, so every caller checks the status: OUT="$(gh_retry ...)" || exit 1
gh_retry() {
  local what="$1" answer="$2" out attempt
  shift 2
  for attempt in 1 2 3; do
    if out="$(gh "$@" 2>&1)"; then
      printf '%s\n' "$out"
      return 0
    fi
    if [ -n "$answer" ] && grep -Fq "$answer" <<< "$out"; then
      return 0
    fi
    if [ "$attempt" -lt 3 ]; then
      sleep $((attempt * 5))
    fi
  done
  echo "::error::could not read $what for the changelog: ${out//$'\n'/ }" >&2
  return 1
}

NOT_A_PR='Could not resolve to a PullRequest' # what GitHub answers for the number of an issue

# The layers below pull request $1 in its stack: down its base branches, the merged pull request whose head that
# branch is and whose merge commit is the same, until a base is the default branch
below() {
  local out base merge next
  out="$(gh_retry "the base of #$1" "$NOT_A_PR" pr view "$1" --repo "$REPO" --json baseRefName,mergeCommit \
    --jq '"\(.baseRefName) \(.mergeCommit.oid)"')" || return 1
  read -r base merge <<< "$out"
  [ -n "$base" ] || return 0 # $1 is no pull request
  while [ "$base" != "$DEFAULT" ]; do
    out="$(gh_retry "the pull request merged from branch $base" '' pr list --repo "$REPO" --state merged --head "$base" \
      --json number,baseRefName,mergeCommit \
      --jq ".[] | select(.mergeCommit.oid == \"$merge\") | \"\(.number) \(.baseRefName)\"")" || return 1
    read -r next base <<< "$out"
    [ -n "$next" ] || break
    echo "$next"
  done
}

# Pull request $1 as one line of JSON; nothing when it is no pull request or is not merged
pr_json() {
  local out
  out="$(gh_retry "#$1" "$NOT_A_PR" api graphql -f owner="${REPO%/*}" -f name="${REPO#*/}" -F number="$1" -f query='
    query($owner: String!, $name: String!, $number: Int!) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          number title merged
          labels(first: 20) { nodes { name } }
          closingIssuesReferences(first: 20) { nodes { number title labels(first: 20) { nodes { name } } } }
        }
      }
    }')" || return 1
  jq -c '.data.repository.pullRequest | select(. != null and .merged) | {
    number, title,
    labels: [.labels.nodes[].name],
    issues: [.closingIssuesReferences.nodes[] | {number, title, labels: [.labels.nodes[].name]}]
  }' <<< "$out" || { echo "::error::could not read the answer for #$1 for the changelog: ${out//$'\n'/ }" >&2; return 1; }
}

# The loops run in the main shell, not in a pipeline or a command substitution, where errexit is off: `|| exit 1` stops
# the script
ALL=""
for n in $MERGES; do
  LAYERS="$(below "$n")" || exit 1
  ALL+="$n $LAYERS "
done
PRS="$(printf '%s\n' $ALL | sort -un)"

PR_LINES=""
for n in $PRS; do
  ONE="$(pr_json "$n")" || exit 1
  PR_LINES+="$ONE"$'\n'
done
PR_JSON="$(jq -s . <<< "$PR_LINES")"

NEW_JSON="$(gh issue list --repo "$REPO" --state open --search "created:>=$PREV_DATE" --limit 100 \
  --json number,title,labels --jq '[.[] | {number, title, labels: [.labels[].name]}]')"

jq -r --argjson new "$NEW_JSON" --arg repo "$REPO" --arg prev "$PREV" --arg tag "$TAG" '
  . as $prs
  | def group:
    if any(.[]; . == "增强" or . == "优化") then "新功能和优化"
    elif any(.[]; . == "bug" or . == "反常") then "修复"
    elif any(.[]; . == "文档") then null
    else "其他" end;
  # A title is plain text, but in the notes it is Markdown: GitHub takes an @name outside a code span for a mention
  # and lists that account under Contributors on the release page (the CSS at-rule @layer would credit the account layer),
  # so each one goes into a code span. A backtick with no closing one does not open a code span.
  def unmention:
    split("`") | length as $n | to_entries
    | map(if .key % 2 == 0 or .key == $n - 1
          then .value | gsub("(?<m>(?<![A-Za-z0-9_])@[A-Za-z0-9][A-Za-z0-9-]*(?:/[A-Za-z0-9][A-Za-z0-9_.-]*)?)"; "`\(.m)`")
          else .value end)
    | join("`");
  def line: "- #\(.number) \(.title | unmention)";

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
  | join("\n\n")' <<< "$PR_JSON"
