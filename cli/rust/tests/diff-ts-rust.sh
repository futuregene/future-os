#!/usr/bin/env bash
#
# Differential harness: TypeScript `future` vs Rust `future` — same argv,
# byte-identical stdout / stderr / exit code.
#
# This is the P4 behavioral-consistency gate for the Rust CLI port
# (cli/rust). Every case below runs the same argv through the TS CLI
# (cli/dist/future, bun-compiled) and the Rust CLI (target/debug/future)
# under an identical controlled environment (fake $HOME, fixed version,
# mock platform server, real agent on a dedicated port) and compares
# stdout, stderr, and the exit code byte for byte.
#
# Requirements:
#   - node + npm + bun (to build the TS CLI, as `make build-cli` does)
#   - rustup with the pinned toolchain (rust-toolchain.toml)
#   - python3 (mock platform server; port probing)
#   - a built future-agent for the "agent" group — auto-detected in
#     target/{debug,release}/future-agent, or set FUTURE_AGENT_BIN
#     (the agent group is SKIPPED, with a notice, when it is absent)
#
# Usage:
#   make test-cli-diff
#   cli/rust/tests/diff-ts-rust.sh [--verbose] [--keep]
#
# Comparison modes:
#   exact     stdout + stderr + exit code must be byte-identical (default)
#   agentdown transport-level gRPC failures: network stacks phrase the
#             error differently (grpc-js vs tonic), so only the exit code,
#             the stdout bytes, and the stderr PREFIX are compared. The
#             rpc.rs module doc documents this accepted divergence.
#
# Excluded from the corpus (documented): `future run` against a live agent
# (real prompt execution) is not diffed; its local-only paths and the
# dead-agent transport error ARE covered below. The `browser` tool's session
# commands (tabs/open/snapshot/click/type/press/screenshot/scroll/console)
# ARE covered via the "browser" scenario (mock CDP endpoint + scripted
# WebSocket). `browser start` against a non-reachable endpoint would spawn a
# real Chrome (non-deterministic) — only the already_running path is diffed.
#
# Notes:
#   - Rebuilds BOTH CLIs with FUTURE_VERSION=0.0.0-diff+local so `--version`
#     (and the baked version.generated.ts) agree regardless of git state.
#   - cli/dist/ and cli/src/version.generated.ts are gitignored build
#     outputs; this script overwrites them (same as `make build-cli`).
#   - Transport-error TEXT (connection refused etc.) is Node/runtime-version
#     dependent; cases that surface it use the "agentdown" mode above.

set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TS_CLI_BUILT="$ROOT/cli/dist/future"
DIFF_VERSION="0.0.0-diff+local"
WORK="$(mktemp -d /tmp/cli-diff.XXXXXX)"
# Both CLIs are run from COPIES under $WORK so that (a) neither executable
# dir has a future-agent sibling (`future init` reports the sibling when
# present; the TS cli/dist has none) and (b) a concurrent rebuild of
# $ROOT/cli/dist/future or target/debug/future (e.g. the loop harness's
# post-commit build, which regenerates version.generated.ts with the leaked
# FUTURE_VERSION) cannot clobber the binaries mid-corpus. Equal layouts are
# required for byte-identical output.
TS_CLI="$WORK/ts-bin/future"
RUST_CLI="$WORK/rust-bin/future"
RUST_CLI_BUILT="$ROOT/target/debug/future"

VERBOSE=0
KEEP=0
for a in "$@"; do
  case "$a" in
    --verbose) VERBOSE=1 ;;
    --keep) KEEP=1 ;;
  esac
done

pass=0; fail=0; skip=0
declare -a FAILED_CASES=()

# ── logging ────────────────────────────────────────────────────────────────

info()  { printf '\033[2m%s\033[0m\n' "$*"; }
ok()    { printf '\033[32m[ok]\033[0m   %s\n' "$*"; }
skipm() { printf '\033[33m[skip]\033[0m %s\n' "$*"; }
bad()   { printf '\033[31m[FAIL]\033[0m %s\n' "$*"; }
err()   { printf '\033[31merror: %s\033[0m\n' "$*" >&2; }

cleanup() {
  teardown_scenario "${current_scenario:-}"
  if [[ "$KEEP" != 1 ]]; then
    rm -rf "$WORK"
  else
    info "workdir kept at $WORK"
  fi
}
trap cleanup EXIT

# ── helpers ────────────────────────────────────────────────────────────────

free_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}

wait_port() {
  local port="$1" tries="${2:-60}"
  for _ in $(seq 1 "$tries"); do
    if python3 -c "import socket,sys; s=socket.socket(); s.settimeout(0.2); sys.exit(0 if s.connect_ex(('127.0.0.1', $port))==0 else 1)" 2>/dev/null; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

# Build the TS CLI with the pinned version (mirrors `make build-cli`).
build_ts() {
  info "building TS CLI (FUTURE_VERSION=$DIFF_VERSION) ..."
  (cd "$ROOT/cli" && \
     FUTURE_VERSION="$DIFF_VERSION" npm run gen-version >/dev/null && \
     FUTURE_VERSION="$DIFF_VERSION" npm run build >/dev/null && \
     FUTURE_VERSION="$DIFF_VERSION" bun build --compile dist/index.js --outfile dist/future >/dev/null 2>&1) \
    || { err "TS CLI build failed (need node/npm/bun + npm install in cli/)"; exit 2; }
  mkdir -p "$(dirname "$TS_CLI")"
  cp "$TS_CLI_BUILT" "$TS_CLI"
}

build_rust() {
  info "building Rust CLI (FUTURE_VERSION=$DIFF_VERSION) ..."
  (cd "$ROOT" && FUTURE_VERSION="$DIFF_VERSION" \
     rustup run 1.97.0 cargo build -p cli-rust >/dev/null 2>&1) \
    || { err "Rust CLI build failed"; exit 2; }
  # Copy to an isolated dir WITHOUT a future-agent sibling (see TS_CLI).
  mkdir -p "$(dirname "$RUST_CLI")"
  cp "$RUST_CLI_BUILT" "$RUST_CLI"
}

# Both binaries must bake the pinned version; anything else means a stale or
# clobbered build (e.g. a concurrent gen-version with the leaked FUTURE_VERSION)
# and would fail every --version case with a misleading diff.
sanity_version() {
  local bin="$1" v
  v="$("$bin" --version 2>/dev/null | head -1)"
  if [[ "$v" != "future v$DIFF_VERSION" ]]; then
    err "$bin --version printed '$v' — expected 'future v$DIFF_VERSION'; stale/clobbered build?"
    exit 2
  fi
}

# ── scenario state (set up per group, torn down on group change) ───────────

current_scenario=""
case_home=""; case_path=""; case_grpc=""
MOCK_PID=""; MOCK2_PID=""; AGENT_PID=""
AGENT_BIN=""
AGENT_HOME=""

fake_home() {  # $1 = name -> echoes dir; creates ~/.future/agent + bin
  local dir="$WORK/homes/$1"
  mkdir -p "$dir/.future/agent" "$dir/.future/bin"
  echo "$dir"
}

write_auth() {  # $1 = home, $2 = json body
  printf '%s\n' "$2" > "$1/.future/agent/auth.json"
}

teardown_scenario() {
  local sc="$1"
  [[ -z "$sc" ]] && return 0
  if [[ -n "$AGENT_PID" ]]; then
    kill "$AGENT_PID" 2>/dev/null; wait "$AGENT_PID" 2>/dev/null
    AGENT_PID=""
  fi
  if [[ -n "$MOCK_PID" ]]; then kill "$MOCK_PID" 2>/dev/null; MOCK_PID=""; fi
  if [[ -n "$MOCK2_PID" ]]; then kill "$MOCK2_PID" 2>/dev/null; MOCK2_PID=""; fi
  info "--- teardown scenario: $sc ---"
}

start_mock() {  # $1 = port, $2 = mode -> echoes pid
  local port="$1" mode="${2:-ok}"
  # stdio redirected AWAY from the caller's pipe: a command substitution
  # waits for EOF on its pipe, so a daemon that inherits it would hang the
  # caller forever.
  python3 "$ROOT/cli/rust/tests/mock-platform-server.py" "$port" "$mode" </dev/null >/dev/null 2>&1 &
  echo $!
}

# ── scenario prep ──────────────────────────────────────────────────────────

prep_static() {
  case_home="$(fake_home static)"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc=""
}

prep_home_none() {  # no auth.json
  case_home="$(fake_home homenone)"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc=""
}

prep_home_auth() {  # auth.json with a future key; base_url used verbatim
  case_home="$(fake_home homeauth)"
  write_auth "$case_home" '{"future": {"type": "api_key", "key": "test-key-123", "base_url": "https://future-os.cn/api"}}'
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc=""
}

prep_home_badjson() {  # corrupt auth.json
  case_home="$(fake_home homebad)"
  printf 'not json{' > "$case_home/.future/agent/auth.json"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc=""
}

prep_http() {  # auth.json points base_url at the mock server
  case_home="$(fake_home http)"
  write_auth "$case_home" "{\"future\": {\"type\": \"api_key\", \"key\": \"test-key-123\", \"base_url\": \"http://127.0.0.1:$MOCK_PORT/api\"}}"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc=""
}

# Stateful cases: the auth file is re-created before EACH binary run so TS
# and Rust start from identical state (e.g. logout removes the key).
prep_logout() {
  case_home="$(fake_home logout)"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc=""
  write_auth "$case_home" '{"future": {"type": "api_key", "key": "test-key-123", "base_url": "https://future-os.cn/api"}}'
}

prep_http_errors() {  # auth.json points at the "errors" mock (401s)
  case_home="$(fake_home httperr)"
  write_auth "$case_home" "{\"future\": {\"type\": \"api_key\", \"key\": \"bad-key\", \"base_url\": \"http://127.0.0.1:$MOCK2_PORT/api\"}}"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc=""
}

prep_init_blocked() {  # .future/bin/future exists as a regular file
  case_home="$(fake_home initblocked)"
  write_auth "$case_home" "{\"future\": {\"type\": \"api_key\", \"key\": \"test-key-123\", \"base_url\": \"http://127.0.0.1:$MOCK_PORT/api\"}}"
  printf 'keep me\n' > "$case_home/.future/bin/future"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc=""
}

prep_init_linked() {  # .future/bin/future already a symlink to a THIRD path,
                      # so both CLIs exercise the re-link (idempotent) path
  case_home="$(fake_home initlinked)"
  write_auth "$case_home" "{\"future\": {\"type\": \"api_key\", \"key\": \"test-key-123\", \"base_url\": \"http://127.0.0.1:$MOCK_PORT/api\"}}"
  ln -s "$WORK/dummy-exe" "$case_home/.future/bin/future"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc=""
}

prep_agent() {  # fake home + session fixtures + REAL agent on a dedicated port
  AGENT_HOME="$(fake_home agent)"
  # A session fixture so list/info/rename/delete render real rows.
  mkdir -p "$AGENT_HOME/.future/agent/sessions"
  cat > "$AGENT_HOME/.future/agent/sessions/abc123.jsonl" <<'EOF'
{"id":"e1","type":"session_info","timestamp":"2026-08-06T13:00:00+08:00","content":{"cwd":"/Users/geilige/future-os","session_name":"Test Session","model":"future/fast"}}
{"id":"e2","type":"user","timestamp":"2026-08-06T13:01:00+08:00","content":[{"type":"text","text":"Hello there, agent! Please help me with the diff test."}]}
{"id":"e3","type":"assistant","timestamp":"2026-08-06T13:02:00+08:00","content":"Sure, here is the answer."}
EOF
  if [[ -z "$AGENT_BIN" ]]; then
    skipm "agent group: no future-agent binary found (set FUTURE_AGENT_BIN or build it)"
    skip=$((skip+1))
    current_scenario="agent-missing"
    return 0
  fi
  AGENT_PORT="$(free_port)"
  HOME="$AGENT_HOME" "$AGENT_BIN" --grpc-addr "127.0.0.1:$AGENT_PORT" >/dev/null 2>&1 &
  AGENT_PID=$!
  wait_port "$AGENT_PORT" || { err "future-agent did not come up on $AGENT_PORT"; exit 2; }
  case_home="$AGENT_HOME"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc="127.0.0.1:$AGENT_PORT"
  info "agent up on $case_grpc (pid $AGENT_PID)"
}

prep_doctor() {  # fake home + auth keys + fake bin dir + dead gRPC address
  case_home="$(fake_home doctor)"
  write_auth "$case_home" "{\"future\": {\"type\": \"api_key\", \"key\": \"test-key-123\", \"base_url\": \"http://127.0.0.1:$MOCK_PORT/api\"}, \"openai\": {\"key\": \"sk-fake-123\"}}"
  local fakebin="$case_home/fake-bin"
  mkdir -p "$fakebin"
  printf '#!/bin/sh\necho "future-agent v9.9.9-diff"\n' > "$fakebin/future-agent"
  printf '#!/bin/sh\necho "future v0.0.0-diff+local"\n' > "$fakebin/future"
  chmod +x "$fakebin/future-agent" "$fakebin/future"
  case_path="$fakebin:/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc="127.0.0.1:1"  # dead — doctor reports the agent as not running
}

prep_agentdown() {  # fake home; gRPC points at a dead port
  case_home="$(fake_home agentdown)"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc="127.0.0.1:1"
}

prep_skills() {  # mock mode "skills": small catalog + download zips
  case_home="$(fake_home skills)"
  write_auth "$case_home" "{\"future\": {\"type\": \"api_key\", \"key\": \"test-key-123\", \"base_url\": \"http://127.0.0.1:$MOCK_PORT/api\"}}"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc=""
}

# skills:installed — future-test-a@0.5.0 + community-x@0.9.0 pre-installed so
# the INSTALLED column and the update path render real data. Re-created fresh
# before EACH binary run (stateful scenario, like logout).
prep_skills_installed() {
  case_home="$(fake_home skillsinst)"
  write_auth "$case_home" "{\"future\": {\"type\": \"api_key\", \"key\": \"test-key-123\", \"base_url\": \"http://127.0.0.1:$MOCK_PORT/api\"}}"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc=""
}

# browser — mock CDP endpoint (mode "browser"): config points at the mock
# with browserKind "chromium" (exercises the /json/version refinement path)
# and refs pre-seeded (post-snapshot state so click/type --ref work).
prep_browser() {
  case_home="$(fake_home browser)"
  write_auth "$case_home" "{\"future\": {\"type\": \"api_key\", \"key\": \"test-key-123\", \"base_url\": \"http://127.0.0.1:$MOCK_PORT/api\"}}"
  case_path="/usr/bin:/bin:/usr/sbin:/sbin"
  case_grpc=""
  reset_browser_fixture
}

# Stateful fixture: config.json + mock tab state re-created fresh before each
# binary run so TS and Rust start from identical browser state.
reset_browser_fixture() {
  mkdir -p "$case_home/.future/agent/browser"
  cat > "$case_home/.future/agent/browser/config.json" <<EOF
{"version": 2, "connection": {"protocol": "cdp", "browserKind": "chromium", "endpoint": "http://127.0.0.1:$MOCK_PORT"}, "refs": {"b1": "#btn-submit", "i1": "input[data-testid='email']"}}
EOF
  if [[ -n "$MOCK_PORT" ]]; then
    curl -s -X POST "http://127.0.0.1:$MOCK_PORT/__reset" >/dev/null 2>&1 || true
  fi
}

reset_skills_installed_fixture() {
  rm -rf "$case_home/.future/agent/skills"
  mkdir -p "$case_home/.future/agent/skills/future-test-a" "$case_home/.future/agent/skills/community-x"
  cat > "$case_home/.future/agent/skills/future-test-a/SKILL.md" <<'EOF'
---
name: Test A
version: 0.5.0
---
# Test A skill
EOF
  cat > "$case_home/.future/agent/skills/community-x/SKILL.md" <<'EOF'
---
name: Community X
version: 0.9.0
---
# Community X skill
EOF
}

# ── case runner ────────────────────────────────────────────────────────────

export_case_env() {
  export HOME="$case_home"
  export PATH="$case_path"
  if [[ -n "$case_grpc" ]]; then
    export FUTURE_AGENT_GRPC_ADDR="$case_grpc"
  else
    unset FUTURE_AGENT_GRPC_ADDR
  fi
  unset HTTP_PROXY HTTPS_PROXY ALL_PROXY NO_PROXY
  unset http_proxy https_proxy all_proxy no_proxy
  unset FUTURE_VERSION
}

# run_case <scenario> <mode> <argv...>
run_case() {
  local scenario="$1" mode="$2"; shift 2
  if [[ "$scenario" == "agent-missing" ]]; then
    return 0  # group was skipped at prep time
  fi

  export_case_env
  local ts_out ts_err ts_code ru_out ru_err ru_code
  ts_out="$(mktemp "$WORK/ts-out.XXXXXX")"; ts_err="$(mktemp "$WORK/ts-err.XXXXXX")"
  ru_out="$(mktemp "$WORK/ru-out.XXXXXX")"; ru_err="$(mktemp "$WORK/ru-err.XXXXXX")"

  # Per-binary state refresh: give each binary an identical starting point.
  if [[ "$scenario" == "init:linked" ]]; then
    ln -sf "$TS_CLI" "$case_home/.future/bin/future"
  elif [[ "$scenario" == "logout" ]]; then
    write_auth "$case_home" '{"future": {"type": "api_key", "key": "test-key-123", "base_url": "https://future-os.cn/api"}}'
  elif [[ "$scenario" == "skills" ]]; then
    rm -rf "$case_home/.future/agent/skills"
  elif [[ "$scenario" == "skills:installed" ]]; then
    reset_skills_installed_fixture
  elif [[ "$scenario" == "browser" ]]; then
    reset_browser_fixture
  fi
  ts_code=$("$TS_CLI" "$@" >"$ts_out" 2>"$ts_err"; echo $?)
  if [[ "$scenario" == "init:linked" ]]; then
    ln -sf "$RUST_CLI" "$case_home/.future/bin/future"
  elif [[ "$scenario" == "logout" ]]; then
    write_auth "$case_home" '{"future": {"type": "api_key", "key": "test-key-123", "base_url": "https://future-os.cn/api"}}'
  elif [[ "$scenario" == "skills" ]]; then
    rm -rf "$case_home/.future/agent/skills"
  elif [[ "$scenario" == "skills:installed" ]]; then
    reset_skills_installed_fixture
  elif [[ "$scenario" == "browser" ]]; then
    reset_browser_fixture
  fi
  ru_code=$("$RUST_CLI" "$@" >"$ru_out" 2>"$ru_err"; echo $?)

  local label="$*"
  [[ -z "$label" ]] && label="(no args)"

  case "$mode" in
    exact)
      if [[ "$ts_code" == "$ru_code" ]] && cmp -s "$ts_out" "$ru_out" && cmp -s "$ts_err" "$ru_err"; then
        pass=$((pass+1)); ok "[$scenario] $label"
      else
        fail=$((fail+1)); bad "[$scenario] $label"
        FAILED_CASES+=("[$scenario] $label")
        show_diff "$ts_code" "$ru_code" "$ts_out" "$ru_out" "$ts_err" "$ru_err"
      fi
      ;;
    agentdown)
      # Transport-error text differs by network stack; compare exit code,
      # stdout bytes, and the stderr PREFIX up to the first colon (e.g.
      # "Error: " vs "Error: 14 UNAVAILABLE: ...").
      local ts_prefix ru_prefix
      ts_prefix="$(sed -n 's/\(^[^:]*:\).*/\1/p' "$ts_err" | head -1)"
      ru_prefix="$(sed -n 's/\(^[^:]*:\).*/\1/p' "$ru_err" | head -1)"
      if [[ "$ts_code" == "$ru_code" ]] && cmp -s "$ts_out" "$ru_out" \
         && [[ -n "$ts_prefix" && "$ts_prefix" == "$ru_prefix" ]]; then
        pass=$((pass+1)); ok "[$scenario] $label (agentdown prefix match)"
      else
        fail=$((fail+1)); bad "[$scenario] $label (agentdown)"
        FAILED_CASES+=("[$scenario] $label")
        show_diff "$ts_code" "$ru_code" "$ts_out" "$ru_out" "$ts_err" "$ru_err"
      fi
      ;;
  esac

  if [[ "$VERBOSE" == 1 ]]; then
    info "    TS exit=$ts_code  Rust exit=$ru_code"
  fi
  rm -f "$ts_out" "$ts_err" "$ru_out" "$ru_err"
}

show_diff() {
  local ts_code="$1" ru_code="$2" ts_out="$3" ru_out="$4" ts_err="$5" ru_err="$6"
  if [[ "$ts_code" != "$ru_code" ]]; then
    info "    exit: TS=$ts_code Rust=$ru_code"
  fi
  if ! cmp -s "$ts_out" "$ru_out"; then
    info "    --- stdout diff (TS < / Rust >) ---"
    diff "$ts_out" "$ru_out" 2>/dev/null | head -20 | sed 's/^/    /'
  fi
  if ! cmp -s "$ts_err" "$ru_err"; then
    info "    --- stderr diff (TS < / Rust >) ---"
    diff "$ts_err" "$ru_err" 2>/dev/null | head -20 | sed 's/^/    /'
  fi
}

# ── corpus ─────────────────────────────────────────────────────────────────

build_ts
build_rust
sanity_version "$TS_CLI"
sanity_version "$RUST_CLI"

MOCK_PORT="$(free_port)"
MOCK2_PORT="$(free_port)"
if [[ -n "${FUTURE_AGENT_BIN:-}" ]]; then
  AGENT_BIN="$FUTURE_AGENT_BIN"
elif [[ -x "$ROOT/target/debug/future-agent" ]]; then
  AGENT_BIN="$ROOT/target/debug/future-agent"
elif [[ -x "$ROOT/target/release/future-agent" ]]; then
  AGENT_BIN="$ROOT/target/release/future-agent"
fi

# add_case <scenario> <mode> <argv...>
add_case() {
  local scenario="$1" mode="$2"; shift 2
  # agent group without a binary was already skipped; do not re-prep.
  if [[ "$scenario" == "agent" && -z "$AGENT_BIN" ]]; then
    return 0
  fi
  if [[ "$scenario" != "$current_scenario" ]]; then
    teardown_scenario "$current_scenario"
    current_scenario="$scenario"
    info "--- scenario: $scenario ---"
    case "$scenario" in
      static)        prep_static ;;
      home:none)     prep_home_none ;;
      home:auth)     prep_home_auth ;;
      home:badjson)  prep_home_badjson ;;
      http)          prep_http ;;
      logout)        prep_logout ;;
      http:errors)   prep_http_errors ;;
      init:blocked)  prep_init_blocked ;;
      init:linked)   prep_init_linked ;;
      agent)         prep_agent ;;
      doctor)        prep_doctor ;;
      agentdown)     prep_agentdown ;;
      skills)        prep_skills ;;
      skills:installed) prep_skills_installed ;;
      browser)       prep_browser ;;
      *) err "unknown scenario $scenario"; exit 2 ;;
    esac
    case "$scenario" in
      http | init:blocked | init:linked | doctor)
        MOCK_PID=$(start_mock "$MOCK_PORT" ok)
        wait_port "$MOCK_PORT" || { err "mock server on $MOCK_PORT did not start"; exit 2; } ;;
      http:errors)
        MOCK2_PID=$(start_mock "$MOCK2_PORT" errors)
        wait_port "$MOCK2_PORT" || { err "mock server on $MOCK2_PORT did not start"; exit 2; } ;;
      skills | skills:installed)
        MOCK_PID=$(start_mock "$MOCK_PORT" skills)
        wait_port "$MOCK_PORT" || { err "mock server on $MOCK_PORT did not start"; exit 2; } ;;
      browser)
        MOCK_PID=$(start_mock "$MOCK_PORT" browser)
        wait_port "$MOCK_PORT" || { err "mock server on $MOCK_PORT did not start"; exit 2; }
        # The CDP WebSocket server binds its own ephemeral port; discover it
        # from /json/version and wait for it to accept connections.
        WS_URL=""
        for _ in $(seq 1 60); do
          WS_URL=$(curl -s "http://127.0.0.1:$MOCK_PORT/json/version" 2>/dev/null | python3 -c 'import sys,json; print(json.load(sys.stdin).get("webSocketDebuggerUrl",""))' 2>/dev/null)
          if [[ -n "$WS_URL" ]]; then break; fi
          sleep 0.25
        done
        WS_PORT=$(python3 -c 'import re,sys; m=re.search(r":(\d+)/", sys.argv[1]); print(m.group(1) if m else "")' "$WS_URL" 2>/dev/null)
        if [[ -z "$WS_PORT" ]]; then
          err "mock CDP ws url missing from /json/version"; exit 2
        fi
        wait_port "$WS_PORT" || { err "mock CDP ws on $WS_PORT did not start"; exit 2; }
        reset_browser_fixture ;;
    esac
  fi
  run_case "$scenario" "$mode" "$@"
}

# static — no environment dependency
add_case static exact --version
add_case static exact -v
add_case static exact version
add_case static exact --help
add_case static exact -h
add_case static exact help
add_case static exact bogus
add_case static exact bogus sub
add_case static exact ""            # no args -> main help
add_case static exact init --help
add_case static exact init -h
add_case static exact init foo
add_case static exact auth
add_case static exact auth --help
add_case static exact auth -h
add_case static exact auth login --help
add_case static exact auth login -h
add_case static exact auth status --help
add_case static exact auth credential --help
add_case static exact auth logout --help
add_case static exact auth bogus
add_case static exact tools
add_case static exact tools --help
add_case static exact tools -h
add_case static exact tools bogus
add_case static exact skills
add_case static exact skills --help
add_case static exact skills -h
add_case static exact skills bogus
add_case static exact account
add_case static exact account --help
add_case static exact account -h
add_case static exact account bogus
add_case static exact models --help
add_case static exact models -h
add_case static exact models --json --help
add_case static exact agent status --help
add_case static exact agent status -h
add_case static exact agent --help
add_case static exact agent -h
add_case static exact agent --json
add_case static exact agent bogus
add_case static exact session
add_case static exact session --help
add_case static exact session -h
add_case static exact session bogus
add_case static exact session bogus sess-1

# run — local-only paths (no gRPC reachable in the static scenario)
add_case static exact run --help
add_case static exact run -h
add_case static exact run                 # no prompt -> exit 1
add_case static exact run --thinking bogus hi
add_case static exact run --mode xml hi
add_case static exact run --permission everywhere hi
add_case static exact run --bogus-flag hi
add_case static exact run @/nonexistent/file-xyz hi

# home:no auth.json — pure local behavior, no network
add_case home:none exact auth status
add_case home:none exact auth credential
add_case home:none exact auth credential --json
add_case home:none exact auth logout
add_case home:none exact account profile
add_case home:none exact account balance
add_case home:none exact account balance --json

# home:auth — auth.json present with key
add_case home:auth exact auth status
add_case home:auth exact auth credential
add_case home:auth exact auth credential --json

# logout is stateful (removes the key) — fresh auth file per binary run
add_case logout exact auth logout

# home:badjson — corrupt auth.json falls back to "Not logged in."
add_case home:badjson exact auth status
add_case home:badjson exact auth credential

# http — mock platform server (happy paths + error bodies)
add_case http exact account profile
add_case http exact account profile --json
add_case http exact account balance
add_case http exact account balance --json
add_case http exact auth login --url http://127.0.0.1:$MOCK_PORT
add_case http exact auth login --url=http://127.0.0.1:$MOCK_PORT

# http — tools list/describe/call against the mock MCP server
add_case http exact tools list
add_case http exact tools list --json
add_case http exact tools describe search_paper
add_case http exact tools describe browser
add_case http exact tools describe image_edit
add_case http exact tools describe mock_special
add_case http exact tools describe nope
add_case http exact tools describe
add_case http exact tools describe --help
add_case http exact tools call --help
add_case http exact tools call search_paper --queries '["x"]'
add_case http exact tools call web_search --query test
add_case http exact tools call mock_special --foo bar
add_case http exact tools call mock_error --x 1
add_case http exact tools call browser
add_case http exact tools call search_paper
add_case http exact tools call search_paper --queries notarray
add_case http exact tools call search_paper --queries '["a"]' --n 99
add_case http exact tools call web_search --query x --timeout -5

# http:errors — account endpoints answer 401 {"error": "bad key"}
add_case http:errors exact account profile
add_case http:errors exact account balance --json

# init — empty skills catalog via mock; then idempotent and blocked runs
add_case http exact init
add_case init:linked exact init
add_case init:blocked exact init

# agent — real future-agent on a dedicated port; ordered stateful cases
add_case agent exact agent status
add_case agent exact agent status --json
add_case agent exact models
add_case agent exact models --json
add_case agent exact session
add_case agent exact session list
add_case agent exact session list --json
add_case agent exact session info abc123
add_case agent exact session info nope
add_case agent exact session info
add_case agent exact session rename abc123 New Name
add_case agent exact session rename nope New Name
add_case agent exact session rename abc123
add_case agent exact session delete nope
add_case agent exact session delete abc123
add_case agent exact session list
add_case agent exact session list --json

# doctor — fully controlled environment
add_case doctor exact doctor

# skills — catalog with two future-* builtins + one community skill; the
# skills dir is reset before each binary run so every case starts clean
add_case skills exact skills list
add_case skills exact skills install future-test-a
add_case skills exact skills install nonexistent
add_case skills exact skills install future-test-b --version v2.1.0
add_case skills exact skills install
add_case skills exact skills install-builtin
add_case skills exact skills uninstall community-x
add_case skills exact skills uninstall nope
add_case skills exact skills update

# skills:installed — fixture (future-test-a@0.5.0 + community-x@0.9.0)
# re-created before each binary run
add_case skills:installed exact skills list
add_case skills:installed exact skills update
add_case skills:installed exact skills install-builtin
add_case skills:installed exact skills uninstall community-x

# agentdown — smoke test: dead gRPC port, normalized comparison
add_case agentdown agentdown agent status
add_case agentdown agentdown run "hello agent"

# browser — mock CDP endpoint (mode "browser") with pre-seeded config + refs.
# Every case resets config.json and the mock tab state per binary run.
add_case browser exact tools call browser --command status
add_case browser exact tools call browser --command status --endpoint http://127.0.0.1:1
add_case browser exact tools call browser --command start --port $MOCK_PORT
add_case browser exact tools call browser --command tabs
add_case browser exact tools call browser --command tabs --action list
add_case browser exact tools call browser --command tabs --action bogus
add_case browser exact tools call browser --command tabs --action select
add_case browser exact tools call browser --command tabs --action select --index 1
add_case browser exact tools call browser --command tabs --action new --url https://example.com/new-tab
add_case browser exact tools call browser --command tabs --action close --index 0
add_case browser exact tools call browser --command open
add_case browser exact tools call browser --command open --url https://example.com/open-page
add_case browser exact tools call browser --command snapshot
add_case browser exact tools call browser --command snapshot --limit 2
add_case browser exact tools call browser --command click
add_case browser exact tools call browser --command click --ref b1
add_case browser exact tools call browser --command click --selector "#btn-submit"
add_case browser exact tools call browser --command type --text hello
add_case browser exact tools call browser --command type --ref i1 --text hello
add_case browser exact tools call browser --command type --ref i1 --text "hello world" --clear false
add_case browser exact tools call browser --command press
add_case browser exact tools call browser --command press --key Enter
add_case browser exact tools call browser --command press --key Control+A
add_case browser exact tools call browser --command press --key F23
add_case browser exact tools call browser --command screenshot --path $WORK/browser-shot.png
add_case browser exact tools call browser --command scroll
add_case browser exact tools call browser --command scroll --direction up --amount 100
add_case browser exact tools call browser --command scroll --ref t1 --direction left --amount 50
add_case browser exact tools call browser --command console
add_case browser exact tools call browser --command console --level warn
add_case browser exact tools call browser --command console --level nope
add_case browser exact tools call browser --command bogus-command

# ── summary ────────────────────────────────────────────────────────────────

teardown_scenario "$current_scenario"
current_scenario=""

echo
echo "=============================================="
echo "  TS vs Rust diff:  $pass passed, $fail failed, $skip skipped"
echo "=============================================="
if [[ "$fail" -gt 0 ]]; then
  echo "Failed cases:"
  for c in "${FAILED_CASES[@]}"; do echo "  - $c"; done
  exit 1
fi
exit 0
