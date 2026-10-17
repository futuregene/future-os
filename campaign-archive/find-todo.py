#!/usr/bin/env python3
"""Find the newest todo whose text contains a marker string. Use this instead of
guessing an id or trusting a CLI's tail - F.53/F.57/F.66 all came from id/capture
mistakes, and the fix is to SEARCH the ledger for what you just created.
Usage: find-todo.py <marker substring> [max]
"""
import json, sys

MARK = sys.argv[1]
MAX = int(sys.argv[2]) if len(sys.argv) > 2 else 5
EV = '.future/goal/goals/goal_160bc247aa63/events.jsonl'

hits = []
for line in open(EV):
    if MARK not in line:
        continue
    try:
        d = json.loads(line)
    except Exception:
        continue
    if d.get('kind') == 'todo_added':
        t = d['todo']
        hits.append((d.get('ts'), t.get('id'), t.get('owner'), t.get('priority'),
                     (t.get('text') or '')[:90].replace('\n', ' ')))

for ts, tid, owner, prio, head in hits[-MAX:]:
    print(f"{tid} owner={owner} prio={prio} ts={ts}\n    {head}")
if not hits:
    print("no todo_added matches that marker")
