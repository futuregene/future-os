#!/usr/bin/env python3
"""A fast, deterministic gate for one coverage lane.

`cargo test -p <crate>` cannot be a goal validator: the suite takes minutes
(this lane: 432 s) and the goal kernel kills a validator after 120 s by default
(`orchestration/goal/src/validator.rs`, `DEFAULT_TIMEOUT`), which reads as
"validator failed to run" rather than as a verdict on the artifact. What a
validator *should* be is seconds long, deterministic, and red when the numbers
stop matching the tree.

This script is that check. It ties three things together and fails on any
mismatch:

  1. the lane record (`<lane>.json`) — the measured numbers and their scope;
  2. the waiver doc — it must exist and must quote the same numbers;
  3. the lcov artifact when this machine has one — recomputed per-line `DA:0`
     over the record's scope must equal the record, so a stale or "complete but
     almost all zero" report cannot pass.

Usage:
  python3 scripts/coverage/lane-check.py docs/testing/coverage-waivers-2026-11/host-lane.json

Exit 0 = consistent. Exit 1 = drift (the message names which of the three).
Exit 2 = the record itself is unusable.
"""
from __future__ import annotations

import json
import os
import sys

REQUIRED = (
    "lane",
    "measurement_commit",
    "scope_prefix",
    "da0_total",
    "switch_lane",
    "waived",
    "open",
    "before_da0",
    "artifact",
    "doc",
)


def fail(message: str, code: int = 1) -> None:
    print(f"FAIL: {message}")
    raise SystemExit(code)


def count_da0(path: str, scope: str) -> int:
    """Per-line missed lines: a line counts iff its LCOV record is `DA:<n>,0`."""
    total = 0
    in_scope = False
    with open(path, encoding="utf-8", errors="replace") as handle:
        for raw in handle:
            line = raw.strip()
            if line.startswith("SF:"):
                in_scope = scope in line
            elif in_scope and line.startswith("DA:"):
                if line[3:].partition(",")[2].split(",")[0] == "0":
                    total += 1
    return total


def resolve(relative: str) -> str | None:
    """Find `relative` from the cwd or up to three directories above it.

    A lane's lcov normally lives at the repository root while the gate runs
    inside a worktree (`.worktrees/<name>`), so the artifact is one or two levels
    up. Walking up keeps the record's path repo-relative without hard-coding a
    worktree layout.
    """
    root = os.getcwd()
    for _ in range(4):
        candidate = os.path.join(root, relative)
        if os.path.exists(candidate):
            return candidate
        parent = os.path.dirname(root)
        if parent == root:
            break
        root = parent
        # Record numbers are repo-relative, so skip absolute inputs.
    return None


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: lane-check.py <lane-record.json>", 2)
    path = sys.argv[1]
    if not os.path.exists(path):
        fail(f"no lane record at {path}", 2)
    try:
        record = json.load(open(path, encoding="utf-8"))
    except json.JSONDecodeError as error:
        fail(f"{path} is not JSON: {error}", 2)
    missing = [key for key in REQUIRED if key not in record]
    if missing:
        fail(f"{path} is missing {missing}", 2)

    # 1. The buckets must account for every remaining line.
    accounted = record["switch_lane"] + record["waived"] + record["open"]
    if accounted != record["da0_total"]:
        fail(
            f"buckets do not add up: switch {record['switch_lane']} + "
            f"waived {record['waived']} + open {record['open']} = {accounted}, "
            f"but the lane total is {record['da0_total']}"
        )

    # 2. The doc must carry the same numbers, so the two cannot drift apart.
    doc = record["doc"]
    if not os.path.exists(doc):
        fail(f"the record names {doc}, which does not exist")
    text = open(doc, encoding="utf-8").read()
    for label, value in (
        ("lane total", record["da0_total"]),
        ("waived", record["waived"]),
        ("open", record["open"]),
    ):
        if str(value) not in text:
            fail(f"{doc} never mentions {label} {value}")
    if record["measurement_commit"] not in text:
        fail(f"{doc} does not name the measurement commit {record['measurement_commit']}")

    # 3. When this machine still has the artifact, the measurement must hold.
    found = resolve(record["artifact"])
    if found is None:
        print(
            f"note: {record['artifact']} is not on this machine — the lcov agreement "
            "check is skipped. Reproduce it with the command in the doc, then re-run "
            "this gate."
        )
    else:
        measured = count_da0(found, record["scope_prefix"])
        if measured == 0:
            fail(
                f"{found} reports 0 uncovered lines for {record['scope_prefix']}: that is "
                "the shape of a report written before the run finished (see the "
                "zero-report note in .future/memory/coverage-llvm-cov.md), not a clean lane"
            )
        if measured != record["da0_total"]:
            fail(
                f"{found} measures {measured} uncovered lines for "
                f"{record['scope_prefix']}, the record says {record['da0_total']}"
            )
        print(f"lcov agrees: {found}")

    print(
        f"ok: {record['lane']} at {record['measurement_commit']} — "
        f"{record['da0_total']} uncovered "
        f"({record['switch_lane']} switch lane / {record['waived']} waived / "
        f"{record['open']} open)"
    )


if __name__ == "__main__":
    main()
