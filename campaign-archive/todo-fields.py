#!/usr/bin/env python3
"""Print a todo's EFFECTIVE fields: the todo_added payload overlaid with every
later todo_updated field for that id. Read the field NAMES from the event before
asserting a value (F.46) - this exists because guessing the key has cost several
rounds in this campaign.
Usage: todo-fields.py <todo_id>
"""
import json, sys

TODO = sys.argv[1]
EV = '.future/goal/goals/goal_160bc247aa63/events.jsonl'

state = {}
for line in open(EV):
    if TODO not in line:
        continue
    try:
        d = json.loads(line)
    except Exception:
        continue
    kind = d.get('kind')
    if kind == 'todo_added' and d['todo'].get('id') == TODO:
        state.update(d['todo'])
    elif kind == 'todo_updated' and d.get('todo_id') == TODO:
        for k, v in d.items():
            if k in ('event_id', 'kind', 'goal_id', 'todo_id', 'ts'):
                continue
            if v is not None:
                state[k] = v
    elif kind == 'todo_completed' and d.get('todo_id') == TODO:
        # Completion is its OWN event kind, not a todo_updated with status=done.
        # Missing this made the reader report "open" for finished todos, which
        # once nearly led me to conclude three flake units were still gated when
        # their gates were satisfied (2026-10-17).
        state['status'] = 'done'
    elif kind in ('todo_superseded', 'todo_archived') and d.get('todo_id') == TODO:
        state['status'] = kind.replace('todo_', '')

# NOTE: `blocks` (in todo_updated) and `blocked_by_gate` (in todo_added) are the SAME
# gate, under two different keys. Print both, or a set gate reads as None - which is a
# value-level version of F.46 (read the KEY before asserting the value).
for k in ('id', 'status', 'priority', 'owner', 'class', 'role', 'blocked_by_gate', 'blocks',
          'required_write_scope', 'acceptance', 'validator', 'no_follow_up'):
    print(f"{k}: {state.get(k)}")
t = state.get('text') or ''
print(f"text_chars: {len(t)}")
print("text_head:", t[:200].replace('\n', ' | '))
