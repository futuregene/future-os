#!/usr/bin/env bash
#
# FutureOS local desktop test (Linux). Mirrors start-desktop-macos.sh: build
# and start a standalone future-agent, run the Tauri desktop against it, then
# stop only the agent started by this script when the desktop exits.
#
# Bubblewrap is deliberately diagnostic-only here. A missing or unsupported
# bwrap must still allow Desktop to start so its sandbox-unavailable UI can be
# tested; future-agent owns the authoritative capability and security checks.
#
# Environment knobs:
#   FUTURE_AGENT_GRPC_ADDR  Agent address (default: 127.0.0.1:50051)
#   DESKTOP_DEV_PORT        Vite dev-server port (default: 5173)
#   REUSE_AGENT             Reuse an agent already listening (default: 0)
#   BUILD_AGENT             Build future-agent first (default: 1)
#   BUILD_CLI               Build future CLI and add it to PATH (default: 1)
#   CLEAN_STALE_APP_TASKS   Cancel stale runs/approvals first (default: 1)
#   RUN_CHECKS              Run desktop/Rust checks first (default: 0)
#   DRY_RUN                 Print diagnostics without changing state (default: 0)

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
DESKTOP_DIR="$ROOT_DIR/desktop"
AGENT_DIR="$ROOT_DIR/agent"
CLI_DIR="$ROOT_DIR/cli"
LOG_DIR="$ROOT_DIR/.logs"

AGENT_ADDR="${FUTURE_AGENT_GRPC_ADDR:-127.0.0.1:50051}"
AGENT_HOST="${AGENT_ADDR%%:*}"
AGENT_PORT="${AGENT_ADDR##*:}"
DESKTOP_DEV_PORT="${DESKTOP_DEV_PORT:-5173}"
AGENT_LOG="$HOME/.future/agent/logs/agent.log"
AGENT_CONSOLE_LOG="$LOG_DIR/future-agent-test.log.console"
AGENT_PID_FILE="$LOG_DIR/future-agent-test.pid"
STARTED_AGENT_PID=""
TAURI_DEV_CONFIG_FILE=""

REUSE_AGENT="${REUSE_AGENT:-0}"
BUILD_AGENT="${BUILD_AGENT:-1}"
BUILD_CLI="${BUILD_CLI:-1}"
CLEAN_STALE_APP_TASKS="${CLEAN_STALE_APP_TASKS:-1}"
RUN_CHECKS="${RUN_CHECKS:-0}"
DRY_RUN="${DRY_RUN:-0}"

fail() {
  echo "error: $*" >&2
  exit 1
}

require_tool() {
  local command_name="$1"
  local hint="$2"
  command -v "$command_name" >/dev/null 2>&1 || fail "missing '$command_name'. $hint"
}

port_is_open() {
  (exec 3<>"/dev/tcp/$AGENT_HOST/$AGENT_PORT") >/dev/null 2>&1
}

pid_looks_like_agent() {
  local pid="$1"
  local command_line

  if ! command -v ps >/dev/null 2>&1; then
    return 0
  fi

  command_line="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  [[ "$command_line" == *"$AGENT_DIR"* || "$command_line" == *"future-agent"* ]]
}

cleanup() {
  if [[ -n "$STARTED_AGENT_PID" ]] && kill -0 "$STARTED_AGENT_PID" 2>/dev/null; then
    echo "Stopping future-agent pid=$STARTED_AGENT_PID"
    kill "$STARTED_AGENT_PID" 2>/dev/null || true
    wait "$STARTED_AGENT_PID" 2>/dev/null || true
  fi
  if [[ -f "$AGENT_PID_FILE" ]] \
    && [[ "$(<"$AGENT_PID_FILE")" == "$STARTED_AGENT_PID" ]]; then
    rm -f -- "$AGENT_PID_FILE"
  fi
  if [[ -n "$TAURI_DEV_CONFIG_FILE" ]]; then
    rm -f -- "$TAURI_DEV_CONFIG_FILE"
  fi
}

stop_pid_file_process() {
  local pid

  [[ -f "$AGENT_PID_FILE" ]] || return 0
  pid="$(<"$AGENT_PID_FILE")"

  if [[ -z "$pid" || "$pid" == "$$" || "$pid" == "${BASHPID:-$$}" \
    || "$pid" == "${PPID:-}" ]]; then
    rm -f -- "$AGENT_PID_FILE"
    return 0
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    rm -f -- "$AGENT_PID_FILE"
    return 0
  fi
  if ! pid_looks_like_agent "$pid"; then
    echo "Ignoring stale agent pid file; pid=$pid is not this test agent."
    rm -f -- "$AGENT_PID_FILE"
    return 0
  fi

  echo "Stopping previous future-agent pid=$pid"
  kill "$pid" 2>/dev/null || true
  sleep 1
  if kill -0 "$pid" 2>/dev/null; then
    echo "Force stopping previous future-agent pid=$pid"
    kill -9 "$pid" 2>/dev/null || true
  fi
  rm -f -- "$AGENT_PID_FILE"
}

wait_for_agent() {
  local attempts=60

  for _ in $(seq 1 "$attempts"); do
    if port_is_open; then
      return 0
    fi
    sleep 1
  done

  echo "future-agent did not become ready at $AGENT_ADDR"
  echo "Agent log: $AGENT_LOG"
  tail -n 80 "$AGENT_LOG" 2>/dev/null || true
  echo "Agent console log (stdout/stderr, panics): $AGENT_CONSOLE_LOG"
  tail -n 80 "$AGENT_CONSOLE_LOG" 2>/dev/null || true
  return 1
}

cancel_stale_app_tasks() {
  local db_path="$HOME/.future/app/app.db"

  [[ -f "$db_path" ]] || return 0
  if ! command -v sqlite3 >/dev/null 2>&1; then
    echo "sqlite3 not found; skipping stale app task cleanup."
    return 0
  fi

  echo "Cancelling stale desktop runs and approvals in $db_path"
  sqlite3 "$db_path" <<'SQL' || echo "Skipping stale app task cleanup because the database is busy or not initialized."
UPDATE approval_requests
SET status = 'cancelled',
    decision_note = 'Cancelled by start-desktop-linux.sh before a fresh desktop test run.',
    decided_at = CAST(strftime('%s','now') AS INTEGER) * 1000,
    updated_at = CAST(strftime('%s','now') AS INTEGER) * 1000
WHERE status = 'pending';

UPDATE runs
SET status = 'cancelled',
    error_message = 'Cancelled by start-desktop-linux.sh before a fresh desktop test run.',
    ended_at = COALESCE(ended_at, CAST(strftime('%s','now') AS INTEGER) * 1000),
    updated_at = CAST(strftime('%s','now') AS INTEGER) * 1000
WHERE status IN ('queued', 'running', 'waiting_approval');
SQL
}

[[ "$(uname -s)" == "Linux" ]] || fail "this script must run on Linux"
require_tool node "Install Node.js 24+."
require_tool npm "npm is included with Node.js."
require_tool cargo "Install Rust from https://rustup.rs."
require_tool rustc "Install Rust from https://rustup.rs."

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -p -- "$LOG_DIR"

echo "FutureOS local desktop test (Linux)"
echo "Workspace: $ROOT_DIR"
echo "Agent gRPC: $AGENT_ADDR"
echo "Desktop dev port: $DESKTOP_DEV_PORT"
if command -v bwrap >/dev/null 2>&1; then
  echo "Bubblewrap: $(command -v bwrap) ($(bwrap --version 2>/dev/null || echo 'version unavailable'))"
else
  echo "Bubblewrap: not installed; Desktop should report sandbox unavailable (binary_missing)."
fi

if [[ "$DRY_RUN" == "1" ]]; then
  echo "DRY_RUN=1; diagnostics only, not cleaning tasks or starting processes."
  exit 0
fi

if [[ "$CLEAN_STALE_APP_TASKS" == "1" ]]; then
  cancel_stale_app_tasks
fi

if [[ ! -d "$DESKTOP_DIR/node_modules" ]]; then
  echo "Installing desktop dependencies..."
  (cd "$DESKTOP_DIR" && npm ci)
fi

if [[ "$RUN_CHECKS" == "1" ]]; then
  echo "Running desktop checks..."
  (cd "$DESKTOP_DIR" && npm run lint)
  (cd "$DESKTOP_DIR" && npm run stylelint)
  (cd "$DESKTOP_DIR" && npm test)
  (cd "$DESKTOP_DIR" && npm run build)
  (cd "$DESKTOP_DIR/src-tauri" && cargo check)
fi

if [[ "$BUILD_AGENT" == "1" ]]; then
  echo "Building future-agent..."
  (cd "$AGENT_DIR" && cargo build)
fi

# Non-fatal: Desktop and the agent still work when this build fails, but skills
# that shell out to `future` will be unavailable in this test session.
if [[ "$BUILD_CLI" == "1" ]]; then
  echo "Building future CLI..."
  (cd "$CLI_DIR" && cargo build) \
    || echo "future CLI build failed; skills that call \`future\` will not work."
fi
if [[ -x "$ROOT_DIR/target/debug/future" ]]; then
  export PATH="$ROOT_DIR/target/debug:$PATH"
fi

if [[ "$REUSE_AGENT" == "1" ]] && port_is_open; then
  echo "Using existing future-agent at $AGENT_ADDR"
else
  stop_pid_file_process
  if port_is_open; then
    echo "Port $AGENT_PORT is already in use, but not by the agent recorded in $AGENT_PID_FILE."
    echo "Stop it manually, or use REUSE_AGENT=1 if it is the intended agent."
    exit 1
  fi

  AGENT_BIN="$ROOT_DIR/target/debug/future-agent"
  [[ -x "$AGENT_BIN" ]] \
    || fail "agent binary not found at $AGENT_BIN (BUILD_AGENT defaults to 1)"

  echo "Starting future-agent..."
  # Launch the workspace artifact directly so the pid file tracks future-agent,
  # not a cargo wrapper that could leave an orphan holding the gRPC port.
  (
    cd "$AGENT_DIR"
    exec "$AGENT_BIN" --grpc-addr "$AGENT_ADDR" --log-file
  ) >"$AGENT_CONSOLE_LOG" 2>&1 &
  STARTED_AGENT_PID="$!"
  echo "$STARTED_AGENT_PID" >"$AGENT_PID_FILE"
  wait_for_agent
  echo "future-agent started pid=$STARTED_AGENT_PID"
  echo "Agent log: $AGENT_LOG"
  echo "Agent console log: $AGENT_CONSOLE_LOG"
fi

# Tauri validates externalBin at compile time even though this dev session uses
# the standalone agent. The placeholder is created only in the build-staging
# directory and only when absent; packaging scripts replace it with the real CLI.
TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
[[ -n "$TRIPLE" ]] || fail "could not determine the host triple from rustc -Vv"
BIN_DIR="$DESKTOP_DIR/src-tauri/binaries"
SIDECAR="$BIN_DIR/future-$TRIPLE"
mkdir -p -- "$BIN_DIR"
if [[ ! -e "$SIDECAR" ]]; then
  : >"$SIDECAR"
  chmod +x "$SIDECAR"
fi

echo "Starting desktop..."
echo "Press Ctrl-C here to stop Desktop and the agent started by this script."

if [[ "$DESKTOP_DEV_PORT" == "5173" ]]; then
  (cd "$DESKTOP_DIR" && FUTURE_AGENT_GRPC_ADDR="$AGENT_ADDR" npm run tauri:dev)
else
  TAURI_DEV_CONFIG_FILE="$(mktemp "${TMPDIR:-/tmp}/futureos-tauri-dev.XXXXXX")"
  printf '%s\n' \
    '{' \
    '  "build": {' \
    "    \"devUrl\": \"http://127.0.0.1:$DESKTOP_DEV_PORT\"," \
    "    \"beforeDevCommand\": \"npm run dev -- --port $DESKTOP_DEV_PORT\"" \
    '  }' \
    '}' >"$TAURI_DEV_CONFIG_FILE"
  (
    cd "$DESKTOP_DIR"
    FUTURE_AGENT_GRPC_ADDR="$AGENT_ADDR" \
      npm run tauri:dev -- --config "$TAURI_DEV_CONFIG_FILE"
  )
fi
