#!/usr/bin/env python3
"""Reproduce the real-E2EE byte measurement for the lean remote lane.

For each of the heaviest completed runs in the local agent database (read-only)
this script:

  1. dumps the run's journal (`run_events.payload`, one JSON object per line),
  2. dumps that session's real `get_session_entries` reply from the live agent
     and converts it to the payload shape the desktop relays (the same
     conversion as `scripts/measure-lean-history.py`, including its
     fail-loudly shape check),
  3. runs `remote::verify_e2e::{measure_real_e2ee_bytes,measure_real_broker_bytes}`
     with both dumps. Those tests start a real bridge (real Noise v2 handshake +
     per-record AEAD), a real subscribed client that decrypts every record,
     replay the journal through the shipping `publish_event`, and read the
     history through the shipping `get_session_entries` command — measuring
     received wire bytes and decrypted bytes in the undeclared and declared
     states.

`--broker fake` measures on the in-process FakeNats (no external process).
`--broker real` first runs the fixture proof (`verify_e2e_real_broker_round_trip`)
and then the same measurement against a real `nats-server` on a loopback port,
printing its version and launch command, and checking the server's own `/connz`
byte accounting against the client-side totals. `--broker both` runs each sample
through both brokers and compares the decoded numbers.

Everything is measured by the Rust test as it runs; this script only starts the
broker, supplies real inputs and prints the raw numbers. It never models the
code under test.

  python3 scripts/measure/verify-e2e-bytes.py --broker both [--samples 3]
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
DESKTOP = ROOT / "desktop" / "src-tauri"
PROTO = ROOT / "packages" / "rpc" / "proto"
SOCKET = Path.home() / ".future" / "run" / "agent.sock"
AGENT_DB = Path.home() / ".future" / "agent" / "agent.db"
WORKDIR = ROOT / "target" / "verify-real-broker"

FAKE_TEST = "remote::verify_e2e::measure_real_e2ee_bytes"
REAL_TEST = "remote::verify_e2e::measure_real_broker_bytes"
REAL_FIXTURE_TEST = "remote::verify_e2e::verify_e2e_real_broker_round_trip"

ENTRY_FIELDS = ["id", "kind", "role", "createdAtMs", "runId", "blocks",
                "metadata", "usage", "run", "session", "checkpoint"]
BLOCK_FIELDS = ["kind", "text", "toolCallId", "name", "arguments", "isError",
                "imageUrl", "providerMetadata", "data"]
JSON_FIELDS = {
    "argumentsJson": "arguments",
    "providerMetadataJson": "providerMetadata",
    "dataJson": "data",
}


class RealBroker:
    """A `nats-server` child process on a loopback port."""

    def __init__(self, process, url, monitor, version, command, log):
        self.process = process
        self.url = url
        self.monitor = monitor
        self.version = version
        self.command = command
        self.log = log

    def stop(self):
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def start_nats_server() -> RealBroker:
    binary = os.environ.get("NATS_SERVER") or shutil.which("nats-server")
    if not binary:
        raise SystemExit("no nats-server on PATH (brew install nats-server), "
                         "and NATS_SERVER is not set")
    version = subprocess.run([binary, "-v"], capture_output=True, text=True,
                             check=True).stdout.strip()
    port, monitor = free_port(), free_port()
    log = WORKDIR / "nats-server.log"
    command = [binary, "-a", "127.0.0.1", "-p", str(port), "-m", str(monitor),
               "-l", str(log)]
    process = subprocess.Popen(command, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True)
    deadline = time.time() + 10
    while time.time() < deadline:
        if process.poll() is not None:
            raise SystemExit(f"nats-server exited early:\n{process.stdout.read()}")
        with socket.socket() as probe:
            probe.settimeout(0.2)
            if probe.connect_ex(("127.0.0.1", port)) == 0:
                break
        time.sleep(0.05)
    else:
        process.kill()
        raise SystemExit("nats-server never accepted a connection")
    return RealBroker(process, f"nats://127.0.0.1:{port}",
                      f"http://127.0.0.1:{monitor}", version, command, log)


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


def run_test(test: str, env_extra: dict) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["cargo", "test", "--lib", test, "--", "--exact", "--ignored",
         "--nocapture", "--test-threads=1"],
        cwd=DESKTOP, env={**os.environ, **env_extra},
        capture_output=True, text=True)


def markers(result: subprocess.CompletedProcess, wanted) -> dict:
    lines = {}
    for line in result.stdout.splitlines():
        for marker, key in wanted:
            if marker in line:
                lines[key] = json.loads(line[line.index(marker) + len(marker):])
    return lines


def measure(journal: Path, session: str, run: str, entries: Path,
            test: str, env_extra: dict) -> dict:
    env = {**env_extra,
           "VERIFY_E2E_JOURNAL": str(journal),
           "VERIFY_E2E_SESSION": session,
           "VERIFY_E2E_RUN": run,
           "VERIFY_E2E_ENTRIES": str(entries)}
    result = run_test(test, env)
    lines = markers(result, [("VERIFY_E2E_LIVE ", "live"),
                             ("VERIFY_E2E_HISTORY ", "history"),
                             ("VERIFY_E2E_NATS_ACCOUNTING ", "accounting")])
    # The markers are printed before the accounting assertion runs, so a
    # crashing test would still look like it reported numbers. The exit code is
    # the only proof the whole test ran.
    if result.returncode != 0 or set(lines) < {"live", "history"}:
        print(result.stdout[-6000:], file=sys.stderr)
        print(result.stderr[-3000:], file=sys.stderr)
        raise SystemExit(f"{test} failed (exit {result.returncode})")
    return lines


def compare(fake: dict, real: dict) -> dict:
    """The decoded numeric results must be identical across brokers: the broker
    relays the same records either way, and anything else is a finding."""
    return {
        "liveEqual": fake["live"] == real["live"],
        "historyEqual": fake["history"] == real["history"],
        "fakeLive": fake["live"],
        "realLive": real["live"],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--broker", choices=["fake", "real", "both"], default="both")
    args = parser.parse_args()

    if not AGENT_DB.exists():
        raise SystemExit(f"no agent database at {AGENT_DB}")
    if not SOCKET.exists():
        raise SystemExit(f"no live agent socket at {SOCKET} "
                         "(start the desktop/TUI agent before measuring history)")
    WORKDIR.mkdir(parents=True, exist_ok=True)

    runs = heavy_runs(args.samples)
    if not runs:
        raise SystemExit("no completed runs in the agent database")

    broker = None
    real_env = {}
    if args.broker in ("real", "both"):
        broker = start_nats_server()
        real_env = {"VERIFY_E2E_NATS_URL": broker.url,
                    "VERIFY_E2E_NATS_MONITOR": broker.monitor}
        print(f"# real broker: {broker.version}")
        print(f"# launch: {' '.join(shlex.quote(part) for part in broker.command)}")
        print(f"# url: {broker.url}  monitor: {broker.monitor}  log: {broker.log}")

    try:
        if broker is not None:
            fixture_result = run_test(REAL_FIXTURE_TEST, real_env)
            found = markers(fixture_result, [("VERIFY_E2E_FIXTURE ", "fixture")])
            if fixture_result.returncode != 0 or "fixture" not in found:
                print(fixture_result.stdout[-6000:], file=sys.stderr)
                print(fixture_result.stderr[-3000:], file=sys.stderr)
                raise SystemExit("the real-broker fixture proof failed "
                                 f"(exit {fixture_result.returncode})")
            print(f"VERIFY_E2E_REAL_FIXTURE "
                  f"{json.dumps({'broker': broker.version, **found['fixture']})}")
            print()

        print(f"# real-E2EE lean-lane measurement: {len(runs)} heaviest completed runs")
        print(f"# agent db: {AGENT_DB}")
        print()
        print(f"{'sample':<7}{'events':>8}  "
              f"{'live undeclared (msg/wire/plain)':>34}  "
              f"{'live declared (msg/wire/plain)':>33}  "
              f"{'history full (un/decl)':>24}  {'history paged (un/decl)':>25}")
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

                results = {}
                if args.broker in ("fake", "both"):
                    results["fake"] = measure(journal, session, run, entries,
                                              FAKE_TEST, {})
                if broker is not None:
                    results["real"] = measure(journal, session, run, entries,
                                              REAL_TEST, real_env)
                for name, lines in results.items():
                    sample = {"broker": name, "session": session, "run": run,
                              **lines}
                    print(f"VERIFY_E2E_SAMPLE {json.dumps(sample, ensure_ascii=False)}")

                if len(results) == 2:
                    verdict = compare(results["fake"], results["real"])
                    print(f"VERIFY_E2E_COMPARE {json.dumps(verdict, ensure_ascii=False)}")
                    if not (verdict["liveEqual"] and verdict["historyEqual"]):
                        raise SystemExit("fake and real broker disagree")

                primary = results.get("real") or results["fake"]
                live = primary["live"]
                history = primary["history"]
                undeclared, declared = live["undeclared"], live["declared"]
                hf_un, hf_de = (history["undeclared"]["full"],
                                history["declared"]["full"])
                hp_un, hp_de = (history["undeclared"]["paged"],
                                history["declared"]["paged"])

                def lane(entry):
                    return f"{entry['messages']}/{entry['wireBytes']}/{entry['plaintextBytes']}"

                print(f"{index:<7}{live['journalEvents']:>8}  "
                      f"{lane(undeclared):>34}  {lane(declared):>33}  "
                      f"{hf_un['plaintextBytes']}/{hf_de['plaintextBytes']:<7}"
                      f"  {hp_un['plaintextBytes']}/{hp_de['plaintextBytes']:<8}")
            finally:
                journal.unlink(missing_ok=True)
                entries.unlink(missing_ok=True)
    finally:
        if broker is not None:
            broker.stop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
