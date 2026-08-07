#!/usr/bin/env bash
#
# tmux screen-consistency harness: Rust TUI vs committed goldens.
#
# The TypeScript TUI was retired (2026-08, same as the CLI); the golden files
# under `tui/tests/golden/` were recorded from the TS pane BEFORE the TS
# sources were deleted. This harness runs the Rust TUI in a tmux pane
# connected to a deterministic mock agent (examples/mock_agent), drives it
# with keystrokes, and byte-compares `capture-pane -e` screens against the
# goldens — a divergence in the port (or an intentional screen change, which
# must be committed together with re-recorded goldens) is caught.
#
# This is the P4 screen-consistency gate for the Rust TUI port: the welcome
# banner, footer token/cache stats, status overlay, help overlay, model
# selector, sessions overlay and Ctrl+C exit.
#
# Requirements:
#   - tmux (panes provide the PTYs)
#   - rustup with the pinned toolchain (rust-toolchain.toml) — builds the
#     Rust TUI (future-tui) and the mock agent example
#
# Usage:
#   make test-tui-tmux          # verify mode (goldens must match)
#   tui/tests/tmux-diff.sh
#   tui/tests/tmux-diff.sh --record     # rewrite goldens from the Rust pane
#   tui/tests/tmux-diff.sh --verbose    # show failing diffs
#   tui/tests/tmux-diff.sh --keep       # keep /tmp/future-tui-tmux-* artifacts
#
# Golden files: tui/tests/golden/<scenario>.txt

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
GOLDEN_DIR="$TUI_DIR/tests/golden"

RUST_TUI="$ROOT/target/debug/future-tui"
MOCK_AGENT="$ROOT/target/debug/examples/mock_agent"

SESSION="tui-diff-$$"
WINDOW="${SESSION}:0"
RUST_PANE="${WINDOW}.0"

WORK="$(mktemp -d /tmp/future-tui-tmux-XXXXXX)"
RUST_HOME="$WORK/home-rust"
mkdir -p "$RUST_HOME"
RUST_OUT="$WORK/rust.out"
if [ "$KEEP" -eq 0 ]; then
  trap 'tmux kill-session -t "$SESSION" 2>/dev/null || true; kill "${MOCK_A_PID:-}" 2>/dev/null || true; rm -rf "$WORK"' EXIT
else
  trap 'tmux kill-session -t "$SESSION" 2>/dev/null || true; kill "${MOCK_A_PID:-}" 2>/dev/null || true; echo "kept $WORK"' EXIT
fi

echo "== TUI screen consistency: Rust vs golden (tmux) =="
echo "work:   $WORK"
[ "$RECORD" -eq 1 ] && echo "mode:   RECORD (goldens <- Rust pane)"
echo "golden: $GOLDEN_DIR"

# ── Preflight ───────────────────────────────────────────────────────────────
if ! command -v tmux >/dev/null 2>&1; then
  echo "SKIP: tmux not found — tmux screen-consistency test requires an interactive"
  echo "      terminal server (run locally, not in headless CI)."
  exit 0
fi
mkdir -p "$GOLDEN_DIR"

# ── Build Rust TUI + mock agent ─────────────────────────────────────────────
echo "-- build future-tui + mock_agent --"
(cd "$ROOT" && rustup run 1.97.0 cargo build -q -p tui-rust --bin future-tui)
(cd "$ROOT" && rustup run 1.97.0 cargo build -q -p tui-rust --example mock_agent)
[ -x "$RUST_TUI" ] || { echo "FATAL: $RUST_TUI missing" >&2; exit 1; }
[ -x "$MOCK_AGENT" ] || { echo "FATAL: $MOCK_AGENT missing" >&2; exit 1; }

# ── Free port for the mock agent ────────────────────────────────────────────
pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}
PORT_A="$(pick_port)"

# ── Start mock agent (deterministic) ────────────────────────────────────────
"$MOCK_AGENT" --port "$PORT_A" > "$WORK/mock-a.log" 2>&1 &
MOCK_A_PID=$!
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
wait_port "$PORT_A" || { echo "FATAL: mock agent (port $PORT_A) did not start" >&2; exit 1; }

# ── Start tmux session: one 80x36 pane ─────────────────────────────────────
tmux kill-session -t "$SESSION" 2>/dev/null || true
tmux new-session -d -s "$SESSION" -x 80 -y 36 \
  "cd $ROOT && HOME=$RUST_HOME $RUST_TUI --grpc-addr 127.0.0.1:$PORT_A; echo RUST_EXIT=\$? > $WORK/rust.exit"

# Sanity: the pane must be exactly 80x36.
PANE_SIZES="$(tmux list-panes -t "$WINDOW" -F '#{pane_width}x#{pane_height}' | sort -u)"
if [ "$PANE_SIZES" != "80x36" ]; then
  echo "FATAL: unexpected pane sizes: $PANE_SIZES (want 80x36)" >&2
  exit 1
fi

capture() { # $1 = out file
  tmux capture-pane -t "$RUST_PANE" -p -e > "$1"
}

# Wait for the welcome screen (banner "future-tui v0.0.0-mock").
wait_welcome() {
  local i
  for i in $(seq 1 300); do
    if tmux capture-pane -t "$RUST_PANE" -p 2>/dev/null | grep -q "future-tui v"; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}
echo "-- starting TUI --"
wait_welcome || { echo "FATAL: Rust TUI never showed welcome" >&2; exit 1; }
sleep 1   # let deferred renders settle

# ── Scenario runner ─────────────────────────────────────────────────────────
PASS=0
FAIL=0
step() { # $1 = scenario name
  local name="$1" golden="$GOLDEN_DIR/$1.txt"
  sleep 1
  capture "$RUST_OUT"

  if [ "$RECORD" -eq 1 ]; then
    cp "$RUST_OUT" "$golden"
    echo "OK[$name]: recorded golden ($(wc -c < "$golden") bytes)"
    PASS=$((PASS+1))
    return
  fi

  if [ ! -f "$golden" ]; then
    echo "FAIL[$name]: golden missing: $golden (run with --record first)"
    FAIL=$((FAIL+1))
    return
  fi
  if ! diff -q "$RUST_OUT" "$golden" >/dev/null; then
    echo "FAIL[$name]: Rust pane drifted from golden"
    if [ "$VERBOSE" -eq 1 ]; then
      diff -u "$golden" "$RUST_OUT" | head -40
    fi
    FAIL=$((FAIL+1))
    return
  fi
  echo "OK[$name]: Rust == golden ($(wc -c < "$golden") bytes)"
  PASS=$((PASS+1))
}

send_lit() { tmux send-keys -t "$RUST_PANE" -l "$1"; }

# Deterministic slash-command submission. Typing "/cmd" triggers the
# autocomplete popup after a 20 ms debounce; a rapid Enter would be consumed
# by the popup (applying the selection) instead of submitting. So: wait for
# the popup, Enter (applies selection, closes popup), Enter (submits).
# If the popup never appeared, the first Enter submits and the second is a
# no-op — either way exactly one submission happens.
submit_cmd() {
  send_lit "$1"
  sleep 0.5
  tmux send-keys -t "$RUST_PANE" Enter
  sleep 0.3
  tmux send-keys -t "$RUST_PANE" Enter
}

# ── Scenarios ───────────────────────────────────────────────────────────────
step welcome

send_lit "hello"
step typed

tmux send-keys -t "$RUST_PANE" Enter
step reply

submit_cmd "/status"
step status

submit_cmd "/help"
step help-overlay

tmux send-keys -t "$RUST_PANE" Escape
step help-closed

submit_cmd "/model"
step model-overlay

tmux send-keys -t "$RUST_PANE" Escape
step model-closed

submit_cmd "/sessions"
step sessions-overlay

tmux send-keys -t "$RUST_PANE" Escape
step sessions-closed

# ── Ctrl+C exit ─────────────────────────────────────────────────────────────
tmux send-keys -t "$RUST_PANE" C-c
sleep 4
RUST_EXIT="$(grep -o '[0-9][0-9]*' "$WORK/rust.exit" 2>/dev/null | head -1 || echo MISSING)"
if [ "$RUST_EXIT" = "0" ]; then
  echo "OK[ctrl-c]: Rust TUI exited 0"
  PASS=$((PASS+1))
else
  echo "FAIL[ctrl-c]: Rust TUI exit code = $RUST_EXIT (want 0)"
  FAIL=$((FAIL+1))
fi

tmux kill-session -t "$SESSION" 2>/dev/null || true

echo
echo "== result: $PASS passed, $FAIL failed =="
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
