#!/usr/bin/env python3
"""Find ledger rows that LOOK like line claims but parse as zero claims.

Why this exists: `reconcile.claimed_uncovered` accepts `| `path.rs` 12, 15 | cat | ... |`
but a trailing count inside the same cell (`| `path.rs` 12, 15 (40 lines) | cat | ... |`)
makes the row parse as NO claim at all. The line count annotation - the most helpful thing
in the row - silently disables the check. Measured on module-tauri-remote.md: five rows,
273 lines, so the pairing certified 331 of the record's 450 while reporting PASS.

A second, separate mechanism does the same thing: a TYPOGRAPHIC EN DASH in a range
(`464-498` written as `464-498` with U+2013) is not recognised as a range at all, so a row
that looks like `| `p.rs` 464-498 | cat | ... |` parses to zero claims as well.

A pairing can therefore be green while certifying far less than its record claims. This
script finds that class anywhere in docs/testing/. It deliberately does NOT flag the
`| `path.rs` (6 lines) | ... |` summary form: that row carries no line reference, so zero
claims is the correct answer for it.
"""
import glob
import os
import re
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__)))
import reconcile as rec  # noqa: E402

# a cell that carries a line reference (digits) but no test-name list
TEST_LIST = re.compile(r"`, ?`")
# a real line reference: the path backtick immediately followed by a number
LINE_REF = re.compile(r"`[^`]*\.rs`\s*\d")
EN_DASH_RANGE = re.compile(r"\d\s*[\u2013\u2014]\s*\d")
TRAILING_COUNT = re.compile(r"\(\d+ lines?\)\s*$")


def scan(path):
    out = []
    with open(path, encoding="utf-8", errors="replace") as fh:
        for n, line in enumerate(fh, 1):
            if not line.startswith("|") or ".rs" not in line:
                continue
            cell = line.split("|")[1].strip()
            if TEST_LIST.search(cell) or not LINE_REF.search(cell):
                continue          # no line reference == a summary row, 0 claims is right
            if rec.claimed_uncovered(line):
                continue
            why = "EN DASH in range" if EN_DASH_RANGE.search(cell) else (
                  "trailing count" if TRAILING_COUNT.search(cell) else "other")
            out.append((n, why, cell[:78]))
    return out


def main():
    # Only the two MECHANICAL classes are actionable: an en dash where an ASCII hyphen
    # belongs, and a count appended inside the claim cell. Anything else (slash-separated
    # line lists, prose cells, review verdict tables) is either a legitimate convention or
    # a human judgement call, and counting it would make this an alarm that always fires.
    total = 0
    mechanical = 0
    for path in sorted(glob.glob("docs/testing/*.md")):
        hits = scan(path)
        if hits:
            print(f"{os.path.basename(path):34s} {len(hits)} row(s) parse as ZERO claims")
            for n, why, cell in hits:
                print(f"    :{n}  [{why}]  {cell}")
        total += len(hits)
        mechanical += sum(1 for _, why, _ in hits if why != "other")
    print(f"TOTAL rows parsing as zero claims: {total} "
          f"(mechanical, i.e. fixable by convention: {mechanical})")
    return 0 if mechanical == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
