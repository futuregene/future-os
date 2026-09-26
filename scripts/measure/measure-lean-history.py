#!/usr/bin/env python3
"""Measure the lean history page: reasoning bodies, tool output and unused
tool-call arguments, against real sessions.

Dumps each session's `get_session_entries` reply from the live (read-only) agent
and runs the shipping Rust trim over it, so the numbers come from the code that
serves the phone rather than from a model of it.

  python3 scripts/measure/measure-lean-history.py [--sessions 3]
  python3 scripts/measure/measure-lean-history.py --pages 3   # per phone page

With `--pages N` each session is cut into the backward pages the phone actually
requests (`N` user exchanges each, mirroring `history_index::read_page`'s
backward rule) and every page is measured separately — the whole-session row is
not what a phone pays, since it never reads the session in one go.

Paging is data selection only: the trim and the byte counts always come from the
shipping Rust code (`remote_host::lean`), never from a Python reimplementation.
"""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
PROTO = ROOT / "packages" / "rpc" / "proto"
SOCKET = Path.home() / ".future" / "run" / "agent.sock"
# The phone's backward cursor for "start at the newest" (mobile sends the same
# sentinel through `latestTimelineWindow`).
NEWEST_PAGE_CURSOR = 9_007_199_254_740_991

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


def to_payload_shape(proto_entry: dict) -> dict:
    """Rebuild the entry as the desktop's relay serializes it.

    grpcurl answers with protojson, which spells every JSON-valued field as a
    *string* under a `…Json` name. The desktop decodes those into real values and
    re-serializes them under camelCase names, and that is the shape the trim (and
    the phone) sees. Measuring the protojson form would silently skip every
    nested field — `arguments` would never be found, so the argument trim would
    never run, and the report would understate the saving.
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
        "createdAtMs": proto_entry.get("createdAtMs"),
        "runId": proto_entry.get("runId"),
        "blocks": blocks,
        "metadata": parse_field(proto_entry.get("metadataJson")),
        "usage": proto_entry.get("usage"),
        "run": proto_entry.get("run"),
        "session": parse_field(proto_entry.get("sessionJson")),
        "checkpoint": parse_field(proto_entry.get("checkpointJson")),
    }
    return {k: source[k] for k in ENTRY_FIELDS if source[k] is not None}


def dump_phone_page(session: str, path: Path, exchanges: int) -> tuple[int, int]:
    """Dump the page the phone actually asks for.

    Not the same as `dump`: the phone sends `before` (a backward cursor) and
    `limit`, so the *agent* selects the newest `exchanges` user exchanges before
    the bridge ever sees the page. Dumping the whole session instead measures
    whatever the 512 KiB budget happens to keep from it, which differs whenever
    the phone's own page would have fit.

    Returns `(entryCount, bytes)` of the raw page for the report's context.
    """
    out = subprocess.run(
        ["grpcurl", "-plaintext", "-max-msg-sz", "268435456",
         "-import-path", str(PROTO), "-proto", "future.proto",
         "-d", json.dumps({"id": "e1", "type": "get_session_entries",
                           "session_id": session,
                           "before": NEWEST_PAGE_CURSOR,
                           "limit": exchanges}),
         f"unix://{SOCKET}", "proto.FutureAgent/ExecuteCommand"],
        capture_output=True, text=True, check=True,
    ).stdout
    entries = json.loads(out)["payload"]["getSessionEntries"]["entries"]
    if not entries:
        raise SystemExit("the phone page dump is empty; the cursor or limit is wrong")
    roles = {entry.get("role") for entry in entries}
    if "user" not in roles:
        raise SystemExit(f"the phone page has no user entry (roles={roles}); "
                         "backward paging would not have selected an exchange")
    payload = [to_payload_shape(entry) for entry in entries]
    path.write_text(json.dumps(payload, separators=(",", ":"), ensure_ascii=False))
    return len(payload), len(json.dumps(payload, separators=(",", ":")))


def dump(session: str, path: Path) -> None:
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
    path.write_text(json.dumps(payload, separators=(",", ":"), ensure_ascii=False))
    # A shape mismatch would silently skip a trim and understate the saving, so
    # fail loudly instead of reporting a number nobody can reproduce.
    trimmable = {
        block["kind"]
        for entry in payload
        for block in entry.get("blocks", [])
    }
    unknown = trimmable - {"reasoning", "tool_call", "tool_result", "text"}
    if unknown:
        raise SystemExit(f"unexpected block kinds in the dump: {sorted(unknown)}")
    if not trimmable:
        raise SystemExit("the dump carries no blocks; the shape conversion is wrong")


def cut_pages(entries: list, exchanges: int) -> list[tuple[int, int]]:
    """The backward pages the phone reads, as `history_index::read_page` cuts
    them: a page ends at the previous page's first user entry and starts at the
    `exchanges`-th user entry before that (0 when fewer remain).

    Page 1 is the newest — the first screen the phone asks for — and the last
    page is the oldest, so page numbers match the order they arrive in."""
    pages: list[tuple[int, int]] = []
    end = len(entries)
    while end > 0:
        users = [i for i, entry in enumerate(entries[:end]) if entry.get("role") == "user"]
        start = users[-exchanges] if len(users) > exchanges else 0
        pages.append((start, end))
        if start == 0:
            break
        end = start
    return pages


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sessions", type=int, default=3)
    parser.add_argument("--pages", type=int, default=0,
                        help="measure in pages of this many user exchanges instead of whole sessions")
    parser.add_argument("--phone-page", type=int, metavar="N", default=0,
                        help="measure the page the phone requests (newest N user "
                             "exchanges) through the shipping trim and 512 KiB budget")
    parser.add_argument("--session", action="append", default=[], metavar="ID",
                        help="measure this session id (repeatable); overrides --sessions")
    args = parser.parse_args()

    source = Path.home() / ".future" / "agent" / "agent.db"
    with sqlite3.connect(source.as_uri() + "?mode=ro", uri=True) as db:
        if args.session:
            rows = [(session, db.execute("SELECT count(*) FROM entries WHERE session_id=?",
                                         (session,)).fetchone()[0])
                    for session in args.session]
        else:
            rows = db.execute(
                "SELECT session_id, count(*) n FROM entries GROUP BY 1 ORDER BY n DESC LIMIT ?",
                (args.sessions,)).fetchall()
    if not rows:
        raise SystemExit("no sessions in the agent database")

    # The newest test binary: a worktree can hold several (one per toolchain),
    # and measuring with a stale one reports the previous revision's trim.
    candidates = sorted(
        (path for path in
         (ROOT / "desktop" / "src-tauri" / "target" / "debug" / "deps").glob("futureos_lib-*")
         if path.is_file() and os.access(path, os.X_OK)),
        key=lambda path: path.stat().st_mtime, reverse=True)
    binary = candidates[0] if candidates else None
    if binary is None:
        raise SystemExit("build the desktop test binary first "
                         "(cd desktop/src-tauri && cargo test --lib lean --no-run)")

    print(f"{'session':<9}{'entries':>9}{'before MiB':>12}{'after MiB':>11}{'saved':>8}"
          f"   blocks (count)")
    for index, (session, _count) in enumerate(rows, 1):
        if args.phone_page:
            with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
                page_path = Path(handle.name)
            try:
                count, raw_bytes = dump_phone_page(session, page_path, args.phone_page)
                result = subprocess.run(
                    [str(binary),
                     "remote_host::business::wire_limits::measure_phone_page_tests::measure_phone_page",
                     "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                    env={**os.environ, "VERIFY_E2E_PHONE_PAGE": str(page_path)},
                    capture_output=True, text=True, check=True)
                line = next(
                    (row for row in result.stdout.splitlines()
                     if "VERIFY_E2E_PHONE_PAGE " in row), None)
                if line is None:
                    print(f"top{index}: no measurement\n{result.stdout[-1500:]}")
                    continue
                data = json.loads(line[line.index("VERIFY_E2E_PHONE_PAGE ")
                                       + len("VERIFY_E2E_PHONE_PAGE "):])
                un, de = data["undeclared"], data["declared"]
                print(f"top{index:<6}{count:>9}{raw_bytes/2**20:>12.2f}"
                      f"{de['wireBytes']/2**20:>11.2f}{data['saved']*100:>7.1f}%"
                      f"   page {count} -> {un['entries']}/{de['entries']} entries; "
                      f"wire {un['wireBytes']} -> {de['wireBytes']}")
            finally:
                page_path.unlink(missing_ok=True)
            continue
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            entries_path = Path(handle.name)
        try:
            dump(session, entries_path)
            entries = json.loads(entries_path.read_text())
            # One measurement per page (the phone never reads a session whole);
            # without --pages the whole session is one "page".
            spans = cut_pages(entries, args.pages) if args.pages else [(0, len(entries))]
            if args.pages and sum(end - start for start, end in spans) != len(entries):
                raise SystemExit(
                    f"page cutting lost entries: {sum(e - s for s, e in spans)} of {len(entries)}")
            for page, (start, end) in enumerate(spans, 1):
                label = f"top{index}" if not args.pages else f"top{index}p{page}"
                # Only the *selection* is Python's; the trim and byte counts are
                # the shipping Rust code's (measure_real_entries).
                with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as page_handle:
                    page_path = Path(page_handle.name)
                page_path.write_text(json.dumps(entries[start:end], separators=(",", ":")))
                try:
                    result = subprocess.run(
                        [str(binary), "remote_host::lean::tests::measure_real_entries",
                         "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                        env={**os.environ, "LEAN_HISTORY_ENTRIES": str(page_path)},
                        capture_output=True, text=True, check=True)
                finally:
                    page_path.unlink(missing_ok=True)
                line = next(
                    (row for row in result.stdout.splitlines()
                     if "LEAN_HISTORY " in row), None)
                if line is None:
                    print(f"{label}: no measurement\n{result.stdout[-1500:]}")
                    continue
                data = json.loads(
                    line[line.index("LEAN_HISTORY ") + len("LEAN_HISTORY "):])
                print(f"{label:<9}{data['entries']:>9}"
                      f"{data['beforeBytes']/2**20:>12.2f}{data['afterBytes']/2**20:>11.2f}"
                      f"{data['saved']*100:>7.1f}%   {data['blockCounts']}")
        finally:
            entries_path.unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
