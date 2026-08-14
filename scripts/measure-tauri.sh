#!/bin/bash
# Measure desktop/src-tauri per-line coverage (lcov DA:0) for the final-tauri residual.
set -uo pipefail
cd "$(dirname "$0")/.."
REALHOME="$HOME"

while IFS='=' read -r k _; do case "$k" in CARGO*) unset "$k";; esac; done < <(env)
export PATH="/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH"
export CARGO_HOME="$REALHOME/.cargo"
export RUSTUP_HOME="$REALHOME/.rustup"

mkdir -p coverage/final-100 target/test-home
triple=$(rustc -Vv | sed -n 's/^host: //p')
mkdir -p desktop/src-tauri/binaries
[ -f "desktop/src-tauri/binaries/future-$triple" ] || : > "desktop/src-tauri/binaries/future-$triple"

export HOME="$PWD/target/test-home"
cargo llvm-cov --manifest-path desktop/src-tauri/Cargo.toml --no-report > coverage/final-100/tauri-run.log 2>&1
echo "tauri run exit=$?"
cargo llvm-cov report --manifest-path desktop/src-tauri/Cargo.toml --lcov --output-path coverage/final-100/tauri-lcov.info >> coverage/final-100/tauri-run.log 2>&1
cargo llvm-cov report --manifest-path desktop/src-tauri/Cargo.toml --summary-only >> coverage/final-100/tauri-run.log 2>&1
export HOME="$REALHOME"

python3 coverage/baseline-100/parse_lcov.py coverage/final-100/tauri-lcov.info coverage/final-100/desktop-tauri-missed.txt
echo "=== done ==="
