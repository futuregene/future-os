#!/usr/bin/env python3
"""gate-drift.py - find blocked todos whose blockers are ALL done (a gate that has silently
opened), and report the LIVE todos in the same write scope so the supervisor can re-point them.

WHY THIS EXISTS: this campaign is segment-based, so a coverage unit finishes one todo and creates a
successor that keeps editing the same files. A flake/review todo gated behind the *first* todo then
becomes dispatchable while its files are still being written by the successor - three times now
(F-TAURI, F-TAURI-APP, F-AGENT). The kernel cannot see this: it honours the dependency graph it was
given, and the graph names a todo, not a file.

Usage: gate-drift.py [--json]
"""
import json
import subprocess
import sys

EV = "/Users/geilige/future-os/.worktrees/cov100-r2/.future/goal/goals/goal_160bc247aa63/events.jsonl"
GOAL = "goal_160bc247aa63"
CWD = "/Users/geilige/future-os/.worktrees/cov100-r2"

todos, blocks, status = {}, {}, {}
for line in open(EV, errors="replace"):
    try:
        e = json.loads(line)
    except Exception:
        continue
    k = e.get("kind")
    if k == "todo_added":
        t = e["todo"]
        todos[t["id"]] = t
        blocks[t["id"]] = list(t.get("blocks") or [])
        status[t["id"]] = (t.get("status") or "open")
    elif k == "todo_updated" and e.get("todo_id") in todos:
        if e.get("blocks") is not None:
            blocks[e["todo_id"]] = list(e["blocks"])
        if e.get("status"):
            status[e["todo_id"]] = e["status"]
    elif k == "todo_completed":
        status[e["todo_id"]] = "done"
    elif k == "todo_superseded":
        status[e["todo_id"]] = "superseded"

# who is running right now, and on what
w = subprocess.run(["future", "goal", "worker", "list", "--goal", GOAL],
                   cwd=CWD, capture_output=True, text=True).stdout
live = {}
for line in w.splitlines()[2:]:
    p = line.split()
    if len(p) >= 4 and p[1] == "running":
        live[p[3]] = p[0]

drift = []
for tid, bl in blocks.items():
    if status.get(tid) not in ("open", "blocked"):
        continue
    if not bl:
        continue
    done = [b for b in bl if status.get(b) in ("done", "superseded")]
    if len(done) != len(bl):
        continue                      # still genuinely gated
    scopes = [s for s in (todos[tid].get("required_write_scope") or [])]
    # which live todos write into this todo's scope?
    overlap = []
    for ltid, agent in live.items():
        if ltid == tid:
            continue        # a todo that is RUNNING matches its own scope; excluding it is what makes
                            # this check mean "a DIFFERENT live todo writes the same files" (without
                            # this, every running flake unit reported itself as blocked-in-fact)
        ls = todos.get(ltid, {})
        lscopes = ls.get("required_write_scope") or []
        for a in scopes:
            for b in lscopes:
                if a.rstrip("/") == b.rstrip("/") or a.startswith(b.rstrip("/") + "/") \
                   or b.startswith(a.rstrip("/") + "/"):
                    overlap.append((ltid, agent, a, b))
    drift.append({"todo": tid, "owner": todos[tid].get("owner"), "blockers_all_done": done,
                  "live_overlap": overlap})

if "--json" in sys.argv:
    print(json.dumps(drift, indent=2))
    raise SystemExit(0)

print(f"todos whose blockers are ALL done but which are not done: {len(drift)}")
for d in drift:
    print(f"\n  {d['todo']} owner={d['owner']}")
    scopes = [s for s in (todos[d["todo"]].get("required_write_scope") or [])]
    # REVIEW UNITS: this tool compares DECLARED WRITE SCOPES, which for a review unit is its report file.
    # REVIEW.md rule 1 requires something else as well - the REVIEWED MODULE must be frozen, because a
    # waiver cites file:line and any later edit shifts the numbering. That precondition is invisible here
    # (the reviewed paths are not in the review's write scope), so say so instead of printing
    # "SAFE TO DISPATCH": that verdict would be true about the wrong thing.
    #
    # NOTE ON DETECTION: an earlier version keyed on a scope of only `docs/testing/review-*.md`, which
    # MISSED this case because the todo's declared scope is EMPTY (owner-scoped reviews fall back to the
    # agent's onboarded workspace). Detect by the owner id as well - `worker-review-*` is the campaign's
    # naming and it is mechanical.
    owner = str(d["owner"] or "")
    is_review = owner.startswith("worker-review-") or (
        bool(scopes) and all(s.startswith("docs/testing/review-") for s in scopes))
    if d["live_overlap"]:
        print("    BLOCKED IN FACT: a live todo writes the same scope -> re-point --blocks:")
        for ltid, agent, a, b in d["live_overlap"][:4]:
            print(f"       {ltid} ({agent}) writes {b}  [scope overlap on {a}]")
    elif is_review:
        print("    REVIEW UNIT - this tool only checked ITS OWN write target (your report file).")
        print("    REVIEW.md rule 1 ALSO requires the REVIEWED module to be frozen: verify by hand that")
        print("    no live todo writes the paths the review will cite, and re-read the line numbers from")
        print("    the module record at review time. Do NOT treat the line above as 'safe to dispatch'.")
    elif status.get(d["todo"]) == "open" and d["todo"] in live:
        print(f"    ALREADY RUNNING ({live[d['todo']]}) - nothing to do")
    else:
        print("    SAFE TO DISPATCH: no live todo writes into its scope")
