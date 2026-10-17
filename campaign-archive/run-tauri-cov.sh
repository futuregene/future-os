#!/usr/bin/env bash
# Baseline coverage: desktop/src-tauri (not a workspace member; own run).
# --test-threads=1 is mandatory on this host: the lib suite deadlocks in parallel
# (HOME<->MOCK lock-order inversion, .future/memory/testing-gotchas.md).
set -uo pipefail

ROOT=/Users/geilige/future-os
cd "$ROOT" || exit 1

for v in $(env | sed -n 's/^\(CARGO[A-Za-z_]*\)=.*/\1/p'); do unset "$v"; done
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo
export RUSTUP_HOME=/Users/geilige/.rustup
export HOME="$ROOT/target/test-home"
mkdir -p "$HOME"
ulimit -n 10240

OUT="$ROOT/coverage/round2"
mkdir -p "$OUT"

# tauri-build aborts without the externalBin placeholders (gitignored artifacts)
make desktop-sidecar-placeholder
echo "placeholder exit=$?"

cd "$ROOT/desktop/src-tauri" || exit 1
echo "== clean @ $(date) =="
cargo llvm-cov clean
echo "clean exit=$?"

echo "== instrumented src-tauri run @ $(date) =="
cargo llvm-cov --no-fail-fast --lcov --output-path "$OUT/tauri-lcov.info" -j 16 \
  -- --test-threads=1
echo "RUN_EXIT=$?"

# Known host failures on this machine abort the combined invocation before the
# report step; the profraw is still on disk, so render the report separately.
if [ ! -s "$OUT/tauri-lcov.info" ]; then
  echo "== combined run produced no report; rendering from profraw =="
  cargo llvm-cov report --lcov --output-path "$OUT/tauri-lcov.info"
  echo "REPORT_EXIT=$?"
fi

echo "== done @ $(date) =="
ls -l "$OUT/tauri-lcov.info"
