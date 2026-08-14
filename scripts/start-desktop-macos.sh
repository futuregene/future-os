#!/bin/bash
set -euo pipefail

echo "FutureOS local desktop test"

SCRIPT_PATH="${BASH_SOURCE[0]}"
SCRIPT_DIR="${SCRIPT_PATH%/*}"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DESKTOP_DIR="$ROOT_DIR/desktop"
AGENT_DIR="$ROOT_DIR/agent"
CLI_DIR="$ROOT_DIR/cli"
LOG_DIR="$ROOT_DIR/.logs"

AGENT_ADDR="${FUTURE_AGENT_GRPC_ADDR:-127.0.0.1:50051}"
AGENT_HOST="${AGENT_ADDR%%:*}"
AGENT_PORT="${AGENT_ADDR##*:}"
DESKTOP_DEV_PORT="${DESKTOP_DEV_PORT:-5173}"
# The agent writes to its default log location (~/.future/agent/logs/agent.log,
# created by the agent itself) via bare `--log-file`; the repo .logs dir only
# holds script state (pid file) and the agent's stdout/stderr capture.
AGENT_LOG="$HOME/.future/agent/logs/agent.log"
AGENT_CONSOLE_LOG="$LOG_DIR/future-agent-test.log.console"
AGENT_PID_FILE="$LOG_DIR/future-agent-test.pid"
STARTED_AGENT_PID=""

REUSE_AGENT="${REUSE_AGENT:-0}"
BUILD_AGENT="${BUILD_AGENT:-1}"
BUILD_CLI="${BUILD_CLI:-1}"
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
  echo "Agent console log (stdout/stderr, panics): $AGENT_CONSOLE_LOG"
  tail -n 80 "$AGENT_CONSOLE_LOG" 2>/dev/null || true
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

  echo "Cancelling stale desktop runs and approvals in $db_path"
  sqlite3 "$db_path" <<'SQL' || echo "Skipping stale app task cleanup because the database is busy or not initialized."
UPDATE approval_requests
SET status = 'cancelled',
    decision_note = 'Cancelled by start-desktop-macos.sh before a fresh desktop test run.',
    decided_at = CAST(strftime('%s','now') AS INTEGER) * 1000,
    updated_at = CAST(strftime('%s','now') AS INTEGER) * 1000
WHERE status = 'pending';

UPDATE runs
SET status = 'cancelled',
    error_message = 'Cancelled by start-desktop-macos.sh before a fresh desktop test run.',
    ended_at = COALESCE(ended_at, CAST(strftime('%s','now') AS INTEGER) * 1000),
    updated_at = CAST(strftime('%s','now') AS INTEGER) * 1000
WHERE status IN ('queued', 'running', 'waiting_approval');
SQL
}

trap cleanup EXIT INT TERM

mkdir -p "$LOG_DIR"

echo "Workspace: $ROOT_DIR"
echo "Agent gRPC: $AGENT_ADDR"
echo "desktop dev port: $DESKTOP_DEV_PORT"

if [[ "$DRY_RUN" == "1" ]]; then
  echo "DRY_RUN=1; startup checks only, not cleaning tasks or starting processes."
  exit 0
fi

if [[ "$CLEAN_STALE_APP_TASKS" == "1" ]]; then
  cancel_stale_app_tasks
fi


if [[ ! -d "$DESKTOP_DIR/node_modules" ]]; then
  echo "Installing desktop dependencies..."
  (cd "$DESKTOP_DIR" && npm ci)
fi

if [[ "${RUN_CHECKS:-0}" == "1" ]]; then
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

# Build the unified Rust CLI (cargo build, matching make build-cli) and put it
# on the agent's PATH, so skills that shell out to `future` resolve it.
# Non-fatal: a failure only means those skills won't work; the desktop test proceeds.
if [[ "$BUILD_CLI" == "1" ]]; then
  echo "Building future CLI..."
  (cd "$CLI_DIR" && cargo build) || \
    echo "future CLI build failed; skills that call \`future\` will not work."
fi
# The agent (started below) inherits this exported PATH.
if [[ -x "$ROOT_DIR/target/debug/future" ]]; then
  export PATH="$ROOT_DIR/target/debug:$PATH"
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
  # `agent` is a member of the root Cargo workspace, so `cargo build` (even when
  # invoked from within agent/) writes the binary to the workspace-level target
  # dir ($ROOT_DIR/target/debug) — NOT $AGENT_DIR/target/debug. Launching a
  # stale crate-local binary here is exactly what breaks `--log-file` support.
  # Point at the workspace output, falling back to a crate-local target for
  # non-workspace checkouts.
  AGENT_BIN="$ROOT_DIR/target/debug/future-agent"
  if [[ ! -x "$AGENT_BIN" ]]; then
    AGENT_BIN="$AGENT_DIR/target/debug/future-agent"
  fi
  if [[ ! -x "$AGENT_BIN" ]]; then
    echo "Agent binary not found at $AGENT_BIN."
    echo "Build it first (BUILD_AGENT defaults to 1) or run with BUILD_AGENT=1."
    exit 1
  fi
  # exec the built binary directly instead of `cargo run`, so $! is the agent's
  # own pid rather than the cargo wrapper's. Otherwise killing the recorded pid
  # leaves the orphaned future-agent child holding the gRPC port.
  # The agent writes structured logs to its default location ($AGENT_LOG) via
  # bare --log-file; stdout/stderr (panics, pre-tracing output) go to
  # $AGENT_CONSOLE_LOG so the same lines are not duplicated into $AGENT_LOG by
  # shell redirection.
  (
    cd "$AGENT_DIR"
    exec "$AGENT_BIN" --log-file
  ) >"$AGENT_CONSOLE_LOG" 2>&1 &
  STARTED_AGENT_PID="$!"
  echo "$STARTED_AGENT_PID" >"$AGENT_PID_FILE"
  wait_for_agent
  echo "future-agent started pid=$STARTED_AGENT_PID"
  echo "Agent log: $AGENT_LOG"
  echo "Agent console log: $AGENT_CONSOLE_LOG"
fi

# Tauri validates bundle.externalBin sidecars (future) at COMPILE time — even
# for `tauri dev`. This script runs the agent as a standalone process and the
# desktop connects to it, so the bundled sidecar is never launched here; it only
# needs to exist. Create an empty placeholder if it is missing (CI and the
# packaging scripts stage the real binary).
TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
BIN_DIR="$DESKTOP_DIR/src-tauri/binaries"
mkdir -p "$BIN_DIR"
sidecar="$BIN_DIR/future-$TRIPLE"
if [[ ! -f "$sidecar" ]]; then
  : >"$sidecar"
  chmod +x "$sidecar"
fi

echo "Starting desktop..."
echo "Press Ctrl-C here to stop the desktop and the agent started by this script."

(
  cd "$DESKTOP_DIR"
  FUTURE_AGENT_GRPC_ADDR="$AGENT_ADDR" npm run tauri:dev
)
