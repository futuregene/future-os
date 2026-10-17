#!/usr/bin/env python3
"""Control for the reconcile.py section-slice defect (TAU/agent-svc finding, 2026-10-17).

slice_section() stops at the next line matching `^#{1,LEVEL} ` and does NOT skip
fenced code blocks. So a `#` at COLUMN 0 inside a fence - e.g. a pasted example
  # -> mentioned in foo.md: 202
  #    MISSING (uncovered, not mentioned): 0
terminates the slice early and everything below it (the ledger rows) becomes
invisible, which reads as MISSING.

Usage: slice-control.py   (no arguments; builds a temp doc and prints the verdict)
"""
import sys

sys.path.insert(0, '.future/cov100')
import importlib.util

spec = importlib.util.spec_from_file_location('reconcile', '.future/cov100/reconcile.py')
rec = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rec)

DOC = """
## 0.5 Residual ledger

How this is checked:

```
python3 .future/cov100/reconcile.py --report /tmp/x.info --section "Residual ledger"
# -> files with uncovered lines=43 uncovered lines=202
#    MISSING (uncovered, not mentioned): 0
```

| row | line |
|---|---|
| `mod.rs` 12 | covered |
"""

DOC_INDENTED = DOC.replace('\n# ->', '\n  # ->').replace('\n#    MISSING', '\n  #    MISSING')

for name, doc in (('fence-level # (as originally written)', DOC),
                  ('indented # (the fix)', DOC_INDENTED)):
    s = rec.slice_section(doc, 'Residual ledger') or ''
    has_row = 'mod.rs' in s
    print(f"{name:38s} slice={len(s):4d} chars  ledger row inside slice: {has_row}")
