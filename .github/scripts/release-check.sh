#!/usr/bin/env bash
# The checks of release.yml. They are kept here, not in the workflow, because Actions prints each step's script
# before it runs it: a refusal written in the step showed in every log, as if it had happened.
# usage: release-check.sh tag <tag>                  (in a clone with `origin`; writes tag/sha/new to GITHUB_OUTPUT;
#                                                   DRY_RUN=true: built and tested only)
#        release-check.sh package <tag> <name> <ext>  (in the folder holding the downloaded package)
set -euo pipefail

case "${1:-}" in
  tag)
    TAG="$2" DRY="${DRY_RUN:-false}"
    if [[ ! "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([-.+][0-9A-Za-z.-]+)?$ ]]; then
      echo "::error::'$TAG' is not a release tag such as v1.2.3 or v1.2.3-beta.1"
      exit 1
    fi
    # Releases use annotated tags only. An existing one keeps its commit, the `<tag>^{}` line (the line without it
    # is the tag object); a lightweight tag has no such line and is refused.
    REFS="$(git ls-remote origin "refs/tags/$TAG" "refs/tags/$TAG^{}")"
    if [ -z "$REFS" ]; then
      NEW=true
      SHA="$GITHUB_SHA"
      # what will happen, not what has: the tag and the release come last, once every build and test has passed
      if [ "$DRY" = true ]; then
        echo "$TAG does not exist yet; a dry run builds and tests $SHA, and creates no tag"
        echo "Dry run: $TAG at $SHA ($GITHUB_REF_NAME) is built and tested on every platform; no tag and no release are made." >> "$GITHUB_STEP_SUMMARY"
      else
        echo "$TAG does not exist yet; once every build and test has passed, it is created as an annotated tag at $SHA"
        echo "Not published yet: $TAG at $SHA ($GITHUB_REF_NAME). Once every build and test has passed, the annotated tag and the release are created; if any fails, nothing is." >> "$GITHUB_STEP_SUMMARY"
      fi
    else
      NEW=false
      SHA="$(grep -F "refs/tags/$TAG^{}" <<< "$REFS" | cut -f1 || true)"
      if [ -z "$SHA" ]; then
        echo "::error::$TAG is a lightweight tag; releases use annotated tags (git tag -a $TAG -m \"WeFlow $TAG\" <commit>)"
        exit 1
      fi
      echo "$TAG is an annotated tag at $SHA"
      if [ "$DRY" = true ]; then
        echo "Dry run: the existing tag $TAG at $SHA is built and tested on every platform; no release is made." >> "$GITHUB_STEP_SUMMARY"
      else
        echo "Not published yet: the existing tag $TAG at $SHA. Once every build and test has passed, the release is created; if any fails, nothing is." >> "$GITHUB_STEP_SUMMARY"
      fi
    fi
    {
      echo "tag=$TAG"
      echo "sha=$SHA"
      echo "new=$NEW"
    } >> "$GITHUB_OUTPUT"
    ;;
  package)
    TAG="$2" NAME="$3" EXT="$4"
    DIR="weflow-${TAG}-${NAME}"
    if command -v sha256sum >/dev/null; then sha256sum -c ./*.sha256; else shasum -a 256 -c ./*.sha256; fi
    if [ "${RUNNER_OS:-}" = "Windows" ]; then 7z x -y "$DIR.zip" > /dev/null; else tar -xzf "$DIR.tar.gz"; fi
    for f in "weflow$EXT" README.md README.zh-CN.md LICENSE; do
      [ -f "$DIR/$f" ] || { echo "::error::$f is missing from the package"; exit 1; }
    done
    echo "the package holds weflow$EXT, README.md, README.zh-CN.md and LICENSE"
    BIN="$DIR/weflow$EXT"
    OUT="$("$BIN" --version)"
    echo "$OUT"
    if [[ "$OUT" != "weflow ${TAG#v} ("* ]]; then
      echo "::error::weflow --version does not match the tag $TAG"
      exit 1
    fi
    echo "weflow --version matches $TAG"
    "$BIN" config path
    ;;
  *)
    echo "usage: $0 tag <tag> | package <tag> <name> <ext>" >&2
    exit 2
    ;;
esac
