#!/usr/bin/env python3
"""Read back the LATEST gate_resolved decision for one or more todo ids.
Exits non-zero if any decision is empty - an empty decision is indistinguishable
from a recorded one in the command's own success output, which is exactly how an
empty string once clobbered a real decision in this campaign (F.64).
Usage: gate-readback.py <todo_id> [...]
"""
import json, sys

EV = '.future/goal/goals/goal_160bc247aa63/events.jsonl'
want = set(sys.argv[1:])
latest = {}
for line in open(EV):
    try:
        d = json.loads(line)
    except Exception:
        continue
    if d.get('kind') == 'gate_resolved' and d.get('todo_id') in want:
        latest[d['todo_id']] = d  # last wins

bad = 0
for t in sys.argv[1:]:
    d = latest.get(t)
    if not d:
        print(f"{t}: NO gate_resolved event")
        bad += 1
        continue
    dec = d.get('decision') or ''
    print(f"{t}: decision_chars={len(dec)}")
    print(f"   head: {dec[:110]}")
    print(f"   tail: {dec[-110:]}")
    if not dec.strip():
        bad += 1
if bad:
    print(f"FAIL: {bad} empty/missing decision(s)")
    sys.exit(1)
print("OK: every decision is non-empty")
