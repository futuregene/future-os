#!/usr/bin/env python3
"""Measure the catalog lane: what one streaming run's activity writes cost the
wire before and after `catalog_activity_v1`.

A run rewrites its thread's `threads.last_message_at` about once a second, and
that column is the session list's ordering key — so before this lane the desktop
re-published *every* session it owns, once per second, for the whole run.

For each of the heaviest completed runs in the local agent database (read-only)
this script

  1. replays that run's journal through the observer's own activity-write rule
     (`agent_bridge::observer::record_message_activity`: at most one write per
     1000 ms of event time, plus every prompt / `agent_end` / `error`, which
     always flush), giving the writes the desktop would publish, and
  2. runs the shipping `spawn_catalog_publisher` against a real store as large as
     this desktop's own, counting the payload bytes it publishes in both
     declaration states (`remote::verify_e2e::measure_catalog_activity_lane`).

The per-write *bytes* therefore come from the code under test, not from a model
of it; only the *write count* is a replay of the rule above, because no shipping
function counts it without a live agent stream. One snapshot or packet is one
sealed record, so the record overhead (`future_remote_crypto` HEADER_LEN +
TAG_LEN = 44 B) is added once per write — it is negligible for a snapshot and
about a fifth of a packet.

What is deliberately *not* counted on either side: the snapshot a client pays
once when it connects, and the pushes a structural change still costs (a
`streaming`/`status` transition, a generated title). Both are unchanged by this
lane, so including them would only blur the comparison.

  python3 scripts/measure/measure-catalog-activity.py [--runs 6] [--writes 20]
"""
import argparse
import json
import os
import re
import sqlite3
import subprocess
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
AGENT_DB = Path.home() / ".future" / "agent" / "agent.db"
APP_DB = Path.home() / ".future" / "app" / "app.db"

# `future_remote_crypto::HEADER_LEN + TAG_LEN`: the fixed part of every sealed
# record. One packet is one record.
RECORD_OVERHEAD_BYTES = 44

MARKER = "CATALOG_ACTIVITY "

# The event types `message_activity_timestamp` answers for: per-token content,
# the prompt, the tools, and the run's own boundaries.
ACTIVITY_TYPES = {
    "user_message",
    "text_chunk",
    "thinking_delta",
    "tool_start",
    "tool_delta",
    "tool_end",
    "agent_end",
    "error",
}
# The subset that always flushes: `record_message_activity` skips its 1000 ms
# throttle for these, so a settled reply keeps its exact final activity time.
FORCED_TYPES = {"user_message", "agent_end", "error"}
# The observer's throttle, verbatim (`observer.rs::record_message_activity`).
THROTTLE_MS = 1_000


def event_time_ms(value):
    if not isinstance(value, str):
        return None
    try:
        return int(datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp() * 1000)
    except ValueError:
        return None


def activity_writes(session, run):
    """The activity writes one run would have produced, by the shipping rule."""
    db = sqlite3.connect(AGENT_DB.as_uri() + "?mode=ro", uri=True)
    rows = db.execute(
        "SELECT payload FROM run_events WHERE session_id=? AND run_id=? ORDER BY idx",
        (session, run),
    ).fetchall()
    db.close()

    last = None
    writes = 0
    for (payload,) in rows:
        event = json.loads(payload)
        at = event_time_ms(event.get("timestamp"))
        if at is None:
            continue
        kind = event.get("event_type")
        if kind not in ACTIVITY_TYPES:
            continue
        if last is None or at - last >= THROTTLE_MS or kind in FORCED_TYPES:
            writes += 1
            last = at
    return len(rows), writes


def local_session_rows():
    """How many sessions this desktop's own catalogue carries, read-only.

    The row count is what the snapshot's size scales with, so the measurement is
    run at the size of the machine that produced the numbers.
    """
    if not APP_DB.exists():
        return None
    db = sqlite3.connect(APP_DB.as_uri() + "?mode=ro", uri=True)
    (rows,) = db.execute(
        "SELECT count(*) FROM threads WHERE deleted_at IS NULL"
        " AND agent_session_id IS NOT NULL AND trim(agent_session_id) != ''"
    ).fetchone()
    db.close()
    return rows


def target_dir():
    """Where the desktop crate's test binary was built.

    `CARGO_TARGET_DIR` wins when set (a worktree that shares the main checkout's
    target dir is the common case while iterating), and a relative value is
    resolved the way cargo resolves it — against the crate's own directory.
    """
    configured = os.environ.get("CARGO_TARGET_DIR")
    if not configured:
        return ROOT / "desktop" / "src-tauri" / "target"
    path = Path(configured)
    return path if path.is_absolute() else ROOT / "desktop" / "src-tauri" / path


def newest_test_binary():
    """The newest desktop test binary.

    Not alphabetically first: `target/debug/deps` keeps every historical build,
    and a stale one silently measures an older revision.
    """
    candidates = [
        path
        for path in (target_dir() / "debug" / "deps").glob("futureos_lib-*")
        if path.is_file() and os.access(path, os.X_OK)
    ]
    binary = max(candidates, key=lambda path: path.stat().st_mtime, default=None)
    if binary is None:
        raise SystemExit(
            "build the desktop test binary first: "
            "cd desktop/src-tauri && cargo test --no-default-features --lib "
            "measure_catalog_activity_lane --no-run"
        )
    return binary


def measure(binary, rows, writes):
    result = subprocess.run(
        [str(binary), "remote::verify_e2e::measure_catalog_activity_lane",
         "--exact", "--ignored", "--nocapture", "--test-threads=1"],
        env={**os.environ, "CATALOG_ACTIVITY_ROWS": str(rows),
             "CATALOG_ACTIVITY_WRITES": str(writes)},
        capture_output=True, text=True,
    )
    # The marker is printed by the test itself, on the line its own harness
    # prefixes with the test name — so it is found by name, not by line start.
    match = re.search(rf"{MARKER}(\{{.*\}})", result.stdout)
    if match is None or result.returncode != 0:
        print(result.stdout[-3000:], result.stderr[-2000:])
        raise SystemExit(f"the measurement test failed (exit {result.returncode})")
    return json.loads(match.group(1))


def record_bytes(payload_bytes, packets):
    """One packet is one sealed record, so the fixed record overhead applies
    per packet rather than per byte."""
    return (payload_bytes + packets * RECORD_OVERHEAD_BYTES) / max(packets, 1)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--runs", type=int, default=6, help="heaviest runs to replay")
    parser.add_argument("--writes", type=int, default=20,
                        help="activity writes the fixture replays (per-write cost is constant)")
    args = parser.parse_args()

    rows = local_session_rows() or 232
    binary = newest_test_binary()
    measured = measure(binary, rows, args.writes)
    declared = measured["declared"]
    undeclared = measured["undeclared"]
    if undeclared["sessionsPackets"] == 0 or declared["activityPackets"] == 0:
        raise SystemExit(
            "the fixture did not publish what the two modes are supposed to show: "
            f"{measured}"
        )

    snapshot_bytes = record_bytes(
        undeclared["sessionsPayloadBytes"], undeclared["sessionsPackets"]
    )
    # One write lifts the moving row to the head of the list, and the host's own
    # order is content (an ordinary single-machine list draws it) — so that one
    # write is a whole list in *both* modes. Everything after it is a packet.
    overtake_bytes = record_bytes(
        declared["sessionsPayloadBytes"], declared["sessionsPackets"]
    )
    packet_bytes = record_bytes(declared["activityPayloadBytes"], declared["activityPackets"])

    print(f"catalog rows (this desktop)   : {rows}")
    print(f"one whole session list        : {snapshot_bytes:,.0f} B (record included)")
    print(f"one activity packet           : {packet_bytes:,.0f} B per write after the first")
    print(f"the first write (a reorder)   : {overtake_bytes:,.0f} B — the host's order is")
    print("                                drawn by an ordinary list, so it is content")
    print()

    print(f"{'sample':7}{'events':>9}{'writes':>8}{'before MB':>12}{'after MB':>10}{'saved':>8}")
    db = sqlite3.connect(AGENT_DB.as_uri() + "?mode=ro", uri=True)
    runs = db.execute(
        """SELECT e.session_id, e.run_id, count(*) n FROM run_events e
           JOIN runs r ON r.session_id=e.session_id AND r.run_id=e.run_id
           WHERE r.status='completed' GROUP BY 1,2 ORDER BY n DESC LIMIT ?""",
        (args.runs,),
    ).fetchall()
    db.close()

    for index, (session, run, _count) in enumerate(runs, 1):
        events, writes = activity_writes(session, run)
        # Before: every write republished the list. After: the overtake plus a
        # packet per later write.
        before = writes * snapshot_bytes
        after = overtake_bytes + max(writes - 1, 0) * packet_bytes
        saved = 1.0 - after / max(before, 1.0)
        print(f"top{index:<4}{events:>9}{writes:>8}{before/1e6:>12.1f}"
              f"{after/1e6:>10.2f}{saved*100:>7.1f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
