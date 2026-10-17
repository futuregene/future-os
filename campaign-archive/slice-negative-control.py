#!/usr/bin/env python3
"""Negative control for selftest section 11: the OLD (fence-blind) slice_section,
verbatim from the pre-fix implementation, must FAIL both fixtures that the new
implementation passes. A regression check that cannot fail is worthless.

Usage: slice-negative-control.py
"""
import re

def old_slice(doc, needle):
    hm = re.search(r"(?m)^#{1,6}.*" + re.escape(needle) + r".*$", doc)
    if not hm:
        return None
    start = hm.start()
    level = len(hm.group(0)) - len(hm.group(0).lstrip("#"))
    body_from = hm.end() + 1
    pat = r"(?m)^#{1,%d} " % level
    m = re.search(pat, doc[min(body_from, len(doc)):])
    end = body_from + m.start() if m else len(doc)
    return doc[start:end]

doc1 = (
    "## 0.5 Residual ledger\n\nHow this is checked:\n\n"
    "```\npython3 reconcile.py --section \"Residual ledger\"\n"
    "# -> files with uncovered lines=43 uncovered lines=202\n"
    "#    MISSING (uncovered, not mentioned): 0\n```\n\n"
    "| row | line |\n|---|---|\n| `mod.rs` 12 | covered |\n"
)
s1 = old_slice(doc1, "Residual ledger") or ""
f1 = "mod.rs" not in s1
print(f"fixture 1 (fence-level #):  ledger row in slice = {'mod.rs' in s1}  slice={len(s1)}  -> OLD FAILS = {f1}")

doc2 = (
    "## 1. Real\n\ntext\n\n```\n## 2. Second real section (a fenced EXAMPLE of it)\n```\n\n"
    "## 2. Second real section\n\nbody two\n"
)
s2 = old_slice(doc2, "2. Second real section") or ""
f2 = "body two" not in s2
print(f"fixture 2 (fenced heading): body in slice = {'body two' in s2}  slice={len(s2)}  -> OLD FAILS = {f2}")
print()
print(f"EXPECTED: both fixtures show OLD FAILS = True. Got: {f1} and {f2}")
