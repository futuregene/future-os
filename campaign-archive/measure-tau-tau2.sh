#!/usr/bin/env bash
# TAU-D final measurement for the 2026-10-16 second segment: BOTH feature flavors
# merged into one lcov, then the private target dir is deleted in the SAME script
# (disk is the shared scarce resource; the lcov is the artifact, the dir is not).
set -uo pipefail

WT=/Users/geilige/future-os/.worktrees/cov100-r2
cd "$WT/desktop/src-tauri" || exit 1
for v in $(env | sed -n 's/^\(CARGO[A-Za-z_]*\)=.*/\1/p'); do unset "$v"; done
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo
export RUSTUP_HOME=/Users/geilige/.rustup
export HOME="$WT/target/test-home"
export CARGO_TARGET_DIR="$WT/target/cov-tauri-tau"
mkdir -p "$HOME"
ulimit -n 10240

make -C "$WT" desktop-sidecar-placeholder
echo "placeholder=$?"

echo "== gui run @ $(date) =="
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
echo "GUI_EXIT=$?"

echo "== headless run @ $(date) =="
cargo llvm-cov --no-report --no-fail-fast -j 4 --no-default-features --features headless -- --test-threads=1
echo "HEADLESS_EXIT=$?"

echo "== report @ $(date) =="
cargo llvm-cov report --lcov --output-path /tmp/tauri-tau2-lcov.info
echo "REPORT_EXIT=$?"
cp /tmp/tauri-tau2-lcov.info "$WT/.future/cov100/tauri-tau-after2-lcov.info" 2>/dev/null

for scope in \
  .worktrees/cov100-r2/desktop/src-tauri/src/headless \
  .worktrees/cov100-r2/desktop/src-tauri/src/terminal \
  .worktrees/cov100-r2/desktop/src-tauri/src/cli_tool.rs \
  .worktrees/cov100-r2/desktop/src-tauri/src/agent_host.rs \
  .worktrees/cov100-r2/desktop/src-tauri/src/agent_supervisor.rs \
  .worktrees/cov100-r2/desktop/src-tauri/src/skills_bootstrap.rs ; do
  echo -n "$scope => "
  python3 "$WT/.future/cov100/covlines.py" total /tmp/tauri-tau2-lcov.info --under "$scope" | tail -1
done

echo "== deleting the private target dir @ $(date) =="
rm -rf "$CARGO_TARGET_DIR"
df -h "$WT" | tail -1
echo "== done @ $(date) =="
