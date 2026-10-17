#!/usr/bin/env python3
"""Progress board: for each running unit, how long since it last did anything and
what it last said. Cheap supervisor-side triage — reading journals costs no worker
turns, and a unit that has been silent for 15+ minutes is worth a look before it
burns an hour.

Usage: progress.py [--idle SECS]
"""
import json
import os
import sys
import time

RUNS = "/Users/geilige/future-os/.worktrees/cov100-r2/.future/goal/runs"
idle_report = 900
if "--idle" in sys.argv:
    idle_report = int(sys.argv[sys.argv.index("--idle") + 1])

rows = []
for name in os.listdir(RUNS):
    path = os.path.join(RUNS, name)
    hdr = None
    tools = 0
    last = 0
    txt = []
    for line in open(path, errors="replace"):
        try:
            e = json.loads(line)
        except Exception:
            continue
        t = e.get("type")
        if t == "run_header":
            hdr = e
        elif t == "tool_start":
            tools += 1
        elif t == "text_chunk" and isinstance(e.get("text"), str):
            txt.append(e["text"])
        if e.get("wall_ts"):
            last = e["wall_ts"]
    if not hdr:
        continue
    rows.append((last, hdr.get("agent_id"), hdr.get("todo_id"), tools,
                 len(txt), os.path.getsize(path) // 1024, "".join(txt)))

rows.sort(reverse=True)
now = int(time.time())
print(f"{'agent':24s} {'idle':>6s} {'tools':>6s} {'journal':>8s}  last words")
for last, agent, todo, tools, ntxt, kb, blob in rows:
    idle = now - (last or 0)
    if idle > idle_report:
        flag = "  <-- SILENT"
    else:
        flag = ""
    tail = blob[-150:].replace("\n", " ") if blob else "(no text)"
    print(f"{agent:24s} {idle:5d}s {tools:6d} {kb:7d}KB {tail}{flag}")
