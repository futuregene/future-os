#!/usr/bin/env bash
# scripts/coverage.sh — workspace test coverage via cargo llvm-cov.
#
# Usage:
#   scripts/coverage.sh                 # full workspace run
#   scripts/coverage.sh -p future-tui   # subset (extra args forwarded to
#                                       # `cargo llvm-cov` test invocation)
#
# Outputs (under coverage/, gitignored):
#   lcov.info    — LCOV report for CI / tooling
#   html/        — browsable HTML report (open coverage/html/index.html)
#   summary.txt  — per-file line-coverage table
# Plus a totals table printed to stdout.
#
# Requires: cargo-llvm-cov (`cargo install cargo-llvm-cov`) and the
# llvm-tools-preview rustup component for the pinned toolchain
# (`rustup component add llvm-tools-preview`).
set -euo pipefail

cd "$(dirname "$0")/.."

out_dir=coverage
mkdir -p "$out_dir"

# Build and run the instrumented test binaries once, keeping the raw profile
# data; the reports below are rendered from it without re-running tests.
# `clean --workspace` drops only stale profile data, preserving the build cache
# (target/llvm-cov-target) so re-runs after adding tests stay incremental.
cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --no-report "$@"

cargo llvm-cov report --lcov --output-path "$out_dir/lcov.info"
# NOTE: --html writes into <output-dir>/html/, so pass the base dir.
cargo llvm-cov report --html --output-dir "$out_dir"
cargo llvm-cov report --text --output-path "$out_dir/summary.txt"
cargo llvm-cov report --summary-only
