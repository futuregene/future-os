#!/usr/bin/env python3
"""Is each feature flavour's report ALIVE? (PLAN §6a-ter, second failure mode)

A two-flavour crate is measured by rendering each flavour separately and unioning. Two
things can silently drop a flavour:

  1. a warm-dir merge loses one flavour's data (FLAKES item 30);
  2. **a test binary killed by a signal writes no profraw at all**, so that flavour
     contributes nothing - and the report still looks plausible, because the `DA:` line
     COUNT is unchanged: the same lines are measured, they simply all read 0.
     (Measured 2026-10-17: an external SIGTERM killed the gui lib test binary and the
     first measurement of that segment had the whole gui flavour reading zero.)

The check that catches both: a flavour's own report must contain at least one NONZERO
hit. Presence of `DA:` records proves nothing (failure mode 2 keeps them).

Usage: flavor-alive.py <flavour-A.info> <flavour-B.info> [...]
Exit: 0 if every report has nonzero hits, 1 otherwise.
"""
import sys


def totals(path):
    covered = uncovered = files = 0
    for line in open(path, encoding='utf-8', errors='replace'):
        if line.startswith('SF:'):
            files += 1
        elif line.startswith('DA:'):
            body = line[3:]
            _n, _, rest = body.partition(',')
            c = rest.split(',')[0]
            try:
                c = int(c)
            except ValueError:
                continue
            if c > 0:
                covered += 1
            else:
                uncovered += 1
    return covered, uncovered, files


bad = 0
for path in sys.argv[1:]:
    try:
        cov, unc, files = totals(path)
    except OSError as error:
        print(f"FAIL  {path}: {error}")
        bad += 1
        continue
    alive = cov > 0
    verdict = "ok  " if alive else "FAIL"
    if not alive:
        bad += 1
    print(f"{verdict} {path}: covered={cov} uncovered={unc} files={files}"
          + ("" if alive else "   <- NO nonzero hits: this flavour produced no data "
                              "(a signal-killed test binary writes no profraw)"))

if bad:
    print(f"\n{bad} flavour report(s) without data - do NOT publish a number from this run")
    sys.exit(1)
print("\nall flavour reports carry nonzero hits")
