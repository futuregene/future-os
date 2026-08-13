#!/usr/bin/env python3
"""Per-crate coverage ratchet (loopx-improvement-report P1-7).

Two modes:

  emit  <llvm-cov.json> <out.json>
      Aggregate a `cargo llvm-cov report --json --summary-only` export into
      per-crate lines/functions/regions totals and write them as JSON.
      scripts/coverage.sh runs this on every measurement; the output
      (coverage/baseline.json) is the *working* artifact and is gitignored.

  check --baseline <floors.json> --current <baseline.json>
      Enforce the ratchet: for every crate listed in the checked-in floor
      file (scripts/coverage-baseline.json), the current line-coverage
      percent must be >= the floor. Only line coverage is enforced (the
      first ratchet metric); functions/regions are recorded for future
      ratchets. Rises are allowed silently (bump the floor file to lock
      them in). A drop fails the build unless the PR itself edits the floor
      file — that diff IS the explicit ratchet-down approval, reviewed like
      any other code change.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

# Workspace source root -> crate name. First matching prefix wins; keep the
# list ordered most-specific first.
CRATE_ROOTS: list[tuple[str, str]] = [
    ("orchestration/loop/", "future-loop"),
    ("agent/", "future-agent"),
    ("channels/", "future-channels"),
    ("cli/", "future-cli"),
    ("rpc/", "future-rpc"),
    ("tui/", "future-tui"),
]

METRICS = ("lines", "functions", "regions")

SCHEMA_VERSION = 1


def crate_of(filename: str, root: str) -> str | None:
    # llvm-cov emits absolute paths; make them workspace-relative first.
    # Normalize separators for Windows runners.
    name = filename.replace("\\", "/")
    if name.startswith(root):
        name = name[len(root) :]
    for prefix, crate in CRATE_ROOTS:
        if name.startswith(prefix):
            return crate
    return None


def _pct(covered: int, count: int) -> float:
    return round(covered / count * 100.0, 2) if count else 100.0


def emit(llvm_cov_json: Path, out: Path) -> int:
    export = json.loads(llvm_cov_json.read_text())
    files = export["data"][0]["files"]
    # scripts/coverage.sh invokes this from the repo root.
    root = str(Path.cwd()).replace("\\", "/").rstrip("/") + "/"

    crates: dict[str, dict[str, dict[str, int]]] = {}
    unmapped: list[str] = []
    for entry in files:
        crate = crate_of(entry["filename"], root)
        if crate is None:
            unmapped.append(entry["filename"])
            continue
        bucket = crates.setdefault(
            crate, {m: {"count": 0, "covered": 0} for m in METRICS}
        )
        for metric in METRICS:
            summary = entry["summary"][metric]
            bucket[metric]["count"] += summary["count"]
            bucket[metric]["covered"] += summary["covered"]

    def render(bucket: dict[str, dict[str, int]]) -> dict[str, dict[str, float | int]]:
        return {
            m: {
                "count": bucket[m]["count"],
                "covered": bucket[m]["covered"],
                "percent": _pct(bucket[m]["covered"], bucket[m]["count"]),
            }
            for m in METRICS
        }

    totals = {m: {"count": 0, "covered": 0} for m in METRICS}
    for bucket in crates.values():
        for metric in METRICS:
            totals[metric]["count"] += bucket[metric]["count"]
            totals[metric]["covered"] += bucket[metric]["covered"]

    doc = {
        "schema_version": SCHEMA_VERSION,
        "metric_basis": (
            "cargo llvm-cov summary via scripts/coverage.sh; "
            "percent = covered/count*100 rounded to 2 decimals"
        ),
        "crates": {crate: render(b) for crate, b in sorted(crates.items())},
        "totals": render(totals),
    }
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(doc, indent=2) + "\n")

    if unmapped:
        print(
            f"coverage-ratchet: warning: {len(unmapped)} file(s) matched no "
            f"crate root (first: {unmapped[0]})",
            file=sys.stderr,
        )
    print(f"coverage-ratchet: wrote {out}")
    for crate, bucket in sorted(crates.items()):
        lines = bucket["lines"]
        print(f"  {crate:<16} lines {_pct(lines['covered'], lines['count']):6.2f}%")
    print(
        f"  {'TOTAL':<16} lines {_pct(totals['lines']['covered'], totals['lines']['count']):6.2f}%"
    )
    return 0


def check(baseline_path: Path, current_path: Path) -> int:
    floors = json.loads(baseline_path.read_text())
    current = json.loads(current_path.read_text())

    floor_crates = floors.get("crates", {})
    current_crates = current.get("crates", {})

    failures: list[str] = []
    rises: list[str] = []
    rows: list[tuple[str, float, float, str]] = []

    names = sorted(set(floor_crates) | {"TOTAL"})
    for name in names:
        floor_entry = (
            floors.get("totals") if name == "TOTAL" else floor_crates.get(name)
        )
        current_entry = (
            current.get("totals") if name == "TOTAL" else current_crates.get(name)
        )
        if floor_entry is None:
            continue
        floor_pct = float(floor_entry["lines"]["percent"])
        if current_entry is None:
            failures.append(f"{name}: crate missing from current measurement")
            rows.append((name, floor_pct, float("nan"), "MISSING"))
            continue
        current_pct = float(current_entry["lines"]["percent"])
        if current_pct + 1e-9 < floor_pct:
            failures.append(
                f"{name}: line coverage {current_pct:.2f}% < floor {floor_pct:.2f}%"
            )
            rows.append((name, floor_pct, current_pct, "FAIL"))
        else:
            status = "ok"
            if current_pct > floor_pct:
                status = "rose"
                rises.append(
                    f"{name}: {floor_pct:.2f}% -> {current_pct:.2f}% "
                    "(consider bumping scripts/coverage-baseline.json)"
                )
            rows.append((name, floor_pct, current_pct, status))

    print("coverage ratchet (line coverage, only-up-or-approved-down):")
    print(f"  {'crate':<16} {'floor':>7} {'current':>8}  status")
    for name, floor_pct, current_pct, status in rows:
        cur = f"{current_pct:7.2f}%" if current_pct == current_pct else "      -"
        print(f"  {name:<16} {floor_pct:6.2f}% {cur}  {status}")

    for msg in rises:
        print(f"  note: {msg}")

    if failures:
        sys.stdout.flush()
        print("\ncoverage ratchet FAILED:", file=sys.stderr)
        for msg in failures:
            print(f"  - {msg}", file=sys.stderr)
        print(
            "\nLine coverage may only go up. To approve an intentional drop, "
            "lower the floor in scripts/coverage-baseline.json in this PR — "
            "that diff is the explicit ratchet-down approval and must be "
            "justified in the PR description.",
            file=sys.stderr,
        )
        return 1

    print("coverage ratchet passed.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="mode", required=True)

    p_emit = sub.add_parser("emit", help="aggregate an llvm-cov JSON export")
    p_emit.add_argument("llvm_cov_json", type=Path)
    p_emit.add_argument("out", type=Path)

    p_check = sub.add_parser("check", help="enforce the ratchet against floors")
    p_check.add_argument("--baseline", type=Path, required=True)
    p_check.add_argument("--current", type=Path, required=True)

    args = parser.parse_args()
    if args.mode == "emit":
        return emit(args.llvm_cov_json, args.out)
    return check(args.baseline, args.current)


if __name__ == "__main__":
    sys.exit(main())
