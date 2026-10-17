#!/usr/bin/env bash
# PLAN 5.1 Rust gates: clippy --all-targets -D warnings, BOTH flavors (exactly the
# two commands in .github/workflows/ci.yml "clippy (desktop backend)").
# Uses a private CARGO_TARGET_DIR and deletes it in the same invocation.
set -uo pipefail

WT=/Users/geilige/future-os/.worktrees/cov100-r2
cd "$WT" || exit 1
for v in $(env | sed -n 's/^\(CARGO[A-Za-z_]*\)=.*/\1/p'); do unset "$v"; done
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo
export RUSTUP_HOME=/Users/geilige/.rustup
export HOME="$WT/target/test-home"
export CARGO_TARGET_DIR="$WT/target/cov-tauri-clippy"
mkdir -p "$HOME"
ulimit -n 10240

make -C "$WT" desktop-sidecar-placeholder

echo "== clippy gui (default features) @ $(date) =="
cargo clippy --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings
echo "CLIPPY_GUI_EXIT=$?"

echo "== clippy headless @ $(date) =="
cargo clippy --no-default-features --features headless --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings
echo "CLIPPY_HEADLESS_EXIT=$?"

echo "== deleting private clippy target dir @ $(date) =="
if [ "${KEEP_TARGET:-0}" = "1" ]; then
  echo "KEEP_TARGET=1: dir kept for iteration"
else
  rm -rf "$CARGO_TARGET_DIR"
fi
df -h "$WT" | tail -1
echo "== done @ $(date) =="
