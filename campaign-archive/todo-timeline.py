#!/usr/bin/env python3
"""Timeline for one or more todos: every lifecycle event with its wall clock, the
effective acceptance at that moment, and each run's outcome. The supervisor's tool
for deciding whether a notification is news or a stale re-delivery.

Usage: todo-timeline.py <todo_id> [<todo_id> ...]
"""
import datetime
import json
import sys

EV = "/Users/geilige/future-os/.worktrees/cov100-r2/.future/goal/goals/goal_160bc247aa63/events.jsonl"
want = set(sys.argv[1:]) or None

rows = []
for line in open(EV, errors="replace"):
    try:
        e = json.loads(line)
    except Exception:
        continue
    tid = e.get("todo_id") or (e.get("todo") or {}).get("id")
    if not tid or (want and tid not in want):
        continue
    k = e.get("kind")
    if k not in ("todo_added", "todo_claimed", "todo_updated", "todo_completed",
                 "todo_superseded", "run_recorded", "delivery_outcome_recorded",
                 "workspace_lock_acquired", "supervisor_note"):
        continue
    ts = datetime.datetime.fromtimestamp(e.get("ts", 0)).strftime("%H:%M:%S")
    ex = ""
    if k == "todo_added":
        t = e["todo"]
        ex = f"owner={t.get('owner')} acc={t.get('acceptance')!r}"
    elif k == "todo_updated":
        bits = []
        for f in ("acceptance", "owner", "status"):
            if e.get(f) is not None:
                bits.append(f"{f}={e.get(f)!r}")
        ex = " ".join(bits)
    elif k == "run_recorded":
        r = e["record"]
        state = r.get("terminal_state")
        err = (r.get("error") or "")[:70]
        ex = f"turn={r.get('turn')} state={state} {'ERR: ' + err if err else 'ok'}"
    elif k == "delivery_outcome_recorded":
        ex = f"outcome={e.get('outcome')} note={(e.get('note') or '')[:60]!r}"
    elif k == "workspace_lock_acquired":
        ex = f"paths={len(e.get('paths') or [])}"
    elif k == "supervisor_note":
        ex = f"{e.get('note_kind')}: {(e.get('message') or '')[:70]}"
    rows.append((e.get("ts", 0), ts, tid, k, ex))

for _, ts, tid, k, ex in sorted(rows):
    print(f"{ts} {tid:22s} {k:26s} {ex}")
