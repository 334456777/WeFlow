#!/usr/bin/env bash
# The measure job of bench.yml on the benchmark runner (#601, #603), run by Git's bash from the root of the job's
# checkout. The runner keeps $WEFLOW_BENCH_DIR (from its .env) between runs, so what a run before set up is used again
# instead of being installed, downloaded or written again: Rust (cargo/, rustup/), the build cache (target/), uv with
# the Python it installs and a venv with psutil (no PowerShell, which the runner's account may not run scripts in),
# and what bench-commits.py keeps there (the accounts, the binaries, the clone the commits are built in).
#
# usage: bench-runner.sh measure   (env: COMMITS, RUNS, SEED, GROUP_MESSAGES, CONVERSATIONS, LOAD_SECONDS, ONLY, FORCE
#                                    as bench.yml passes them)
#        bench-runner.sh clean     what the next run will not use: the Rust toolchains other than this commit's (an
#                                   older commit built here may have brought its own), the build cache once it passes
#                                   TARGET_LIMIT_MIB, what the clone no longer reaches, and this run's files in the
#                                   checkout (bench-commits.py already removed the binaries and accounts)
set -euo pipefail

: "${WEFLOW_BENCH_DIR:?the runner has no WEFLOW_BENCH_DIR in its .env}"
dir=$(cygpath -m "$WEFLOW_BENCH_DIR")
export CARGO_HOME="$dir/cargo" RUSTUP_HOME="$dir/rustup" CARGO_TARGET_DIR="$dir/target"
# a POSIX path: PATH is split at colons, and D:/ has one
export PATH="$(cygpath -u "$CARGO_HOME/bin"):$PATH"
target=x86_64-pc-windows-msvc
tmp=$(cygpath -m "$RUNNER_TEMP")
venv="$dir/venv-psutil-7.2.2"
UV_VERSION=0.12.19
UV_SHA256=6dbb02d79e419522f1c500f0adb1cddcff0cda7d59b0d66ea7f5e3b4a1b2f5f0
export UV_PYTHON_INSTALL_DIR="$dir/python" UV_CACHE_DIR="$dir/uv-cache"
TARGET_LIMIT_MIB=30720

case "${1:-}" in
  measure)
    if [ ! -x "$CARGO_HOME/bin/rustup.exe" ]; then
      curl -sSfLo "$tmp/rustup-init.exe" https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe
      "$tmp/rustup-init.exe" -y --no-modify-path --default-toolchain none --profile minimal
    fi
    uv="$dir/uv-$UV_VERSION/uv.exe"
    if [ ! -x "$uv" ]; then
      curl -sSfLo "$tmp/uv.zip" "https://github.com/astral-sh/uv/releases/download/$UV_VERSION/uv-x86_64-pc-windows-msvc.zip"
      echo "$UV_SHA256  $tmp/uv.zip" | sha256sum -c --quiet
      unzip -oq "$tmp/uv.zip" -d "$dir/uv-$UV_VERSION"
    fi
    if [ ! -x "$venv/Scripts/python.exe" ]; then
      "$uv" venv --quiet --python 3.13 "$venv"
      "$uv" pip install --quiet --python "$venv/Scripts/python.exe" psutil==7.2.2
    fi

    echo "::group::build the tools"
    rustup show active-toolchain || rustup toolchain install
    rustup target add "$target"
    cargo build --release -p weflow-cli --target "$target" --bin weflow --example bench_account --example serve_bench
    mkdir -p tools
    cp "$CARGO_TARGET_DIR/$target/release/weflow.exe" "$CARGO_TARGET_DIR/$target/release/examples/bench_account.exe" \
      "$CARGO_TARGET_DIR/$target/release/examples/serve_bench.exe" tools/
    echo "::endgroup::"

    git fetch -q origin gh-pages
    git show origin/gh-pages:bench/data/index.json > index.json 2>/dev/null || rm -f index.json

    "$venv/Scripts/python.exe" .github/scripts/bench-commits.py --commits "$COMMITS" --bench-dir "$dir" --tools tools \
      --index index.json --out results --runs "$RUNS" --load-seconds "$LOAD_SECONDS" --only "$ONLY" \
      --seed "$SEED" --group-messages "$GROUP_MESSAGES" --conversations "$CONVERSATIONS" ${FORCE:+--force}
    ;;
  clean)
    keep=$(rustup show active-toolchain 2>/dev/null | cut -d' ' -f1 || true)
    if [ -n "$keep" ]; then
      for t in $(rustup toolchain list | cut -d' ' -f1); do
        [ "$t" = "$keep" ] || rustup toolchain uninstall "$t"
      done
    fi
    if [ -d "$CARGO_TARGET_DIR" ]; then
      size=$(du -sm "$CARGO_TARGET_DIR" | cut -f1)
      if [ "$size" -gt "$TARGET_LIMIT_MIB" ]; then
        echo "the build cache holds $size MiB, past $TARGET_LIMIT_MIB: removed"
        rm -rf "$CARGO_TARGET_DIR"
      fi
    fi
    if [ -d "$dir/src/.git" ]; then
      git -C "$dir/src" reflog expire --expire=now --all
      git -C "$dir/src" gc --quiet --prune=now
    fi
    rm -rf tools results index.json
    ;;
  *)
    echo "usage: $0 measure | clean" >&2
    exit 2
    ;;
esac
