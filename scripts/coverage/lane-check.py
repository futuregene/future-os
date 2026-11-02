#!/usr/bin/env python3
"""A fast, deterministic gate for one coverage lane.

`cargo test -p <crate>` cannot be a goal validator: the suite takes minutes
(this lane: 430 s) and the goal kernel kills a validator after 120 s by default
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

A total alone is a weak claim: the same 2505 lines can hide a waiver that was
quietly covered beside a new gap, and the doc would still agree. So when the
record names an `inventory` (the per-file `DA:0` line sets, written by
`missed-json.py`), the agreement is checked **per line** and the drift is
printed by file and line. That is the check the campaign's own accounting does
(`buckets6.py`); this gate carries the cheap half of it.

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


def missed_lines(path: str, scope: str) -> dict[str, set[int]]:
    """Per-file missed lines: a line counts iff its LCOV record is `DA:<n>,0`."""
    found: dict[str, set[int]] = {}
    current: str | None = None
    with open(path, encoding="utf-8", errors="replace") as handle:
        for raw in handle:
            line = raw.strip()
            if line.startswith("SF:"):
                current = line[3:] if scope in line else None
                if current is not None:
                    found.setdefault(current, set())
            elif current and line.startswith("DA:"):
                if line[3:].partition(",")[2].split(",")[0] == "0":
                    found[current].add(int(line[3:].partition(",")[0]))
    return {key: lines for key, lines in found.items() if lines}


def count_da0(path: str, scope: str) -> int:
    return sum(len(lines) for lines in missed_lines(path, scope).values())


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


def ranges(numbers: list[int]) -> str:
    out = []
    if not numbers:
        return ""
    start = prev = numbers[0]
    for number in numbers[1:]:
        if number == prev + 1:
            prev = number
            continue
        out.append((start, prev))
        start = prev = number
    out.append((start, prev))
    return ",".join(str(a) if a == b else f"{a}-{b}" for a, b in out)


def check_inventory(record: dict, found: str, scope: str) -> None:
    """Per-line agreement between the artifact and the recorded inventory."""
    path = record.get("inventory")
    if not path:
        print("note: the record names no inventory — only the total is checked")
        return
    inventory_path = resolve(path)
    if inventory_path is None:
        print(
            f"note: {path} is not on this machine — the per-line check is skipped. "
            "Reproduce it with missed-json.py (see the doc), then re-run this gate."
        )
        return
    recorded = {
        key.split(scope)[-1]: set(lines)
        for key, lines in json.load(open(inventory_path, encoding="utf-8")).items()
    }
    measured = {key.split(scope)[-1]: lines for key, lines in missed_lines(found, scope).items()}
    drift = []
    for name in sorted(set(recorded) | set(measured)):
        before = recorded.get(name, set())
        after = measured.get(name, set())
        appeared = sorted(after - before)
        vanished = sorted(before - after)
        if appeared or vanished:
            drift.append(f"  {name}: +[{ranges(appeared)}] -[{ranges(vanished)}]")
    if drift:
        fail(
            f"{found} disagrees with {inventory_path} on the exact lines "
            "(+ = uncovered now, - = covered now):\n" + "\n".join(drift)
        )
    print(f"inventory agrees: {inventory_path} ({len(recorded)} files)")


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

    # 3. When this machine still has the artifact, the measurement must hold —
    #    the total, and, when the record carries one, the exact line sets.
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
        check_inventory(record, found, record["scope_prefix"])

    print(
        f"ok: {record['lane']} at {record['measurement_commit']} — "
        f"{record['da0_total']} uncovered "
        f"({record['switch_lane']} switch lane / {record['waived']} waived / "
        f"{record['open']} open)"
    )


if __name__ == "__main__":
    main()
