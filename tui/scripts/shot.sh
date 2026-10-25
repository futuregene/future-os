#!/usr/bin/env bash
# shot.sh — capture one real workbench frame and render it to PNG.
#
#   tui/scripts/shot.sh <scenario> <COLSxROWS> [out.png]
#
# A *real* session, not the demo: the workbench attaches to an agent and draws
# its real transcript, real goal todos and real usage. Bring your own isolated
# instance first — never point this at the agent a running `future tui` uses:
#
#   export FUTURE_HOME=/tmp/wb-shot
#   mkdir -p "$FUTURE_HOME/agent" "$FUTURE_HOME/tasks"
#   cp ~/.future/agent/{settings.json,models.json} "$FUTURE_HOME/agent/"
#   cp -R ~/.future/agent/skills "$FUTURE_HOME/agent/skills"
#   sqlite3 ~/.future/tasks/tasks.db ".backup $FUTURE_HOME/tasks/tasks.db"
#   FUTURE_VERSION=$(git describe --tags --always)+local \
#     cargo build -q -p future-tui -p future-cli
#   FUTURE_HOME=$FUTURE_HOME ./target/debug/future agent --grpc-addr 127.0.0.1:5511 &
#   export FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5511
#
# Scenarios: launchpad chat todo usage skills scrolled help palette palette-filter
#            light full-panel drag dead-command
#
# Exit codes: 2 = usage or missing tool, 1 = tmux/capture failure.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
# The main checkout, which is where the lab tools and the built binaries live
# even when this script runs from a task worktree.
MAIN=$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir | xargs dirname)
SCENARIO=${1:-chat}
SIZE=${2:-120x38}
OUT=${3:-$ROOT/docs/tui/assets/workbench-$SCENARIO-$SIZE.png}

COLS=${SIZE%x*}
ROWS=${SIZE#*x}
SESS=${SHOT_SESSION:-wbshot}
PY=${SHOT_PY:-$MAIN/.future/workbench-lab/.venv/bin/python}
[ -x "$PY" ] || PY=${SHOT_PY:-python3}
ANSI2PNG=${ANSI2PNG:-$MAIN/.future/workbench-lab/tools/ansi2png.py}
CLICK=${SHOT_CLICK:-$MAIN/.future/workbench-lab/tools/click.py}
TUI=${SHOT_TUI:-$MAIN/target/debug/future-tui}
TMP=$(mktemp -d)
trap 'tmux kill-session -t "$SESS" 2>/dev/null || true; rm -rf "$TMP"' EXIT

[ -x "$TUI" ] || { echo "no future-tui at $TUI (cargo build -p future-tui)" >&2; exit 2; }
[ -f "$ANSI2PNG" ] || { echo "no ansi2png.py at $ANSI2PNG" >&2; exit 2; }

# ─── small helpers ──────────────────────────────────────────────────────────
send() { tmux send-keys -t "$SESS" "$@" ; }

# One SGR 1006 report, hex-encoded so tmux passes it through untouched.
mouse() { # x y [button] [M|m]
  local x=$1 y=$2 button=${3:-0} final=${4:-M}
  local seq
  seq=$(printf '\033[<%d;%d;%d%s' "$button" "$x" "$y" "$final")
  # shellcheck disable=SC2046
  tmux send-keys -t "$SESS" -H $(printf '%s' "$seq" | od -An -tx1 | tr -s ' \n' ' ')
}

click_word() { # row needle — clicks the first occurrence on that 1-based row
  "$PY" "$CLICK" "$SESS" "$1" "$2" >/dev/null
}

key() { tmux send-keys -t "$SESS" "$1"; sleep "${2:-1.2}"; }

start() {
  tmux kill-session -t "$SESS" 2>/dev/null || true
  tmux new-session -d -s "$SESS" -x "$COLS" -y "$ROWS" -c "$ROOT"
  sleep 0.5
  local args=""
  if [ -n "${SESSION_ID:-}" ]; then args="--session $SESSION_ID"; fi
  # shellcheck disable=SC2086
  send "FUTURE_HOME=${FUTURE_HOME:?set FUTURE_HOME} \
FUTURE_AGENT_GRPC_ADDR=${FUTURE_AGENT_GRPC_ADDR:-auto} $TUI $args" Enter
  sleep "${SHOT_SETTLE:-9}"
}

# ─── scenarios ──────────────────────────────────────────────────────────────
case "$SCENARIO" in
  launchpad)
    SESSION_ID= start;;
  chat)
    start;;
  todo)
    start; click_word 7 待办; sleep 1.5;;
  usage)
    start; click_word 7 用量; sleep 1.5;;
  skills)
    start; click_word 7 技能; sleep 1.5;;
  scrolled)
    start
    mouse 20 20 0 M; mouse 20 20 0 m
    for _ in 1 2 3; do mouse 20 20 64 M; done
    sleep 1.5;;
  help)
    start; key '?';;
  palette)
    start; key '/';;
  palette-filter)
    start; key '/'; send 'theme'; sleep 1;;
  light)
    start; key '/'; send 'theme'; sleep 0.8; send Enter; sleep 2;;
  full-panel)
    start; key C-b;;
  drag)
    start; mouse 84 12 0 M; mouse 86 12 32 M; mouse 90 12 32 M; mouse 90 12 0 m; sleep 1.5;;
  dead-command)
    start; key '/'; send 'usage'; sleep 0.8; send Enter; sleep 1.5;;
  *)
    echo "unknown scenario: $SCENARIO" >&2; exit 2;;
esac

tmux capture-pane -p -e -t "$SESS" > "$TMP/frame.ansi"
"$PY" "$ANSI2PNG" "$TMP/frame.ansi" "$OUT" --cols "$COLS" --rows "$ROWS"
echo "wrote $OUT (${COLS}x${ROWS}) from $SESS"
