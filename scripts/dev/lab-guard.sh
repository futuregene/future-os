#!/usr/bin/env bash
# Launch an isolated Agent experiment, or refuse to.
#
# A lab run must be *provably* unable to reach the developer's real data root.
# The accident this exists for: a launch that forgot to export `FUTURE_HOME`
# still ran, and only the desktop's own lock stopped it from opening the real
# `~/.future`. Luck is not a design, so every gate below fails closed.
#
# Gates, in order:
#
#   1. `FUTURE_HOME` must be SET and NON-EMPTY. Unset or empty means the app
#      falls back to `~/.future` — the exact case that slipped through.
#   2. It must be absolute. A relative or `~`-prefixed value is *ignored* by the
#      app (it requires an absolute root), so it silently falls back too.
#   3. `~` is expanded and symlinks are resolved (`os.path.realpath`) BEFORE the
#      whitelist comparison, so `/tmp/link -> ~/.future` cannot sneak past.
#   4. The resolved path must sit under `/tmp`, `$TMPDIR`, or
#      `/private/var/folders`. On macOS `$TMPDIR` lives under `/var/folders/…`,
#      so a `/tmp`-only rule would reject perfectly good lab roots. Every root
#      is resolved the same way the candidate is, and the path must be *under*
#      a root (the bare root itself is refused: it is shared, not a lab).
#
# And for cleanup:
#
#   5. `kill` never trusts a column position. It takes candidate pids from
#      `lsof -Fp` (files open under the lab), keeps only those whose process
#      command line names this guard and this exact lab root, and refuses any
#      pid that holds anything under the real data root. Anything it skips is
#      printed with its reason.
#
# This script is never on a real entry point's path: `future headless`,
# `future agent` and `future tui` do not read it. Only a lab script opts in.
#
#   lab-guard.sh check <FUTURE_HOME>
#   lab-guard.sh run   <lab-root> -- <command> [args...]
#   lab-guard.sh pids  <lab-root>
#   lab-guard.sh kill  <lab-root> [--dry-run]
#
# `run` sets HOME/USERPROFILE to <lab-root>, FUTURE_HOME to <lab-root>/.future,
# and clears FUTURE_AGENT_GRPC_ADDR / FUTURE_AGENT_SOCKET — an inherited TCP
# address or socket override would point a "lab" run straight at the real
# Agent, which is the same class of accident as an unset home.

set -u

SELF=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/$(basename -- "${BASH_SOURCE[0]}")

die() { printf 'lab-guard: %s\n' "$*" >&2; exit 2; }

usage() {
  sed -n '3,48p' "$SELF" | sed 's/^# \{0,1\}//'
}

# Resolve one FUTURE_HOME value and answer OK/NO with the canonical path.
# Exit status: 0 allowed, 1 refused.
resolve_home() {
  python3 - "$1" <<'PY'
import os
import sys

raw = sys.argv[1] if len(sys.argv) > 1 else ""


def answer(ok, text):
    print(("OK " if ok else "NO ") + text)
    sys.exit(0 if ok else 1)


if raw.strip() == "":
    answer(False, "FUTURE_HOME is unset or empty; the app would fall back to ~/.future")

expanded = os.path.expanduser(raw)
if not os.path.isabs(expanded):
    answer(
        False,
        "FUTURE_HOME=%r is not absolute; the app ignores it and falls back to ~/.future" % raw,
    )

# Symlinks resolved here, before any comparison: a link into the real home must
# be seen as the real home.
canonical = os.path.realpath(expanded)

roots = []
for candidate in ("/tmp", os.environ.get("TMPDIR"), "/private/var/folders", "/var/folders"):
    if not candidate:
        continue
    resolved = os.path.realpath(os.path.expanduser(candidate))
    if resolved not in roots:
        roots.append(resolved)

for root in roots:
    if canonical == root:
        answer(False, "%s is the bare temp root itself, not a lab directory under it" % canonical)
    if canonical.startswith(root.rstrip(os.sep) + os.sep):
        answer(True, canonical)

answer(False, "%s is outside the isolation whitelist (%s)" % (canonical, ", ".join(roots)))
PY
}

check_home() {
  local value=$1 result
  result=$(resolve_home "$value") || {
    printf 'lab-guard: refused: %s\n' "${result#NO }" >&2
    return 1
  }
  printf '%s\n' "${result#OK }"
}

# Absolute path of the real data root this guard must never touch.
real_future_root() {
  local home=${HOME:-}
  [ -n "$home" ] || return 1
  python3 -c 'import os,sys; print(os.path.realpath(os.path.expanduser(sys.argv[1])))' "$home/.future"
}

# Does this pid hold any file under the real data root?
touches_real_root() {
  local pid=$1 real
  real=$(real_future_root) || return 1
  lsof -p "$pid" 2>/dev/null | grep -qF -- "$real"
}

cmd_check() {
  [ $# -eq 1 ] || die "check takes exactly one FUTURE_HOME value"
  local resolved
  resolved=$(check_home "$1") || exit 1
  printf 'lab-guard: ok: %s\n' "$resolved"
}

cmd_run() {
  [ $# -ge 3 ] || die "run needs <lab-root> -- <command>"
  local root=$1
  shift
  [ "$1" = "--" ] || die "run needs -- before the command"
  shift
  # Re-exec as the supervisor so this process's command line carries the lab
  # root; that is what `pids`/`kill` verify before signalling anything.
  exec "$SELF" __supervise "$root" -- "$@"
}

cmd_supervise() {
  [ $# -ge 3 ] || die "__supervise needs <lab-root> -- <command>"
  local root=$1
  shift
  [ "$1" = "--" ] || die "__supervise needs -- before the command"
  shift
  # A lab root that is itself a `.future` directory would be nested twice
  # (`<root>/.future/.future`); naming it is a mistake worth refusing plainly.
  case $root in
    */.future) die "pass the lab root, not its .future directory (got $root)" ;;
  esac
  local future="$root/.future"
  check_home "$future" >/dev/null || exit 1
  mkdir -p "$future/agent" || die "cannot create $future"
  chmod 700 "$root" 2>/dev/null || true

  # An inherited endpoint override points the "lab" run at the real Agent.
  unset FUTURE_AGENT_GRPC_ADDR FUTURE_AGENT_SOCKET DESKTOP_AGENT_MODE
  export HOME="$root" USERPROFILE="$root" FUTURE_HOME="$future"
  printf 'lab-guard: %s (HOME=%s FUTURE_HOME=%s)\n' "$*" "$root" "$future" >&2

  "$@" &
  local child=$!
  trap 'kill -TERM "$child" 2>/dev/null' TERM INT HUP
  wait "$child"
  local status=$?
  # 128+N means the wait was interrupted by signal N; report the child's fate.
  if [ "$status" -gt 128 ]; then
    kill -TERM "$child" 2>/dev/null
    wait "$child" 2>/dev/null
  fi
  exit "$status"
}

# Process command lines that name this guard AND this exact lab root.
supervisor_pids() {
  local root=$1 marker
  marker="__supervise $root --"
  ps -Ao pid=,command= | while read -r pid command; do
    case $command in
      *lab-guard.sh*"$marker"*) printf '%s\n' "$pid" ;;
    esac
  done
}

# Every descendant of a pid, deepest last. A lab child need not hold any file
# under the lab (a sleep, an in-flight model call), so parentage is the only
# thing that finds it.
descendants() {
  local parent=$1 child
  for child in $(pgrep -P "$parent" 2>/dev/null); do
    printf '%s\n' "$child"
    descendants "$child"
  done
}

# Every pid belonging to the lab: each verified supervisor, its descendants, and
# anything still holding a file under the lab directory (a child that outlived
# its supervisor). Excludes this shell and its parent.
lab_pids() {
  local root=$1 pid
  {
    for pid in $(supervisor_pids "$root"); do
      printf '%s\n' "$pid"
      descendants "$pid"
    done
    # `lsof <dir>` matches only processes holding that directory itself, so a
    # recursive scan is what finds a child that opened a file inside the lab.
    # Labs are small trees; this stays cheap.
    lsof +D "$root" -Fp 2>/dev/null | sed -n 's/^p//p'
  } | sort -u | while read -r pid; do
    [ -n "$pid" ] || continue
    if [ "$pid" = "$$" ] || [ "$pid" = "$PPID" ]; then
      continue
    fi
    printf '%s\n' "$pid"
  done
}

cmd_pids() {
  [ $# -eq 1 ] || die "pids takes exactly one lab root"
  local root=$1 future pid command
  future=$(check_home "$root/.future") >/dev/null || exit 1
  local found=0
  for pid in $(lab_pids "$root"); do
    command=$(ps -o command= -p "$pid" 2>/dev/null) || continue
    if touches_real_root "$pid"; then
      printf '%s (SKIP: holds files under the real data root)\n' "$pid"
      continue
    fi
    printf '%s %s\n' "$pid" "$command"
    found=$((found + 1))
  done
  printf '# %s verified pid(s) for %s\n' "$found" "$future" >&2
}

# Signal the verified set, TERM first so a supervisor's trap can forward it to
# its child, then KILL whatever is left after a short grace period.
signal_set() {
  local signal=$1
  shift
  local pid
  for pid in "$@"; do
    kill "-$signal" "$pid" 2>/dev/null || true
  done
}

cmd_kill() {
  [ $# -ge 1 ] || die "kill needs <lab-root> [--dry-run]"
  local root=$1 dry_run=0
  [ "${2:-}" = "--dry-run" ] && dry_run=1
  # Refuse to operate at all unless the lab root itself is isolated.
  check_home "$root/.future" >/dev/null || exit 1

  # A supervisor is identified by its command line naming this exact root; a
  # descendant inherits that provenance through parentage.
  local supervisors verified=() pid command
  supervisors=$(supervisor_pids "$root")
  for pid in $(lab_pids "$root"); do
    command=$(ps -o command= -p "$pid" 2>/dev/null) || continue
    if touches_real_root "$pid"; then
      printf 'lab-guard: skip %s: it holds files under the real data root\n' "$pid" >&2
      continue
    fi
    # Provenance: either the command line names the root, or the pid is a
    # descendant of a supervisor that does.
    case $command in
      *"$root"*) verified+=("$pid") ;;
      *)
        local proven=0 parent
        parent=$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ')
        while [ -n "${parent:-}" ] && [ "$parent" -gt 1 ] 2>/dev/null; do
          case " $supervisors " in
            *" $parent "*) proven=1; break ;;
          esac
          parent=$(ps -o ppid= -p "$parent" 2>/dev/null | tr -d ' ')
        done
        if [ "$proven" -eq 1 ]; then
          verified+=("$pid")
        else
          printf 'lab-guard: skip %s: neither its command line nor its parentage names %s\n' \
            "$pid" "$root" >&2
        fi
        ;;
    esac
  done

  if [ ${#verified[@]} -eq 0 ]; then
    printf 'lab-guard: nothing to kill for %s\n' "$root" >&2
    return 0
  fi
  if [ "$dry_run" -eq 1 ]; then
    for pid in "${verified[@]}"; do
      printf 'would signal %s %s\n' "$pid" "$(ps -o command= -p "$pid" 2>/dev/null)"
    done
    printf 'lab-guard: would-kill %s process(es) for %s\n' "${#verified[@]}" "$root" >&2
    return 0
  fi

  signal_set TERM "${verified[@]}"
  local deadline=$((SECONDS + 3)) leftover=()
  while [ "$SECONDS" -lt "$deadline" ]; do
    leftover=()
    for pid in "${verified[@]}"; do
      kill -0 "$pid" 2>/dev/null && leftover+=("$pid")
    done
    [ ${#leftover[@]} -eq 0 ] && break
    sleep 0.2
  done
  if [ ${#leftover[@]} -gt 0 ]; then
    signal_set KILL "${leftover[@]}"
    printf 'lab-guard: escalated to KILL for %s process(es)\n' "${#leftover[@]}" >&2
  fi
  printf 'lab-guard: killed %s process(es) for %s\n' "${#verified[@]}" "$root" >&2
}

case ${1:-} in
  check) shift; cmd_check "$@" ;;
  run) shift; cmd_run "$@" ;;
  __supervise) shift; cmd_supervise "$@" ;;
  pids) shift; cmd_pids "$@" ;;
  kill) shift; cmd_kill "$@" ;;
  -h | --help | help | "") usage ;;
  *) die "unknown command: $1 (try --help)" ;;
esac
