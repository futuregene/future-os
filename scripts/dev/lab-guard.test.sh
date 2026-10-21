#!/usr/bin/env bash
# Regression tests for scripts/dev/lab-guard.sh.
#
# The guard exists because a launch that forgot to export FUTURE_HOME still ran,
# and only the desktop's own lock stopped it from opening the real ~/.future.
# These cases pin every gate, including the ones that only matter when something
# is *not* allowed to happen: a refusal is invisible unless something asserts it.
set -u

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
GUARD=$HERE/lab-guard.sh
failures=0
cases=0

report() { printf '  %-6s %s\n' "$1" "$2"; }
ok() { cases=$((cases + 1)); report ok "$1"; }
bad() {
  cases=$((cases + 1))
  failures=$((failures + 1))
  report FAIL "$1"
}

# check_value <expected-exit> <value> <label>
check_value() {
  local expected=$1 value=$2 label=$3 output status
  output=$("$GUARD" check "$value" 2>&1)
  status=$?
  if [ "$status" -eq "$expected" ]; then
    ok "$label (exit $status)"
  else
    bad "$label: expected exit $expected, got $status: $output"
  fi
}

REAL_FUTURE="${HOME}/.future"

# 1. Unset and empty are the accident itself: the app would fall back to ~/.future.
check_value 1 "" "empty FUTURE_HOME is refused"
check_value 1 "$REAL_FUTURE" "the real data root is refused"
check_value 1 "$HOME" "the real home is refused"

# 2. Non-absolute values are ignored by the app, so they are refused outright.
check_value 1 "lab/.future" "a relative path is refused"
check_value 1 "~/lab/.future" "a ~ path expands into the real home and is refused"

# 3. The whitelist, resolved: /tmp, $TMPDIR and their /private aliases.
check_value 0 "/tmp/lab-guard-test/.future" "a /tmp lab is allowed"
check_value 0 "${TMPDIR%/}/lab-guard-test/.future" "a \$TMPDIR lab is allowed"
check_value 0 "/private/var/folders/ab/lab/.future" "a /private/var/folders lab is allowed"
check_value 1 "/tmp" "the bare /tmp root is refused"
check_value 1 "/tmpfoo/.future" "a /tmp prefix look-alike is refused"

# 4. Symlinks are resolved before the comparison.
link=/tmp/lab-guard-escape-link
ln -sfn "$REAL_FUTURE" "$link"
check_value 1 "$link" "a symlink into the real data root is refused"
ln -sfn "$HOME" /tmp/lab-guard-escape-home
check_value 1 "/tmp/lab-guard-escape-home/.future" "a symlinked real home is refused"
rm -f "$link" /tmp/lab-guard-escape-home

# 5. A lab root that is itself a .future directory is a mistake worth naming.
root=/tmp/lab-guard-run
rm -rf "$root"
"$GUARD" run "$REAL_FUTURE" -- true >/dev/null 2>&1
[ $? -eq 2 ] && ok "run refuses a .future directory as the lab root" ||
  bad "run should refuse .future as the lab root"

# 6. run exports an isolated HOME/FUTURE_HOME and clears endpoint overrides,
#    because an inherited override points a "lab" run at the real Agent.
export FUTURE_AGENT_GRPC_ADDR=127.0.0.1:59999 FUTURE_AGENT_SOCKET=/tmp/lab-guard-unused.sock
out=$("$GUARD" run "$root" -- sh -c 'printf "%s|%s|%s|%s" "$HOME" "$FUTURE_HOME" "${FUTURE_AGENT_GRPC_ADDR:-cleared}" "${FUTURE_AGENT_SOCKET:-cleared}"' 2>/dev/null)
unset FUTURE_AGENT_GRPC_ADDR FUTURE_AGENT_SOCKET
expected="$root|$root/.future|cleared|cleared"
[ "$out" = "$expected" ] && ok "run isolates HOME/FUTURE_HOME and clears endpoint overrides" ||
  bad "run environment: expected '$expected', got '$out'"

# 7. kill: a supervisor and its child are both signalled. The child holds no file
#    under the lab, so only parentage finds it; SIGKILL alone would orphan it.
rm -rf "$root"
"$GUARD" run "$root" -- sleep 120 >/dev/null 2>&1 &
sleep 2
pids=$("$GUARD" pids "$root" 2>/dev/null | grep -c '^[0-9]')
[ "$pids" -ge 2 ] && ok "pids reports the supervisor and its child ($pids)" ||
  bad "pids should report at least 2 processes, got $pids"
"$GUARD" kill "$root" >/dev/null 2>&1
sleep 1
leftover=$(pgrep -f "sleep 120" | wc -l | tr -d ' ')
[ "$leftover" = "0" ] && ok "kill leaves no orphan behind" ||
  bad "kill left $leftover process(es) running"

# 8. kill refuses a pid that holds anything under the real data root, even when
#    its command line names the lab root.
rm -rf "$root"
mkdir -p "$root/.future"
touch "$root/.future/marker"
trick=$(mktemp /tmp/lab-guard-trick.XXXXXX)
cat >"$trick" <<EOF
#!/bin/bash
exec -a "lab-guard __supervise $root -- future agent" \\
  sleep 30 3<$root/.future/marker 4<"$REAL_FUTURE/agent/agent-instance.lock"
EOF
chmod +x "$trick"
"$trick" &
victim=$!
# Job-control notices from the signal under test are not this test's output.
disown "$victim" 2>/dev/null || true
sleep 1
"$GUARD" kill "$root" >/dev/null 2>&1
if kill -0 "$victim" 2>/dev/null; then
  ok "kill refuses a pid holding the real data root"
else
  bad "kill signalled a pid that holds the real data root"
fi
kill -9 "$victim" 2>/dev/null
wait "$victim" 2>/dev/null
rm -f "$trick"
rm -rf "$root"

printf '\n%s case(s), %s failure(s)\n' "$cases" "$failures"
[ "$failures" -eq 0 ]
