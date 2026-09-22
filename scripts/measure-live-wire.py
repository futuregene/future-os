"""Exact live wire cost: reconstruct the desktop's published body and compare
1:1 forwarding against per-stream time-window coalescing."""
import collections
import datetime
import json
import pathlib
import sqlite3

DELTAS = {"text_chunk", "thinking_delta", "tool_delta", "toolcall_delta", "text_delta"}
# Read-only, so the agent may keep writing while this runs.
DB = pathlib.Path.home() / ".future" / "agent" / "agent.db"
db = sqlite3.connect(DB.as_uri() + "?mode=ro", uri=True)


def body(event_type, data, run_id, idx, epoch, timestamp, session_idx, run_sequence, session_id):
    """Mirror of remote::publisher::build_event_body (schemaVersion 2)."""
    return json.dumps({
        "schemaVersion": 2, "sessionId": session_id, "type": event_type, "data": data,
        "runId": run_id, "idx": idx, "epoch": epoch,
        "eventId": f"{session_id}:{run_id}:{epoch}:{idx}",
        "timestamp": timestamp or "", "sessionIdx": session_idx, "runSequence": run_sequence,
    }, ensure_ascii=False, separators=(",", ":"))


def load(session, run):
    rows = db.execute(
        "SELECT payload FROM run_events WHERE session_id=? AND run_id=? ORDER BY idx",
        (session, run)).fetchall()
    out = []
    for (payload,) in rows:
        event = json.loads(payload)
        out.append({
            "type": event["event_type"], "idx": event.get("idx"),
            "data": event["data"] or "{}", "timestamp": event.get("timestamp"),
            "run_id": run, "epoch": event.get("epoch"),
            "run_sequence": event.get("run_sequence"),
            "session_idx": event.get("session_idx", -1),
        })
    return out


def today(events, session):
    total = 0
    envelope_only = 0
    for e in events:
        size = len(body(e["type"], e["data"], e["run_id"], e["idx"], e["epoch"],
                         e["timestamp"], e.get("session_idx", -1), e["run_sequence"], session))
        total += size
        envelope_only += size - len(e["data"])
    return total, envelope_only


def coalesced(events, session, window_ms):
    total = 0
    published = 0
    pending = {}
    order = []
    window_start = None

    def flush():
        nonlocal total, published
        for key in order:
            entry = pending.pop(key, None)
            if entry is None:
                continue
            data = {"text": entry["text"]}
            if key[1]:
                data["tool_id"] = key[1]
            if entry["count"] > 1:
                data["coalescedCount"] = entry["count"]
            payload = json.dumps(data, ensure_ascii=False, separators=(",", ":"))
            total += len(body(key[0], payload, entry["run_id"], entry["last_idx"], entry["epoch"],
                              entry["timestamp"], -1, entry["run_sequence"], session))
            published += 1
        order.clear()

    for e in events:
        if e["type"] not in DELTAS:
            flush()
            window_start = None
            total += len(body(e["type"], e["data"], e["run_id"], e["idx"], e["epoch"],
                              e["timestamp"], -1, e["run_sequence"], session))
            published += 1
            continue
        try:
            data = json.loads(e["data"])
        except ValueError:
            data = {}
        if not isinstance(data.get("text"), str):
            flush()
            window_start = None
            total += len(body(e["type"], e["data"], e["run_id"], e["idx"], e["epoch"],
                              e["timestamp"], -1, e["run_sequence"], session))
            published += 1
            continue
        key = (e["type"], data.get("tool_id") or data.get("tool_call_id") or "")
        if window_start is None:
            window_start = e["timestamp"]
        entry = pending.get(key)
        if entry is None:
            entry = {"text": "", "count": 0, "last_idx": e["idx"], "run_id": e["run_id"],
                     "epoch": e["epoch"], "run_sequence": e["run_sequence"], "timestamp": e["timestamp"]}
            pending[key] = entry
            order.append(key)
        text = data["text"]
        entry["text"] = text if data.get("snapshot") is True else entry["text"] + text
        entry["count"] += 1
        entry["last_idx"] = e["idx"]
        entry["timestamp"] = e["timestamp"]
        if window_start and e["timestamp"]:
            elapsed = (datetime.datetime.fromisoformat(e["timestamp"])
                       - datetime.datetime.fromisoformat(window_start)).total_seconds() * 1000
            if elapsed >= window_ms:
                flush()
                window_start = None
    flush()
    return total, published


runs = db.execute("""SELECT e.session_id, e.run_id, count(*) n FROM run_events e
    JOIN runs r ON r.session_id=e.session_id AND r.run_id=e.run_id
    WHERE r.status='completed' GROUP BY 1,2 ORDER BY n DESC LIMIT 6""").fetchall()
print(f"{'sample':7}{'events':>9}{'today MB':>10}{'envelope':>10}{'env %':>7}"
      f"{'100ms MB':>10}{'reduction':>11}{'ev/s @60':>10}")
for index, (session, run, _n) in enumerate(runs, 1):
    events = load(session, run)
    base, envelope = today(events, session)
    merged, published = coalesced(events, session, 100)
    print(f"top{index:<4}{len(events):>9}{base/1e6:>10.2f}{envelope/1e6:>10.2f}"
          f"{100*envelope/base:>6.0f}%{merged/1e6:>10.2f}{base/merged:>10.1f}x{published/ (base/1e6/0.017):>10.0f}")
