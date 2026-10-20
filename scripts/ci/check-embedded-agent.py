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
        # Shared lifecycle assertions run in their owner once, independent of
        # either entry's features. Keep real entry effects in separate suites.
        ("shared-host", ["-p", "future-host", "--lib", "agent_host::"]),
        (
            "gui-entry-state",
            ["-p", "futureos", "--lib",
             "gui::startup_paths_tests::process_state_initializes_without_a_window"],
        ),
        (
            "gui-entry-providers",
            ["-p", "futureos", "--lib",
             "commands::providers::tests::command_wrappers_delegate_to_the_agent"],
        ),
        (
            "gui-entry-ready",
            ["-p", "futureos", "--lib",
             "gui::startup_paths_tests::the_ready_event_starts_the_window_work_and_serves_the_terminal"],
        ),
        (
            "gui-entry-exit",
            ["-p", "futureos", "--lib",
             "gui::startup_paths_tests::the_exit_event_closes_admission_and_publishes_the_stop"],
        ),
        (
            "headless-entry",
            ["-p", "futureos", "--no-default-features", "--features", "headless",
             "--test", "headless_cli"],
        ),
    ]
    if platform.system() == "Windows":
        suites.append((
            "windows-gui-image",
            ["-p", "futureos", "--test", "windows_stack"],
        ))
        suites.append((
            "windows-shell-console",
            ["-p", "future-agent", "--lib",
             "sandbox::backend::tests::plain_windows_shell_runs_without_a_console"],
        ))
        suites.append((
            "windows-shell-detached-host",
            ["-p", "future-agent", "--lib",
             "rpc::session::tests::shells_run_after_console_detach"],
        ))
        suites.append((
            "windows-rpc-shell-console",
            ["-p", "future-agent", "--lib",
             "rpc::session::tests::execute_shell_does_not_create_a_console"],
        ))
    summary = {
        "platform": platform.platform(),
        "commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True, encoding="utf-8"
        ).strip(),
        "suites": [],
        "status": "running",
    }
    summary_path = output / "summary.json"
    try:
        for name, args in suites:
            command = ["cargo", "test", "--locked", *args]
            command += ["--", "--test-threads=1", "--nocapture"]
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
