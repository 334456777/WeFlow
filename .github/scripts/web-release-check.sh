#!/usr/bin/env bash
# The checks of web-release.yml. They are kept here, not in the workflow, because Actions prints each step's script
# before it runs it: a refusal written in the step showed in every log, as if it had happened.
# usage: web-release-check.sh version <version asked, or empty for the tag's>   (run at the repository root)
#        web-release-check.sh unpublished <image:version>   (3 attempts, WEB_RELEASE_RETRY_DELAY s apart, default 5)
#        web-release-check.sh anonymous <image:version> <the zip pushed>
set -euo pipefail

case "${1:-}" in
  version)
    want="${2:-${GITHUB_REF_NAME#web-v}}"
    have=$(bun -p "require('./web/package.json').version")
    if [ "$want" != "$have" ]; then
      echo "::error::asked for $want, but web/package.json is $have"
      exit 1
    fi
    echo "web/package.json is $have, the version asked for"
    {
      echo "version=$have"
      echo "api=$(bun -p "require('./web/package.json').weflow.apiVersion")"
      echo "min=$(bun -p "require('./web/package.json').weflow.minBackendVersion")"
    } >> "$GITHUB_OUTPUT"
    ;;
  unpublished)
    # weflow trusts a version never to change, so one is published once. The push goes on only when the registry itself
    # answers that there is no such version: a network error, a 5xx or 429, a lost login or a missing oras says nothing
    # about the version, and pushing on would replace one that may be published.
    ref="${2:?usage: $0 unpublished <image:version>}"
    if ! command -v oras > /dev/null; then
      echo "::error::oras is not installed, so it cannot be told whether $ref is published"
      exit 1
    fi
    delay="${WEB_RELEASE_RETRY_DELAY:-5}"
    for attempt in 1 2 3; do
      if out=$(oras manifest fetch --descriptor "$ref" 2>&1); then
        echo "::error::$ref is published already; raise the version in web/package.json"
        exit 1
      fi
      # oras (oras-go) turns the registry's HTTP 404 of the manifest into "<reference>: not found" without looking at
      # the error code in the body (MANIFEST_UNKNOWN, NAME_UNKNOWN); every other answer, such as 401 or 403, keeps its
      # own text. oras itself retries 429 and 5xx; the retries here are for the rest (connection, DNS).
      if grep -qF -- "$ref: not found" <<< "$out"; then
        echo "$ref is not published yet"
        exit 0
      fi
      printf '%s\n' "attempt $attempt of 3 gave no answer about $ref:" "$out"
      if [ "$attempt" -lt 3 ]; then
        sleep "$delay"
      fi
    done
    echo "::error::cannot tell whether $ref is published, so nothing is pushed: ${out%%$'\n'*}"
    exit 1
    ;;
  anonymous)
    oras logout ghcr.io
    mkdir pulled
    if oras pull "$2" -o pulled; then
      cmp pulled/weflow-web.zip "$3"
      echo "anonymous download: ok"
    else
      echo "::warning::${2%:*} cannot be downloaded without a login; make the package public once (Package settings → Change visibility)"
    fi
    ;;
  *)
    echo "usage: $0 version [version] | unpublished <image:version> | anonymous <image:version> <zip>" >&2
    exit 2
    ;;
esac
