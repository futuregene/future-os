#!/usr/bin/env python3
"""Read back a recorded delivery outcome note (F.54/F.61 discipline: check the
LENGTH and the LAST line, not just the first, because a truncated note looks
like a complete one)."""
import json, sys

TODO = sys.argv[1]
EV = '.future/goal/goals/goal_160bc247aa63/events.jsonl'

for line in open(EV):
    if TODO not in line or 'delivery_outcome_recorded' not in line:
        continue
    d = json.loads(line)
    n = d.get('note') or ''
    print(f"outcome={d.get('outcome')} chars={len(n)}")
    print("FIRST 160:", n[:160].replace('\n', ' | '))
    print("LAST  160:", n[-160:].replace('\n', ' | '))
    print()
