#!/usr/bin/env bash
#
# tmux screen-consistency harness: TypeScript TUI vs Rust TUI port.
#
# Runs both TUIs side by side in a tmux window (two 80x36 panes), each
# connected to its own deterministic mock agent (examples/mock_agent), drives
# them with identical keystrokes, and byte-compares `capture-pane -e` screens
# at each scenario step. Golden files record the TS pane's screen; verify mode
# checks BOTH panes against the golden byte-for-byte, so a divergence in the
# port OR a drift in the reference is caught.
#
# This is the P4 screen-consistency gate for the Rust TUI port (tui/rust):
# the welcome banner, footer token/cache stats, status overlay, help overlay,
# model selector, sessions overlay and Ctrl+C exit parity.
#
# Requirements:
#   - tmux (panes provide the PTYs)
#   - bun (runs the TS TUI directly: tui/src/index.ts)
#   - rustup with the pinned toolchain (rust-toolchain.toml) — builds the
#     Rust TUI (future-tui) and the mock agent example
#
# Usage:
#   make test-tui-tmux          # verify mode (goldens must match)
#   tui/rust/tests/tmux-diff.sh
#   tui/rust/tests/tmux-diff.sh --record     # regenerate goldens from TS pane
#   tui/rust/tests/tmux-diff.sh --verbose    # show failing diffs
#   tui/rust/tests/tmux-diff.sh --keep       # keep /tmp/future-tui-tmux-* artifacts
#
# Golden files: tui/rust/tests/golden/<scenario>.txt

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

# Resolve repo root from this script's location: tui/rust/tests/.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
TUI_DIR="$ROOT/tui"
RUST_DIR="$TUI_DIR/rust"
GOLDEN_DIR="$RUST_DIR/tests/golden"

RUST_TUI="$ROOT/target/debug/future-tui"
MOCK_AGENT="$ROOT/target/debug/examples/mock_agent"

SESSION="tui-diff-$$"
WINDOW="${SESSION}:0"
TS_PANE="${WINDOW}.0"
RUST_PANE="${WINDOW}.1"

WORK="$(mktemp -d /tmp/future-tui-tmux-XXXXXX)"
TS_HOME="$WORK/home-ts"
RUST_HOME="$WORK/home-rust"
mkdir -p "$TS_HOME" "$RUST_HOME"
TS_OUT="$WORK/ts.out"
RUST_OUT="$WORK/rust.out"
if [ "$KEEP" -eq 0 ]; then
  trap 'tmux kill-session -t "$SESSION" 2>/dev/null || true; kill "${MOCK_A_PID:-}" "${MOCK_B_PID:-}" 2>/dev/null || true; rm -rf "$WORK"' EXIT
else
  trap 'tmux kill-session -t "$SESSION" 2>/dev/null || true; kill "${MOCK_A_PID:-}" "${MOCK_B_PID:-}" 2>/dev/null || true; echo "kept $WORK"' EXIT
fi

echo "== TUI screen consistency: TS vs Rust (tmux) =="
echo "work:   $WORK"
[ "$RECORD" -eq 1 ] && echo "mode:   RECORD (goldens <- TS pane)"
echo "golden: $GOLDEN_DIR"

# ── Preflight ───────────────────────────────────────────────────────────────
if ! command -v tmux >/dev/null 2>&1; then
  echo "SKIP: tmux not found — tmux screen-consistency test requires an interactive"
  echo "      terminal server (run locally, not in headless CI)."
  exit 0
fi
command -v tmux >/dev/null || { echo "FATAL: tmux not found" >&2; exit 1; }
command -v bun >/dev/null || { echo "FATAL: bun not found" >&2; exit 1; }
[ -f "$TUI_DIR/src/index.ts" ] || { echo "FATAL: $TUI_DIR/src/index.ts missing" >&2; exit 1; }
mkdir -p "$GOLDEN_DIR"

# ── Build Rust TUI + mock agent ─────────────────────────────────────────────
echo "-- build future-tui + mock_agent --"
(cd "$ROOT" && rustup run 1.97.0 cargo build -q -p tui-rust --bin future-tui)
(cd "$ROOT" && rustup run 1.97.0 cargo build -q -p tui-rust --example mock_agent)
[ -x "$RUST_TUI" ] || { echo "FATAL: $RUST_TUI missing" >&2; exit 1; }
[ -x "$MOCK_AGENT" ] || { echo "FATAL: $MOCK_AGENT missing" >&2; exit 1; }

# ── Free ports for the two mock agents ──────────────────────────────────────
pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}
PORT_A="$(pick_port)"
PORT_B="$(pick_port)"
while [ "$PORT_B" = "$PORT_A" ]; do PORT_B="$(pick_port)"; done

# ── Start mock agents (one per TUI; deterministic & identical) ──────────────
"$MOCK_AGENT" --port "$PORT_A" > "$WORK/mock-a.log" 2>&1 &
MOCK_A_PID=$!
"$MOCK_AGENT" --port "$PORT_B" > "$WORK/mock-b.log" 2>&1 &
MOCK_B_PID=$!
wait_port() {
  local port="$1" i
  for i in $(seq 1 50); do
    if python3 -c "import socket,sys; s=socket.socket(); s.settimeout(0.2); sys.exit(0 if s.connect_ex(('127.0.0.1',$port))==0 else 1)" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}
wait_port "$PORT_A" || { echo "FATAL: mock agent A (port $PORT_A) did not start" >&2; exit 1; }
wait_port "$PORT_B" || { echo "FATAL: mock agent B (port $PORT_B) did not start" >&2; exit 1; }

# ── Start tmux session: two 80x36 panes side by side ────────────────────────
# Window 161x36 splits into two 80x36 panes (border column excluded).
tmux kill-session -t "$SESSION" 2>/dev/null || true
tmux new-session -d -s "$SESSION" -x 161 -y 36 \
  "cd $TUI_DIR && HOME=$TS_HOME bun run src/index.ts --grpc-addr 127.0.0.1:$PORT_A; echo TS_EXIT=\$? > $WORK/ts.exit" \; \
  set-option -t "$SESSION" pane-border-status off
tmux split-window -h -t "$WINDOW" \
  "cd $ROOT && HOME=$RUST_HOME $RUST_TUI --grpc-addr 127.0.0.1:$PORT_B; echo RUST_EXIT=\$? > $WORK/rust.exit"

# Sanity: both panes must be exactly 80x36 for byte-comparable captures.
PANE_SIZES="$(tmux list-panes -t "$WINDOW" -F '#{pane_width}x#{pane_height}' | sort -u)"
if [ "$PANE_SIZES" != "80x36" ]; then
  echo "FATAL: unexpected pane sizes: $PANE_SIZES (want 80x36)" >&2
  exit 1
fi

capture() { # $1 = pane, $2 = out file
  tmux capture-pane -t "$1" -p -e > "$2"
}

# Wait for the welcome screen on BOTH panes (banner "future-tui v0.0.0-mock").
wait_welcome() {
  local pane="$1" i
  for i in $(seq 1 300); do
    if tmux capture-pane -t "$pane" -p 2>/dev/null | grep -q "future-tui v"; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}
echo "-- starting TUIs --"
wait_welcome "$TS_PANE" || { echo "FATAL: TS TUI never showed welcome" >&2; exit 1; }
wait_welcome "$RUST_PANE" || { echo "FATAL: Rust TUI never showed welcome" >&2; exit 1; }
sleep 1   # let deferred renders settle

# ── Scenario runner ─────────────────────────────────────────────────────────
PASS=0
FAIL=0
step() { # $1 = scenario name
  local name="$1" ts_golden="$GOLDEN_DIR/$1.txt"
  sleep 1
  capture "$TS_PANE" "$TS_OUT"
  capture "$RUST_PANE" "$RUST_OUT"

  # Live TS vs Rust diff (the primary gate).
  if ! diff -q "$TS_OUT" "$RUST_OUT" >/dev/null; then
    echo "FAIL[$name]: TS vs Rust pane differ"
    if [ "$VERBOSE" -eq 1 ]; then
      diff -u "$TS_OUT" "$RUST_OUT" | head -40
    fi
    FAIL=$((FAIL+1))
    return
  fi

  if [ "$RECORD" -eq 1 ]; then
    cp "$TS_OUT" "$ts_golden"
    echo "OK[$name]: recorded golden ($(wc -c < "$ts_golden") bytes)"
    PASS=$((PASS+1))
    return
  fi

  if [ ! -f "$ts_golden" ]; then
    echo "FAIL[$name]: golden missing: $ts_golden (run with --record first)"
    FAIL=$((FAIL+1))
    return
  fi
  if ! diff -q "$TS_OUT" "$ts_golden" >/dev/null; then
    echo "FAIL[$name]: TS pane drifted from golden"
    if [ "$VERBOSE" -eq 1 ]; then
      diff -u "$ts_golden" "$TS_OUT" | head -40
    fi
    FAIL=$((FAIL+1))
    return
  fi
  if ! diff -q "$RUST_OUT" "$ts_golden" >/dev/null; then
    echo "FAIL[$name]: Rust pane drifted from golden"
    if [ "$VERBOSE" -eq 1 ]; then
      diff -u "$ts_golden" "$RUST_OUT" | head -40
    fi
    FAIL=$((FAIL+1))
    return
  fi
  echo "OK[$name]: TS == Rust == golden ($(wc -c < "$ts_golden") bytes)"
  PASS=$((PASS+1))
}

send_both() { tmux send-keys -t "$TS_PANE" "$@"; tmux send-keys -t "$RUST_PANE" "$@"; }
send_both_lit() { tmux send-keys -t "$TS_PANE" -l "$1"; tmux send-keys -t "$RUST_PANE" -l "$1"; }

# Deterministic slash-command submission. Typing "/cmd" triggers the
# autocomplete popup after a 20 ms debounce; a rapid Enter would be consumed
# by the popup (applying the selection) instead of submitting. So: wait for
# the popup, Enter (applies selection, closes popup), Enter (submits).
# If the popup never appeared, the first Enter submits and the second is a
# no-op — either way exactly one submission happens.
submit_cmd() {
  send_both_lit "$1"
  sleep 0.5
  send_both Enter
  sleep 0.3
  send_both Enter
}

# ── Scenarios ───────────────────────────────────────────────────────────────
step welcome

send_both_lit "hello"
step typed

send_both Enter
step reply

submit_cmd "/status"
step status

submit_cmd "/help"
step help-overlay

send_both Escape
step help-closed

submit_cmd "/model"
step model-overlay

send_both Escape
step model-closed

submit_cmd "/sessions"
step sessions-overlay

send_both Escape
step sessions-closed

# ── Ctrl+C exit parity ──────────────────────────────────────────────────────
send_both C-c
sleep 4
TS_EXIT="$(grep -o '[0-9][0-9]*' "$WORK/ts.exit" 2>/dev/null | head -1 || echo MISSING)"
RUST_EXIT="$(grep -o '[0-9][0-9]*' "$WORK/rust.exit" 2>/dev/null | head -1 || echo MISSING)"
if [ "$TS_EXIT" = "0" ] && [ "$RUST_EXIT" = "0" ]; then
  echo "OK[ctrl-c]: both exited 0 (TS=$TS_EXIT Rust=$RUST_EXIT)"
  PASS=$((PASS+1))
else
  echo "FAIL[ctrl-c]: exit codes differ (TS=$TS_EXIT Rust=$RUST_EXIT)"
  FAIL=$((FAIL+1))
fi

tmux kill-session -t "$SESSION" 2>/dev/null || true

echo
echo "== result: $PASS passed, $FAIL failed =="
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
