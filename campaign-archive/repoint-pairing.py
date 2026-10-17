#!/usr/bin/env python3
"""Re-point ONE pairing's report (and the expected counts that follow from it).

Refuses to act unless the pairing's current values match what the caller says they
are - so a stale instruction cannot silently rewrite a live registry entry.

Usage: repoint-pairing.py <label> <old_report> <new_report> <old_uncovered> <new_uncovered>
"""
import json, sys

label, old_report, new_report, old_n, new_n = sys.argv[1:6]
P = '.future/cov100/pairings.json'
doc = json.load(open(P, encoding='utf-8'))

hit = None
for p in doc['pairings']:
    if p.get('label') == label:
        hit = p
        break

if hit is None:
    print(f"ABORT: no pairing labelled {label}")
    sys.exit(1)

if hit.get('report') != old_report:
    print(f"ABORT: {label}'s report is {hit.get('report')!r}, not the expected {old_report!r}")
    sys.exit(1)
if hit.get('expect', {}).get('uncovered') != int(old_n):
    print(f"ABORT: {label}'s expected uncovered is {hit.get('expect', {}).get('uncovered')}, not {old_n}")
    sys.exit(1)

hit['report'] = new_report
hit['expect']['uncovered'] = int(new_n)
hit['expect']['claims'] = int(new_n)

open(P, 'w', encoding='utf-8').write(json.dumps(doc, indent=2, ensure_ascii=False) + "\n")

back = json.load(open(P, encoding='utf-8'))
for p in back['pairings']:
    if p.get('label') == label:
        print(f"re-pointed {label}: report={p['report']} expect={p['expect']}")
