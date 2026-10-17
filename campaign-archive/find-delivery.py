#!/usr/bin/env python3
"""Print the delivery / todo events for one todo id from the goal event log.
Usage: find-delivery.py <todo_id> [max_chars]
"""
import json, sys

TODO = sys.argv[1]
MAX = int(sys.argv[2]) if len(sys.argv) > 2 else 3000
EV = '.future/goal/goals/goal_160bc247aa63/events.jsonl'

for line in open(EV):
    try:
        d = json.loads(line)
    except Exception:
        continue
    if TODO not in line:
        continue
    typ = d.get('type') or d.get('kind') or ''
    ts = d.get('ts') or d.get('timestamp') or ''
    print(f"--- {ts} {typ}")
    print(json.dumps(d, ensure_ascii=False)[:MAX])
    print()
