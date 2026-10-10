#!/usr/bin/env python3
"""Does the candidate order change the recommendation, or is it run-to-run noise?

A single pair of calls cannot tell those apart, and the evaluation measured the
noise at ~9% of questions: the same code answers differently between identical
runs. So this runs each order N times and compares the distributions, which is
what a "the order matters" claim actually requires.

It exists because one pair of calls *looked* order-sensitive during the RPC pass
(forward refused, reversed picked a skill). Repeated, both orders produced the
same set of answers — and that run surfaced something more useful: the option text
decides near-miss races. For an ambiguous prompt, one extra clause in a candidate's
description changed the answer from 0/16 to 12/12.

Not part of CI (needs grpcurl, a credential and a live gateway):

  future agent --home /tmp/reco-test --verbose --log-file
  python3 scripts/skill_reco/order-sensitivity.py --runs 8
"""

import argparse
import collections
import json
import os
import subprocess
import time

REPO = os.environ.get("FUTURE_REPO", "/Users/geilige/future-os")
HOME_DIR = os.environ.get("RECO_TEST_HOME", "/tmp/reco-test")
SOCKET = f"{HOME_DIR}/run/agent.sock"
AUTH = f"{HOME_DIR}/agent/auth.json"
REAL_AUTH = os.path.expanduser("~/.future/agent/auth.json")

QUERY = "帮我把这张照片转成水彩风格，再做一个 PDF 幻灯片"

# Both candidates apply to QUERY, which is the point: the model has to prefer one,
# and the pair below is the one whose description wording flips the answer.
IMAGE = {"name": "future-image", "description": "Generate images from text, edit supplied images"}
SLIDES = {"name": "future-slides", "description": "Turn a report or outline into PNG slides and a PDF"}


def suggest(candidates):
    command = {"id": "o1", "type": "suggest_skill", "suggest_query": QUERY,
               "suggest_candidates": candidates}
    proc = subprocess.run(
        ["grpcurl", "-plaintext", "-import-path", f"{REPO}/packages/rpc/proto",
         "-proto", "future.proto", "-d", json.dumps(command), f"unix://{SOCKET}",
         "proto.FutureAgent/ExecuteCommand"],
        capture_output=True, text=True, timeout=90,
    )
    response = json.loads(proc.stdout)
    skill = ((response.get("payload") or {}).get("suggestSkill") or {}).get("skill") or {}
    return skill.get("name") or "(refused)"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--runs", type=int, default=6)
    parser.add_argument("--short-descriptions", action="store_true",
                        help="use the shorter wording that never picked future-image")
    args = parser.parse_args()

    if not os.path.exists(REAL_AUTH):
        raise SystemExit("no real credential to test with")
    with open(REAL_AUTH) as handle:
        real = json.load(handle)
    os.makedirs(os.path.dirname(AUTH), exist_ok=True)
    with open(AUTH, "w") as handle:
        json.dump({"future": real["future"]}, handle)

    image, slides = IMAGE, SLIDES
    if args.short_descriptions:
        image = {"name": "future-image", "description": "Generate images from text"}
        slides = {"name": "future-slides", "description": "Turn a report into PNG slides and a PDF"}

    try:
        outcomes = {}
        for label, candidates in (("image first", [image, slides]),
                                  ("slides first", [slides, image])):
            seen = []
            for _ in range(args.runs):
                name = suggest(candidates)
                seen.append(name)
                print(f"  {label:14} → {name}")
                time.sleep(0.2)
            outcomes[label] = collections.Counter(seen)
        print()
        for label, counts in outcomes.items():
            total = sum(counts.values())
            spread = ", ".join(f"{name}={count}/{total}" for name, count in counts.most_common())
            print(f"{label:14} {spread}")

        a = set(outcomes["image first"])
        b = set(outcomes["slides first"])
        print()
        if a == b and len(a) == 1:
            print("VERDICT: both orders gave the same single answer — order does not matter")
        elif a == b:
            print(f"VERDICT: both orders gave the same *set* of answers {sorted(a)} — the "
                  "difference in one pair of calls is run-to-run noise, not order")
        elif a.isdisjoint(b):
            print(f"VERDICT: the orders disagree systematically ({sorted(a)} vs {sorted(b)}) "
                  "— candidate order changes the answer")
        else:
            print(f"VERDICT: overlapping sets ({sorted(a)} vs {sorted(b)}) — the orders "
                  "share answers, so a single pair proves nothing; more runs needed")
    finally:
        if os.path.exists(AUTH):
            os.remove(AUTH)


if __name__ == "__main__":
    main()
