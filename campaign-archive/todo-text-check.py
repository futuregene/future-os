#!/usr/bin/env python3
"""Verify the EFFECTIVE text of a todo contains given marker strings, and report
its length. Exits non-zero if any marker is missing - so an overwrite that drops
specifics is caught by machine, not by re-reading (F.60/F.64 family).
Usage: todo-text-check.py <todo_id> <marker> [<marker> ...]
"""
import json, sys

TODO = sys.argv[1]
MARKERS = sys.argv[2:]
EV = '.future/goal/goals/goal_160bc247aa63/events.jsonl'

text = None
for line in open(EV):
    if TODO not in line:
        continue
    try:
        d = json.loads(line)
    except Exception:
        continue
    if d.get('kind') == 'todo_added' and d.get('todo', {}).get('id') == TODO:
        text = d['todo'].get('text', '')
    elif d.get('kind') == 'todo_updated' and d.get('todo_id') == TODO and d.get('text'):
        text = d['text']

if text is None:
    print(f"{TODO}: no text found")
    sys.exit(1)

print(f"{TODO}: effective text {len(text)} chars")
missing = 0
for m in MARKERS:
    n = text.count(m)
    print(f"  {m:24s} {n}")
    if n == 0:
        missing += 1
if missing:
    print(f"FAIL: {missing} marker(s) missing")
    sys.exit(1)
print("OK: every marker present")
