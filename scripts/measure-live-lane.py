#!/usr/bin/env python3
"""Measure the real live-event lane: today's 1:1 forwarding against coalescing,
and again against the lean feed a phone can declare (`lean_events_v1`).

Dumps the heaviest completed runs from the agent database (read-only), then runs
the shipped Rust code over exactly the bodies the desktop would publish, so the
numbers come from the real merge and rewrite paths and real event traffic — not
from a model of either.

  python3 scripts/measure-live-lane.py [--runs N] [--windows 50,100,250]
"""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def dump(session: str, run: str, path: Path) -> int:
    source = Path.home() / ".future" / "agent" / "agent.db"
    with sqlite3.connect(source.as_uri() + "?mode=ro", uri=True) as db:
        rows = db.execute(
            "SELECT payload FROM run_events WHERE session_id=? AND run_id=? ORDER BY idx",
            (session, run)).fetchall()
    with path.open("w") as sink:
        for (payload,) in rows:
            sink.write(payload)
            sink.write("\n")
    return len(rows)


def run_measurement(binary: Path, test: str, env_extra: dict) -> dict | None:
    result = subprocess.run(
        [str(binary), test, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
        env=env_extra, capture_output=True, text=True, check=True)
    marker = "LEAN_LANE " if "lean" in test else "LIVE_LANE "
    line = next((row for row in result.stdout.splitlines() if marker in row), None)
    if line is None:
        print(result.stdout[-2000:])
        return None
    return json.loads(line[line.index(marker) + len(marker):])


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runs", type=int, default=6)
    parser.add_argument("--windows", default="",
                        help="comma-separated window sweep in ms, e.g. 50,100,250,500")
    args = parser.parse_args()

    source = Path.home() / ".future" / "agent" / "agent.db"
    with sqlite3.connect(source.as_uri() + "?mode=ro", uri=True) as db:
        runs = db.execute("""SELECT e.session_id, e.run_id, count(*) n FROM run_events e
            JOIN runs r ON r.session_id=e.session_id AND r.run_id=e.run_id
            WHERE r.status='completed' GROUP BY 1,2 ORDER BY n DESC LIMIT ?""",
            (args.runs,)).fetchall()

    binary = next((path for path in sorted((ROOT / "desktop" / "src-tauri" / "target" / "debug" / "deps").glob("futureos_lib-*"))
                   if path.is_file() and os.access(path, os.X_OK)), None)
    if binary is None:
        raise SystemExit("build the desktop test binary first "
                         "(cd desktop/src-tauri && cargo test --no-default-features --lib publish --no-run)")

    print(f"{'sample':7}{'events':>9}{'window':>8}{'full MB':>10}{'coalesced MB':>14}"
          f"{'full msgs':>11}{'lean MB':>10}{'lean merged MB':>16}{'saved':>8}{'lean msgs':>11}")
    windows = [int(value) for value in args.windows.split(",") if value.strip()] or [None]
    for index, (session, run, _count) in enumerate(runs, 1):
        with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as handle:
            journal = Path(handle.name)
        try:
            dump(session, run, journal)
            for window in windows:
                env = {**os.environ, "SYNC_MEASURE_JOURNAL": str(journal),
                       "SYNC_MEASURE_SESSION": session, "SYNC_MEASURE_RUN": run}
                if window is not None:
                    env["SYNC_MEASURE_WINDOW_MS"] = str(window)
                full = run_measurement(
                    binary, "remote::publisher::coalesce::tests::measure_real_journal", env)
                lean = run_measurement(
                    binary, "remote::publisher::coalesce::tests::measure_real_journal_lean", env)
                if full is None or lean is None:
                    print(f"top{index}: no measurement")
                    continue
                saved = 1.0 - lean["coalescedBytes"] / max(full["coalescedBytes"], 1)
                print(f"top{index:<4}{full['events']:>9}{full['windowMs']:>8}"
                      f"{full['todayBytes']/1e6:>10.2f}{full['coalescedBytes']/1e6:>14.2f}"
                      f"{full['published']:>11}{lean['leanBytes']/1e6:>10.2f}"
                      f"{lean['coalescedBytes']/1e6:>16.2f}"
                      f"{saved*100:>7.1f}%{lean['published']:>11}")
        finally:
            journal.unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
