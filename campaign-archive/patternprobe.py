#!/usr/bin/env python3
"""Adversarial probe for the shipped ledger-row patterns: which rows must be
accepted, which must be REJECTED, and does the shipped module agree?

Every case carries an expectation, because the two mistakes are not symmetric:

* a **false claim** (row accepted that is not a ledger row) becomes a false STALE,
  which sends a reviewer to "fix" a correct row -- the harmful direction;
* a **missed claim** (row rejected that is a claim) hides a row from the STALE half
  -- loud only because `reconcile.py` now prints a vacuity WARNING.

Exit code is the verdict: 0 = every case matches its expectation, 1 = a mismatch
(printed with the pattern name that accepted or refused it). Run it after touching
any `LEDGER_ROW*` pattern -- the pattern set is discovered, not restated.

    python3 .future/cov100/patternprobe.py [--contrast]

`--contrast` also runs every case against the PRE-tightening tag class (any short
word) and marks the cases that flip, so the probe can be shown to discriminate.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ledgerrows as lr  # noqa: E402

rec = lr.load_reconcile()

# The tag class as it was before the audit.
LOOSE = re.compile(
    r"^\s*\|\s*`?[A-Za-z][A-Za-z0-9 _.-]{0,12}`?\s*\|\s*"
    r"`?([A-Za-z0-9_./-]+\.(?:tsx|ts|rs))`?\s*:\s*`?([0-9,\-\s]+?)\s*`?\s*\|")

# (row, expect_accept, why)
CASES = [
    # --- the four shapes that must be accepted -------------------------------
    ("| `src/a.rs:12,15` | 2 | unreachable-by-construction | why |", True,
     "first cell: backticked path with colon"),
    ("| `src/a.rs:12` | why |", True, "first cell: single line"),
    ("| src/a.rs:12 | why |", True, "first cell: unbackticked path"),
    ("| `src/a.tsx`:200 | why |", True, "tsx, colon outside the backticks"),
    ("| D12 | `src/a.rs:200` | unreachable-by-construction | why |", True,
     "tag-first (module-desktop.md)"),
    ("| D1 | `src/a.rs:200` | category | why |", True, "tag-first, one digit"),
    ("| `a.rs` 12, 34 | why |", True, "path + space + lines, no colon"),
    ("| `a.rs` 161-165, 621-622 | category | why |", True,
     "spaced shape with ranges"),
    # --- counts / percentages are NOT line numbers ---------------------------
    ("| `a.rs` 2 tests | pass |", False, "count of tests, not a line number"),
    ("| `src/logfile.rs` | 227/229, 2 | 227/229 | 2 | 99.13 |", False,
     "line numbers in a LATER cell (the 8 still-blind docs)"),
    # --- trailing prose after the numbers ------------------------------------
    ("| `providers/imessage.rs` 464-498 (33 lines) | note |", False,
     "spaced shape with trailing prose"),
    ("| `src/a.rs:12` (see §3) | why |", False, "prose after the numbers"),
    # --- prose / tag cells that must NOT open the tag-first door -------------
    ("| plain prose | `src/a.rs:12` | why |", False,
     "any short word must NOT work as a tag (the tightened class)"),
    ("| category | `src/a.rs:200` | why |", False,
     "a lower-case word is not a tag"),
    ("| DDD12 | `src/a.rs:200` | why |", False, "three-letter prefix, out of class"),
    ("| D1234 | `src/a.rs:200` | why |", False, "four-digit tag, out of class"),
    ("| D12x | `src/a.rs:200` | why |", False, "trailing junk on the tag"),
    ("| see §4 of the previous campaign |", False, "prose row, no path"),
    ("| --- | --- | --- |", False, "table separator"),
    # --- real rows from the 8 docs whose ledgers must be RESHAPED, not parsed ----
    # These are the reason the fix for those docs is doc-side, not another pattern:
    # every one of them LOOKS like a claim to a looser rule and is not one.
    ("| `llm/schema.rs` 2, `llm/adapters/anthropic.rs` 1 | 3 | test name |", False,
     "module-agent.md: a COUNT per file, not line numbers (a looser rule reads it "
     "as claims about line 2 and line 1)"),
    ("| `agent/src/logfile.rs` | 227/229, 2 | 227/229 | 2 | 99.13 |", False,
     "module-agent-session.md: coverage counts in later cells, slashed"),
    ("| `desktop/src-tauri/src/runtime.rs` | unreachable-by-construction | Lines "
     "44-49 are the `#[cfg(not(test))]` twin |", False,
     "module-tauri-core.md: the line range is prose inside a later cell"),
    ("| `desktop/src-tauri/src/windows_power.rs` (waived in §5.1 — listed for size "
     "only) | 42 | `unreachable-in-this-environment` | why |", False,
     "module-tauri.md: cell 1 is the path plus prose, the count is cell 2"),
    ("| `future-cli` main / `bin.rs` / `event_wire_contract.rs` | 2 / 11 / 2 | ok |",
     False, "module-cli-tui.md: several files and counts in one cell, slashed"),
]


def main():
    contrast = "--contrast" in sys.argv
    names = [n for n, _ in lr.shipped_patterns(rec)]
    bad = 0
    flips = 0
    print("shipped shapes:", ", ".join(names))
    for row, expect, why in CASES:
        m = lr.shipped_match(row, rec)
        got = m is not None
        which = "?"
        if got:
            for name, pat in lr.shipped_patterns(rec):
                if pat.match(row):
                    which = name
                    break
        verdict = "ok  " if got == expect else "FAIL"
        if got != expect:
            bad += 1
        cell = f"{m.group(1)}:{m.group(2)}" if got else "-"
        extra = ""
        if contrast:
            saved = rec.LEDGER_ROW_TAGGED
            rec.LEDGER_ROW_TAGGED = LOOSE
            try:
                loose_got = lr.shipped_match(row, rec) is not None
            finally:
                rec.LEDGER_ROW_TAGGED = saved
            if loose_got != got:
                flips += 1
                extra = (f"   [pre-tightening tag class: "
                         f"{'accept' if loose_got else 'reject'}]")
        print(f"  {verdict} {'accept' if got else 'reject':6s} by {which:18s} "
              f"{cell:28s} {why}{extra}")
    print()
    print(f"cases={len(CASES)} mismatches={bad}" + (
        f" cases flipped by the pre-tightening tag class={flips}"
        if contrast else ""))
    if bad:
        print("A mismatch means a `LEDGER_ROW*` pattern changed behaviour: a false "
              "accept is a false-STALE risk, a false reject hides rows.")
    if contrast and not flips:
        print("WARNING: the contrast run found no difference -- this probe cannot "
              "discriminate the tag classes, so it is not evidence either way.")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
