#!/usr/bin/env bash
# Verify measure-final.sh's gate step end to end, without writing outside this unit's
# write set. Run by selftest.py, so a change to either the harness or the tool's --md
# output is caught by the same command as everything else.
#
# The probe copy lives in the SAME directory as the real script, because measure-final.sh
# derives ROOT from "$(dirname "$0")/../.." — a copy in /tmp resolves ROOT to "/" and
# every path in the run breaks (observed: `mkdir: //target: Read-only file system`).
# Only OUT is redirected, to a temp dir. The copy is removed on every exit path.
set -uo pipefail
cd /Users/geilige/future-os/.worktrees/cov100-r2 || exit 1
TOOLS="$PWD/.future/cov100"
TMP=$(mktemp -d /tmp/cov100-measure-probe.XXXXXX)
cp "$TOOLS/measure-final.sh" "$TOOLS/.measure-final-probe.sh"
trap 'rm -f "$TOOLS/.measure-final-probe.sh"' EXIT
sed -i '' "s|^OUT=\"\$ROOT/coverage/final\"|OUT=\"$TMP\"|" "$TOOLS/.measure-final-probe.sh"
grep -n "^OUT=" "$TOOLS/.measure-final-probe.sh"

echo "=== run with --tracks none (all four measurement legs skipped) ==="
bash "$TOOLS/.measure-final-probe.sh" --tracks none > "$TMP/stdout.log" 2> "$TMP/stderr.log"
rc=$?
echo "HARNESS_EXIT=$rc"
echo "--- stderr ---"
cat "$TMP/stderr.log"
echo "--- the report's gate section (as a reviewer sees it) ---"
sed -n '/## Pairing gate/,$p' "$TMP/FINAL-REPORT.md" | head -18
echo "--- checks ---"
printf 'gate heading count: %s (want 1)\n' "$(grep -c '^## Pairing gate' "$TMP/FINAL-REPORT.md")"
printf 'pairing rows: %s (want >= 17)\n' "$(grep -c '^| `' "$TMP/FINAL-REPORT.md")"
printf 'captured exit-code line: %s (want 1)\n' "$(grep -c 'gate step exit: ' "$TMP/FINAL-REPORT.md")"
printf 'snapshot step line: %s (want 1)\n' "$(grep -c 'snapshot step exit: ' "$TMP/FINAL-REPORT.md")"
printf 'snapshot file written: %s (want 1)\n' "$([ -s "$TMP/pairing-gate-snapshot.json" ] && echo 1 || echo 0)"
echo "PROBE_DIR=$TMP"

fail=0
[ "$rc" = 0 ] || { echo "FAIL: harness exited $rc with --tracks none" >&2; fail=1; }
[ "$(grep -c '^## Pairing gate' "$TMP/FINAL-REPORT.md")" = 1 ] \
  || { echo "FAIL: gate heading count" >&2; fail=1; }
[ "$(grep -c '^| `' "$TMP/FINAL-REPORT.md")" -ge 21 ] \
  || { echo "FAIL: pairing rows missing from the report" >&2; fail=1; }
[ "$(grep -c 'gate step exit: ' "$TMP/FINAL-REPORT.md")" = 1 ] \
  || { echo "FAIL: the report does not carry the gate's exit code" >&2; fail=1; }
# the snapshot must be written AND verify against the tree it was taken from
[ -s "$TMP/pairing-gate-snapshot.json" ] \
  || { echo "FAIL: the harness wrote no gate snapshot" >&2; fail=1; }
if [ -s "$TMP/pairing-gate-snapshot.json" ]; then
  python3 "$TOOLS/staletable.py" --verify-stamp "$TMP/pairing-gate-snapshot.json" \
    > "$TMP/verify.log" 2>&1
  vrc=$?
  echo "verify-stamp exit: $vrc (want 0 — the snapshot describes this tree)"
  [ "$vrc" = 0 ] || { echo "FAIL: the harness's own snapshot does not verify" >&2; fail=1; }
fi

# Second run: a RED gate must still produce a report (reported step, not a precondition)
# and must exit 0 from the harness's own point of view. Driven by the negative fixture,
# so this exercises the real path rather than a simulated failure.
echo
echo "=== run with a deliberately red gate (negative fixture) ==="
cp "$TOOLS/measure-final.sh" "$TOOLS/.measure-final-probe.sh"
sed -i '' -e "s|^OUT=\"\$ROOT/coverage/final\"|OUT=\"$TMP/red\"|" \
          -e "s|--md > \"\$gate_log\"|--md --pairings \"$TOOLS/selftest/pairings-negative.json\" > \"\$gate_log\"|" \
          "$TOOLS/.measure-final-probe.sh"
bash "$TOOLS/.measure-final-probe.sh" --tracks none > "$TMP/red-stdout.log" 2> "$TMP/red-stderr.log"
rc_red=$?
echo "HARNESS_EXIT(red gate)=$rc_red"
echo "--- stderr (want the gate warning, not an abort) ---"
cat "$TMP/red-stderr.log"
echo "--- the red report's verdicts ---"
grep -E '^\| `(neg-|tui)|^\*\*Failing|gate step exit:' "$TMP/red/FINAL-REPORT.md" | head -8
[ "$rc_red" = 0 ] || { echo "FAIL: a red gate aborted the harness (rc=$rc_red)" >&2; fail=1; }
[ "$(grep -c 'gate step exit: 1' "$TMP/red/FINAL-REPORT.md")" = 1 ] \
  || { echo "FAIL: the red report does not carry exit 1" >&2; fail=1; }
[ "$(grep -c '^\*\*Failing pairings\*\*' "$TMP/red/FINAL-REPORT.md")" = 1 ] \
  || { echo "FAIL: the red report does not list failing pairings" >&2; fail=1; }
exit $fail
