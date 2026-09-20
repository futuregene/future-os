#!/usr/bin/env python3
"""Composition of the semantic run snapshot the phone downloads on a cold open.

Asks the *live* agent for `get_run_snapshot` (read-only) for the heaviest runs
and breaks the transferred projection down by event type, payload bytes and
stream count — the raw material for deciding what a fold could still remove.
"""
import collections
import json
import subprocess
import sqlite3
from pathlib import Path

PROTO = Path(__file__).resolve().parents[1] / "packages" / "rpc" / "proto"
SOCKET = Path.home() / ".future" / "run" / "agent.sock"
DELTA_TYPES = ("text_chunk", "thinking_delta", "tool_delta", "toolcall_delta")


def snapshot(session: str, run: str) -> dict:
    out = subprocess.run(
        ["grpcurl", "-plaintext", "-max-msg-sz", "67108864",
         "-import-path", str(PROTO), "-proto", "future.proto",
         "-d", json.dumps({"id": "snap", "type": "get_run_snapshot",
                           "session_id": session, "run_id": run}),
         f"unix://{SOCKET}", "proto.FutureAgent/ExecuteCommand"],
        capture_output=True, text=True, check=True,
    ).stdout
    return json.loads(json.loads(out)["data"])


def main() -> int:
    db = sqlite3.connect(
        (Path.home() / ".future" / "agent" / "agent.db").as_uri() + "?mode=ro", uri=True)
    runs = db.execute("""SELECT e.session_id, e.run_id, count(*) n FROM run_events e
        JOIN runs r ON r.session_id=e.session_id AND r.run_id=e.run_id
        WHERE r.status='completed' GROUP BY 1,2 ORDER BY n DESC LIMIT 6""").fetchall()
    for index, (session, run, events) in enumerate(runs, 1):
        data = snapshot(session, run)
        if not data.get("runSnapshot"):
            print(f"top{index}: snapshot unavailable")
            continue
        projection = data["projection"]["events"]
        counts = collections.Counter(event["type"] for event in projection)
        sizes = collections.Counter()
        streams = collections.defaultdict(set)
        for event in projection:
            size = len(event.get("data") or "") + 80
            sizes[event["type"]] += size
            if event["type"] in ("tool_delta", "toolcall_delta"):
                try:
                    payload = json.loads(event["data"])
                except ValueError:
                    payload = {}
                streams[event["type"]].add(payload.get("tool_id") or payload.get("tool_call_id") or "?")
        total = sum(sizes.values())
        delta_bytes = sum(sizes[t] for t in DELTA_TYPES)
        print(f"\ntop{index}: raw={events} projected={len(projection)} snapshot={total/1024:.0f} KB, "
              f"delta-carrying events={sum(counts[t] for t in DELTA_TYPES)} "
              f"({delta_bytes/1024:.0f} KB = {100*delta_bytes/total:.0f}%)")
        for name, size in sizes.most_common():
            extra = ""
            if name in ("tool_delta", "toolcall_delta"):
                extra = f"  distinct tool_ids={len(streams[name])}"
            print(f"    {name:18}{counts[name]:>6} events{size/1024:>9.0f} KB{extra}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
