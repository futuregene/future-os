#!/usr/bin/env python3
"""Split a STALE list into the two very different things `reconcile.py` lumped
together, with the full lists (the tool's own output prints the split inline but
truncates the tails).

* **COVERED** — the line has a `DA:` record with a non-zero count. A real
  disagreement between ledger and report.
* **ABSENT** — the report has no `DA:` record for the line at all (blank line,
  comment, `#[cfg]`-eliminated, or a different revision). The ledger may be
  perfectly right; `claimed_uncovered()` expands `a-b` ranges to EVERY line in them,
  so a range spanning a blank line manufactures one of these per blank.

Both need the same caveat: a count is only meaningful when the ledger and the report
describe the same revision.

The split itself lives in `reconcile.py` (`split_stale` / `report_counts`) and is
imported here, so this tool, `reconcile.py`'s inline output and `staletable.py`
cannot drift apart.

    python3 .future/cov100/staledetail.py --report R --scope S [--scope S2 …] \
        --doc DOC [--section S] [--label L] [--all]
"""
import argparse
import importlib.util
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ledgerrows as lr  # noqa: E402

rec = lr.load_reconcile()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", required=True)
    ap.add_argument("--scope", required=True, action="append")
    ap.add_argument("--doc", required=True)
    ap.add_argument("--section", default=None)
    ap.add_argument("--label", default="unit")
    ap.add_argument("--all", action="store_true",
                    help="print every line, not just the first 40 per half")
    args = ap.parse_args()

    per_file = rec.key_files(rec.report_lines(args.report, args.scope), args.scope)
    counts = rec.report_counts(args.report, args.scope)

    doc = open(args.doc, encoding="utf-8", errors="replace").read()
    if args.section:
        doc = rec.slice_section(doc, args.section) or ""

    stale = []
    for path, ln in sorted(rec.claimed_uncovered(doc)):
        targets = rec.claim_targets(path, ln, per_file)
        if not targets:
            continue
        if all(per_file[t] and ln not in per_file[t] for t in targets):
            stale.append((targets[0], ln))
    covered, absent = rec.split_stale(stale, counts)

    n = 10 ** 9 if args.all else 40
    print(f"[{args.label}] claims={len(rec.claimed_uncovered(doc))} "
          f"stale={len(covered) + len(absent)}  "
          f"COVERED={len(covered)} (DA>0 -- real disagreement)  "
          f"ABSENT={len(absent)} (no DA record -- not evidence)")
    for key, ln in covered[:n]:
        print(f"    COVERED {key}:{ln} count={counts[key][ln]}")
    for key, ln in absent[:n]:
        print(f"    absent  {key}:{ln} (no DA record)")
    if not args.all:
        for label, seq in (("COVERED", covered), ("absent", absent)):
            if len(seq) > 40:
                print(f"    ... and {len(seq) - 40} more {label} (use --all)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
