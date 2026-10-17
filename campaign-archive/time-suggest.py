#!/usr/bin/env python3
"""Time one `suggest_reports()` call, to find where `staletable.py --suggest` spends its
300+ seconds (F.74: the endgame's documented repair path times out).

Usage: time-suggest.py [label]
"""
import importlib.util, json, time, sys

def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod

st = load('st', '.future/cov100/staletable.py')
rec = load('rec', '.future/cov100/reconcile.py')
st.rec = rec

label = sys.argv[1] if len(sys.argv) > 1 else 'goal'
pairings = json.load(open('.future/cov100/pairings.json'))['pairings']
p = [x for x in pairings if x['label'] == label][0]
scopes = [s.replace('${ROOT}', '/Users/geilige/future-os/.worktrees/cov100-r2')
          for s in p['scopes']]

t = time.time()
m = st.measure(rec, p['report'], scopes, p['doc'], p.get('section')) or {}
print(f"measure():        {time.time() - t:6.1f}s")

t = time.time()
c = st.suggest_reports(rec, p, m)
print(f"suggest_reports(): {time.time() - t:6.1f}s  -> {len(c)} candidates")
