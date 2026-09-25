#!/usr/bin/env python3
"""Reproduce the real-E2EE byte measurement for the lean remote lane.

For each of the heaviest completed runs in the local agent database (read-only)
this script:

  1. dumps the run's journal (`run_events.payload`, one JSON object per line),
  2. dumps that session's real `get_session_entries` reply from the live agent
     and converts it to the payload shape the desktop relays (the same
     conversion as `scripts/measure-lean-history.py`, including its
     fail-loudly shape check),
  3. runs `remote::verify_e2e::measure_real_e2ee_bytes` with both dumps. That
     test starts a real bridge (real Noise v2 handshake + per-record AEAD), a
     real subscribed client that decrypts every record, replays the journal
     through the shipping `publish_event`, and reads the history through the
     shipping `get_session_entries` command — measuring received wire bytes and
     decrypted bytes in the undeclared and declared states.

Everything is measured by the Rust test as it runs; this script only supplies
real inputs and prints the raw numbers. It never models the code under test.

  python3 scripts/measure/verify-e2e-bytes.py [--samples 3]
"""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
DESKTOP = ROOT / "desktop" / "src-tauri"
PROTO = ROOT / "packages" / "rpc" / "proto"
SOCKET = Path.home() / ".future" / "run" / "agent.sock"
AGENT_DB = Path.home() / ".future" / "agent" / "agent.db"

TEST = "remote::verify_e2e::measure_real_e2ee_bytes"

ENTRY_FIELDS = ["id", "kind", "role", "createdAtMs", "runId", "blocks",
                "metadata", "usage", "run", "session", "checkpoint"]
BLOCK_FIELDS = ["kind", "text", "toolCallId", "name", "arguments", "isError",
                "imageUrl", "providerMetadata", "data"]
JSON_FIELDS = {
    "argumentsJson": "arguments",
    "providerMetadataJson": "providerMetadata",
    "dataJson": "data",
}


def parse_field(value):
    return json.loads(value) if isinstance(value, str) else None


def as_int(value):
    """protojson spells int64 as a decimal *string*; the desktop decodes the
    real protobuf (prost yields numbers), so convert back to the wire view."""
    if isinstance(value, str):
        try:
            return int(value)
        except ValueError:
            return value
    return value


def normalize_ints(value, keys):
    if value is None:
        return None
    return {key: as_int(item) if key in keys else item
            for key, item in value.items()}


def to_payload_shape(proto_entry: dict) -> dict:
    """Rebuild an entry the way the desktop relay serializes it.

    grpcurl answers with protojson, which spells every JSON-valued field as a
    *string* under a `…Json` name; the desktop decodes those into real values
    and re-serializes them under camelCase names. Measuring the protojson form
    would silently skip every nested field.
    """
    blocks = []
    for proto_block in proto_entry.get("blocks", []):
        source = {name: proto_block.get(name) for name in BLOCK_FIELDS}
        for proto_name, payload_name in JSON_FIELDS.items():
            source[payload_name] = parse_field(proto_block.get(proto_name))
        blocks.append({k: v for k, v in source.items() if v is not None})
    source = {
        "id": proto_entry.get("id"),
        "kind": proto_entry.get("kind"),
        "role": proto_entry.get("role"),
        "createdAtMs": as_int(proto_entry.get("createdAtMs")),
        "runId": proto_entry.get("runId"),
        "blocks": blocks,
        "metadata": parse_field(proto_entry.get("metadataJson")),
        "usage": normalize_ints(proto_entry.get("usage"),
                               ["inputTokens", "outputTokens",
                                "cacheReadTokens", "cacheWriteTokens"]),
        "run": normalize_ints(proto_entry.get("run"), ["durationMs"]),
        "session": parse_field(proto_entry.get("sessionJson")),
        "checkpoint": parse_field(proto_entry.get("checkpointJson")),
    }
    return {k: source[k] for k in ENTRY_FIELDS if source[k] is not None}


def dump_journal(session: str, run: str, path: Path) -> int:
    with sqlite3.connect(AGENT_DB.as_uri() + "?mode=ro", uri=True) as db:
        rows = db.execute(
            "SELECT payload FROM run_events WHERE session_id=? AND run_id=? ORDER BY idx",
            (session, run)).fetchall()
    with path.open("w") as sink:
        for (payload,) in rows:
            sink.write(payload)
            sink.write("\n")
    return len(rows)


def dump_entries(session: str, path: Path) -> int:
    out = subprocess.run(
        ["grpcurl", "-plaintext", "-max-msg-sz", "268435456",
         "-import-path", str(PROTO), "-proto", "future.proto",
         "-d", json.dumps({"id": "e1", "type": "get_session_entries",
                           "session_id": session}),
         f"unix://{SOCKET}", "proto.FutureAgent/ExecuteCommand"],
        capture_output=True, text=True, check=True,
    ).stdout
    entries = json.loads(out)["payload"]["getSessionEntries"]["entries"]
    payload = [to_payload_shape(entry) for entry in entries]
    # A shape mismatch would silently skip a trim and understate the saving, so
    # fail loudly instead of reporting a number nobody can reproduce.
    kinds = {block["kind"] for entry in payload for block in entry.get("blocks", [])}
    unknown = kinds - {"reasoning", "tool_call", "tool_result", "text"}
    if unknown:
        raise SystemExit(f"unexpected block kinds in the dump: {sorted(unknown)}")
    if not kinds:
        raise SystemExit("the dump carries no blocks; the shape conversion is wrong")
    path.write_text(json.dumps(payload, separators=(",", ":"), ensure_ascii=False))
    return len(payload)


def heavy_runs(limit: int):
    with sqlite3.connect(AGENT_DB.as_uri() + "?mode=ro", uri=True) as db:
        return db.execute(
            """SELECT e.session_id, e.run_id, count(*) n FROM run_events e
               JOIN runs r ON r.session_id=e.session_id AND r.run_id=e.run_id
               WHERE r.status='completed' GROUP BY 1,2 ORDER BY n DESC LIMIT ?""",
            (limit,)).fetchall()


def measure(journal: Path, session: str, run: str, entries: Path) -> dict:
    env = {**os.environ,
           "VERIFY_E2E_JOURNAL": str(journal),
           "VERIFY_E2E_SESSION": session,
           "VERIFY_E2E_RUN": run,
           "VERIFY_E2E_ENTRIES": str(entries)}
    result = subprocess.run(
        ["cargo", "test", "--lib", TEST, "--", "--exact", "--ignored",
         "--nocapture", "--test-threads=1"],
        cwd=DESKTOP, env=env, capture_output=True, text=True)
    lines = {}
    for line in result.stdout.splitlines():
        for marker, key in (("VERIFY_E2E_LIVE ", "live"),
                            ("VERIFY_E2E_HISTORY ", "history")):
            if marker in line:
                lines[key] = json.loads(line[line.index(marker) + len(marker):])
    if set(lines) != {"live", "history"}:
        print(result.stdout[-6000:], file=sys.stderr)
        print(result.stderr[-3000:], file=sys.stderr)
        raise SystemExit(f"the measurement test produced no VERIFY_E2E markers "
                         f"(exit {result.returncode})")
    return lines


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--samples", type=int, default=3)
    args = parser.parse_args()

    if not AGENT_DB.exists():
        raise SystemExit(f"no agent database at {AGENT_DB}")
    if not SOCKET.exists():
        raise SystemExit(f"no live agent socket at {SOCKET} "
                         "(start the desktop/TUI agent before measuring history)")

    runs = heavy_runs(args.samples)
    if not runs:
        raise SystemExit("no completed runs in the agent database")

    print(f"# real-E2EE lean-lane measurement: {len(runs)} heaviest completed runs")
    print(f"# agent db: {AGENT_DB}")
    print(f"# test: {TEST} (real Noise handshake + per-record AEAD, real bridge)")
    print()
    header = (f"{'sample':<7}{'events':>8}  "
              f"{'live undeclared (msg/wire/plain)':>34}  "
              f"{'live declared (msg/wire/plain)':>33}  "
              f"{'history full (un/decl)':>24}  {'history paged (un/decl)':>25}")
    print(header)
    raw = []
    for index, (session, run, count) in enumerate(runs, 1):
        with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as handle:
            journal = Path(handle.name)
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            entries = Path(handle.name)
        try:
            dumped = dump_journal(session, run, journal)
            if dumped != count:
                raise SystemExit(f"journal dump incomplete: {dumped} != {count}")
            dump_entries(session, entries)
            lines = measure(journal, session, run, entries)
            raw.append({"session": session, "run": run, **lines})

            live, history = lines["live"], lines["history"]
            undeclared, declared = live["undeclared"], live["declared"]
            hf_un, hf_de = history["undeclared"]["full"], history["declared"]["full"]
            hp_un, hp_de = history["undeclared"]["paged"], history["declared"]["paged"]

            def lane(entry):
                return f"{entry['messages']}/{entry['wireBytes']}/{entry['plaintextBytes']}"

            print(f"{index:<7}{live['journalEvents']:>8}  "
                  f"{lane(undeclared):>34}  {lane(declared):>33}  "
                  f"{hf_un['plaintextBytes']}/{hf_de['plaintextBytes']:<7}"
                  f"  {hp_un['plaintextBytes']}/{hp_de['plaintextBytes']:<8}")
        finally:
            journal.unlink(missing_ok=True)
            entries.unlink(missing_ok=True)

    print()
    print("# raw measurement lines (one JSON per sample)")
    for sample in raw:
        print(f"VERIFY_E2E_SAMPLE {json.dumps(sample, ensure_ascii=False)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
