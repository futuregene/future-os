#!/usr/bin/env python3
"""Census: which docs have ODD code-fence parity (F.72)?

An odd count means one fence is never closed, so every heading after it is invisible
to the fence-aware slicer - i.e. the tripwire and the GATE stop covering that document,
and the only symptom was a misleading "no heading contains X".

Usage: fence-parity-census.py [root ...]
"""
import importlib.util, os, sys

spec = importlib.util.spec_from_file_location('reconcile', '.future/cov100/reconcile.py')
rec = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rec)

roots = sys.argv[1:] or ['docs', '.future/cov100']
files = []
for r in roots:
    if os.path.isfile(r):
        files.append(r)
    else:
        for dp, _, names in os.walk(r):
            files += [os.path.join(dp, n) for n in names if n.endswith('.md')]

bad = 0
for path in sorted(files):
    try:
        text = open(path, encoding='utf-8', errors='replace').read()
    except OSError:
        continue
    n, first_open = rec.fence_parity(text)
    if first_open is not None:
        bad += 1
        print(f"  ODD ({n:3d}) {path}  fence opened at line {first_open} is never closed")

print(f"\n{len(files)} markdown files scanned, {bad} with odd fence parity")
