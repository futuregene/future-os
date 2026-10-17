#!/usr/bin/env python3
"""For the pairings whose slice is fence-truncated, does the VERDICT change?

Runs staletable's own measure() twice per affected pairing - once through the current
fence-blind slice_section (i.e. exactly what the gate computes today) and once through
a fence-aware slice - and compares `missing`.

Usage: pairing-verdict-compare.py
"""
import importlib.util, json, re, sys

spec = importlib.util.spec_from_file_location('reconcile', '.future/cov100/reconcile.py')
rec = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rec)

spec2 = importlib.util.spec_from_file_location('staletable', '.future/cov100/staletable.py')
st = importlib.util.module_from_spec(spec2)
spec2.loader.exec_module(st)


def fence_aware_slice(doc, needle):
    hm = re.search(r"(?m)^#{1,6}.*" + re.escape(needle) + r".*$", doc)
    if not hm:
        return None
    start = hm.start()
    level = len(hm.group(0)) - len(hm.group(0).lstrip("#"))
    body_from = hm.end() + 1
    pat = re.compile(r"(?m)^#{1,%d} " % level)
    in_fence, pos = False, body_from
    for line in doc[body_from:].split("\n"):
        if line.startswith("```"):
            in_fence = not in_fence
        elif not in_fence and pat.match(line):
            return doc[start:pos]
        pos += len(line) + 1
    return doc[start:]


original = rec.slice_section
pairings = json.load(open('.future/cov100/pairings.json'))['pairings']
for p in pairings:
    doc, section = p.get('doc'), p.get('section')
    if not doc or not section:
        continue
    scopes = [s.replace('${ROOT}', '/Users/geilige/future-os/.worktrees/cov100-r2')
              for s in (p.get('scopes') or [])]
    text = open(doc, encoding='utf-8', errors='replace').read()
    cur = original(text, section)
    safe = fence_aware_slice(text, section)
    if cur is None or safe is None or len(cur) == len(safe):
        continue

    # the gate's own measure(), reached through whatever slice_section returns
    rec.slice_section = lambda t, n, _c=cur: _c
    st.rec = rec
    m_cur = st.measure(rec, p['report'], scopes, doc, section)
    rec.slice_section = lambda t, n, _s=safe: _s
    m_safe = st.measure(rec, p['report'], scopes, doc, section)
    rec.slice_section = original

    if isinstance(m_cur, dict):
        a, b = m_cur.get('missing'), m_safe.get('missing')
    else:
        a = b = m_cur
    if a != b and not (isinstance(a, list) and isinstance(b, list) and len(a) == len(b)):
        print(f"  VERDICT CHANGES  {p.get('label'):22s} section={section!r}: "
              f"missing {len(a) if isinstance(a, list) else a} -> "
              f"{len(b) if isinstance(b, list) else b}")
    else:
        print(f"  no verdict change {p.get('label'):22s} "
              f"(missing {len(a) if isinstance(a, list) else a} either way)")
