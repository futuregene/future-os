#!/usr/bin/env bash
# Baseline coverage: Rust workspace members (10 crates).
# One atomic invocation (build + test + report) so a stale profraw merge cannot
# silently zero the report. See .future/memory/coverage-llvm-cov.md.
set -uo pipefail

ROOT=/Users/geilige/future-os
cd "$ROOT" || exit 1

# env hygiene: Homebrew rust shadows rustup, stale CARGO vars poison unit hashes
for v in $(env | sed -n 's/^\(CARGO[A-Za-z_]*\)=.*/\1/p'); do unset "$v"; done
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
# HOME isolation keeps tests away from the real ~/.future; CARGO_HOME/RUSTUP_HOME
# must be pinned back, or cargo re-downloads and rebuilds the whole graph.
export CARGO_HOME=/Users/geilige/.cargo
export RUSTUP_HOME=/Users/geilige/.rustup
export HOME="$ROOT/target/test-home"
mkdir -p "$HOME"

OUT="$ROOT/coverage/round2"
mkdir -p "$OUT"

echo "== clean (drops stale profile data, keeps build cache) @ $(date) =="
cargo llvm-cov clean --workspace
echo "clean exit=$?"

echo "== instrumented workspace run @ $(date) =="
cargo llvm-cov --workspace --no-fail-fast --lcov \
  --output-path "$OUT/workspace-lcov.info" -j 16
echo "RUN_EXIT=$?"

echo "== done @ $(date) =="
ls -l "$OUT/workspace-lcov.info"
