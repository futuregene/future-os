#!/usr/bin/env python3
"""Print the FULL completion evidence for a todo (the CLI and my usual helpers show
only a truncated tail, which hides the measurement block at the top).
Usage: todo-evidence.py <todo_id> [head_chars]
"""
import json, sys

TODO = sys.argv[1]
N = int(sys.argv[2]) if len(sys.argv) > 2 else 4000
EV = '.future/goal/goals/goal_160bc247aa63/events.jsonl'

best = None
for line in open(EV):
    if TODO not in line:
        continue
    try:
        d = json.loads(line)
    except Exception:
        continue
    if d.get('kind') == 'todo_completed' and d.get('todo_id') == TODO:
        best = d.get('evidence') or ''

if best is None:
    print("no todo_completed evidence for", TODO)
    sys.exit(1)
print(f"evidence: {len(best)} chars")
print(best[:N])
if len(best) > N:
    print(f"\n... [{len(best) - N} more chars]")
