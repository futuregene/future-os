#!/usr/bin/env python3
"""Which currently-running turns were claimed while a non-empty acceptance
contract was in force? Those can still end on a token mismatch (the contract is
snapshotted at claim time). Reports the claim time and the contract in force
then, so the supervisor knows whether the noise is over or still coming."""
import datetime
import json

EV = "/Users/geilige/future-os/.worktrees/cov100-r2/.future/goal/goals/goal_160bc247aa63/events.jsonl"

# acceptance timeline per todo
acc = {}
claims = {}
status = {}
for line in open(EV, errors="replace"):
    try:
        e = json.loads(line)
    except Exception:
        continue
    k = e.get("kind")
    tid = e.get("todo_id") or (e.get("todo") or {}).get("id")
    if k == "todo_added":
        acc[tid] = (e.get("ts", 0), e["todo"].get("acceptance"))
        status[tid] = e["todo"].get("status") or "open"
    elif k == "todo_updated":
        if e.get("acceptance") is not None:
            acc[tid] = (e.get("ts", 0), e.get("acceptance"))
        if e.get("status"):
            status[tid] = e.get("status")
    elif k == "todo_claimed" and tid:
        claims.setdefault(tid, []).append(e.get("ts", 0))
    elif k == "todo_completed":
        status[tid] = "done"
    elif k == "todo_superseded":
        status[tid] = "superseded"

print("open todos whose LAST claim happened while a contract was in force:")
n = 0
for tid, ts_list in sorted(claims.items()):
    if status.get(tid) not in ("open", "blocked"):
        continue
    last = max(ts_list)
    # contract in force at that moment
    ts_a, a = acc.get(tid, (0, None))
    effective = a if ts_a <= last else None
    if effective:
        when = datetime.datetime.fromtimestamp(last).strftime("%H:%M:%S")
        print(f"  {tid} claimed {when} -> contract {effective!r}")
        n += 1
print("at risk:", n, "(0 = the token-mismatch noise is over)")
