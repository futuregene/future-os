#!/usr/bin/env python3
"""Which `--verify` validators are STRUCTURALLY broken - i.e. fail regardless of the work?

WHY (measured 2026-10-17, todo_a2bc4dbd618d): a ticket carried
`python3 .future/cov100/reconcile.py --section S10.4`, which exits 2 with a USAGE ERROR because reconcile.py
requires --report/--scope/--doc. The unit got "verify gate rejected (exit 2)" twice and could not fix it:
`todo update` has no --verify flag. A validator that cannot start is worse than no validator, because it
produces a failure signal about the WORK when the fault is in the COMMAND - and this campaign has already lost
turns to contracts that could not be satisfied (F.83).

The check here is deliberately cheap and conservative: it runs the validator with a tiny timeout and reports
the ones that fail IMMEDIATELY with a usage/argument error, i.e. before they can be measuring anything. A
validator that fails after doing work is the unit's business, not this tool's.

usage: verify-census.py [--goal G] [--run]
  default: list every ticket with a validator and classify it by SHAPE (no execution)
  --run:   actually execute each validator (2 s timeout) and report exit 1/2/3 with the first stderr line
"""
import json
import subprocess
import sys

GOAL = "goal_160bc247aa63"
for i, a in enumerate(sys.argv):
    if a == "--goal" and i + 1 < len(sys.argv):
        GOAL = sys.argv[i + 1]
RUN = "--run" in sys.argv


def tickets():
    """Read validators from the LEDGER, not the projection.

    MEASURED (2026-10-17): `future goal status --format json` does not carry `validator` at all, so a census
    written against the projection reports "no ticket carries a validator" while 31 of them do. That is the
    F.71/F.78 family again - the projection is a summary, and a field it omits is not a field that is absent.
    The ledger has it, so read the ledger.
    """
    import os
    ev = os.path.join(".future", "goal", "goals", GOAL, "events.jsonl")
    rows = []
    try:
        with open(ev, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                if "todo_added" not in line:
                    continue
                try:
                    d = json.loads(line)
                except Exception:
                    continue
                t = d.get("todo") or {}
                if t.get("validator"):
                    rows.append((t["id"], t.get("status") or "?", t["validator"]))
    except FileNotFoundError:
        print(f"no ledger at {ev}")
    # Current status comes from the projection; the validator only from the ledger. Joining the two is the
    # honest form: either source alone misleads (the projection has no validator; todo_added has no status).
    cur = {}
    out = subprocess.run(["future", "goal", "status", "--goal", GOAL, "--format", "json"],
                         capture_output=True, text=True).stdout
    try:
        for g in json.loads(out):
            for t in g.get("todos", []):
                cur[t["id"]] = t.get("status") or "?"
    except Exception:
        pass
    return [(tid, cur.get(tid, "?"), v) for tid, _s, v in rows]


def main():
    rows = tickets()
    if not rows:
        print("no ticket carries a validator")
        return 0
    print(f"{len(rows)} ticket(s) carry a validator\n")
    broken = []
    for tid, status, v in rows:
        print(f"{tid}  {status[:14]:16s} {v[:100]}")
        if RUN:
            p = subprocess.run(v, shell=True, capture_output=True, text=True, timeout=20)
            first = ((p.stderr or "").strip().splitlines() or [""])[0][:100]
            tag = ""
            # exit 2 with an argparse usage error = the command never started
            if p.returncode == 2 and ("usage:" in (p.stderr or "").lower()
                                      or "required" in (p.stderr or "").lower()
                                      or "unrecognized" in (p.stderr or "").lower()):
                tag = "   <== STRUCTURALLY BROKEN (usage error)"
                broken.append(tid)
            print(f"      rc={p.returncode}{tag}  {first}")
    print()
    if broken:
        print(f"STRUCTURALLY BROKEN: {', '.join(broken)}")
        print("A validator that cannot start reports a failure about the WORK; tell the unit to ignore it,")
        print("state the evidence in the handoff instead, and note that a failing validator does not block")
        print("completion (PLAN 6a). `todo update` has no --verify, so it cannot be repaired in place.")
        return 1
    print("no structurally-broken validator found" if RUN else "(shapes only; re-run with --run to execute)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
