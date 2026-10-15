#!/usr/bin/env python3
"""Native embedded-Agent regression gate; called by CI on all three OSes.

Each service/host fixture owns its HOME, FUTURE_HOME and local IPC endpoint.
Run only the lifecycle suites here, not the full unit suites. JSON and logs
under target/embedded-agent-ci are uploaded even when a command fails.
"""

import json
import os
from pathlib import Path
import platform
import re
import subprocess
import time


def check_launchers(root, output, base_env, summary):
    if platform.system() == "Windows":
        command = [str(Path(os.environ["SystemRoot"]) / "System32/cmd.exe"), "/d", "/c",
                   str(root / "scripts/dev/start-desktop-windows.bat")]
    else:
        name = "macos" if platform.system() == "Darwin" else "linux"
        command = ["bash", str(root / f"scripts/dev/start-desktop-{name}.sh")]
    cases = [("auto", "auto", "5173", 0), ("external", "external", "05173", 0),
             ("invalid-mode", "sidecar", "5173", 1), ("uppercase-mode", "AUTO", "5173", 1),
             ("zero-port", "auto", "0", 1), ("long-port", "auto", "0005173", 1),
             ("overflow-port", "auto", "65536", 1), ("invalid-port", "auto", "bad", 1)]
    summary["launchers"] = []
    for name, mode, port, failure in cases:
        env = base_env.copy()
        env.update(DRY_RUN="1", FUTURE_DESKTOP_AGENT_MODE=mode, DESKTOP_DEV_PORT=port,
                   CARGO_TARGET_DIR="target/dev launcher probe")
        result = subprocess.run(command, cwd=root, env=env, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                text=True, encoding="utf-8", errors="replace", timeout=30)
        (output / f"launcher-{name}.log").write_text(result.stdout, encoding="utf-8")
        if bool(result.returncode) != bool(failure):
            raise RuntimeError(f"Launcher {name} returned {result.returncode}: {result.stdout}")
        if not failure:
            expected = str(root / "target/dev launcher probe/debug/future")
            if "DRY_RUN=1" not in result.stdout or expected not in result.stdout:
                raise RuntimeError(f"Launcher {name} lost dry-run or absolute target: {result.stdout}")
        summary["launchers"].append({"name": name, "exit_code": result.returncode})
        print(f"Launcher dry-run {name}: passed", flush=True)


def main():
    root = Path(__file__).resolve().parents[2]
    output = root / "target" / "embedded-agent-ci"
    output.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.setdefault("CARGO_BUILD_JOBS", "2")
    env["CARGO_PROFILE_TEST_DEBUG"] = "1"
    suites = [
        ("connection", ["-p", "future-rpc", "--lib", "connection::tests"]),
        ("service", ["-p", "future-agent", "--test", "service_startup"]),
        (
            "cli-tools",
            ["--manifest-path", "desktop/src-tauri/Cargo.toml",
             "--no-default-features", "--features", "headless",
             "--lib", "cli_tool::tests"],
        ),
        (
            "gui-host",
            ["--manifest-path", "desktop/src-tauri/Cargo.toml", "--lib", "agent_host::tests"],
        ),
        (
            "headless-host",
            [
                "--manifest-path", "desktop/src-tauri/Cargo.toml",
                "--no-default-features", "--features", "headless",
                "--lib", "agent_host::tests",
            ],
        ),
    ]
    summary = {
        "platform": platform.platform(),
        "commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True
        ).strip(),
        "suites": [],
        "status": "running",
    }
    summary_path = output / "summary.json"
    try:
        check_launchers(root, output, env, summary)
        for name, args in suites:
            command = ["cargo", "test", "--locked", *args, "--", "--test-threads=1", "--nocapture"]
            print(f"Running {name}: {' '.join(command)}", flush=True)
            started = time.monotonic()
            passed = 0
            with (output / f"{name}.log").open("w", encoding="utf-8") as log:
                with subprocess.Popen(
                    command, cwd=root, env=env, stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace",
                ) as child:
                    for line in child.stdout:
                        print(line, end="", flush=True)
                        log.write(line)
                        log.flush()
                        match = re.search(r"test result: ok\. (\d+) passed;", line)
                        if match:
                            passed += int(match.group(1))
                    code = child.wait()
            summary["suites"].append({
                "name": name, "command": command, "exit_code": code,
                "passed": passed, "seconds": round(time.monotonic() - started, 3),
            })
            # A renamed filter silently running zero tests is not acceptance.
            if code != 0 or passed == 0:
                raise RuntimeError(f"{name} failed: exit={code}, passed={passed}")
        summary["status"] = "passed"
    except Exception as error:
        summary["status"] = "failed"
        summary["error"] = str(error)
        raise
    finally:
        summary_path.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
