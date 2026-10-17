#!/usr/bin/env python3
"""Print per-line Istanbul hit counts for one file, straight from a
coverage-final.json. Used to verify a reviewer's claim that a probe moved
specific lines from 0 to >0 — the reviewer's probe cannot assert the private
refs, so the per-line count is the evidence.
"""
import json
import sys

report, needle, lines = sys.argv[1], sys.argv[2], sys.argv[3]
want = [int(x) for x in lines.split(",") if x.strip()]

data = json.load(open(report))
hit_any = False
for path, cov in data.items():
    if needle not in path:
        continue
    smap, scount = cov.get("statementMap") or {}, cov.get("s") or {}
    per = {}
    for sid, meta in smap.items():
        ln = meta["start"]["line"]
        per[ln] = max(per.get(ln, 0), scount.get(sid, 0))
    print(path)
    for ln in want:
        print("   line %d: hits=%s" % (ln, per.get(ln)))
    hit_any = True
if not hit_any:
    print("no file matching %r in %s" % (needle, report))
    sys.exit(2)
