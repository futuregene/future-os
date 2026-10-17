#!/usr/bin/env python3
"""Which ledger rows does `reconcile.py`'s STALE half actually see?

`claimed_uncovered()` reads a claim from a markdown row's first cell (or its second
when the first is a bare tag), and its original pattern demanded the closing pipe
IMMEDIATELY after the line numbers. It therefore accepted `` | `src/a.rs`:12 | … ``
and `` | src/a.rs:12 | … `` but rejected `` | `src/a.rs:12` | … `` — one backtick
pair around path and number together, the commonest shape in this repo's ledgers.
For such a doc `stale` came out 0 because no row was ever parsed, not because the
ledger was correct (vacuously green). Fixed in todo_7449334225b8, along with a
WARNING from `reconcile.py` when a doc slice has table rows and 0 parsed claims; two
further shapes (bare-tag-first, and `path` + space + lines with no colon) were added
by a second agent on the same file, and this tool discovers them instead of restating
them (see `ledgerrows.py`).

Run this from the worktree root:

    python3 .future/cov100/rowscan.py [docs/testing/module-*.md …] [--section S] [--samples]
    python3 .future/cov100/rowscan.py --sections       # which sections carry claims, per doc
    python3 .future/cov100/rowscan.py --needle 4c docs/testing/module-cli.md
                                                      # does that --section needle resolve
                                                      # to one heading, or several?

That last one exists because a needle that matches two headings makes `--section`
land on the first, which may be a cross-reference rather than the ledger (measured:
`--section "4c"` on `module-cli.md` resolves to `### Segment 3 … behind §4c`, ~500
lines above the ledger, and the run then reads `mentioned: 0`).

Columns: `rows` = markdown table rows; `before` = rows the pre-fix pattern accepted
(0 means the STALE half never fired for that doc before the fix); `shipped` = rows
some shipped `LEDGER_ROW*` pattern accepts (counted by calling the shipped module);
`claims` = distinct `(path, line)` pairs `claimed_uncovered()` returns (ranges expand,
duplicates collapse, so this can be above or below `shipped`); `no-colon` = rows the
spaceless `| `path.rs` 12 |` shape would add if it were not shipped. `--samples`
prints example rows for the docs still at 0.
"""
import glob
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ledgerrows as lr  # noqa: E402

# A shape this repo uses that no shipped pattern accepts: the path in backticks,
# then a space, then numbers — kept here only to stay comparable with the shipped
# `LEDGER_ROW_SPACED` where that exists.
NO_COLON = re.compile(
    r"^\s*\|\s*`?([A-Za-z0-9_./-]+\.(?:tsx|ts|rs))`?\s+([0-9][0-9,\-\s]*?)\s*`?\s*\|")

PATHY = re.compile(r"[A-Za-z0-9_./-]+\.(?:tsx|ts|rs)")


def count(pattern, doc):
    return sum(1 for line in doc.splitlines() if pattern.match(line))


def rows(doc):
    return sum(1 for line in doc.splitlines() if re.match(r"^\s*\|\s*", line))


def samples(doc, rec, limit=3):
    """Rows that look like a ledger path:line but no shipped shape accepts.

    Printed for the docs still at 0: either their ledger uses a shape worth
    supporting, or the rows below are not claims at all (a coverage summary, a table
    of test names) and rejecting them is correct.
    """
    out = []
    for line in doc.splitlines():
        if not re.match(r"^\s*\|\s*", line) or lr.shipped_match(line, rec):
            continue
        if PATHY.search(line) and re.search(r"[:\s]\s*\d", line):
            out.append(line.strip()[:140])
        if len(out) >= limit:
            break
    return out


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    show = "--samples" in sys.argv
    sections_mode = "--sections" in sys.argv
    needle = None
    if "--needle" in sys.argv:
        needle = sys.argv[sys.argv.index("--needle") + 1]
        args = [a for a in args if a != needle]
    section = None
    if "--section" in sys.argv:
        section = sys.argv[sys.argv.index("--section") + 1]
        args = [a for a in args if a != section]
    paths = args or sorted(glob.glob("docs/testing/module-*.md"))
    if not paths:
        print("no docs matched (run from the worktree root)", file=sys.stderr)
        return 2
    rec = lr.load_reconcile()

    if needle is not None:
        # The ambiguity check: which headings does this --section needle resolve to?
        # slice_section() takes the first, so more than one is a standing hazard.
        for p in paths:
            doc = open(p, encoding="utf-8", errors="replace").read()
            hits = rec.sections_matching(doc, needle)
            mark = "  <-- AMBIGUOUS: slice_section takes the first" if len(hits) > 1 else ""
            print(f"{p}: {len(hits)} heading(s) contain {needle!r}{mark}")
            for level, text in hits:
                print(f"    {'#' * level} {text}")
        return 0

    if sections_mode:
        # Which sections carry claims at all -- the index you need before trusting a
        # STALE count, and the reason a wide invocation can show covered rows.
        for p in paths:
            doc = open(p, encoding="utf-8", errors="replace").read()
            by_sec = rec.claims_by_section(doc)
            print(f"=== {p}: {len(by_sec)} section(s) carry claims")
            for head, claims in sorted(by_sec.items(), key=lambda kv: -len(kv[1])):
                print(f"    {len(claims):4d} claims  {head[:78]}")
        return 0
    shapes = ", ".join(n for n, _ in lr.shipped_patterns(rec))
    blind_now, blind_before = [], []
    print(f"shipped shapes: {shapes}")
    print(f"{'doc':52s} {'rows':>5s} {'before':>7s} {'shipped':>8s} "
          f"{'claims':>7s} {'no-colon':>9s}")
    for path in paths:
        doc = open(path, encoding="utf-8", errors="replace").read()
        label = path
        if section:
            # the SLICE, not the doc: the STALE half only sees rows inside it, so a
            # whole-doc count can be green for a slice that parses nothing
            doc = rec.slice_section(doc, section) or ""
            label = f"{path} [{section}]"
        n = rows(doc)
        before = count(lr.BEFORE, doc)
        shipped_rows = sum(1 for line in doc.splitlines()
                           if lr.shipped_match(line, rec))
        shipped_claims = len(rec.claimed_uncovered(doc))
        nc = count(NO_COLON, doc)
        flag = ""
        if n and before == 0:
            blind_before.append(label)
            flag = "  <-- STALE half never fired before the fix"
        if n and not shipped_claims:
            blind_now.append(label)
            flag = "  <-- STILL blind after the fix"
        if len(label) > 52:
            print(label)
            label = "  " + label[-50:]
        print(f"{label:52s} {n:5d} {before:7d} {shipped_rows:8d} "
              f"{shipped_claims:7d} {nc:9d}{flag}")
    print()
    print(f"docs whose STALE half never fired before the fix (rows>0, before==0):"
          f" {len(blind_before)}")
    print(f"docs still blind after the fix (rows>0, shipped claims==0): {len(blind_now)}")
    for label in blind_now:
        print(f"  {label}")
    if show:
        print()
        for label in blind_now:
            doc = open(label.split(" [")[0], encoding="utf-8", errors="replace").read()
            if section:
                doc = rec.slice_section(doc, section) or ""
            print(f"--- {label}")
            for s in samples(doc, rec):
                print(f"    {s}")
    print()
    print("pre-fix pattern:", lr.BEFORE_SRC)
    return 0


if __name__ == "__main__":
    sys.exit(main())
