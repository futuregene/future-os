#!/usr/bin/env bash
# Baseline coverage: desktop frontend (vitest) + mobile (jest).
set -uo pipefail

ROOT=/Users/geilige/future-os
OUT="$ROOT/coverage/round2"
mkdir -p "$OUT"

echo "== desktop vitest coverage @ $(date) =="
cd "$ROOT/desktop" || exit 1
npx vitest run --coverage 2>&1 | tail -40
echo "VITEST_EXIT=${PIPESTATUS[0]}"
cp -f "$ROOT/desktop/coverage/coverage-final.json" "$OUT/desktop-web-coverage-final.json" 2>/dev/null

echo "== mobile jest coverage @ $(date) =="
cd "$ROOT/mobile" || exit 1
npx jest --runInBand --coverage 2>&1 | tail -40
echo "JEST_EXIT=${PIPESTATUS[0]}"
cp -f "$ROOT/mobile/coverage/lcov.info" "$OUT/mobile-lcov.info" 2>/dev/null

echo "== done @ $(date) =="
