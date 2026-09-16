#!/usr/bin/env python3
"""Serve a read-only browser probe using an isolated copy of the real agent DB.
Only the snapshot is writable; never starts/stops the user's agent or copies auth.
Build the ignored Rust test first; pass its executable via --test-binary.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "sync-browser-measurement"


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_port(port, process):
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"child exited {process.returncode}; inspect local logs")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.1)
    raise TimeoutError(f"child did not open loopback port {port}")


def serve(binary):
    os.umask(0o077)
    OUT.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(ROOT / "scripts" / "measure-sync-browser.html", OUT / "index.html")
    if not (OUT / "bundle.js").is_file():
        raise RuntimeError("build measure-sync-browser.ts with esbuild before starting")
    children = []
    def stop(_signal, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    # An online SQLite backup gives one consistent snapshot including WAL.
    with tempfile.TemporaryDirectory(prefix="isolated-", dir=OUT) as temporary:
        home = Path(temporary)
        database = home / ".future" / "agent" / "agent.db"
        database.parent.mkdir(parents=True)
        source = Path.home() / ".future" / "agent" / "agent.db"
        print("Creating consistent private database snapshot (no credentials copied)", flush=True)
        with sqlite3.connect(source.as_uri() + "?mode=ro", uri=True) as src, sqlite3.connect(database) as dst:
            src.backup(dst, pages=1024)
            runs = dst.execute("""SELECT e.session_id,e.run_id,count(*) n FROM run_events e
                JOIN runs r ON r.session_id=e.session_id AND r.run_id=e.run_id
                WHERE r.status='completed' GROUP BY e.session_id,e.run_id ORDER BY n DESC""").fetchall()
        chosen = [("largest", runs[0]), ("second", runs[1]), ("median_completed", runs[len(runs)//2])]
        samples = [{"label": label, "session": row[0], "run": row[1], "expected_events": row[2]} for label,row in chosen]
        (home / "MEASUREMENT_ISOLATED_HOME").touch()
        agent_port, web_port = free_port(), free_port()
        while web_port == agent_port:
            web_port = free_port()
        env = {key: value for key,value in os.environ.items() if not key.startswith("FUTURE_")}
        env.update(HOME=str(home), USERPROFILE=str(home), FUTURE_AGENT_GRPC_ADDR=f"http://127.0.0.1:{agent_port}",
                   SYNC_MEASURE_SAMPLES=json.dumps(samples), SYNC_MEASURE_WEB_ROOT=str(OUT), SYNC_MEASURE_WEB_PORT=str(web_port))
        try:
            with (OUT / "agent.log").open("w") as agent_log, (OUT / "probe.log").open("w") as probe_log:
                agent = subprocess.Popen([shutil.which("future"), "agent", "--grpc-addr", f"127.0.0.1:{agent_port}"],
                                         env=env, cwd=home, stdout=agent_log, stderr=subprocess.STDOUT)
                children.append(agent)
                wait_port(agent_port, agent)
                probe = subprocess.Popen([binary, "remote_host::sync_measurement::serve_real_snapshot", "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                                         env=env, cwd=ROOT, stdout=probe_log, stderr=subprocess.STDOUT)
                children.append(probe)
                wait_port(web_port, probe)
                info = {"pid":os.getpid(), "agentPid":agent.pid, "probePid":probe.pid, "url":f"http://127.0.0.1:{web_port}",
                        "samples":[{"label":s["label"],"events":s["expected_events"]} for s in samples],
                        "sourceCompletedRuns":len(runs), "agentVersion":subprocess.check_output([shutil.which("future"),"--version"],text=True).strip()}
                (OUT / "ready.json").write_text(json.dumps(info,indent=2))
                print(json.dumps(info), flush=True)
                # Hard lifetime bound; stop manually after reading the measurements.
                probe.wait(timeout=1800)
        finally:
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait()
    print("Isolated agent stopped; database snapshot deleted", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--test-binary", required=True)
    parser.add_argument("--serve", action="store_true")
    args = parser.parse_args()
    if args.serve:
        try:
            serve(args.test_binary)
        except KeyboardInterrupt:
            pass
    else:
        OUT.mkdir(parents=True, exist_ok=True)
        with (OUT / "runner.log").open("w") as log:
            process = subprocess.Popen([sys.executable, __file__, "--serve", "--test-binary", args.test_binary],
                                       stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        print(json.dumps({"runnerPid":process.pid,"statusFile":str(OUT/"ready.json"),"log":str(OUT/"runner.log")}))
