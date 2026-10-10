#!/usr/bin/env python3
"""Audit the lean history trim on one real page: which keys a shell row keeps
versus loses, and that the identity a lazy fetch needs survives.

Dumps the newest backward page of the heaviest session from the live
(read-only) agent, then runs the shipping Rust trim over it and prints, per
tool-call row, the run id, tool name, call id and argument keys before and
after — plus the byte totals the shipping code measured.

Selection only: the trim and every byte count come from
`remote_host::lean::lean_entries` through the desktop test binary
(`remote_host::lean::tests::measure_real_entries`), never from a Python copy.

  python3 scripts/measure/audit-lean-page.py [--session SESSION_ID] [--page 1]

Build the measurement binary first:

  (cd desktop/src-tauri && cargo test --lib lean --no-run)
"""
import argparse
import json
import os
import sqlite3
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def load_measure_history():
    """Load measure-lean-history.py for its dump/pagination helpers."""
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "measure_lean_history", ROOT / "scripts" / "measure" / "measure-lean-history.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def measurement_binary():
    candidates = sorted(
        (path for path in
         (ROOT / "desktop" / "src-tauri" / "target" / "debug" / "deps").glob("futureos_lib-*")
         if path.is_file() and os.access(path, os.X_OK)),
        key=lambda path: path.stat().st_mtime, reverse=True)
    if not candidates:
        raise SystemExit("build the desktop test binary first "
                         "(cd desktop/src-tauri && cargo test --lib lean --no-run)")
    return candidates[0]


def calls(entries):
    """(runId, name, toolCallId, argument keys) for every tool-call block."""
    out = []
    for entry in entries:
        for block in entry.get("blocks", []):
            if block.get("kind") == "tool_call":
                out.append((entry.get("runId"), block.get("name"), block.get("toolCallId"),
                            None if "arguments" not in block else sorted(block["arguments"])))
    return out


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--session", help="session id (default: the one with the most entries)")
    parser.add_argument("--exchanges", type=int, default=3,
                        help="page size in user exchanges (the phone's is 3)")
    parser.add_argument("--page", type=int, default=1, help="1 = the newest page")
    args = parser.parse_args()

    measure = load_measure_history()
    if args.session is None:
        with sqlite3.connect(
                (Path.home() / ".future" / "agent" / "agent.db").as_uri() + "?mode=ro",
                uri=True) as db:
            args.session = db.execute(
                "SELECT session_id FROM entries GROUP BY 1 ORDER BY count(*) DESC LIMIT 1"
            ).fetchone()[0]

    dump_path = Path(tempfile.mktemp(suffix=".json"))
    measure.dump(args.session, dump_path)
    entries = json.loads(dump_path.read_text())
    spans = measure.cut_pages(entries, args.exchanges)
    if not 1 <= args.page <= len(spans):
        raise SystemExit(f"page {args.page} out of range 1..{len(spans)}")
    start, end = spans[args.page - 1]
    page = entries[start:end]

    page_path = Path(tempfile.mktemp(suffix=".json"))
    trimmed_path = Path(tempfile.mktemp(suffix=".json"))
    page_path.write_text(json.dumps(page, separators=(",", ":")))
    result = subprocess.run(
        [str(measurement_binary()), "remote_host::lean::tests::measure_real_entries",
         "--exact", "--ignored", "--nocapture", "--test-threads=1"],
        env={**os.environ, "LEAN_HISTORY_ENTRIES": str(page_path),
             "LEAN_HISTORY_OUT": str(trimmed_path)},
        capture_output=True, text=True)
    if result.returncode != 0:
        print(result.stdout[-2000:])
        print(result.stderr[-2000:])
        raise SystemExit("measurement binary failed")
    measurements = [line for line in result.stdout.splitlines() if "LEAN_HISTORY " in line]
    trimmed = json.loads(trimmed_path.read_text())

    all_calls = calls(page)
    print("session:", args.session)
    print("page:", args.page, "of", len(spans), "-", len(page), "entries")
    for line in measurements:
        print(line[line.index("LEAN_HISTORY "):])
    print("assistant entries with runId:",
          sum(1 for e in page if e.get("role") == "assistant" and e.get("runId")), "/",
          sum(1 for e in page if e.get("role") == "assistant"))
    print("tool_call blocks:", len(all_calls), " with toolCallId:",
          sum(1 for call in all_calls if call[2]))
    for label, kinds in (("shell/other", lambda name: name not in ("read", "write", "edit")),
                         ("read/write/edit", lambda name: name in ("read", "write", "edit"))):
        before = [c for c in calls(page) if kinds(c[1])]
        after = [c for c in calls(trimmed) if kinds(c[1])]
        print(f"{label} rows BEFORE (runId, name, toolCallId, argument keys):")
        for call in before[:4]:
            print("   ", call)
        print(f"{label} rows AFTER:")
        for call in after[:4]:
            print("   ", call)
    for path in (dump_path, page_path, trimmed_path):
        path.unlink()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
