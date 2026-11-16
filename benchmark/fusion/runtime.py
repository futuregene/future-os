#!/usr/bin/env python3
"""Run an isolated Host with local Fusion settings, leaving the source home intact."""

import argparse
import datetime
import json
import os
import shutil
import socket
import sqlite3
import subprocess
import time
import uuid
from pathlib import Path

from datasets import ROOT, read_json, write_json


def local_settings(source):
    from future_runner import Future

    try:
        client = Future(source["binary"])
        client.env["FUTURE_HOME"] = str(source["home"])
        return client.call(["fusion", "settings", "--json"])
    except (RuntimeError, subprocess.TimeoutExpired):
        pass
    path = source["home"] / "fusion/fusion.db"
    wal = path.with_name("fusion.db-wal")
    if wal.exists():
        raise ValueError(
            "Source Fusion has a WAL and its Agent is unreachable; start the source Host"
        )
    before = path.stat()
    connection = sqlite3.connect(path.as_uri() + "?mode=ro&immutable=1", uri=True)
    try:
        row = connection.execute(
            "SELECT value FROM fusion_meta WHERE key='settings'"
        ).fetchone()
    finally:
        connection.close()
    after = path.stat()
    if wal.exists() or (before.st_mtime_ns, before.st_size) != (
        after.st_mtime_ns,
        after.st_size,
    ):
        raise ValueError("Source Fusion changed during snapshot; retry")
    if row is None:
        raise ValueError(
            "No saved local Fusion configuration; refusing to substitute defaults"
        )
    return json.loads(row[0])


def wait_for_ready(process, home, endpoint, fusion_enabled, binary, env, timeout=60):
    """Publish running only after Agent transport and the Fusion owner are live."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(
                f"Isolated Host exited during startup: {process.returncode}"
            )
        try:
            # Host/Agent startup is lazy. A read-only CLI request activates the
            # owned service; waiting only on a socket would wait for itself.
            probe = subprocess.run(
                [str(binary), "session", "list", "--json"],
                env=env,
                capture_output=True,
                text=True,
                timeout=10,
                check=False,
            )
            if probe.returncode:
                time.sleep(0.25)
                continue
            host, port = endpoint.rsplit(":", 1)
            with socket.create_connection((host, int(port)), timeout=0.5):
                pass
            if fusion_enabled:
                connection = sqlite3.connect(
                    (home / "fusion/fusion.db").as_uri() + "?mode=ro",
                    uri=True,
                    timeout=0.5,
                )
                try:
                    row = connection.execute(
                        "SELECT value FROM fusion_meta WHERE key='host'"
                    ).fetchone()
                finally:
                    connection.close()
                age = (
                    time.time() * 1000 - json.loads(row[0]).get("at", 0)
                    if row
                    else float("inf")
                )
                if not 0 <= age < 5000:
                    time.sleep(0.25)
                    continue
            return
        except (OSError, sqlite3.Error, ValueError, subprocess.TimeoutExpired):
            pass
        time.sleep(0.25)
    raise RuntimeError(
        "Isolated Host startup did not become ready; inspect host-stdio.log"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary", type=Path, default=ROOT.parent.parent / "target/debug/future"
    )
    parser.add_argument(
        "--source-home",
        type=Path,
        default=Path(os.environ.get("FUTURE_HOME", Path.home() / ".future")),
    )
    parser.add_argument(
        "--single-only",
        action="store_true",
        help="start a single-model benchmark without reading saved Fusion settings",
    )
    parser.add_argument(
        "--resume-home", type=Path, help="reuse a stopped benchmark runtime home"
    )
    parser.add_argument(
        "--fusion-settings",
        type=Path,
        help="explicit Fusion settings snapshot for the isolated run",
    )
    args = parser.parse_args()
    if args.single_only and args.fusion_settings:
        parser.error("--fusion-settings requires a Fusion runtime")
    local = (ROOT / ".local").resolve()
    if args.resume_home is None:
        home = local / ("instance-" + uuid.uuid4().hex[:12])
    else:
        home = args.resume_home.resolve()
        if (
            home.parent != local
            or not home.name.startswith("instance-")
            or not (home / "agent").is_dir()
        ):
            parser.error("--resume-home requires a stopped local benchmark instance")
        if (home / "agent").resolve() != home / "agent":
            parser.error("--resume-home requires a private agent directory")
    if home == args.source_home.resolve():
        parser.error("source and benchmark runtime must use separate homes")
    binary = args.binary.resolve()
    settings = None
    if not args.single_only:
        settings = (
            read_json(args.fusion_settings)
            if args.fusion_settings
            else local_settings({"binary": binary, "home": args.source_home.resolve()})
        )
    if settings is not None and not settings["enabled"]:
        raise ValueError("Local Fusion is disabled")
    if args.resume_home:
        previous = ROOT / ".local/runtime.json"
        if previous.exists():
            saved = read_json(previous)
            if (
                saved["home"] == str(home)
                and saved["stored_local_fusion_settings"] != settings
            ):
                raise ValueError("Stored Fusion snapshot changed while resuming")
        if settings is not None and not (home / "fusion/fusion.db").exists():
            raise ValueError("The resumed Fusion runtime has no saved configuration")
    agent = home / "agent"
    agent.mkdir(parents=True, mode=0o700, exist_ok=args.resume_home is not None)
    home.chmod(0o700)
    private = []
    process = None
    log = None
    try:
        for filename in (
            "auth.json",
            "models.json",
            ".future-models-cache.json",
            "settings.json",
        ):
            source = args.source_home / "agent" / filename
            if source.exists():
                destination = agent / filename
                if args.resume_home is not None and filename == "settings.json":
                    continue
                shutil.copyfile(source, destination)
                destination.chmod(0o600)
                if filename != "settings.json":
                    private.append(destination)
        # Seed only settings, never source history, tasks, or pending discussions.
        if settings is not None and args.resume_home is None:
            fusion = home / "fusion"
            fusion.mkdir()
            connection = sqlite3.connect(fusion / "fusion.db")
            connection.execute(
                "CREATE TABLE fusion_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL)"
            )
            connection.execute(
                "INSERT INTO fusion_meta VALUES('settings',?)", (json.dumps(settings),)
            )
            connection.commit()
            connection.close()
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            endpoint = f"127.0.0.1:{reservation.getsockname()[1]}"
        document = {
            "home": str(home),
            "endpoint": endpoint,
            "stored_local_fusion_settings": settings,
        }
        env = dict(os.environ, FUTURE_HOME=str(home), FUTURE_AGENT_GRPC_ADDR=endpoint)
        env.pop("FUTURE_AGENT_SOCKET", None)
        log = (home / "host-stdio.log").open(
            "w" if args.resume_home is None else "a", encoding="utf-8"
        )
        process = subprocess.Popen(
            [str(binary), "host", "--agent-grpc-addr", endpoint],
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        document["host_pid"] = process.pid
        document["phase"] = "starting"
        document["launcher_pid"] = os.getpid()
        document["started_at_utc"] = datetime.datetime.now(
            datetime.timezone.utc
        ).isoformat()
        temporary = ROOT / ".local/runtime-next.json"
        write_json(temporary, document)
        temporary.replace(ROOT / ".local/runtime.json")
        wait_for_ready(process, home, endpoint, settings is not None, binary, env)
        document["phase"] = "running"
        document["ready_at_utc"] = datetime.datetime.now(
            datetime.timezone.utc
        ).isoformat()
        write_json(ROOT / ".local/runtime.json", document)
        print("Isolated runtime: benchmark/fusion/.local/runtime.json", flush=True)
        print("Host output is retained in its private home/host-stdio.log.", flush=True)
        print(
            "Run bench.py in a second terminal. Ctrl+C stops only this Host.",
            flush=True,
        )
        try:
            process.wait()
        except KeyboardInterrupt:
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=20)
    finally:
        try:
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
        finally:
            for path in private:
                path.unlink(missing_ok=True)
            if log is not None:
                log.close()
            snapshot = ROOT / ".local/runtime.json"
            if process is not None and snapshot.exists():
                current = read_json(snapshot)
                # A newer launcher may already own the shared snapshot.
                if current.get("host_pid") == process.pid and current.get(
                    "home"
                ) == str(home):
                    current.update(
                        phase="startup_failed"
                        if current.get("phase") == "starting"
                        else "exited",
                        exit_code=process.poll(),
                        exited_at_utc=datetime.datetime.now(
                            datetime.timezone.utc
                        ).isoformat(),
                    )
                    write_json(snapshot, current)
            print("Removed copied authentication/model configuration.", flush=True)


if __name__ == "__main__":
    main()
