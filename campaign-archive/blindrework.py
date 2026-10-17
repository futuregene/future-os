#!/usr/bin/env python3
"""Per-doc rework facts for the ledgers `reconcile.py` still cannot see.

For each doc the shipped patterns parse 0 claims from, this prints:

* `rows` -- markdown table rows in the whole doc;
* `path+num in cell1` -- rows whose FIRST cell carries a file path AND a digit.
  These are the population a looser rule would start calling claims, and they are
  why loosening is the wrong fix: the numbers in that cell are usually COUNTS
  (`| `llm/schema.rs` 2, `llm/adapters/anthropic.rs` 1 | 3 | test name |`), so a
  pattern that accepts them invents claims about line 2 and line 1. Verified as
  must-reject cases in `patternprobe.py`;
* `path:LINE in prose` -- rows whose line-level citation sits inside prose in a later
  cell (`Lines 44-49 are ...`, `the summary counts 1 uncovered line`). Parsing prose
  is exactly the false-STALE direction `claimed_uncovered()` refuses;
* `counts in later cells` -- rows that are coverage arithmetic (`227/229, 2`).

The actionable conclusion is doc-side: put the citation in the first cell the way
`module-goal.md` §11.5 and `module-tauri-shell.md` §4 already do.

    python3 .future/cov100/blindrework.py [doc …]     # default: the blind set
"""
import glob
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ledgerrows as lr  # noqa: E402

rec = lr.load_reconcile()
PATHY = re.compile(r"[A-Za-z0-9_./-]+\.(?:tsx|ts|rs)")
PATHLINE = re.compile(r"[A-Za-z0-9_./-]+\.(?:tsx|ts|rs)[`\s]*:\s*\d")


def cells(line):
    return [c.strip() for c in line.strip().strip("|").split("|")]


def classify(lines):
    out = {"rows": len(lines), "claim-shaped": 0, "path+num cell1": 0,
           "prose cite": 0, "counts later": 0, "cites_anywhere": 0,
           "cites_in_tables": 0, "cites_outside": 0, "samples": {}}
    for line in lines:
        # every `path:NNN` citation in the doc, wherever it sits
        hits = PATHLINE.findall(line)
        if hits:
            out["cites_anywhere"] += len(hits)
            if re.match(r"^\s*\|", line):
                out["cites_in_tables"] += len(hits)
            else:
                out["cites_outside"] += len(hits)
                out["samples"].setdefault("citation outside a table", line)
        if not re.match(r"^\s*\|\s*", line):
            continue
        cs = cells(line)
        if len(cs) < 2:
            continue
        c1 = cs[0]
        if PATHLINE.search(c1):
            out["claim-shaped"] += 1
            continue
        if PATHY.search(c1) and re.search(r"\d", c1):
            out["path+num cell1"] += 1
            out["samples"].setdefault("path+num cell1", line)
            continue
        rest = " | ".join(cs[1:])
        if PATHLINE.search(rest):
            out["prose cite"] += 1
            out["samples"].setdefault("prose cite", line)
        elif re.search(r"\d+\s*/\s*\d+", rest):
            out["counts later"] += 1
            out["samples"].setdefault("counts later", line)
    return out


def main():
    args = sys.argv[1:]
    if args:
        docs = args
    else:
        # the blind set that matters: the per-module records whose ledger the STALE
        # half cannot read. Scoped to module records on purpose -- a doc with no
        # ledger at all (docs/README.md, the architecture pages) is not a rework
        # candidate, it is simply not a ledger.
        docs = []
        for p in sorted(glob.glob("docs/testing/module-*.md")) + [
                "docs/architecture/channels-test-coverage.md"]:
            text = open(p, encoding="utf-8", errors="replace").read()
            if rec.claimed_uncovered(text) == set():
                docs.append(p)
    print(f"{'doc':44s} {'rows':>5s} {'cites':>6s} {'in tbl':>7s} {'outside':>8s} "
          f"{'claim-shape':>12s} {'path+num c1':>12s} {'prose cite':>10s} "
          f"{'counts later':>12s}")
    for p in docs:
        text = open(p, encoding="utf-8", errors="replace").read()
        r = classify(text.splitlines())
        if not r["rows"]:
            continue
        print(f"{p:44s} {r['rows']:5d} {r['cites_anywhere']:6d} "
              f"{r['cites_in_tables']:7d} {r['cites_outside']:8d} "
              f"{r['claim-shaped']:12d} {r['path+num cell1']:12d} "
              f"{r['prose cite']:10d} {r['counts later']:12d}")
        for k in ("path+num cell1", "prose cite", "counts later",
                  "citation outside a table"):
            if k in r["samples"]:
                print(f"      {k:14s} {r['samples'][k].strip()[:118]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
