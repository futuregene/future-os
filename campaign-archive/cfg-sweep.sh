#!/usr/bin/env bash
# Sweep every module record for `platform-unmeasured` rows whose cited line has no cfg above it -
# AG-A's mechanical check (segment 10, F-22), applied campaign-wide with a tool that reads the row's
# CATEGORY cell rather than any mention of the phrase.
set -uo pipefail
cd /Users/geilige/future-os/.worktrees/cov100-r2 || exit 1
T=/Users/geilige/future-os/.future/cov100/cfg-check.py
OUT=/tmp/cfg-sweep.txt
: > "$OUT"
for d in docs/testing/module-*.md docs/architecture/channels-test-coverage.md; do
  [ -f "$d" ] || continue
  if grep -q 'platform-unmeasured' "$d"; then
    echo "===== $d" >> "$OUT"
    python3 "$T" "$d" 2>&1 | tail -n +1 >> "$OUT"
    echo >> "$OUT"
  fi
done
echo "sweep written to $OUT"
echo
grep -E "^rows whose CATEGORY|^with a cfg|CANDIDATES: |^===== |^   doc line" "$OUT" | grep -B1 -A20 "CANDIDATES" | head -40
echo
echo "=== totals ==="
grep -c "^rows whose CATEGORY cell is platform-unmeasured" "$OUT"
grep -oE "CANDIDATES: [0-9]+" "$OUT" | awk '{s+=$2} END {print "candidates across all docs:", s}'