#!/usr/bin/env python3
"""Does the fence-blindness trap change any PAIRING's verdict today?

For every pairing in pairings.json, slice its doc with the CURRENT slice_section
(fence-blind) and with a fence-AWARE slice, and report where they differ. A pairing
whose section is truncated by the trap has had its gate verdict computed over a
slice that hides rows - which is a gate-correctness question, not a doc-example one.

Usage: pairing-slice-compare.py
"""
import importlib.util, json, re, sys

spec = importlib.util.spec_from_file_location('reconcile', '.future/cov100/reconcile.py')
rec = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rec)


def fence_aware_slice(doc, needle):
    """Same contract as rec.slice_section, but markdown-correct: a heading inside a
    fenced code block is not a heading."""
    hm = re.search(r"(?m)^#{1,6}.*" + re.escape(needle) + r".*$", doc)
    if not hm:
        return None
    start = hm.start()
    level = len(hm.group(0)) - len(hm.group(0).lstrip("#"))
    body_from = hm.end() + 1
    pat = re.compile(r"(?m)^#{1,%d} " % level)
    in_fence = False
    pos = body_from
    for line in doc[body_from:].split("\n"):
        if line.startswith("```"):
            in_fence = not in_fence
        elif not in_fence and pat.match(line):
            return doc[start:pos]
        pos += len(line) + 1
    return doc[start:]


pairings = json.load(open('.future/cov100/pairings.json'))['pairings']
differ = 0
for p in pairings:
    doc_path, section = p.get('doc'), p.get('section')
    if not doc_path or not section:
        continue
    try:
        doc = open(doc_path, encoding='utf-8', errors='replace').read()
    except OSError:
        print(f"  {p.get('label'):22s} doc missing: {doc_path}")
        continue
    cur = rec.slice_section(doc, section)
    safe = fence_aware_slice(doc, section)
    if cur is None or safe is None:
        continue
    if len(cur) != len(safe):
        differ += 1
        print(f"  DIFFERS  {p.get('label'):22s} section={section!r}\n"
              f"           current slice {len(cur)} chars, fence-aware {len(safe)} chars "
              f"(+{len(safe) - len(cur)})")
print(f"\n{len(pairings)} pairings checked, {differ} with a truncated slice")
