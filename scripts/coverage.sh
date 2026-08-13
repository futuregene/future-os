#!/usr/bin/env bash
# scripts/coverage.sh — workspace test coverage via cargo llvm-cov.
#
# Usage:
#   scripts/coverage.sh                 # full workspace run
#   scripts/coverage.sh --check         # run + enforce the coverage ratchet
#                                       # against scripts/coverage-baseline.json
#                                       # (CI mode; exit 1 on any floor drop)
#   scripts/coverage.sh -p future-tui   # subset (extra args forwarded to
#                                       # `cargo llvm-cov` test invocation)
#
# Outputs (under coverage/, gitignored):
#   lcov.info      — LCOV report for CI / tooling
#   html/          — browsable HTML report (open coverage/html/index.html)
#   summary.txt    — per-file line-coverage table
#   llvm-cov.json  — raw `--json --summary-only` export
#   baseline.json  — per-crate lines/functions/regions aggregation
#                    (scripts/coverage_ratchet.py emit; P1-7 ratchet input)
# Plus a totals table printed to stdout.
#
# Requires: cargo-llvm-cov (`cargo install cargo-llvm-cov`), the
# llvm-tools-preview rustup component for the pinned toolchain
# (`rustup component add llvm-tools-preview`), and python3.
set -euo pipefail

cd "$(dirname "$0")/.."

out_dir=coverage
mkdir -p "$out_dir"

# Separate the script's own flags from cargo-llvm-cov passthrough args.
check=0
args=()
for a in "$@"; do
  case "$a" in
    --check) check=1 ;;
    *) args+=("$a") ;;
  esac
done

# Build and run the instrumented test binaries once, keeping the raw profile
# data; the reports below are rendered from it without re-running tests.
# `clean --workspace` drops only stale profile data, preserving the build cache
# (target/llvm-cov-target) so re-runs after adding tests stay incremental.
cargo llvm-cov clean --workspace
# ${args[@]+...} keeps an empty array legal under `set -u` on bash 3.2 (macOS).
cargo llvm-cov --workspace --no-report ${args[@]+"${args[@]}"}

cargo llvm-cov report --lcov --output-path "$out_dir/lcov.info"
# NOTE: --html writes into <output-dir>/html/, so pass the base dir.
cargo llvm-cov report --html --output-dir "$out_dir"
cargo llvm-cov report --text --output-path "$out_dir/summary.txt"
cargo llvm-cov report --json --summary-only --output-path "$out_dir/llvm-cov.json"
python3 scripts/coverage_ratchet.py emit "$out_dir/llvm-cov.json" "$out_dir/baseline.json"
cargo llvm-cov report --summary-only

# P1-7 ratchet: line coverage may only go up; an intentional drop is approved
# by lowering the floor in scripts/coverage-baseline.json in the same PR.
if [[ "$check" == 1 ]]; then
  python3 scripts/coverage_ratchet.py check \
    --baseline scripts/coverage-baseline.json \
    --current "$out_dir/baseline.json"
fi
