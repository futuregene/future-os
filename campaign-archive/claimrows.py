#!/usr/bin/env python3
"""Print the rows the shipped ledger patterns parse in a doc, with their line cells.

Use this before trusting a STALE number: on a doc whose ledger is prose, the rows a
pattern accepts may be incidental tables (`| `a.rs:12` | before | after |`) rather
than claims, and a false claim is what turns into a false STALE. With the shipped
shapes discovered dynamically (`ledgerrows.py`), the listing also shows *which*
shape matched, so a new pattern's effect on a doc is inspectable in one command.

    python3 .future/cov100/claimrows.py <doc> [--section S] [--limit N]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ledgerrows as lr  # noqa: E402

limit = 20
if "--limit" in sys.argv:
    limit = int(sys.argv[sys.argv.index("--limit") + 1])
args = [a for a in sys.argv[1:] if not a.startswith("--")]
if "--limit" in sys.argv:
    args = [a for a in args if a != str(limit)]
section = None
if "--section" in sys.argv:
    section = sys.argv[sys.argv.index("--section") + 1]
    args = [a for a in args if a != section]

rec = lr.load_reconcile()

for path in args:
    doc = open(path, encoding="utf-8", errors="replace").read()
    if section:
        doc = rec.slice_section(doc, section) or ""
    rows = []
    for line in doc.splitlines():
        for name, pat in lr.shipped_patterns(rec):
            m = pat.match(line)
            if m:
                rows.append((name, m.group(1), m.group(2), line))
                break
    print(f"=== {path}{f' [{section}]' if section else ''}: {len(rows)} rows parsed, "
          f"{len(rec.claimed_uncovered(doc))} distinct claims")
    for name, path_cell, lines_cell, line in rows[:limit]:
        print(f"    {name:18s} path={path_cell:32s} lines={lines_cell!r:20s} "
              f"{line.strip()[:80]}")
