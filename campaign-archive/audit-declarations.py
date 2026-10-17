#!/usr/bin/env python3
"""Audit open todos for declaration defects the guard cannot catch:

  * an owner id that was never onboarded (the todo then cannot be claimed by the
    agent actually running the unit — observed: a unit created its successor under
    a different name, `cov100-tauri-headless`);
  * an empty `required_write_scope` (the guard falls back to the agent's declared
    workspace, which is usually right but is not what the task meant — and the
    supervisor's snapshot attribution has nothing to match on).
"""
import json

EV = "/Users/geilige/future-os/.worktrees/cov100-r2/.future/goal/goals/goal_160bc247aa63/events.jsonl"

todos = {}
onboarded = set()
status = {}
for line in open(EV, errors="replace"):
    try:
        e = json.loads(line)
    except Exception:
        continue
    k = e.get("kind")
    if k == "todo_added":
        t = e["todo"]
        todos[t["id"]] = t
        status[t["id"]] = t.get("status") or "open"
    elif k == "todo_updated" and e.get("todo_id") in todos:
        if e.get("owner"):
            todos[e["todo_id"]]["owner"] = e["owner"]
        if e.get("status"):
            status[e["todo_id"]] = e["status"]
    elif k == "agent_onboarded":
        onboarded.add(e.get("agent_id"))
    elif k == "todo_completed":
        status[e["todo_id"]] = "done"
    elif k == "todo_superseded":
        status[e["todo_id"]] = "superseded"

print("onboarded agents:", ", ".join(sorted(a for a in onboarded if a)))
bad = 0
for tid, t in sorted(todos.items()):
    if status.get(tid) not in ("open", "blocked"):
        continue
    owner = t.get("owner")
    scope = t.get("required_write_scope") or []
    if not owner:
        continue
    if owner not in onboarded:
        print(f"  NOT ONBOARDED  {tid} owner={owner} scope_entries={len(scope)}")
        bad += 1
    elif not scope:
        print(f"  EMPTY SCOPE    {tid} owner={owner}")
        bad += 1
print("problems found:", bad)
