#!/usr/bin/env python3
"""Which STAGED files belong to a unit that is STILL RUNNING?

WHY (measured twice, 2026-10-17): `snapshot.sh` commits in-flight work, which is its job - units
cannot commit. But a unit that is still running can REVISE what I just committed. Both known
instances went wrong that way:

  * the tauri-flake unit implemented a cfg(test) 60 s tolerance, the snapshot committed it, and the
    unit then MEASURED it and WITHDREW it - so HEAD carried a change its author had already rejected
    (fixed in commit 5ac746aff);
  * an earlier snapshot swept `shadow_review/snapshot.rs` in while its owner was mid-run.

No marker check can catch this: there is no `TEMP DIAG` in a withdrawn budget, and the diff looks
like deliverable code. What IS knowable at commit time is the thing that predicts the risk - **the
file's owner still has a live lease** - so this prints it, and the correct response is to note the
ticket and check that unit's NEXT delivery for a withdrawal (`grep -i 'withdraw\\|reverted'`).

Not a refusal: committing a running unit's work is usually right (it is how the campaign moves).
It is a "watch this" list. Exit status is always 0 - it is an input to a decision, not a gate.
"""
import json
import os
import re
import subprocess
import sys
import time

EV = ".future/goal/goals/goal_160bc247aa63/events.jsonl"


def live_holders():
    """agent_id -> (todo_id, expires_at) for every todo with an unexpired lease."""
    out = subprocess.run(["future", "goal", "status", "--goal", "goal_160bc247aa63",
                          "--format", "json"], capture_output=True, text=True).stdout
    try:
        goals = json.loads(out)
    except Exception:
        return {}
    now = int(time.time())
    holders = {}
    for g in goals:
        for t in g.get("todos", []):
            who, exp = t.get("claimed_by"), t.get("lease_expires_at") or 0
            if who and exp > now:
                holders.setdefault(who, []).append((t["id"], exp))
    return holders


def run_started_at(todo_id):
    """When did this ticket's current run begin? Used by the mtime check below."""
    best = None
    try:
        with open(EV) as fh:
            for line in fh:
                if todo_id not in line:
                    continue
                try:
                    d = json.loads(line)
                except Exception:
                    continue
                if (d.get("kind") or d.get("type")) == "todo_claimed" and d.get("todo_id") == todo_id:
                    ts = d.get("ts") or 0
                    if ts and (best is None or ts > best):
                        best = ts
    except FileNotFoundError:
        pass
    return best


def touched_since(paths, since, prefixes):
    """Staged files under `prefixes` whose mtime is NEWER than `since`.

    WHY (F.84a, measured 2026-10-17): the workspace check above keys off the agent's ONBOARDED
    workspaces. A supervisor-granted scope extension lives in the TICKET TEXT, not in the agent's
    profile - so when I granted one unit `agent/src/session/persistence.rs`, `git add -u` swept its
    in-flight patch into my commit and the guard said nothing, because that path is not in the
    agent's declared workspaces. The mtime is the fact that does not depend on declarations.

    `prefixes` keeps it from becoming an alarm that always fires (the campaign has recorded that
    failure twice): a file touched after the run began is only interesting when it lives under the
    same TOP-LEVEL tree the agent works in (`agent/`, `cli/`, `desktop/`, `docs/`...), not when it
    is my own tooling under `.future/`. Without that, every commit during a live run would warn.
    """
    hits = []
    for p in paths:
        top = p.split("/")[0] + "/"
        if top not in prefixes:
            continue
        try:
            if os.path.getmtime(p) > since:
                hits.append(p)
        except OSError:
            continue
    return hits


def top_trees(ws_list):
    """The top-level directories an agent's declared workspaces live in."""
    out = set()
    for w in ws_list:
        marker = "cov100-r2/"
        rel = w.split(marker)[-1] if marker in w else w
        if "/" in rel:
            out.add(rel.split("/")[0] + "/")
    return out


def workspaces():
    """agent_id -> declared workspaces, from the LAST agent_onboarded event per agent."""
    ws = {}
    try:
        with open(EV) as fh:
            for line in fh:
                if "agent_onboarded" not in line:
                    continue
                try:
                    d = json.loads(line)
                except Exception:
                    continue
                if d.get("agent_id"):
                    ws[d["agent_id"]] = d.get("workspaces") or []
    except FileNotFoundError:
        pass
    return ws


def _match(paths, holders, ws, root):
    """Pure matcher, so the control is deterministic and does not depend on live leases."""
    hits = []
    for path in paths:
        abspath = f"{root}/{path}"
        for agent, todo_list in holders.items():
            for declared in ws.get(agent, []):
                d = declared.rstrip("/")
                if abspath == d or abspath.startswith(d + "/"):
                    hits.append((path, agent, todo_list[0][0]))
    return sorted(set(hits))


def self_test():
    root = "/r"
    ws = {"worker-x": ["/r/desktop/src-tauri/src/remote_peer", "/r/docs/testing/x.md"]}
    holders = {"worker-x": [("todo_abc", time.time() + 600)]}
    ok = True

    # 1. the matcher itself
    cases = [
        (["desktop/src-tauri/src/remote_peer/session.rs"], 1, "a file UNDER the declared dir"),
        (["docs/testing/x.md"], 1, "an exact file match"),
        (["desktop/src-tauri/src/remote/session.rs"], 0,
         "a SIBLING whose name shares a string prefix - must NOT flag (the kernel uses "
         "Path::starts_with, which is component-wise)"),
    ]
    for paths, want, why in cases:
        got = len(_match(paths, holders, ws, root))
        verdict = "ok  " if got == want else "FAIL"
        if got != want:
            ok = False
        print(f"  {verdict} matcher: {got} hit(s), wanted {want}: {why}")

    # 2. the PLUMBING - the case that hid the missing workspaces() call. A dict with two keys
    #    unpacked into two names gives two STRINGS, and with nothing staged the loop never ran.
    lines, rc = run(["desktop/src-tauri/src/remote_peer/session.rs"], holders, ws, root)
    good = rc == 0 and any("worker-x" in l for l in lines) and "STILL RUNNING" in lines[0]
    print(f"  {'ok  ' if good else 'FAIL'} plumbing: a flagged file reports the owner and its ticket")
    ok = ok and good

    lines, rc = run([], holders, ws, root)
    good = rc == 0 and len(lines) == 1 and lines[0].startswith("none (0 staged")
    print(f"  {'ok  ' if good else 'FAIL'} plumbing: nothing staged prints one 'none' line, no crash")
    ok = ok and good

    lines, rc = run([], {}, ws, root)
    good = rc == 0 and "no unit holds a live lease" in lines[0]
    print(f"  {'ok  ' if good else 'FAIL'} plumbing: no live lease short-circuits by name")
    ok = ok and good
    return 0 if ok else 1


def run(staged, holders, ws, root):
    """The full pipeline, factored out so the control exercises the REAL plumbing.

    This exists because my first version had `holders, ws = live_holders()` (the call to
    `workspaces()` was missing), which unpacked the dict's two KEYS into two strings. With nothing
    staged the loop never ran, so it printed `len(holders)` = 20 - the CHARACTER COUNT of the agent
    id "worker-tauri-appdata" - and looked plausible. It would have raised AttributeError the first
    time a real file was staged, i.e. exactly when the check was needed. The self-test above tests
    `_match` directly and so did not cover this: a control that does not run the real path is the
    failure this campaign keeps recording (F.34)."""
    if not holders:
        return ["none (no unit holds a live lease)"], 0
    hits = []
    for path in staged:
        abspath = f"{root}/{path}"
        for agent, todo_list in holders.items():
            for declared in ws.get(agent, []):
                d = declared.rstrip("/")
                if abspath == d or abspath.startswith(d + "/"):
                    left = max(0, int((max(e for _, e in todo_list) - time.time()) / 60))
                    hits.append((path, agent, todo_list[0][0], left))
    if not hits:
        out = [f"none ({len(staged)} staged file(s), {len(holders)} live unit(s), no overlap)"]
    else:
        out = ["STAGED FILES OWNED BY A UNIT THAT IS STILL RUNNING — expect possible revision:"]
        for path, agent, todo, left in sorted(set(hits)):
            out.append(f"      {path}")
            out.append(f"        <- {agent} ({todo}, lease {left}m left)")
        out += [
            "      These are its in-flight files, not its final answer: when that unit next",
            "      delivers, check its record for a WITHDRAWAL (grep -i 'withdraw|reverted')",
            "      and confirm HEAD still matches what it now believes (INCIDENTS F.80).",
        ]

    # F.84a: files touched AFTER a live run began, even when the path is outside that agent's
    # onboarded workspaces (a supervisor-granted scope extension is invisible to the check above).
    extra = []
    for agent, todo_list in holders.items():
        prefixes = top_trees(ws.get(agent, []))
        if not prefixes:
            continue
        for todo, _exp in todo_list:
            since_ts = run_started_at(todo)
            if not since_ts:
                continue
            for p in touched_since(staged, since_ts, prefixes):
                extra.append((p, agent, todo))
    if extra:
        out.append("")
        out.append("ALSO: staged files MODIFIED AFTER a live run began, in a tree that run works in")
        out.append("      (F.84a: catches a granted scope that is not in the agent's declared")
        out.append("      workspaces. Attribution here is BY TREE, not by proof - the mtime says")
        out.append("      the file changed during that run, not who changed it):")
        for p, agent, todo in sorted(set(extra)):
            out.append(f"      {p}   <- tree of {agent} ({todo})")
        out.append("      Same response as above: this may be that unit's in-flight work, not its "
                   "final answer.")
    return out, 0


def main():
    if "--self-test" in sys.argv:
        return self_test()
    if "--skip-paths" in sys.argv:
        # Machine-readable form for snapshot.sh: one path per line, the files a live unit still owns.
        # NOTE: snapshot.sh calls this AFTER `git add -u`, so the candidates are in the INDEX
        # (`--cached`), not in the unstaged working tree. Reading the unstaged diff here returned
        # nothing and the skip silently did nothing - the "check that cannot fail" trap again.
        staged = subprocess.run(["git", "diff", "--cached", "--name-only"],
                                capture_output=True, text=True).stdout.split()
        staged = [p for p in staged if not p.startswith(".future/goal/")]
        holders, ws = live_holders(), workspaces()
        root = subprocess.run(["git", "rev-parse", "--show-toplevel"],
                              capture_output=True, text=True).stdout.strip()
        for path, _agent, _todo in _match(staged, holders, ws, root):
            print(path)
        return 0
    staged = subprocess.run(["git", "diff", "--cached", "--name-only"],
                            capture_output=True, text=True).stdout.split()
    # only files that a workspace declaration could plausibly cover
    staged = [p for p in staged if not p.startswith(".future/goal/")]
    holders, ws = live_holders(), workspaces()
    root = subprocess.run(["git", "rev-parse", "--show-toplevel"],
                          capture_output=True, text=True).stdout.strip()
    lines, rc = run(staged, holders, ws, root)
    for line in lines:
        print(line)
    return rc


if __name__ == "__main__":
    sys.exit(main())
