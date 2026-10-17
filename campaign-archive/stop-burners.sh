#!/usr/bin/env bash
# stop-burners.sh <pidfile> [--apply]
#
# Stops a batch of CPU burners recorded by a scratch script, WITHOUT killing
# anything that is not that burner. Two safety properties, because this touches
# processes:
#   1. every pid must currently be a process whose command line matches the burner
#      shape AND is still the same start time we recorded (etime is not stable, so
#      we check the command line only, and refuse if it does not match);
#   2. dry-run by default; --apply required to signal.
#
# WHY THIS EXISTS: F-CLI-5's burn.sh (48 x `timeout 2400 sh -c 'while :; do :; done'`)
# outlived its run by ~20 minutes and was degrading three peer units mid-measurement
# (F.65). The batch is identifiable: the count matches its pid file and its script
# writes exactly those pids.
set -uo pipefail

PIDFILE=$1
APPLY=0
[ "${2:-}" = "--apply" ] && APPLY=1

if [ ! -f "$PIDFILE" ]; then echo "no such pidfile: $PIDFILE"; exit 1; fi

n_total=0; n_match=0; n_mismatch=0
declare -a TARGETS=()
while read -r pid; do
  [ -n "$pid" ] || continue
  n_total=$((n_total + 1))
  cmd=$(ps -o args= -p "$pid" 2>/dev/null || true)
  if [ -z "$cmd" ]; then
    continue                                   # already gone
  fi
  case "$cmd" in
    *"sh -c while :; do :; done"*|*"timeout 2400 sh -c"*)
      n_match=$((n_match + 1)); TARGETS+=("$pid")
      ;;
    *)
      n_mismatch=$((n_mismatch + 1))
      echo "  REFUSING $pid - not a burner: ${cmd:0:80}"
      ;;
  esac
done < "$PIDFILE"

echo "pids in file: $n_total   matching burners alive: $n_match   mismatches: $n_mismatch"
if [ "$n_mismatch" -gt 0 ]; then
  echo "ABORT: a pid is not what we expected - investigate before signalling."
  exit 3
fi
if [ "$n_match" -eq 0 ]; then
  echo "nothing to do (batch already ended)."
  exit 0
fi

if [ "$APPLY" != 1 ]; then
  echo "DRY RUN: would TERM ${n_match} burners: ${TARGETS[*]:0:120} ..."
  exit 0
fi

echo "sending TERM to ${n_match} burners"
kill -TERM "${TARGETS[@]}" 2>/dev/null || true
sleep 3
alive=0
for pid in "${TARGETS[@]}"; do
  if ps -p "$pid" >/dev/null 2>&1; then alive=$((alive + 1)); fi
done
echo "still alive after TERM: $alive"
if [ "$alive" -gt 0 ]; then
  echo "sending KILL to the stragglers"
  for pid in "${TARGETS[@]}"; do
    if ps -p "$pid" >/dev/null 2>&1; then kill -KILL "$pid" 2>/dev/null || true; fi
  done
  sleep 2
fi
echo "remaining burner processes on the host: $(ps -eo args 2>/dev/null | grep -c 'sh -c while :; do :; done')"
