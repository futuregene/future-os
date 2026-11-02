#!/usr/bin/env python3
"""Scan the Agent's stored session stream for U+FFFD replacement characters.

Answers one question when someone reports "garbled output in the desktop": did
this machine corrupt the text, or did the character arrive already damaged? It
only reads. It never writes to the database, starts an Agent, or calls a model.

Run it by hand:

    python3 scripts/scan_replacement_chars.py
    python3 scripts/scan_replacement_chars.py --session 0de8366a-064f-41a4-821a-fec39f8c4b82
    python3 scripts/scan_replacement_chars.py --context 60 --json > fffd.json

Not part of CI: a hit is worth recording, not worth gating a build on.

Why the deltas answer the question
----------------------------------
`stream_changes` is the append-only record of what a provider's stream sent: one
row per event, with the body bytes in `content_objects` / `content_segments`.
The Agent decodes every SSE line with `std::str::from_utf8`
(`agent/src/llm/sse.rs`) and fails the run on invalid bytes, so a character can
only reach this table already valid. A row that holds the three bytes
`EF BF BD` therefore carries the replacement character as a code point, not a
truncated sequence.

Read a hit like this:

* `position=interior` — the U+FFFD sits inside one delta, the rest of which is
  intact, and the content object is valid UTF-8. The provider/model sent the
  replacement character; nothing was lost locally and there is nothing to
  repair.
* `position=edge` — it sits at the first or last character of a delta, or the
  same passage is damaged at different places in different runs. That is the
  shape of a local byte split: check whether the neighbouring delta continues
  the same character.
* `not valid UTF-8` above zero — the stored bytes are not a character encoding
  at all. That is local damage and a bug, not an upstream dropped character.

Two traps this script already avoids:

* A segment boundary is a *byte* offset, so `content_segments` can end inside a
  multi-byte character. The bytes must be joined before decoding; decoding each
  segment on its own invents replacement characters that are not in the data.
* A snippet from `future session history` is byte-bounded and can show
  replacement characters at its own edges for the same reason. Read the whole
  entry before concluding the record is damaged.
"""

from __future__ import annotations

import argparse
import json
import os
import sqlite3
import sys
import uuid
from pathlib import Path

# Frozen, append-only event codes; see `KINDS` in
# agent/src/session/content/compact.rs. A code outside this list is reported
# numerically instead of guessed.
KINDS = (
    "agent_start",
    "agent_end",
    "message_start",
    "message_end",
    "text_start",
    "text_chunk",
    "text_end",
    "thinking_start",
    "thinking_delta",
    "thinking_end",
    "tool_start",
    "tool_delta",
    "tool_input_end",
    "tool_end",
    "tool_result",
    "usage",
    "model_changed",
    "tool_sandboxed",
)

REPLACEMENT = "\ufffd"
REPLACEMENT_BYTES = b"\xef\xbf\xbd"

QUERY = """
SELECT c.id, r.session_id, r.run_id, c.event_kind, c.unknown_kind, c.first_cursor, c.last_cursor,
       c.content_id, c.byte_offset, c.byte_length
FROM stream_changes c
JOIN stream_runs r ON r.id = c.stream_id
WHERE c.content_id IS NOT NULL{session_filter}
ORDER BY c.content_id, c.id
"""


def kind_name(code: int, unknown: str | None) -> str:
    """`event_kind = 0` means the writer did not know it; the name is in
    `unknown_kind`."""
    if 1 <= code <= len(KINDS):
        return KINDS[code - 1]
    if unknown:
        return str(unknown)
    return f"kind_{code}"


def default_db() -> Path:
    home = os.environ.get("FUTURE_HOME")
    root = Path(home) if home else Path.home() / ".future"
    return root / "agent" / "agent.db"


def session_variants(value: str) -> tuple[bytes | None, str]:
    """The same session id in both shapes the stream table has used."""
    text = value.strip()
    try:
        return uuid.UUID(text).bytes, text
    except ValueError:
        pass
    compact = text.replace("-", "")
    if len(compact) == 32:
        try:
            return bytes.fromhex(compact), text
        except ValueError:
            pass
    return None, text


def show_id(value) -> str:
    if isinstance(value, (bytes, bytearray)):
        if len(value) == 16:
            return str(uuid.UUID(bytes=bytes(value)))
        return value.decode("utf-8", "replace")
    return str(value)


def open_read_only(path: Path) -> sqlite3.Connection:
    return sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True, timeout=15)


def require_tables(connection: sqlite3.Connection) -> None:
    names = {row[0] for row in connection.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    missing = [name for name in ("stream_changes", "stream_runs", "content_segments") if name not in names]
    if missing:
        raise SystemExit(
            "this database has no " + ", ".join(missing) + " table: it predates the stream layout, "
            "so there are no deltas to scan (the text lives in message_blocks instead)"
        )


class Contents:
    """Content bodies, read one object at a time.

    `content_segments` hold byte ranges of one body and can end mid-character,
    so the segments are joined before anything is decoded.
    """

    def __init__(self, connection: sqlite3.Connection) -> None:
        self.connection = connection
        self.cached_id: int | None = None
        self.cached_body = b""
        self.read = 0
        self.invalid = 0

    def body_of(self, content_id: int) -> bytes:
        if self.cached_id == content_id:
            return self.cached_body
        segments = self.connection.execute(
            "SELECT body FROM content_segments WHERE content_id = ? ORDER BY byte_offset",
            (content_id,),
        ).fetchall()
        body = b"".join(
            segment[0] if isinstance(segment[0], (bytes, bytearray)) else str(segment[0]).encode()
            for segment in segments
        )
        self.read += 1
        try:
            body.decode("utf-8")
        except UnicodeDecodeError:
            self.invalid += 1
        self.cached_id, self.cached_body = content_id, body
        return body


def context_of(text: str, start: int, end: int, width: int) -> str:
    left = max(0, start - width)
    right = min(len(text), end + width)
    prefix = "…" if left > 0 else ""
    suffix = "…" if right < len(text) else ""
    count = end - start
    marker = "[U+FFFD]" if count == 1 else f"[U+FFFD×{count}]"
    return f"{prefix}{text[left:start]}{marker}{text[end:right]}{suffix}"


def scan(connection: sqlite3.Connection, session: str | None, width: int) -> dict:
    contents = Contents(connection)
    session_filter = ""
    parameters: tuple = ()
    if session:
        raw, text = session_variants(session)
        session_filter = " AND (r.session_id = ? OR r.session_id = ?)"
        parameters = (raw, text)

    scanned = 0
    hits: list[dict] = []
    for (
        change_id,
        session_id,
        run_id,
        kind,
        unknown_kind,
        first_cursor,
        last_cursor,
        content_id,
        offset,
        length,
    ) in connection.execute(QUERY.format(session_filter=session_filter), parameters):
        scanned += 1
        chunk = contents.body_of(content_id)[offset : offset + length]
        if REPLACEMENT_BYTES not in chunk:
            continue
        # Display only: the body was validated as UTF-8 when it was read.
        text = chunk.decode("utf-8", "replace")
        index = 0
        while index < len(text):
            if text[index] != REPLACEMENT:
                index += 1
                continue
            start = index
            while index < len(text) and text[index] == REPLACEMENT:
                index += 1
            hits.append(
                {
                    "sessionId": show_id(session_id),
                    "runId": show_id(run_id),
                    "changeId": change_id,
                    "kind": kind_name(kind, unknown_kind),
                    "kindCode": kind,
                    "cursor": [first_cursor, last_cursor],
                    "contentId": content_id,
                    "byteOffset": offset,
                    "byteLength": length,
                    "charOffset": start,
                    "position": "edge" if start == 0 or index == len(text) else "interior",
                    "context": context_of(text, start, index, width),
                }
            )
    return {
        "deltasScanned": scanned,
        "contentObjects": contents.read,
        "invalidContents": contents.invalid,
        "hits": hits,
    }


def group_hits(hits: list[dict], per_group: int) -> list[dict]:
    groups: dict[tuple[str, str], list[dict]] = {}
    for hit in hits:
        groups.setdefault((hit["sessionId"], hit["runId"]), []).append(hit)
    grouped = []
    for (session_id, run_id), items in groups.items():
        grouped.append(
            {
                "sessionId": session_id,
                "runId": run_id,
                "hits": len(items),
                "kinds": sorted({item["kind"] for item in items}),
                "shown": items[:per_group],
            }
        )
    return grouped


def print_report(report: dict, per_group: int) -> None:
    hits = report["hits"]
    kinds: dict[str, int] = {}
    for hit in hits:
        kinds[hit["kind"]] = kinds.get(hit["kind"], 0) + 1
    print(f"database: {report['database']}")
    print(
        f"deltas scanned: {report['deltasScanned']}   "
        f"with U+FFFD: {len(hits)}   sessions: {len({hit['sessionId'] for hit in hits})}"
    )
    if kinds:
        print("  by kind: " + ", ".join(f"{name}={count}" for name, count in sorted(kinds.items())))
    print(f"content objects read: {report['contentObjects']}   not valid UTF-8: {report['invalidContents']}")
    if report["invalidContents"]:
        print("  ! bytes that are not UTF-8 at all mean local damage, not an upstream dropped character")
    if not hits:
        print("\nno replacement characters in the recorded stream")
        return

    edge = sum(1 for hit in hits if hit["position"] == "edge")
    print(
        f"\npositions: interior={len(hits) - edge} (provider sent it)   "
        f"edge={edge} (possible local byte split)"
    )
    for group in group_hits(hits, per_group):
        print(f"\nsession {group['sessionId']}")
        print(f"  run {group['runId']}   {group['hits']} delta(s)   kinds={','.join(group['kinds'])}")
        for hit in group["shown"]:
            first, last = hit["cursor"]
            print(
                f"    [{hit['kind']}] change {hit['changeId']}  cursor {first}..{last}"
                f"  content {hit['contentId']}+{hit['byteOffset']}  position={hit['position']}"
                f"  charOffset={hit['charOffset']}"
            )
            print(f"      {hit['context']}")
        if group["hits"] > len(group["shown"]):
            print(f"    … {group['hits'] - len(group['shown'])} more (raise --max-per-group)")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Read-only scan of Agent session deltas for U+FFFD.")
    parser.add_argument("--db", help="agent.db path (default: $FUTURE_HOME/agent, else ~/.future/agent)")
    parser.add_argument("--session", help="restrict the scan to one session id (UUID or legacy text id)")
    parser.add_argument("--context", type=int, default=30, help="characters of context per side (default 30, max 500)")
    parser.add_argument("--max-per-group", type=int, default=5, help="hits printed per session/run (default 5; the count is always complete)")
    parser.add_argument("--json", action="store_true", help="emit one JSON object instead of the report")
    arguments = parser.parse_args(argv)
    # Always UTF-8: a Windows console is UTF-8 (PEP 528), and a redirected run
    # is easier to grep than the locale code page. U+FFFD survives either way;
    # only a console that cannot encode the surrounding text degrades to "?".
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError):
        pass

    width = max(0, min(arguments.context, 500))
    per_group = max(0, arguments.max_per_group)
    path = Path(arguments.db) if arguments.db else default_db()
    if not path.exists():
        print(f"no database at {path}", file=sys.stderr)
        return 1

    connection = None
    try:
        connection = open_read_only(path)
        require_tables(connection)
        report = scan(connection, arguments.session, width)
    except sqlite3.OperationalError as error:
        print(f"could not read {path}: {error}", file=sys.stderr)
        if "locked" in str(error) or "busy" in str(error):
            print("another connection is writing right now; retry in a few seconds", file=sys.stderr)
        return 1
    finally:
        if connection is not None:
            connection.close()

    report["database"] = str(path)
    report["groups"] = group_hits(report["hits"], per_group)
    if arguments.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
    else:
        print_report(report, per_group)
    return 0


if __name__ == "__main__":
    try:
        status = main(sys.argv[1:])
        sys.stdout.flush()
    except BrokenPipeError:
        # Piping into `head` closes stdout early; a report is not an error.
        os.dup2(os.open(os.devnull, os.O_WRONLY), sys.stdout.fileno())
        status = 0
    raise SystemExit(status)
