#!/bin/bash
set -euo pipefail

echo "FutureOS local GUI test"

SCRIPT_PATH="${BASH_SOURCE[0]}"
SCRIPT_DIR="${SCRIPT_PATH%/*}"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
GUI_DIR="$ROOT_DIR/gui"
AGENT_DIR="$ROOT_DIR/agent"
LOG_DIR="$ROOT_DIR/.logs"

AGENT_ADDR="${FUTURE_AGENT_GRPC_ADDR:-127.0.0.1:50051}"
AGENT_HOST="${AGENT_ADDR%%:*}"
AGENT_PORT="${AGENT_ADDR##*:}"
GUI_DEV_PORT="${GUI_DEV_PORT:-5173}"
AGENT_LOG="$LOG_DIR/future-agent-test.log"
AGENT_PID_FILE="$LOG_DIR/future-agent-test.pid"
STARTED_AGENT_PID=""

REUSE_AGENT="${REUSE_AGENT:-0}"
BUILD_AGENT="${BUILD_AGENT:-1}"
CLEAN_STALE_APP_TASKS="${CLEAN_STALE_APP_TASKS:-1}"
DRY_RUN="${DRY_RUN:-0}"

cleanup() {
  if [[ -n "$STARTED_AGENT_PID" ]] && kill -0 "$STARTED_AGENT_PID" 2>/dev/null; then
    echo "Stopping future-agent pid=$STARTED_AGENT_PID"
    kill "$STARTED_AGENT_PID" 2>/dev/null || true
    wait "$STARTED_AGENT_PID" 2>/dev/null || true
  fi
  if [[ -f "$AGENT_PID_FILE" ]] && [[ "$(cat "$AGENT_PID_FILE" 2>/dev/null || true)" == "$STARTED_AGENT_PID" ]]; then
    rm -f "$AGENT_PID_FILE"
  fi
}

wait_for_agent() {
  local attempts=60

  for _ in $(seq 1 "$attempts"); do
    if nc -z "$AGENT_HOST" "$AGENT_PORT" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done

  echo "future-agent did not become ready at $AGENT_ADDR"
  echo "Agent log: $AGENT_LOG"
  tail -n 80 "$AGENT_LOG" 2>/dev/null || true
  return 1
}

stop_pid_file_process() {
  local pid_file="$1"
  local label="$2"
  local pid

  if [[ ! -f "$pid_file" ]]; then
    return 0
  fi

  pid="$(cat "$pid_file" 2>/dev/null || true)"
  if [[ -z "$pid" ]] || [[ "$pid" == "$$" ]] || [[ "$pid" == "${BASHPID:-$$}" ]] || [[ "$pid" == "${PPID:-}" ]]; then
    rm -f "$pid_file"
    return 0
  fi

  if ! kill -0 "$pid" 2>/dev/null; then
    rm -f "$pid_file"
    return 0
  fi

  if ! pid_looks_like_agent "$pid"; then
    echo "Ignoring stale $label pid file; pid=$pid is not this test agent."
    rm -f "$pid_file"
    return 0
  fi

  echo "Stopping previous $label pid=$pid"
  kill "$pid" 2>/dev/null || true
  sleep 1

  if kill -0 "$pid" 2>/dev/null; then
    echo "Force stopping previous $label pid=$pid"
    kill -9 "$pid" 2>/dev/null || true
  fi

  rm -f "$pid_file"
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

cancel_stale_app_tasks() {
  local db_path="$HOME/.future/app/app.db"

  if [[ ! -f "$db_path" ]]; then
    return 0
  fi
  if ! command -v sqlite3 >/dev/null 2>&1; then
    echo "sqlite3 not found; skipping stale app task cleanup."
    return 0
  fi

  echo "Cancelling stale GUI runs and approvals in $db_path"
  sqlite3 "$db_path" <<'SQL' || echo "Skipping stale app task cleanup because the database is busy or not initialized."
UPDATE approval_requests
SET status = 'cancelled',
    decision_note = 'Cancelled by start-gui-test.sh before a fresh GUI test run.',
    decided_at = CAST(strftime('%s','now') AS INTEGER) * 1000,
    updated_at = CAST(strftime('%s','now') AS INTEGER) * 1000
WHERE status = 'pending';

UPDATE runs
SET status = 'cancelled',
    error_message = 'Cancelled by start-gui-test.sh before a fresh GUI test run.',
    ended_at = COALESCE(ended_at, CAST(strftime('%s','now') AS INTEGER) * 1000),
    updated_at = CAST(strftime('%s','now') AS INTEGER) * 1000
WHERE status IN ('queued', 'running', 'waiting_approval');

UPDATE tool_calls
SET status = 'failed',
    ended_at = COALESCE(ended_at, CAST(strftime('%s','now') AS INTEGER) * 1000)
WHERE status = 'running';
SQL
}

trap cleanup EXIT INT TERM

mkdir -p "$LOG_DIR"

echo "Workspace: $ROOT_DIR"
echo "Agent gRPC: $AGENT_ADDR"
echo "GUI dev port: $GUI_DEV_PORT"

if [[ "$DRY_RUN" == "1" ]]; then
  echo "DRY_RUN=1; startup checks only, not cleaning tasks or starting processes."
  exit 0
fi

if [[ "$CLEAN_STALE_APP_TASKS" == "1" ]]; then
  cancel_stale_app_tasks
fi

if [[ "${RUN_CHECKS:-0}" == "1" ]]; then
  echo "Running GUI checks..."
  (cd "$GUI_DIR" && npm run lint)
  (cd "$GUI_DIR" && npm run stylelint)
  (cd "$GUI_DIR" && npm test)
  (cd "$GUI_DIR" && npm run build)
  (cd "$GUI_DIR/src-tauri" && cargo check)
fi

if [[ ! -d "$GUI_DIR/node_modules" ]]; then
  echo "Installing GUI dependencies..."
  (cd "$GUI_DIR" && npm ci)
fi

if [[ "$BUILD_AGENT" == "1" ]]; then
  echo "Building future-agent..."
  (cd "$AGENT_DIR" && cargo build)
fi

if [[ "$REUSE_AGENT" == "1" ]] && nc -z "$AGENT_HOST" "$AGENT_PORT" >/dev/null 2>&1; then
  echo "Using existing future-agent at $AGENT_ADDR"
else
  stop_pid_file_process "$AGENT_PID_FILE" "future-agent"
  if nc -z "$AGENT_HOST" "$AGENT_PORT" >/dev/null 2>&1; then
    echo "Port $AGENT_PORT is already in use, but not by the agent process recorded in $AGENT_PID_FILE."
    echo "Stop the old process manually, or run with REUSE_AGENT=1 if you intentionally want to reuse it."
    exit 1
  fi
  echo "Starting future-agent..."
  (
    cd "$AGENT_DIR"
    cargo run
  ) >"$AGENT_LOG" 2>&1 &
  STARTED_AGENT_PID="$!"
  echo "$STARTED_AGENT_PID" >"$AGENT_PID_FILE"
  wait_for_agent
  echo "future-agent started pid=$STARTED_AGENT_PID"
  echo "Agent log: $AGENT_LOG"
fi

echo "Starting GUI..."
echo "Press Ctrl-C here to stop the GUI and the agent started by this script."

(
  cd "$GUI_DIR"
  FUTURE_AGENT_GRPC_ADDR="$AGENT_ADDR" npm run tauri:dev
)
