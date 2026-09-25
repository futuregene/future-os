#!/usr/bin/env python3
"""Read-only real trace playback, loopback only. Never starts the user's agent.
Raw events exist only in memory/HTTP responses, never in committed artifacts.
Run after measure-mobile-performance.mjs. Default launches a bounded child;
--serve runs it in the foreground. Stop the printed PID when done.
"""
import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import threading

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "mobile-performance"
DB = Path.home() / ".future" / "agent" / "agent.db"
HTML = """<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'>
<title>Mobile performance — private real trace playback</title>
<style>body{font:16px system-ui;max-width:900px;margin:24px auto;padding:12px}button{padding:14px;margin:8px}pre{white-space:pre-wrap}aside{color:#666}</style>
<h1>Mobile TS performance / energy proxies</h1><aside>Real historical traces, local read-only SQLite. Controlled playback, not live NATS or native React Native UI. No battery-power claim.</aside>
<button id=run>Run 3-round real-trace A/B</button><button id=json>Measure large real JSON decoding</button><button id=markdown>Measure long real Markdown replies</button><button id=back>Test local return response</button>
<p id=status>Ready. Raw conversation contents are never displayed.</p><pre id=output></pre>
<script src=/baseline.js></script><script src=/current.js></script><script src=/probe.js></script>"""


def database():
    return sqlite3.connect(DB.as_uri() + "?mode=ro", uri=True)


def serve():
    os.umask(0o077)
    with database() as db:
        rows = db.execute("""SELECT e.session_id,e.run_id,count(*) n,sum(length(e.payload)) chars
            FROM run_events e JOIN runs r ON r.session_id=e.session_id AND r.run_id=e.run_id
            WHERE r.status='completed' GROUP BY e.session_id,e.run_id ORDER BY n DESC""").fetchall()
    if not rows:
        raise RuntimeError("No completed real traces available")
    samples = [rows[0]]
    medium = next((row for row in rows if 5000 <= row[2] <= 20000), None)
    if medium and medium != samples[0]:
        samples.append(medium)

    class Handler(SimpleHTTPRequestHandler):
        def log_message(self, *_args):
            pass  # Never log raw events or conversation IDs.

        def end_headers(self):
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Security-Policy", "default-src 'self'; connect-src 'self'; style-src 'unsafe-inline'; frame-ancestors 'none'")
            super().end_headers()

        def reply(self, data, kind="application/json"):
            body = data.encode() if isinstance(data, str) else json.dumps(data).encode()
            self.send_response(200)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path == "/":
                return self.reply(HTML, "text/html; charset=utf-8")
            if self.path == "/results":
                file = OUT / "results.json"
                return self.reply(json.loads(file.read_text()) if file.exists() else {"results": []})
            if self.path == "/documents":
                with database() as db:
                    rows = db.execute("""SELECT b.text FROM message_blocks b JOIN entries e
                        ON e.session_id=b.session_id AND e.position=b.entry_position
                        WHERE e.role=? AND b.kind=? AND instr(b.text,?)>0
                        ORDER BY length(b.text) DESC LIMIT 2""", ("assistant", "text", "[")).fetchall()
                return self.reply([{"index": index, "text": row[0]} for index, row in enumerate(rows)])
            if self.path == "/samples":
                return self.reply([{"index": i, "events": row[2], "payloadChars": row[3]} for i, row in enumerate(samples)])
            if self.path.startswith("/sample/"):
                try:
                    index = int(self.path.removeprefix("/sample/"))
                    if not 0 <= index < len(samples):
                        raise ValueError("sample index")
                    s, r, count, _chars = samples[index]
                    with database() as db:
                        rows = db.execute("SELECT idx,payload FROM run_events WHERE session_id=? AND run_id=? ORDER BY sequence", (s, r)).fetchall()
                    events = []
                    for expected, (idx, payload) in enumerate(rows):
                        if idx != expected:
                            raise ValueError("non-contiguous trace; do not silently reindex real events")
                        event = json.loads(payload)
                        events.append({"type": event["event_type"], "data": event["data"], "runId": "trace-run", "idx": idx})
                    if len(events) != count or events[-1]["type"] != "agent_end":
                        raise ValueError("trace changed or is not terminal")
                    return self.reply(events)
                except (ValueError, IndexError, KeyError):
                    return self.send_error(422, "Trace validation failed")
            if self.path in ("/baseline.js", "/current.js", "/probe.js"):
                return super().do_GET()
            self.send_error(404)

        def do_POST(self):
            # Only aggregate metrics from this loopback origin may be saved.
            if self.path != "/results" or self.headers.get("Origin") != f"http://{self.headers.get('Host')}":
                return self.send_error(403)
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 < length < 1024 * 1024:
                return self.send_error(413)
            value = json.loads(self.rfile.read(length))
            (OUT / "results.json").write_text(json.dumps(value, indent=2))
            self.reply({"saved": True})

    server = ThreadingHTTPServer(("127.0.0.1", 0), partial(Handler, directory=str(OUT)))
    info = {"url": f"http://127.0.0.1:{server.server_port}", "samples": [{"events": r[2], "payloadChars": r[3]} for r in samples]}
    (OUT / "ready.json").write_text(json.dumps(info))
    print(json.dumps(info), flush=True)
    timer = threading.Timer(1800, server.shutdown)
    timer.daemon = True
    timer.start()
    try:
        server.serve_forever()
    finally:
        timer.cancel()
        server.server_close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--serve", action="store_true")
    args = parser.parse_args()
    if not (OUT / "probe.js").is_file():
        raise SystemExit("Build the browser probe first")
    if args.serve:
        serve()
    else:
        with (OUT / "server.log").open("w") as log:
            child = subprocess.Popen([sys.executable, __file__, "--serve"], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        print(json.dumps({"pid": child.pid, "ready": str(OUT / "ready.json"), "log": str(OUT / "server.log")}))
