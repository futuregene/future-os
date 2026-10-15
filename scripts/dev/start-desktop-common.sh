#!/usr/bin/env bash
# Shared macOS/Linux launcher. Desktop owns the embedded service; this script
# never starts/kills an Agent or edits its database/lock/metadata.
set -euo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
DESKTOP_DIR="$ROOT_DIR/desktop"
MODE="${FUTURE_DESKTOP_AGENT_MODE:-auto}"
case "$MODE" in auto|external) ;; *) echo "FUTURE_DESKTOP_AGENT_MODE must be auto or external" >&2; exit 1 ;; esac
export FUTURE_DESKTOP_AGENT_MODE="$MODE"
DESKTOP_DEV_PORT="${DESKTOP_DEV_PORT:-5173}"
[[ "$DESKTOP_DEV_PORT" =~ ^[0-9]{1,5}$ ]] || { echo "DESKTOP_DEV_PORT must be a port number" >&2; exit 1; }
DESKTOP_DEV_PORT=$((10#$DESKTOP_DEV_PORT))
[[ "$DESKTOP_DEV_PORT" -ge 1 && "$DESKTOP_DEV_PORT" -le 65535 ]] || { echo "DESKTOP_DEV_PORT must be between 1 and 65535" >&2; exit 1; }
CLI_TARGET="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
[[ "$CLI_TARGET" = /* ]] || CLI_TARGET="$ROOT_DIR/$CLI_TARGET"
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then export CARGO_TARGET_DIR="$CLI_TARGET"; fi
LOG_DIR="$ROOT_DIR/.logs"
DESKTOP_PID=""
CONFIG_FILE=""
cleanup() {
  trap '' INT TERM
  if [[ -n "$DESKTOP_PID" ]] && kill -0 -- "-$DESKTOP_PID" 2>/dev/null; then
    kill -TERM -- "-$DESKTOP_PID" 2>/dev/null || true
    for ((attempt=0; attempt<100; attempt++)); do
      kill -0 -- "-$DESKTOP_PID" 2>/dev/null || break
      sleep 0.1
    done
    if kill -0 -- "-$DESKTOP_PID" 2>/dev/null; then
      kill -KILL -- "-$DESKTOP_PID" 2>/dev/null || true
    fi
  fi
  [[ -z "$CONFIG_FILE" ]] || rm -f -- "$CONFIG_FILE"
}
exec_in_own_session() {
  exec perl -MPOSIX=setsid -e 'setsid() != -1 or die "setsid: $!\n"; exec @ARGV or die "exec: $!\n";' -- "$@"
}
echo "Workspace: $ROOT_DIR"
echo "Agent mode: $MODE (auto: compatible external or embedded when absent; external: attach only)"
echo "Endpoint: ${FUTURE_AGENT_GRPC_ADDR:-auto}; Desktop performs readiness/version/root/lock checks"
echo "Desktop dev port: $DESKTOP_DEV_PORT"
echo "CLI artifact: $CLI_TARGET/debug/future"
if [[ "${DRY_RUN:-0}" == 1 ]]; then
  echo "DRY_RUN=1; no builds, writes, service probes or process launches."
  exit 0
fi
command -v perl >/dev/null || { echo "perl is required for owned Desktop process-group cleanup" >&2; exit 1; }
mkdir -p "$LOG_DIR"
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
command -v node >/dev/null || { echo "Node.js is required for Desktop development" >&2; exit 1; }
# Probe before any Cargo/Tauri child. Empty values override Cargo wrapper config
# for this launch and its children, without changing the invoking shell.
: >"$LOG_DIR/rust-cache-preflight.log"
CACHE_BYPASS="$(node "$ROOT_DIR/scripts/dev/start-desktop-rust-cache.mjs" "$LOG_DIR/rust-cache-preflight.log")"
while IFS= read -r wrapper; do
  case "$wrapper" in
    RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER) export "$wrapper=" ;;
    '') ;;
    *) echo "Invalid Rust cache preflight response" >&2; exit 1 ;;
  esac
done <<< "$CACHE_BYPASS"
[[ -d "$DESKTOP_DIR/node_modules" ]] || (cd "$ROOT_DIR" && npm ci)
if [[ "${BUILD_CLI:-1}" == 1 ]]; then
  if ! (cd "$ROOT_DIR" && cargo build -p future-cli) >"$LOG_DIR/future-cli-dev.build.log" 2>&1; then
    echo "CLI build failed; embedded Agent can still run. CLI-dependent skills need a matching future binary." >&2
  fi
fi
TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
[[ -n "$TRIPLE" ]] || exit 1
STAGED="$DESKTOP_DIR/src-tauri/binaries/future-$TRIPLE"
mkdir -p "$(dirname -- "$STAGED")"
if [[ -x "$CLI_TARGET/debug/future" && -s "$CLI_TARGET/debug/future" ]]; then
  install -m 755 "$CLI_TARGET/debug/future" "$STAGED"
  export PATH="$CLI_TARGET/debug:$PATH"
elif [[ ! -s "$STAGED" ]]; then
  echo "No development CLI artifact; only CLI-dependent features are unavailable." >&2
  # Compile-only externalBin placeholder. Never considered a usable tool.
  : >"$STAGED"
fi
if [[ "${RUN_CHECKS:-0}" == 1 ]]; then
  (cd "$DESKTOP_DIR" && npm run lint && npm run stylelint)
  (cd "$DESKTOP_DIR/src-tauri" && cargo check)
fi
CONFIG_FILE="$(mktemp "${TMPDIR:-/tmp}/futureos-tauri-dev.XXXXXX")"
printf '{"build":{"devUrl":"http://127.0.0.1:%s","beforeDevCommand":"npm run dev -- --strictPort --port %s"}}\n' "$DESKTOP_DEV_PORT" "$DESKTOP_DEV_PORT" >"$CONFIG_FILE"
echo "Starting Desktop; Ctrl-C terminates this dev process group. Quit in the app for coordinated cleanup."
echo "Log: $LOG_DIR/futureos-desktop-dev.log.console"
(cd "$DESKTOP_DIR"; exec_in_own_session npm run tauri:dev -- --config "$CONFIG_FILE") </dev/null > >(tee "$LOG_DIR/futureos-desktop-dev.log.console") 2>&1 &
DESKTOP_PID="$!"
status=0
wait "$DESKTOP_PID" || status=$?
exit "$status"
