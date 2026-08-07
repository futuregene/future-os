#!/usr/bin/env bash
#
# Golden render harness: Rust TUI vs recorded TypeScript outputs.
#
# The TypeScript TUI was retired (2026-08, same as the CLI): its render
# outputs were recorded per corpus case into
# `tui/tests/golden/parity-ts.golden` BEFORE the TS sources were deleted, so
# the port keeps a byte-identical reference without a second implementation.
# This harness drives the Rust implementation with the shared corpus
# (`tui/tests/parity-corpus.json`) and byte-compares its output against the
# golden:
#
#     <kind>|<name>|<base64(JSON.stringify(result))>
#
# per case, for the MarkdownRenderer (pulldown-cmark adapter), ChatArea
# (including the streaming prefix-cache path) and the terminal-image
# helpers. Both sides serialize results with the same escaping rules
# (serde_json == JSON.stringify, verified) and the standard base64 alphabet
# with padding (== Buffer.toString("base64")).
#
# Re-recording the golden is possible only from the pre-retirement TS tree
# (`bun render-parity.ts <corpus>`); it is committed, not regenerated.
#
# Requirements:
#   - rustup with the pinned toolchain (rust-toolchain.toml) — the Rust
#     side runs as `cargo run -p future-tui --example render_parity`
#
# Usage:
#   make test-tui-diff
#   tui/tests/golden-diff.sh [--verbose] [--keep]
#
# Environment: output files land in /tmp/future-tui-diff-*; pass --keep to
# keep them for inspection.

set -euo pipefail

VERBOSE=0
KEEP=0
RECORD=0
for arg in "$@"; do
  case "$arg" in
    --record) RECORD=1 ;;
    --verbose) VERBOSE=1 ;;
    --keep) KEEP=1 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

# Resolve repo root from this script's location: tui/tests/.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TUI_DIR="$ROOT/tui"
CORPUS="$TUI_DIR/tests/parity-corpus.json"
GOLDEN="$TUI_DIR/tests/golden/parity-ts.golden"

WORK="$(mktemp -d /tmp/future-tui-diff-XXXXXX)"
RUST_OUT="$WORK/rust.out"
if [ "$KEEP" -eq 0 ]; then
  trap 'rm -rf "$WORK"' EXIT
fi

echo "== TUI render parity: Rust vs golden (recorded from TS) =="
echo "corpus: $CORPUS"
echo "golden: $GOLDEN"
echo "work:   $WORK"
[ "$RECORD" -eq 1 ] && echo "mode:   RECORD (golden <- Rust output)"

# Sanity: corpus + golden exist.
[ -f "$CORPUS" ] || { echo "FATAL: corpus missing: $CORPUS" >&2; exit 1; }
[ -f "$GOLDEN" ] || { echo "FATAL: golden missing: $GOLDEN" >&2; exit 1; }

# ── Rust side (cargo run --example render_parity) ──────────────────────────
echo "-- Rust (cargo run -p future-tui --example render_parity) --"
(cd "$ROOT" && rustup run 1.97.0 cargo run -q -p future-tui --example render_parity -- "$CORPUS") > "$RUST_OUT"
RUST_LINES="$(wc -l < "$RUST_OUT")"
echo "   $RUST_LINES cases"

if [ "$RECORD" -eq 1 ]; then
  cp "$RUST_OUT" "$GOLDEN"
  echo "RECORD: $GOLDEN rewritten from the Rust renderer ($RUST_LINES cases)"
  exit 0
fi

# ── Byte compare ───────────────────────────────────────────────────────────
GOLDEN_LINES="$(wc -l < "$GOLDEN")"
if [ "$GOLDEN_LINES" -ne "$RUST_LINES" ]; then
  echo "FAIL: line count differs (golden=$GOLDEN_LINES Rust=$RUST_LINES)" >&2
  exit 1
fi

if diff -u "$GOLDEN" "$RUST_OUT" > "$WORK/diff.txt"; then
  echo "PASS: $RUST_LINES/$RUST_LINES cases byte-identical"
  exit 0
fi

echo "FAIL: $(grep -c '^[<>]' "$WORK/diff.txt") differing lines across $(grep -c '^---' "$WORK/diff.txt" || true) hunks" >&2
if [ "$VERBOSE" -eq 1 ]; then
  cat "$WORK/diff.txt" >&2
else
  echo "rerun with --verbose to see the diff, or --keep to retain $WORK" >&2
fi
exit 1
