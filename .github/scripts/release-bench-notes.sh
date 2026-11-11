#!/usr/bin/env bash
# The 性能基准 section of a published release's notes (#516, #519, #601): the link to the comparison in the benchmark
# viewer and the lines of its highlights.md (`.github/bench/cli.mjs`), put before the 下载 anchor (at the end when the
# notes have none). The section sits between two markers, so running it again replaces the section instead of adding
# another. Used by bench.yml, after a release (release.yml with `bench`) or when an older release is measured
# afterwards. The highlights are of the first shape of benchmark account measured (#610); `<more.md>`, when given and
# not empty, holds a line for each other shape, with the link to its comparison.
# usage: release-bench-notes.sh <tag> <base> <viewer url> <highlights.md> [<more.md>]
# env: GH_TOKEN, GITHUB_REPOSITORY; with GITHUB_STEP_SUMMARY the section is shown on the run's page too.
set -euo pipefail

tag=$1 base=$2 url=$3 highlights=$4 more=${5:-}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

{
  echo "<!-- bench -->"
  echo "## 性能基准"
  echo
  echo "与 \`$base\` 对比。两个版本在同一台自托管 Windows x64 机器上、以合成的基准账号分别测得：**[在基准测试页查看]($url)**"
  echo
  cat "$highlights"
  if [ -n "$more" ] && [ -s "$more" ]; then
    cat "$more"
  fi
  echo
  echo "<!-- /bench -->"
} > "$work/section.md"
gh release view "$tag" --repo "$GITHUB_REPOSITORY" --json body -q .body > "$work/body.md"
awk -v f="$work/section.md" '
  { sub(/\r$/, "") }
  /^<!-- bench -->$/ { skip = 1; next }
  skip && /^<!-- \/bench -->$/ { skip = 0; after = 1; next }
  skip { next }
  after && $0 == "" { after = 0; next }
  { after = 0 }
  $0 == "<a name=\"weflow-download\"></a>" && !done { while ((getline l < f) > 0) print l; print ""; done = 1 }
  { print }
  END { if (!done) { print ""; while ((getline l < f) > 0) print l } }
' "$work/body.md" |
  # without the blank lines at the end, which the body as read ends with and running it again would add to
  awk 'NF { for (; blank > 0; blank--) print ""; print; next } { blank++ }' > "$work/notes.md"
gh release edit "$tag" --repo "$GITHUB_REPOSITORY" --notes-file "$work/notes.md"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  cat "$work/section.md" >> "$GITHUB_STEP_SUMMARY"
fi
