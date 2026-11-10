#!/usr/bin/env bash
# Profile-guided release build of `weflow` for one target (#500), run on a runner of that target:
#   1. an instrumented `weflow` and `bench_account`, built in target/pgo-gen;
#   2. a small synthetic account from `bench_account` (fixed seed, with media; no real data ever reaches CI);
#   3. a training run of that `weflow` on it: exports in every format and with media, the reads of the CLI, and
#      every read of `weflow serve`'s command API (pgo-serve.py);
#   4. the merged profile, and `weflow` built with it at target/<target>/release, where an ordinary release build
#      would put it.
# Code the training does not run is optimised as rarely used. aarch64-pc-windows-msvc gets an ordinary release
# build: its instrumented program crashes as it starts (rustc's profiler runtime there).
# usage: pgo-build.sh <target triple> <executable suffix: "" or .exe>
set -euo pipefail
TARGET=$1
EXT=$2
# RUSTFLAGS replaces the rustflags of .cargo/config.toml, so the static C runtime of Windows is repeated here (#676)
case "$TARGET" in
  *-windows-msvc) CRT=" -Ctarget-feature=+crt-static" ;;
  *) CRT="" ;;
esac
if [ "$TARGET" = aarch64-pc-windows-msvc ]; then
  cargo build --release -p weflow-cli --bin weflow --target "$TARGET"
  exit
fi
GEN=target/pgo-gen
WORK=target/pgo-work
rm -rf "$WORK"
mkdir -p "$WORK"
# absolute paths: rustc and the trained program run in other directories
abs() { if command -v cygpath >/dev/null; then cygpath -aw "$1"; else realpath "$1"; fi; }
RAW=$(abs "$WORK")/raw
if [ -n "$EXT" ]; then PYTHON=python; else PYTHON=python3; fi

rustup component add llvm-tools
SYSROOT=$(rustc --print sysroot)
HOST=$(rustc -vV | sed -n 's/^host: //p')
PROFDATA="$SYSROOT/lib/rustlib/$HOST/bin/llvm-profdata$EXT"

export RUSTFLAGS="-Cprofile-generate=$RAW$CRT"
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

# Nothing here goes to the network: no `--media` with emoji, no Moments media or HTML (avatars), no AI. A failed run
# only leaves its code out of the profile: the build goes on.
run() { "$WEFLOW" --json "$@" > /dev/null || echo "::warning::PGO training run failed: $*"; }
OUT=$WORK/out
for format in json chatlab chatlab-jsonl txt html excel sql weclone arkme-json; do
  run export messages "$GROUP" --format "$format" --out "$OUT/$format"
done
run export messages "$GROUP" --format json --media image,voice,video --out "$OUT/media"
run export messages "$FRIEND" --format html --media image,voice,video --out "$OUT/friend"
run export media --session "$GROUP" --out "$OUT/media-only"
run export sessions --out "$OUT/sessions.json"
run export contacts --out "$OUT/contacts.json"
run export footprint --out "$OUT/footprint.json"
run chat voice "$FRIEND" --out "$OUT/voice"
run chat sessions
run chat contacts
run chat detail "$GROUP"
run chat messages "$GROUP" --limit 5000
run chat messages "$GROUP" --limit 5000 --parsed
run chat search 好 --limit 500
run chat search 好 --session-id "$GROUP" --limit 500 --parsed
run chat dates "$GROUP"
run chat date-counts "$GROUP"
run chat images "$GROUP"
run chat voice-messages "$GROUP"
run chat resources --limit 2000
run chat export-stats
run analytics overall
run analytics rankings
run analytics time
run group list
run group members "$GROUP" --counts
run group ranking "$GROUP"
run group hours "$GROUP"
run group media "$GROUP"
run report annual years
run report annual generate
run report dual generate --friend "$FRIEND"
run sns timeline
run sns export --format json --out "$OUT/sns.json"
run biz accounts
"$PYTHON" .github/scripts/pgo-serve.py "$WEFLOW" "$GROUP" "$FRIEND" || echo "::warning::PGO serve training failed"

"$PROFDATA" merge -o "$WORK/weflow.profdata" "$RAW"
export RUSTFLAGS="-Cprofile-use=$(abs "$WORK")/weflow.profdata$CRT"
cargo build --release -p weflow-cli --bin weflow --target "$TARGET"
rm -rf "$GEN" "$WORK"
