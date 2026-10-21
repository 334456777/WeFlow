#!/usr/bin/env bash
# The checks of web-release.yml. They are kept here, not in the workflow, because Actions prints each step's script
# before it runs it: a refusal written in the step showed in every log, as if it had happened.
# usage: web-release-check.sh version <version asked, or empty for the tag's>   (run at the repository root)
#        web-release-check.sh unpublished <image:version>
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
    # weflow trusts a version never to change, so one is published once
    if oras manifest fetch "$2" > /dev/null 2>&1; then
      echo "::error::$2 is published already; raise the version in web/package.json"
      exit 1
    fi
    echo "$2 is not published yet"
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
