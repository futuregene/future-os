#!/usr/bin/env bash
#
# Differential render harness: TypeScript TUI vs Rust TUI port — identical
# corpus inputs, byte-identical ANSI render output.
#
# This is the P2 render-parity gate for the Rust TUI port (tui/rust). It
# drives both implementations with the shared corpus
# (tui/rust/tests/parity-corpus.json) and byte-compares their outputs:
#
#     <kind>|<name>|<base64(JSON.stringify(result))>
#
# per case, for the MarkdownRenderer (pulldown-cmark adapter), ChatArea
# (including the streaming prefix-cache path) and the terminal-image
# helpers. Both sides serialize results with the same escaping rules
# (serde_json == JSON.stringify, verified) and the standard base64 alphabet
# with padding (== Buffer.toString("base64")).
#
# Requirements:
#   - bun (runs tui/render-parity.ts directly; resolves node_modules in tui/)
#   - rustup with the pinned toolchain (rust-toolchain.toml) — the Rust
#     side runs as `cargo run -p tui-rust --example render_parity`
#
# Usage:
#   make test-tui-diff
#   tui/rust/tests/diff-ts-rust.sh [--verbose] [--keep]
#
# Environment: output files land in /tmp/future-tui-diff-*; pass --keep to
# keep them for inspection.

set -euo pipefail

VERBOSE=0
KEEP=0
for arg in "$@"; do
  case "$arg" in
    --verbose) VERBOSE=1 ;;
    --keep) KEEP=1 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

# Resolve repo root from this script's location: tui/rust/tests/.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
TUI_DIR="$ROOT/tui"
RUST_DIR="$TUI_DIR/rust"
CORPUS="$RUST_DIR/tests/parity-corpus.json"

WORK="$(mktemp -d /tmp/future-tui-diff-XXXXXX)"
TS_OUT="$WORK/ts.out"
RUST_OUT="$WORK/rust.out"
if [ "$KEEP" -eq 0 ]; then
  trap 'rm -rf "$WORK"' EXIT
fi

echo "== TUI render parity: TS vs Rust =="
echo "corpus: $CORPUS"
echo "work:   $WORK"

# Sanity: corpus exists and both sides can read it.
[ -f "$CORPUS" ] || { echo "FATAL: corpus missing: $CORPUS" >&2; exit 1; }

# ── TypeScript side (bun, direct execution) ────────────────────────────────
echo "-- TS (bun render-parity.ts) --"
(cd "$TUI_DIR" && bun render-parity.ts "$CORPUS") > "$TS_OUT"
TS_LINES="$(wc -l < "$TS_OUT")"
echo "   $TS_LINES cases"

# ── Rust side (cargo run --example render_parity) ──────────────────────────
echo "-- Rust (cargo run -p tui-rust --example render_parity) --"
(cd "$ROOT" && rustup run 1.97.0 cargo run -q -p tui-rust --example render_parity -- "$CORPUS") > "$RUST_OUT"
RUST_LINES="$(wc -l < "$RUST_OUT")"
echo "   $RUST_LINES cases"

# ── Byte compare ───────────────────────────────────────────────────────────
if [ "$TS_LINES" -ne "$RUST_LINES" ]; then
  echo "FAIL: line count differs (TS=$TS_LINES Rust=$RUST_LINES)" >&2
  exit 1
fi

if diff -u "$TS_OUT" "$RUST_OUT" > "$WORK/diff.txt"; then
  echo "PASS: $TS_LINES/$TS_LINES cases byte-identical"
  exit 0
fi

echo "FAIL: $(grep -c '^[<>]' "$WORK/diff.txt") differing lines across $(grep -c '^---' "$WORK/diff.txt" || true) hunks" >&2
if [ "$VERBOSE" -eq 1 ]; then
  cat "$WORK/diff.txt" >&2
else
  echo "rerun with --verbose to see the diff, or --keep to retain $WORK" >&2
fi
exit 1
