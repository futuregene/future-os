#!/usr/bin/env python3
"""Which ACCEPTANCE CONTRACTS can fail spuriously?

Why (F.83/F.85): `completion::evidence_error` checks `record.evidence`, which the executor fills from
`tail(&summary.text, 4_000)` - the run's NARRATION, not the `todo complete --evidence` report the author
curates. A token therefore only matches if the model happens to say it in its final prose. Short,
conventional tokens ("MISSING 0", "re-measured") tend to appear there anyway. INVENTED PHRASES
("verdict table in docs/testing/review-goal-2.md") and COMPARISON EXPRESSIONS ("DA:270 >= 1") do not.

This lists every non-done ticket whose contract contains such a token, so the risk is visible BEFORE a
delivery is judged failed for it. Read-only.

usage: acceptance-risk.py [--goal G]
"""
import json
import re
import subprocess
import sys

GOAL = "goal_160bc247aa63"
for i, a in enumerate(sys.argv):
    if a == "--goal" and i + 1 < len(sys.argv):
        GOAL = sys.argv[i + 1]


def risky(tok):
    """A token is risky if it is a phrase, a comparison, or a path with prose around it."""
    t = tok.strip()
    if not t:
        return None
    if ">=" in t or "<=" in t or "==" in t:
        return "comparison expression (will not appear verbatim in narration)"
    if len(t.split()) >= 3:
        return "phrase of 3+ words (narration will not repeat it)"
    if re.search(r"\.md\b", t) and len(t.split()) >= 2:
        return "prose wrapped around a filename"
    return None


def self_test():
    """The control: the tool must flag the contract that ACTUALLY fired twice today."""
    revgoal2 = ("verdict table in docs/testing/review-goal-2.md,"
                "fixture falsification attempted for all three,"
                "DA:270 >= 1,DA:235 = 512,DA:157 >= 1,DA:315/316/317 >= 1,"
                "reconciled 77 with MISSING 0 and no STALE,"
                "no assertion weakened,"
                "ruling recorded on both test-only fixes")
    flagged = [x for x in revgoal2.split(",") if risky(x)]
    ok = True
    print(f"  {'ok  ' if len(flagged) >= 4 else 'FAIL'} the REV-GOAL-2 contract is flagged "
          f"({len(flagged)} risky tokens of 9)")
    ok = ok and len(flagged) >= 4
    for tok in flagged:
        print(f"        {tok!r} -> {risky(tok)}")
    # and a contract that must NOT be flagged: the conventional one the other units use
    conventional = "re-measured,MISSING 0,STALE 0"
    clean = [x for x in conventional.split(",") if risky(x)]
    print(f"  {'ok  ' if not clean else 'FAIL'} the conventional contract is NOT flagged "
          f"({len(clean)} risky of 3)")
    ok = ok and not clean
    return 0 if ok else 1


def main():
    if "--self-test" in sys.argv:
        return self_test()
    out = subprocess.run(["future", "goal", "status", "--goal", GOAL, "--format", "json"],
                         capture_output=True, text=True).stdout
    goals = json.loads(out)
    rows = []
    for g in goals:
        for t in g.get("todos", []):
            acc = t.get("acceptance")
            if not acc:
                continue
            if "done" in t["status"] or "superseded" in t["status"]:
                continue
            toks = [x for x in acc.split(",") if x.strip()]
            bad = [(x, risky(x)) for x in toks]
            bad = [(x, w) for x, w in bad if w]
            if bad:
                rows.append((t["id"], t["status"], str(t.get("owner")), toks, bad))
    if not rows:
        print("no open ticket carries a phrase-shaped acceptance token")
        return 0
    print(f"{len(rows)} open ticket(s) carry a token that can fail for the WRONG reason:\n")
    for tid, status, owner, toks, bad in rows:
        print(f"{tid}  {status}  owner={owner}")
        print(f"   contract: {len(toks)} token(s), {len(bad)} risky")
        for tok, why in bad:
            print(f"      ! {tok!r} -> {why}")
        print()
    print("What to do: these fire on the NARRATION tail, so a compliant unit can still be marked")
    print("failed (F.83). When one fires, read the completion evidence before judging the delivery;")
    print("and prefer a --verify command, or natural report phrases such as 'MISSING 0'.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
