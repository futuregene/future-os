"""Verdict hygiene for a cargo-mutants sample: which failing test killed each mutant,
and whether any catch rests only on a test that was flaky before the w-flake port fix.

Reads cargo-mutants' own outcomes.json (the authoritative mutant<->log mapping: the
`_NNN` suffix on a log file name is a RUN-ORDER dedup counter, so the mapping must
come from the JSON, never from the file name).

Usage:
    python mutation/verify-attribution.py mutation/out-queue-full/mutants.out
    python mutation/verify-attribution.py mutation/out-policy-stable/mutants.out --flaky a,b,c
    python mutation/verify-attribution.py mutation/out-compat/mutants.out --missed-only

Prints, per mutant: the tests that actually failed in ITS log, and flags
  * `ONLY-FLAKY`  — every failing test is on the pre-fix flake list (an unusable catch)
  * `touches-flaky` — some failing test is on the flake list, alongside others
  * `NONE`        — caught/missed with no failing test in the log (a review target)
"""
from __future__ import annotations

import json
import pathlib
import re
import sys

FLAKY_DEFAULT = [
    "providers::telegram::tests::the_webhook_serves_verified_deliveries_and_rejects_the_rest",
    "providers::telegram::tests::a_webhook_configured_with_explicit_addr_and_path_binds_there",
    "transport::ws::tests::a_healthy_connection_resets_the_backoff_before_the_next_failure",
    "bridge::queue::tests::an_idle_conversation_is_evicted_and_its_worker_stops",
    "providers::email::tests::a_message_already_delivered_under_another_uid_is_marked_without_a_turn",
    "delivery::tests::the_queue_is_bounded_and_drops_finished_entries_first",
]

FAILED_RE = re.compile(r"^test (\S+) \.\.\. FAILED", re.M)


def failing_tests(log: str) -> list[str]:
    seen: list[str] = []
    for m in FAILED_RE.finditer(log):
        if m.group(1) not in seen:
            seen.append(m.group(1))
    return seen


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    flags = [a for a in sys.argv[1:] if a.startswith("--")]
    out = pathlib.Path(args[0])
    flaky = FLAKY_DEFAULT
    for f in flags:
        if f.startswith("--flaky="):
            flaky = [x for x in f.split("=", 1)[1].split(",") if x]
    missed_only = "--missed-only" in flags
    quiet = "--quiet" in flags

    doc = json.loads((out / "outcomes.json").read_text(encoding="utf-8"))
    only_flaky, touches_flaky, no_failure = [], [], []
    rows = []
    for entry in doc["outcomes"]:
        scen = entry["scenario"]
        if "Mutant" not in scen:
            continue
        name = scen["Mutant"]["name"]
        summary = entry["summary"]
        if missed_only and summary != "MissedMutant":
            continue
        log = (out / entry["log_path"]).read_text(encoding="utf-8", errors="replace")
        fails = failing_tests(log)
        hit = [t for t in fails if t in flaky]
        rows.append((name, summary, fails, hit))
        if summary == "CaughtMutant":
            if fails and len(hit) == len(fails):
                only_flaky.append(name)
            elif hit:
                touches_flaky.append(name)
        if not fails and summary in ("CaughtMutant", "MissedMutant"):
            no_failure.append((name, summary))

    for name, summary, fails, hit in rows:
        print(f"{summary:13s} {name}")
        print(f"   failing tests ({len(fails)}): " + (", ".join(fails) if fails else "NONE"))
        if hit:
            print(f"   on the pre-fix flake list: {', '.join(hit)}")
    print("=" * 100)
    print(f"mutants inspected: {len(rows)}")
    print(f"  ONLY-FLAKY catches (unusable): {len(only_flaky)}: {only_flaky}")
    print(f"  catches that touch a flake-listed test alongside others: {len(touches_flaky)}: {touches_flaky}")
    print(f"  caught/missed with NO failing test in the log: {len(no_failure)}: {no_failure}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
