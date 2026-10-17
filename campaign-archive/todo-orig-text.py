#!/usr/bin/env python3
"""Print the ORIGINAL todo_added text for a todo (what a later `todo update --text`
replaced). Needed because replacing text can silently drop specifics that the
replacement only POINTS at (F.60/F.64 family: verify what you overwrote).
Usage: todo-orig-text.py <todo_id>
"""
import json, sys

TODO = sys.argv[1]
EV = '.future/goal/goals/goal_160bc247aa63/events.jsonl'

for line in open(EV):
    if TODO not in line:
        continue
    try:
        d = json.loads(line)
    except Exception:
        continue
    if d.get('kind') == 'todo_added' and d.get('todo', {}).get('id') == TODO:
        print(d['todo'].get('text', ''))
        break
