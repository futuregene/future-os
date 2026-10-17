#!/usr/bin/env bash
# flake-rate.sh — measure how often a test command fails, and which tests.
#
# Usage:
#   .future/cov100/flake-rate.sh --label <name> --runs 30 [--timeout 900] \
#       [--logs DIR] -- <command...>
#
# Prints one line per run and a summary: runs, failures, timeout (deadlock)
# count, and every distinct failing test with how many runs it appeared in.
# A fix is only accepted with a before *and* after rate from this tool (or an
# equivalent count) recorded in the module record.
#
# Exit status: 0 always (the summary is the result); a nonzero exit means the
# command itself could not be started.
set -uo pipefail

label=""; runs=10; tmo=900; logs=""
while [ $# -gt 0 ]; do
  case "$1" in
    --label)   label="$2"; shift 2 ;;
    --runs)    runs="$2"; shift 2 ;;
    --timeout) tmo="$2"; shift 2 ;;
    --logs)    logs="$2"; shift 2 ;;
    --)        shift; break ;;
    *)         break ;;
  esac
done
[ -n "$label" ] || { echo "usage: flake-rate.sh --label NAME --runs N -- cmd..." >&2; exit 2; }
[ $# -gt 0 ] || { echo "no command given" >&2; exit 2; }
[ -n "$logs" ] || logs="${TMPDIR:-/tmp}/flake-$label"
mkdir -p "$logs"

fails=0; timeouts=0
: > "$logs/summary.txt"
for i in $(seq 1 "$runs"); do
  log="$logs/$label-$i.log"
  timeout "$tmo" "$@" > "$log" 2>&1
  rc=$?
  status="pass"
  if [ "$rc" -eq 124 ]; then status="TIMEOUT"; timeouts=$((timeouts+1))
  elif [ "$rc" -ne 0 ]; then status="FAIL(rc=$rc)"; fails=$((fails+1)); fi
  printf '%-4s run %3d/%d  %s\n' "$label" "$i" "$runs" "$status"
  printf 'run %d rc=%d %s\n' "$i" "$rc" "$status" >> "$logs/summary.txt"
  if [ "$status" != "pass" ]; then
    grep -E '^test .* FAILED|^failures:$' -A 40 "$log" 2>/dev/null \
      | grep -E '^    [A-Za-z_:]' >> "$logs/failing-tests.txt" 2>/dev/null
    grep -E 'panicked at|assertion .* failed|^thread .* panicked' "$log" 2>/dev/null \
      | head -5 >> "$logs/panic-lines.txt"
  fi
done

echo
echo "=== flake-rate [$label] ==="
echo "runs=$runs failures=$fails timeouts=$timeouts"
if [ -s "$logs/failing-tests.txt" ]; then
  echo "distinct failing tests (appearances):"
  sort "$logs/failing-tests.txt" | sed 's/^ *//' | uniq -c | sort -rn
fi
if [ -s "$logs/panic-lines.txt" ]; then
  echo "first panic line per failing run (sample):"
  sort -u "$logs/panic-lines.txt" | head -10
fi
echo "logs: $logs"
