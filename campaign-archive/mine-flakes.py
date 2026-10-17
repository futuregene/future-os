#!/usr/bin/env python3
"""Mine the workers' own transcripts for flake evidence.

The campaign asked every unit to record the flakes it hit in its module record.
Some will; some are mid-segment. Meanwhile their journals already contain the
observations — this extracts candidate passages so the supervisor can turn them
into inventory without spending worker turns.

Usage: mine-flakes.py [--runs DIR] [--logs DIR]
"""
import glob
import json
import os
import re
import sys

RUNS = "/Users/geilige/future-os/.worktrees/cov100-r2/.future/goal/runs"
LOGS = "/Users/geilige/future-os/.worktrees/cov100-r2/.future/goal/detached/goal_160bc247aa63"

# phrases that indicate the unit hit a non-deterministic outcome
PATTERNS = [
    r"\bflake[sd]?\b", r"\bflaky\b", r"\bintermittent\b", r"\bnon-?determin\w*\b",
    r"\brace\b", r"\brandom\b", r"\bsometimes\b", r"\bidle deadline\b",
    r"\bdeadlock\w*\b", r"\bhang\w*\b", r"\btimed? out\b", r"\bEWOULDBLOCK\b",
    r"\b\d+/\d+ (?:runs|attempts|times)\b",
]
RX = re.compile("|".join(PATTERNS), re.I)


def mine_runs():
    out = {}
    for path in sorted(glob.glob(os.path.join(RUNS, "*.live.jsonl"))):
        hdr = None
        chunks = []
        for line in open(path, errors="replace"):
            if '"run_header"' in line:
                try:
                    hdr = json.loads(line)
                except Exception:
                    pass
                continue
            if '"text_chunk"' not in line:
                continue
            try:
                e = json.loads(line)
            except Exception:
                continue
            if isinstance(e.get("text"), str):
                chunks.append(e["text"])
        if not hdr or not chunks:
            continue
        text = "".join(chunks)
        agent = hdr.get("agent_id")
        hits = []
        for m in RX.finditer(text):
            s = max(0, m.start() - 220)
            snippet = text[s:m.end() + 260].replace("\n", " ")
            hits.append(snippet.strip())
        if hits:
            out.setdefault(agent, []).extend(hits)
    return out


def mine_logs():
    out = {}
    for path in sorted(glob.glob(os.path.join(LOGS, "*.log"))):
        agent = os.path.basename(path).split("-1")[0]
        try:
            text = open(path, errors="replace").read()
        except Exception:
            continue
        hits = [m.group(0) + " … " + text[m.start():m.start() + 160].replace("\n", " ")
                for m in re.finditer(r"(?i)(flake|flaky|intermittent|has been running for over)", text)]
        if hits:
            out.setdefault(agent, []).extend(hits[:6])
    return out


def main():
    runs = mine_runs()
    print("=== journal text (deduped, max 6 per agent) ===")
    for agent, hits in sorted(runs.items()):
        uniq = []
        seen = set()
        for h in hits:
            key = h[:80].lower()
            if key in seen:
                continue
            seen.add(key)
            uniq.append(h)
        print(f"\n--- {agent} ({len(uniq)} candidate passages)")
        for h in uniq[:6]:
            print("   *", h[:300])
    logs = mine_logs()
    print("\n=== detached logs ===")
    for agent, hits in sorted(logs.items()):
        print(f"--- {agent}")
        for h in hits[:4]:
            print("   *", h[:200])


if __name__ == "__main__":
    main()
