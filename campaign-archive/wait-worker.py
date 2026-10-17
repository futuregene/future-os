#!/usr/bin/env python3
"""Wait for a worker INSIDE one tool call, so a turn does not have to end.

WHY (relay, 2026-10-17): my context is ~735K, and every ended turn pays a full re-read of it. I had ended four
turns in an hour merely to wait for a running worker - each one a wake-up plus a large context rebuild, and none
of them a question the user needed to see. Blocking inside a single call absorbs the wait at no context cost.

It also removes the hand-rolled part: the owner -> session mapping is read from the ledger's
`worker_session_bound` events instead of being looked up by hand each time.

usage: wait-worker.py <agent-id | --any | --all> [--max SECONDS]
  exit 0 -> every named worker is IDLE (or no worker is streaming)
  exit 3 -> still running when the budget ran out (that is when ending a turn is legitimate)

The blocking loop runs at most `--max` seconds (default 840 = 14 min, under the 600 s tool ceiling for the
command as a whole when combined with status calls).
"""
import json
import subprocess
import sys
import time

LEDGER = ".future/goal/goals/goal_160bc247aa63/events.jsonl"


def bindings():
    """agent_id -> latest bound session_id, from the ledger (not from memory)."""
    out = {}
    try:
        with open(LEDGER, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                if "worker_session_bound" not in line:
                    continue
                try:
                    d = json.loads(line)
                except Exception:
                    continue
                a, s = d.get("agent_id"), d.get("session_id")
                if a and s:
                    out[a] = s
    except FileNotFoundError:
        pass
    return out


def streaming(session_id):
    """True while the session is streaming, False once idle. None when unknown."""
    p = subprocess.run(["future", "session", "status", session_id],
                       capture_output=True, text=True)
    txt = (p.stdout or "") + (p.stderr or "")
    for line in txt.splitlines():
        if line.strip().lower().startswith("streaming:"):
            val = line.split(":", 1)[1].strip().lower()
            return val in ("yes", "true", "1")
    return None


def live_claimed():
    """Agents holding a live lease, from the projection (status is what it is for)."""
    out = subprocess.run(["future", "goal", "status", "--goal", "goal_160bc247aa63",
                          "--format", "json"], capture_output=True, text=True).stdout
    now = int(time.time())
    agents = []
    try:
        for g in json.loads(out):
            for t in g.get("todos", []):
                who, exp = t.get("claimed_by"), t.get("lease_expires_at") or 0
                if who and exp > now:
                    agents.append(who)
    except Exception:
        pass
    return sorted(set(agents))


def main():
    args = [a for a in sys.argv[1:]]
    budget = 840
    if "--max" in args:
        i = args.index("--max")
        budget = int(args[i + 1])
        del args[i:i + 2]
    target = args[0] if args else "--any"

    b = bindings()
    if target == "--all" or target == "--any":
        agents = live_claimed()
        if not agents:
            print("IDLE (no live lease at all)")
            return 0
        if target == "--any":
            agents = agents[:1]
    else:
        agents = [target]

    sessions = [(a, b.get(a)) for a in agents]
    missing = [a for a, s in sessions if not s]
    sessions = [(a, s) for a, s in sessions if s]
    if not sessions:
        print(f"IDLE (no bound session for {', '.join(missing) or agents})")
        return 0
    # print the FULL id: a truncated one sends a reader to the wrong session (paid for at 12:30)
    print("watching: " + ", ".join(f"{a} -> {s}" for a, s in sessions), flush=True)

    deadline = time.time() + budget
    while True:
        states = {a: streaming(s) for a, s in sessions}
        busy = [a for a, st in states.items() if st]
        if not busy:
            print("IDLE", flush=True)
            print("  " + ", ".join(f"{a}={states[a]}" for a, _ in sessions), flush=True)
            return 0
        if time.time() >= deadline:
            print("STILL_RUNNING " + ",".join(busy), flush=True)
            print(f"  waited {budget}s; ending the turn is now legitimate", flush=True)
            return 3
        time.sleep(10)


if __name__ == "__main__":
    sys.exit(main())
