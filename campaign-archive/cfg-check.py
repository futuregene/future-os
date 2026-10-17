#!/usr/bin/env python3
"""cfg-check.py - triage `platform-unmeasured` rows by their cfg context.

WHY: AG-A's segment-10 finding (F-22) was a real MISCLASSIFICATION - `cli.rs:195/197` were carried as
`platform-unmeasured` while the `--linux-sandbox-helper` argv rewrite is NOT cfg-gated, so it compiles
and can run on this host. The mechanical check it proposed: a genuinely platform-unmeasured line should
sit inside a `#[cfg(...)]` region.

FIXED after measuring its own false positives: the first version filtered on the WORD appearing anywhere
in the row, so rows that were already reclassified but explain the old category in prose
(`| cli.rs 228, 234-236 | unreachable-in-this-environment | **reclassified from platform-unmeasured:** … |`)
were counted as platform-unmeasured. It now parses the row's CELLS and reads the CATEGORY cell only.
Same bug family as F.21/F.22 (reading a row's mention instead of its status) - and the third time this
session a checker of mine trusted text over structure.

Usage: cfg-check.py <doc> [start end] [--window N]
"""
import os
import re
import sys

WT = "/Users/geilige/future-os/.worktrees/cov100-r2"
doc = sys.argv[1]
args = [a for a in sys.argv[2:] if not a.startswith("--")]
lo, hi = (int(args[0]), int(args[1])) if len(args) >= 2 else (1, 10 ** 9)
window = 40
if "--window" in sys.argv:
    window = int(sys.argv[sys.argv.index("--window") + 1])

lines = open(f"{WT}/{doc}", errors="replace").read().splitlines()
hi = min(hi, len(lines))

# a claim cell: `path:lines` or `path` n, n - matched per CELL, not per line
CLAIM = re.compile(r"`?([A-Za-z0-9_./-]+\.(?:rs|tsx|ts))`?\s*:?\s*`?([0-9][0-9,\-\s]*?)\s*`?$")


def cells(line):
    if not line.lstrip().startswith("|"):
        return []
    return [c.strip() for c in line.strip().strip("|").split("|")]


rows = []
for i, line in enumerate(lines[lo - 1:hi], lo):
    cs = cells(line)
    if len(cs) < 2:
        continue
    # the CATEGORY cell must BE platform-unmeasured (allowing a trailing marker like ✔)
    cat_idx = next((k for k, c in enumerate(cs)
                    if re.fullmatch(r"`?platform-unmeasured`?\s*[✔*]*", c)), None)
    if cat_idx is None:
        continue
    # the claim is the cell immediately before the category (tag-first ledgers may have a tag first)
    claim_cell = cs[cat_idx - 1] if cat_idx >= 1 else cs[0]
    m = CLAIM.match(claim_cell.strip("`").strip())
    if not m:
        m = re.match(r"`?([A-Za-z0-9_./-]+\.(?:rs|tsx|ts))`?\s*:?\s*`?([0-9][0-9,\-\s]*?)`?",
                     claim_cell.strip("`").strip())
    if not m:
        continue
    path, nums = m.group(1).lstrip("./"), m.group(2)
    for part in nums.split(","):
        part = part.strip()
        rng = re.match(r"^(\d+)\s*-\s*(\d+)$", part)
        if rng:
            rows.append((i, path, int(rng.group(1))))
        elif part.isdigit():
            rows.append((i, path, int(part)))

print(f"rows whose CATEGORY cell is platform-unmeasured: {len(rows)} (window={window})")

index = {}
for root, dirs, files in os.walk(WT):
    dirs[:] = [d for d in dirs if d not in ("target", ".git", "node_modules", "coverage", ".worktrees")]
    for f in files:
        if f.endswith((".rs", ".tsx", ".ts")):
            index.setdefault(f, []).append(os.path.join(root, f))

cache = {}
def source_for(path):
    if path in cache:
        return cache[path]
    if os.path.exists(f"{WT}/{path}"):
        cache[path] = f"{WT}/{path}"
        return cache[path]
    base = os.path.basename(path)
    best = None
    for c in index.get(base, []):
        if c.endswith("/" + path) and (best is None or len(c) > len(best)):
            best = c
    cache[path] = best
    return best

nofg, ok = [], 0
for docline, path, n in rows:
    src = source_for(path)
    if not src:
        print(f"  doc:{docline} {path}:{n}  SOURCE NOT FOUND")
        continue
    sl = open(src, errors="replace").read().splitlines()
    if n > len(sl):
        print(f"  doc:{docline} {path}:{n}  BEYOND EOF")
        continue
    cfg = None
    for k in range(n - 1, max(-1, n - 1 - window), -1):
        if re.search(r"#\[cfg\(|cfg!\(|cfg! *\(", sl[k]):
            cfg = (k + 1, sl[k].strip())
            break
    if cfg:
        ok += 1
        print(f"  doc:{docline} {path}:{n}  cfg within {window}: line {cfg[0]} {cfg[1][:56]}")
    else:
        nofg.append((docline, path, n, os.path.relpath(src, WT)))
        print(f"  doc:{docline} {path}:{n}  NO CFG within {window} lines")

print()
print(f"with a cfg (`#[cfg]` OR `cfg!()`) nearby: {ok}; CANDIDATES: {len(nofg)}")
for d, p, n, s in nofg:
    print(f"   doc line {d}: {s}:{n}")
