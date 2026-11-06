#!/usr/bin/env bash
# Profile-guided release build of `weflow` for one target (#500), run on a runner of that target:
#   1. an instrumented `weflow` and `bench_account`, built in target/pgo-gen;
#   2. a small synthetic account from `bench_account` (fixed seed, with media; no real data ever reaches CI);
#   3. a training run of that `weflow` on it: exports in every format, one with media, and the main reads;
#   4. the merged profile, and `weflow` built with it at target/<target>/release, where an ordinary release build
#      would put it.
# usage: pgo-build.sh <target triple> <executable suffix: "" or .exe>
set -euo pipefail
TARGET=$1
EXT=$2
GEN=target/pgo-gen
WORK=target/pgo-work
rm -rf "$WORK"
mkdir -p "$WORK"
# absolute paths: rustc and the trained program run in other directories
abs() { if command -v cygpath >/dev/null; then cygpath -aw "$1"; else realpath "$1"; fi; }
RAW=$(abs "$WORK")/raw

rustup component add llvm-tools
SYSROOT=$(rustc --print sysroot)
HOST=$(rustc -vV | sed -n 's/^host: //p')
PROFDATA="$SYSROOT/lib/rustlib/$HOST/bin/llvm-profdata$EXT"

export RUSTFLAGS="-Cprofile-generate=$RAW"
cargo build --release -p weflow-cli --bin weflow --target "$TARGET" --target-dir "$GEN"
cargo build --release -p weflow-cli --example bench_account --target "$TARGET" --target-dir "$GEN"
WEFLOW="$GEN/$TARGET/release/weflow$EXT"

# the account is written by the instrumented programs too: what they count meanwhile is thrown away
HOME_DIR=$(LLVM_PROFILE_FILE="$(abs "$WORK")/discard-%p.profraw" \
  "$GEN/$TARGET/release/examples/bench_account$EXT" "$WORK/account" --weflow "$WEFLOW" --seed 7 \
  --group-messages 30000 --conversations 60 | tail -n 1 | tr -d '\r')
rm -rf "$RAW" "$WORK"/discard-*.profraw
export WEFLOW_HOME=$HOME_DIR WEFLOW_LANG=en
manifest="$WORK/account/manifest.json"
GROUP=$(sed -n 's/.*"username": *"\([^"]*@chatroom\)".*/\1/p' "$manifest" | head -n 1)
FRIEND=$(sed -n 's/.*"username": *"\([^"@]*\)".*/\1/p' "$manifest" | head -n 1)
test -n "$GROUP" && test -n "$FRIEND"

# a failed run only leaves its code out of the profile: the build goes on
run() { "$WEFLOW" --json "$@" > /dev/null || echo "::warning::PGO training run failed: $*"; }
for format in json chatlab chatlab-jsonl txt html excel sql weclone arkme-json; do
  run export messages "$GROUP" --format "$format" --out "$WORK/out/$format"
done
run export messages "$GROUP" --format json --media image,voice,video --out "$WORK/out/media"
run export messages "$FRIEND" --format html --media image,voice,video --out "$WORK/out/friend"
run chat sessions
run chat messages "$GROUP" --limit 5000 --parsed
run chat search 好 --limit 500
run chat date-counts "$GROUP"
run chat images "$GROUP"
run analytics overall
run analytics rankings
run group members "$GROUP" --counts
run report annual generate --year 2025

"$PROFDATA" merge -o "$WORK/weflow.profdata" "$RAW"
export RUSTFLAGS="-Cprofile-use=$(abs "$WORK")/weflow.profdata"
cargo build --release -p weflow-cli --bin weflow --target "$TARGET"
rm -rf "$GEN" "$WORK"
