#!/usr/bin/env python3
"""Rows the shipped LEDGER_ROW accepts but `claimed_uncovered()` turns into ZERO
claims. Such a row is parsed and then discarded, which looks the same as a correct
row in the output — the same silent-skip class the STALE-half fix was about.

    python3 .future/cov100/zeroclaims.py <doc>
"""
import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location(
    "cov100_reconcile", os.path.join(HERE, "reconcile.py"))
rec = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rec)

for path in sys.argv[1:]:
    doc = open(path, encoding="utf-8", errors="replace").read()
    bad = []
    for line in doc.splitlines():
        m = rec.LEDGER_ROW.match(line)
        if not m:
            continue
        parts = [p.strip() for p in m.group(2).split(",")]
        got = [p for p in parts
               if p.isdigit() or (p.count("-") == 1 and all(
                   q.strip().isdigit() for q in p.split("-")))]
        if not got:
            bad.append((repr(m.group(2)), line.strip()[:130]))
    print(f"{path}: rows accepted={sum(1 for l in doc.splitlines() if rec.LEDGER_ROW.match(l))} "
          f"claims={len(rec.claimed_uncovered(doc))} zero-claim rows={len(bad)}")
    for g2, line in bad[:6]:
        print(f"    group(2)={g2}\n      {line}")
