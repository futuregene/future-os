#!/usr/bin/env bash
# Second pass: complete the gui-flavor half of the merged TAU-D measurement
# (the first attempt's gui run was terminated externally at 18:33), then
# re-render the report over BOTH flavors' profraw.
set -uo pipefail
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri || exit 1
for v in $(env | sed -n 's/^\(CARGO[A-Za-z_]*\)=.*/\1/p'); do unset "$v"; done
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo
export RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home
export CARGO_TARGET_DIR=/Users/geilige/future-os/.worktrees/cov100-r2/target/cov-tauri-tau
mkdir -p "$HOME"
ulimit -n 10240
make -C /Users/geilige/future-os/.worktrees/cov100-r2 desktop-sidecar-placeholder
echo "== gui re-run @ $(date) =="
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
echo "GUI_EXIT=$?"
echo "== report @ $(date) =="
cargo llvm-cov report --lcov --output-path /tmp/tauri-tau-lcov.info
echo "REPORT_EXIT=$?"
ls -l /tmp/tauri-tau-lcov.info
echo "== done @ $(date) =="
