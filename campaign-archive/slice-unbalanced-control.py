#!/usr/bin/env python3
"""Control for the UNBALANCED-FENCE failure mode of the fence-aware slice_section
(found by the app-data unit 2026-10-17, after my F.70 fix made the slicer
fence-aware).

If a document contains an ODD number of ``` markers BEFORE the ledger heading, the
slicer's fence state is inverted from that point on: every real heading is treated as
"inside a fence", so no heading matches and the slice is None - i.e. the tripwire
becomes inoperative for that document, SILENTLY (callers see "section not found").

Usage: slice-unbalanced-control.py
"""
import importlib.util, sys

spec = importlib.util.spec_from_file_location('reconcile', '.future/cov100/reconcile.py')
rec = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rec)

BALANCED = (
    "## 0. Intro\n\ntext\n\n```\ncode one\n```\n\n"
    "## 0.5 Residual ledger\n\n| row | line |\n|---|---|\n| `mod.rs` 12 | uncovered |\n"
)
UNBALANCED = (
    "## 0. Intro\n\ntext\n\n```\ncode one\n\n"          # <- never closed
    "## 0.5 Residual ledger\n\n| row | line |\n|---|---|\n| `mod.rs` 12 | uncovered |\n"
)

for name, doc in (("balanced", BALANCED), ("UNBALANCED (odd fence count)", UNBALANCED)):
    s = rec.slice_section(doc, "Residual ledger")
    if s is None:
        print(f"{name:28s} slice=None  -> caller sees 'section not found' (SILENT)")
    else:
        print(f"{name:28s} slice={len(s):4d} chars, row inside={'mod.rs' in s}")
