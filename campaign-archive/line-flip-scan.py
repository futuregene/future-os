#!/usr/bin/env python3
"""Check whether an agent-crate line FLIPS across every report on disk.

The ticket repeated the reviewer's §9.1 premise that session/journal_compaction.rs:171
"flips" (covered in the baseline and first run, uncovered in the last two). The unit
measured 39/39 reports refuting the flip. This reproduces that count from the reports
themselves: for each /tmp/*.info that contains the file, print the DA value.

Usage: line-flip-scan.py <file-suffix> <line> [glob]
"""
import glob, sys

suffix = sys.argv[1]            # e.g. /agent/src/session/journal_compaction.rs
line = int(sys.argv[2])
pattern = sys.argv[3] if len(sys.argv) > 3 else '/tmp/*.info'

vals = {}
for path in sorted(glob.glob(pattern)):
    keep = False
    for row in open(path, errors='replace'):
        if row.startswith('SF:'):
            keep = row[3:].strip().endswith(suffix)
            continue
        if keep and row.startswith('DA:'):
            n = int(row[3:].split(',')[0])
            if n == line:
                c = int(row[3:].split(',')[1].split(',')[0])
                vals.setdefault(c, []).append(path.split('/')[-1])
                break
    else:
        continue

total = sum(len(v) for v in vals.values())
print(f"{suffix}:{line} read in {total} report(s) that contain the file")
for c in sorted(vals):
    names = vals[c]
    shown = ', '.join(names[:3]) + (' …' if len(names) > 3 else '')
    print(f"  DA:{line} = {c:<4} in {len(names):3d} report(s)   ({shown})")
if len(vals) == 1:
    print(f"  -> NO FLIP: every report agrees (value {list(vals)[0]})")
else:
    print(f"  -> FLIPS between {sorted(vals)}")
