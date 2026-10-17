#!/usr/bin/env python3
"""Verify the F.72 guard end to end:

  1. partition of a balanced doc            -> slice returned (unchanged behaviour)
  2. an UNBALANCED doc, section NOT found   -> DocFenceError naming the fence line
  3. an UNBALANCED doc, section found BEFORE the stray fence -> slice returned
     (a false alarm would be worse than the old message: this must NOT raise)
  4. negative control: the pre-F.72 slicer on case 2 returned None (blaming the
     heading), which is exactly what the guard replaces

Usage: slice-fence-guard-control.py
"""
import importlib.util, re, sys

spec = importlib.util.spec_from_file_location('reconcile', '.future/cov100/reconcile.py')
rec = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rec)

BAL = ("## 0. Intro\n\ntext\n\n```\ncode\n```\n\n"
       "## 0.5 Ledger\n\n| a | b |\n|---|---|\n| `x.rs` 1 | 0 |\n")

# stray fence AFTER the ledger heading: the ledger is still reachable
UNBAL_AFTER = ("## 0.5 Ledger\n\n| a | b |\n|---|---|\n| `x.rs` 1 | 0 |\n\n"
               "```\nnever closed\n")
# stray fence BEFORE the ledger heading: every later heading is invisible
UNBAL_BEFORE = ("## 0. Intro\n\ntext\n\n```\nnever closed\n\n"
                "## 0.5 Ledger\n\n| a | b |\n|---|---|\n| `x.rs` 1 | 0 |\n")

def old_slice(doc, needle):          # the pre-F.72 behaviour, verbatim
    hm = re.search(r"(?m)^#{1,6}.*" + re.escape(needle) + r".*$", doc)
    if not hm:
        return None
    start = hm.start()
    level = len(hm.group(0)) - len(hm.group(0).lstrip("#"))
    body_from = hm.end() + 1
    pat = r"(?m)^#{1,%d} " % level
    m = re.search(pat, doc[min(body_from, len(doc)):])
    return doc[start:(body_from + m.start() if m else len(doc))]

def run_case(n, doc, needle, want):
    try:
        s = rec.slice_section(doc, needle)
        got = f"slice {len(s)} chars, row={'x.rs' in s}" if s else "None"
    except rec.DocFenceError as e:
        got = f"DocFenceError: {e}"
    ok = (want == 'slice') and got.startswith('slice') or \
         (want == 'raise') and got.startswith('DocFenceError')
    print(f"  {'ok  ' if ok else 'FAIL'} case {n}: {got}")

print("balanced doc, ledger reachable:")
run_case(1, BAL, "0.5 Ledger", 'slice')
print("UNBALANCED, stray fence BEFORE the heading (headings invisible):")
run_case(2, UNBAL_BEFORE, "0.5 Ledger", 'raise')
print("UNBALANCED, stray fence AFTER the heading (must NOT raise - false alarm):")
run_case(3, UNBAL_AFTER, "0.5 Ledger", 'slice')
print()
print(f"negative control (pre-F.72 slicer on case 2): {old_slice(UNBAL_BEFORE, '0.5 Ledger')!r}"
      "  <- blames the heading, which is what the guard replaces")
