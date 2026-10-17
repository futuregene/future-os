#!/usr/bin/env python3
"""Cost of the acceptance-token mechanism: turns that ended on a token mismatch.
Kept as evidence for INCIDENTS.md section A — a design error of the supervisor's,
measured rather than estimated."""
import json

EV = "/Users/geilige/future-os/.worktrees/cov100-r2/.future/goal/goals/goal_160bc247aa63/events.jsonl"
total = 0.0
n = 0
names = []
for line in open(EV, errors="replace"):
    try:
        e = json.loads(line)
    except Exception:
        continue
    if e.get("kind") != "run_recorded":
        continue
    r = e["record"]
    err = r.get("error") or ""
    if "acceptance contract unmet" in err:
        total += r.get("cost_delta") or 0
        n += 1
        names.append("{} t{}".format(r.get("agent_id"), r.get("turn")))
print("{} turns ended on an acceptance-token mismatch, {:.2f} credits".format(n, total))
for x in names:
    print("   ", x)
