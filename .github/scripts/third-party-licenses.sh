#!/usr/bin/env bash
# The licenses of what a release is built from, for its packages (release.yml, #1111): THIRD-PARTY-LICENSES.html,
# which cargo-about makes from .github/about/ (every crate and its license text), and the license files it does not
# see, named as THIRD-PARTY-LICENSES.html links to them: of the code in this workspace's crates that comes from other
# projects (about.toml leaves these crates out), and of the C libraries built into the program, which the crates that
# bundle them do not list (their own licenses cover the bindings).
# Usage: third-party-licenses.sh <out dir>   (cargo-about and jq on PATH; run from the repository's root)
set -euo pipefail
OUT=$1
mkdir -p "$OUT"

cargo about generate --fail \
  --config .github/about/about.toml \
  --manifest-path crates/weflow-cli/Cargo.toml \
  --output-file "$OUT/THIRD-PARTY-LICENSES.html" \
  .github/about/licenses.hbs

META=$(cargo metadata --format-version 1 --locked)
# copy <crate> <path in its sources, may be a glob> <file name in OUT>
copy() {
  local dir file
  dir=$(jq -r --arg name "$1" '[.packages[] | select(.name == $name)][0].manifest_path // empty' <<< "$META")
  if [ -z "$dir" ]; then
    echo "$1 is not a dependency any more: drop it from $0 and from licenses.hbs" >&2
    exit 1
  fi
  dir=$(dirname "$dir")
  file=$(compgen -G "$dir/$2" | head -n 1 || true)
  if [ -z "$file" ]; then
    echo "no $2 in $1 ($dir)" >&2
    exit 1
  fi
  cp "$file" "$OUT/$3"
}
cp crates/weflow-silk/vendor/LICENSE-SKYPE.txt "$OUT/silk.txt"
cp crates/weflow-native/src/windows_db_key/LICENSE "$OUT/wx_key.txt"
# linux_key_helper/LICENSE is the same file
cp crates/weflow-native/src/macos_key_helper/LICENSE "$OUT/weflow-desktop.txt"
copy zstd-sys 'zstd/LICENSE' zstd.txt
copy bzip2-sys 'bzip2-*/LICENSE' bzip2.txt
copy lzma-sys 'xz-*/COPYING' xz.txt
copy libmimalloc-sys 'c_src/mimalloc/v2/LICENSE' mimalloc.txt
ls -l "$OUT"
