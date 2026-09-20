"""Compare, for the same runs: the live event firehose vs the cold-open snapshot."""
import collections
import json
import sqlite3
import subprocess
from pathlib import Path

PROTO = Path(__file__).resolve().parents[1] / "packages" / "rpc" / "proto"
SOCKET = Path.home() / ".future" / "run" / "agent.sock"
db = sqlite3.connect((Path.home() / ".future" / "agent" / "agent.db").as_uri() + "?mode=ro", uri=True)
runs = db.execute("""SELECT e.session_id, e.run_id, count(*) n FROM run_events e
    JOIN runs r ON r.session_id=e.session_id AND r.run_id=e.run_id
    WHERE r.status='completed' GROUP BY 1,2 ORDER BY n DESC LIMIT 6""").fetchall()


def snapshot(session, run):
    out = subprocess.run(
        ["grpcurl", "-plaintext", "-max-msg-sz", "67108864", "-import-path", str(PROTO),
         "-proto", "future.proto",
         "-d", json.dumps({"id": "s", "type": "get_run_snapshot", "session_id": session, "run_id": run}),
         f"unix://{SOCKET}", "proto.FutureAgent/ExecuteCommand"],
        capture_output=True, text=True, check=True).stdout
    return json.loads(json.loads(out)["data"])["projection"]["events"]


print(f"{'sample':7}{'raw ev':>8}{'LIVE wire':>11}{'snapshot':>10}{'ratio':>7}"
      f"{'proj ev':>9}{'tool_delta':>12}{'of snap':>9}")
for index, (session, run, events) in enumerate(runs, 1):
    rows = db.execute("SELECT length(payload) FROM run_events WHERE session_id=? AND run_id=?",
                      (session, run)).fetchall()
    live = sum(row[0] for row in rows)
    projection = snapshot(session, run)
    total = sum(len(e.get("data") or "") + 80 for e in projection)
    deltas = sum(len(e.get("data") or "") + 80 for e in projection if e["type"] == "tool_delta")
    print(f"top{index:<4}{events:>8}{live/1e6:>10.1f}M{total/1024:>9.0f}K{live/total:>7.0f}x"
          f"{len(projection):>9}{deltas/1024:>11.0f}K{100*deltas/total:>8.0f}%")
