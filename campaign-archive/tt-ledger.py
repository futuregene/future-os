#!/usr/bin/env python3
"""Locate the ledger entries whose recorded commands SWALLOW --test-threads as a
filter, and name the todo each belongs to (so the affected RATES can be listed).

Rule: cargo splits at the first `--`; libtest stops option parsing at the first `--`
it then sees. So a further `--` before `--test-threads` inside the binary's args means
the flag is a test-name filter, not an option.

Usage: tt-ledger.py [events.jsonl]
"""
import json, re, sys

EV = sys.argv[1] if len(sys.argv) > 1 else '.future/goal/goals/goal_160bc247aa63/events.jsonl'
LINE = re.compile(r'(cargo\s+(?:llvm-cov|test)\b[^"\\]*?--test-threads\S*)')

n = 0
for raw in open(EV, encoding='utf-8', errors='replace'):
    try:
        d = json.loads(raw)
    except Exception:
        continue
    blob = json.dumps(d, ensure_ascii=False)
    for m in LINE.finditer(blob):
        cmd = m.group(1)
        parts = cmd.split(' -- ', 1)
        if len(parts) < 2:
            continue
        binargs = parts[1]
        tt = binargs.find('--test-threads')
        if tt < 0:
            continue
        n += 1
        if binargs.find(' -- ', 0, tt) >= 0:
            kind = d.get('kind')
            tid = d.get('todo_id') or (d.get('todo') or {}).get('id') or ''
            agent = d.get('agent_id') or ''
            print(f"SWALLOWED  ts={d.get('ts')} kind={kind} todo={tid} agent={agent}")
            print(f"    {cmd[:150]}")
print(f"\nscanned {n} invocations")
