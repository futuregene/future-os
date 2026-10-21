#!/usr/bin/env python3
"""Run the selected Host suite in CI, optionally with an isolated real broker."""

import argparse
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--with-broker", action="store_true")
    parser.add_argument("--suite", choices=("all", "remote", "core"), default="all")
    args = parser.parse_args()
    if os.environ.get("GITHUB_ACTIONS") != "true":
        parser.error("Host automated acceptance runs in CI; do not start local NATS tests")
    command = ["cargo", "test", "--locked", "--no-fail-fast", "-p", "future-host"]
    if args.suite == "remote":
        command.extend(["--", "remote"])
    elif args.suite == "core":
        command.extend(["--", "--skip", "remote"])
    if not args.with_broker:
        return subprocess.call(command, cwd=ROOT)

    # A fresh loopback port and a private broker log. Never attach to, stop or
    # reconfigure a developer's broker or Agent process.
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    with tempfile.TemporaryDirectory(prefix="future-host-ci-") as directory:
        log_path = Path(directory) / "broker.log"
        with log_path.open("wb") as log:
            broker = subprocess.Popen(
                ["nats-server", "-a", "127.0.0.1", "-p", str(port)],
                stdout=log,
                stderr=subprocess.STDOUT,
            )
            try:
                deadline = time.monotonic() + 20
                while True:
                    if broker.poll() is not None:
                        raise RuntimeError("isolated NATS broker exited before readiness")
                    try:
                        with socket.create_connection(("127.0.0.1", port), timeout=1) as probe:
                            with probe.makefile("rb") as stream:
                                greeting = stream.readline(8192)
                            if not greeting.startswith(b"INFO ") or not greeting.endswith(b"\n"):
                                raise RuntimeError("broker did not send a complete NATS INFO greeting")
                        break
                    except (ConnectionError, TimeoutError):
                        if time.monotonic() >= deadline:
                            raise RuntimeError("isolated NATS broker readiness deadline exceeded")
                        time.sleep(0.05)
                environment = dict(os.environ)
                environment["VERIFY_E2E_NATS_URL"] = f"nats://127.0.0.1:{port}"
                print("Host core suite: isolated real NATS broker ready", flush=True)
                return subprocess.call(command, cwd=ROOT, env=environment)
            except Exception:
                print(log_path.read_text(errors="replace")[-4000:], file=sys.stderr)
                raise
            finally:
                broker.terminate()
                try:
                    broker.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    broker.kill()
                    broker.wait(timeout=5)


if __name__ == "__main__":
    sys.exit(main())
