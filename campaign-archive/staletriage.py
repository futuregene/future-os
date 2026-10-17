#!/usr/bin/env python3
"""Triage the COVERED half: is a disagreement a ledger error, or line drift?

`reconcile.py` now reports STALE rows with `DA>0` as COVERED. In a worktree where
several units edit source files while a doc's report was measured somewhere else,
most such rows have an innocent explanation, and calling them ledger errors would
send a reviewer to "fix" correct rows -- the harmful direction this whole check is
built to avoid. Three cheap discriminators, reported per covered line:

* `nearby:` an uncovered line in the SAME file within `--near` lines. A cluster of
  those means the citation drifted (the doc's line numbers are from another revision),
  not that the row is wrong;
* `file modified:` the source file differs from HEAD (`git diff --numstat`), so the
  revision the report measured is not the revision on disk;
* `count:` the `DA:` count, i.e. how strongly the report disagrees.

    python3 .future/cov100/staletriage.py --report R --scope S [--scope S2 …] \
        --doc DOC [--section S] [--near 5] [--label L]
"""
import argparse
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ledgerrows as lr  # noqa: E402

rec = lr.load_reconcile()


def modified_files():
    """{basename: numstat line} for every modified/untracked source file."""
    out = {}
    for cmd in (["git", "diff", "--numstat"], ["git", "status", "--short"]):
        try:
            p = subprocess.run(cmd, capture_output=True, text=True, check=False)
        except OSError:
            continue
        for line in p.stdout.splitlines():
            for tok in line.replace("\t", " ").split():
                if tok.endswith((".rs", ".ts", ".tsx")):
                    out[os.path.basename(tok)] = line.strip()[:60]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", required=True)
    ap.add_argument("--scope", required=True, action="append")
    ap.add_argument("--doc", required=True)
    ap.add_argument("--section", default=None)
    ap.add_argument("--label", default="unit")
    ap.add_argument("--near", type=int, default=5)
    args = ap.parse_args()

    per_file = rec.key_files(rec.report_lines(args.report, args.scope), args.scope)
    counts = rec.report_counts(args.report, args.scope)
    text = open(args.doc, encoding="utf-8", errors="replace").read()
    if args.section:
        text = rec.slice_section(text, args.section) or ""

    stale = []
    for path, ln in sorted(rec.claimed_uncovered(text)):
        targets = rec.claim_targets(path, ln, per_file)
        if not targets:
            continue
        if all(per_file[t] and ln not in per_file[t] for t in targets):
            stale.append((targets[0], ln))
    covered, absent = rec.split_stale(stale, counts)
    mods = modified_files()

    by_cover, by_drift, alone = [], [], []
    for key, ln in covered:
        uncovered = sorted(per_file.get(key) or [])
        near = [u for u in uncovered if abs(u - ln) <= args.near]
        cnt = rec.counts_for(key, counts).get(ln, 0)
        row = (key, ln, cnt, near[:3], os.path.basename(key))
        (by_drift if near else by_cover).append(row)

    print(f"[{args.label}] claims={len(rec.claimed_uncovered(text))} "
          f"stale={len(covered) + len(absent)}  COVERED={len(covered)}  "
          f"ABSENT={len(absent)}")
    print(f"  of the {len(covered)} covered: {len(by_drift)} have an uncovered line "
          f"within {args.near} (line drift), {len(by_cover)} do not")
    for label, rows in (("drift-suspect", by_drift), ("standalone", by_cover)):
        if not rows:
            continue
        print(f"  --- {label} ({len(rows)})")
        for key, ln, cnt, near, base in rows[:25]:
            mark = " MODIFIED" if base in mods else ""
            print(f"      {key}:{ln} count={cnt} nearby_uncovered={near}{mark}")
    mods_in_scope = sorted({os.path.basename(k) for k, _, _, _, _ in covered}
                           & set(mods))
    print(f"  files with covered-but-claimed lines that are modified in the worktree: "
          f"{len(mods_in_scope)} {mods_in_scope[:6]}")
    del alone
    return 0


if __name__ == "__main__":
    sys.exit(main())
