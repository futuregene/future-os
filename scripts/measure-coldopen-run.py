#!/usr/bin/env python3
"""Drive one cold-open measurement end to end, unattended.

Starts the isolated probe (private SQLite snapshot + isolated Agent on a fresh
loopback port), opens the probe page in headless Chrome over CDP, waits for the
scenario to POST its results back, then stops everything. The user's real agent,
desktop and database are never touched.

Usage:
  python3 scripts/measure-coldopen-run.py --bundle <bundle.js> [--samples N]
                                          [--label stepN] [--cdp-port 9333]
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "sync-browser-measurement"
RESULTS = ROOT / "target" / "sync-measurements"


def wait_for(predicate, timeout, what):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.2)
    raise TimeoutError(f"timed out waiting for {what}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bundle", required=True, help="esbuild output for measure-coldopen.ts")
    parser.add_argument("--samples", type=int, default=6)
    parser.add_argument("--label", default="run")
    parser.add_argument("--cdp-port", type=int, default=9333)
    parser.add_argument("--test-binary", required=True)
    parser.add_argument("--agent-binary", help="Standalone future-agent to measure instead of the installed one")
    # Each pass measures every sample in both open modes, and the harness runs
    # the plain and gzip passes against one snapshot, so the wall clock is about
    # twice a single-encoding run.
    parser.add_argument("--timeout", type=int, default=1800)
    args = parser.parse_args()

    RESULTS.mkdir(parents=True, exist_ok=True)
    shutil.rmtree(OUT, ignore_errors=True)
    OUT.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(args.bundle, OUT / "bundle.js")
    (OUT / "results.json").unlink(missing_ok=True)

    launcher_command = [sys.executable, str(ROOT / "scripts" / "measure-sync-browser.py"),
                        "--test-binary", args.test_binary, "--sample-count", str(args.samples),
                        "--html", "measure-coldopen.html"]
    if args.agent_binary:
        launcher_command.extend(["--agent-binary", args.agent_binary])
    launcher = subprocess.run(launcher_command, capture_output=True, text=True, check=True)
    runner = json.loads(launcher.stdout.strip().splitlines()[-1])
    print(f"probe runner pid {runner['runnerPid']}", flush=True)
    try:
        ready = wait_for(
            lambda: json.loads((OUT / "ready.json").read_text()) if (OUT / "ready.json").is_file() else None,
            90, "probe ready.json",
        )
        url = ready["url"]
        print(f"probe url {url} (agent {ready['agentVersion']}, {ready['sourceCompletedRuns']} completed runs)", flush=True)
        steps = OUT / "steps.json"
        steps.write_text(json.dumps([{"wait": 500}]))

        # A private CDP port + profile: never reuses or disturbs the screenshot
        # harness's persistent headless Chrome on 9222.
        chrome = None
        if not _cdp_alive(args.cdp_port):
            chrome = subprocess.Popen([
                _chrome_path(), "--headless=new", f"--remote-debugging-port={args.cdp_port}",
                f"--user-data-dir={OUT / 'chrome-profile'}", "--no-first-run",
                "--no-default-browser-check", "--disable-gpu", "--hide-scrollbars", "about:blank",
            ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            wait_for(lambda: _cdp_alive(args.cdp_port), 30, "CDP port")
        try:
            driver = subprocess.run([
                "node", str(ROOT / "scripts" / "screenshots" / "cdp.mjs"),
                "--port", str(args.cdp_port), "--match", "127.0.0.1",
                "--w", "800", "--h", "600", "--mobile", "false",
                "--url", url, "--ready", "!!window.__measureDone",
                "--ready-timeout", str(args.timeout * 1000), "--steps", str(steps),
            ], capture_output=True, text=True, timeout=args.timeout)
            print(driver.stdout.strip()[-500:], flush=True)
            if driver.returncode != 0:
                print(driver.stderr.strip()[-2000:], file=sys.stderr, flush=True)
        finally:
            if chrome is not None:
                chrome.terminate()

        results = wait_for(
            lambda: (OUT / "results.json").read_text() if (OUT / "results.json").is_file() else None,
            120, "scenario results",
        )
        target = RESULTS / f"coldopen-{args.label}.json"
        target.write_text(results)
        print(f"\nwrote {target}")
        print(results)
    finally:
        _stop(runner["runnerPid"])
    return 0


def _chrome_path() -> str:
    for candidate in ("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                      shutil.which("google-chrome"), shutil.which("chromium")):
        if candidate and Path(candidate).is_file():
            return candidate
    raise SystemExit("no Chrome found")


def _cdp_alive(port: int) -> bool:
    import socket
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.3):
            return True
    except OSError:
        return False


def _stop(runner_pid: int) -> None:
    """SIGTERM the runner: it stops its own agent/probe children and deletes the
    private database snapshot. Never touches any other process."""
    try:
        os.kill(runner_pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    for _ in range(50):
        try:
            os.kill(runner_pid, 0)
        except ProcessLookupError:
            print("probe runner stopped")
            return
        time.sleep(0.2)
    print(f"warning: probe runner {runner_pid} still alive", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
