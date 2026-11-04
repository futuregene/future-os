#!/usr/bin/env python3
"""Exercise the built Host/CLI against a local SSE fixture; no paid model calls."""

import argparse
import hashlib
import json
import signal
import socket
import sqlite3
import subprocess
import threading
import time
import uuid
from collections import Counter
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch

from datasets import ROOT, digest, write_json
from future_runner import Future

FRAGMENT = "思考🙂" * 2048
CHUNKS = 64
ANSWER = "FINAL_ANSWER: 7"


def free_endpoint():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return f"127.0.0.1:{sock.getsockname()[1]}"


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary", default=str(ROOT.parent.parent / "target/debug/future")
    )
    parser.add_argument(
        "--expect-entry-policy", choices=("present", "absent"), default="present"
    )
    parser.add_argument(
        "--check-idle",
        action="store_true",
        help="exercise a two-second fixture-only inactivity threshold",
    )
    args = parser.parse_args()
    home = ROOT / ".local" / ("verification-" + uuid.uuid4().hex[:12])
    (home / "agent").mkdir(parents=True, mode=0o700)
    home.chmod(0o700)
    counts = Counter()
    lock = threading.Lock()
    arrivals = []
    entry_policies = []
    entry_tool_choices = []
    stalled = set()
    barrier = threading.Barrier(3, timeout=15)

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            model = request["model"]
            if model == "parent":
                system = "\n".join(
                    strings(
                        [
                            m
                            for m in request["messages"]
                            if m.get("role") in ("system", "developer")
                        ]
                    )
                )
                entry_policies.append("Fusion is enabled for this run only." in system)
                entry_tool_choices.append(request.get("tool_choice"))
            with lock:
                counts[model] += 1
                attempt = counts[model]
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Connection", "close")
            self.end_headers()
            self.close_connection = True
            time.sleep(0.3)

            def emit(delta, finish=None):
                chunk = {
                    "id": "fixture",
                    "object": "chat.completion.chunk",
                    "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
                }
                self.wfile.write(
                    ("data: " + json.dumps(chunk, ensure_ascii=False) + "\n\n").encode()
                )
                self.wfile.flush()

            if model in ("long-open", "long-closed") and attempt == 1:
                for _ in range(CHUNKS):
                    emit({"reasoning_content": FRAGMENT})
                if model == "long-closed":
                    emit({"content": "Partial response."})
                # Deliberate EOF without a terminal finish or [DONE].
                return
            prompt = "\n".join(strings(request.get("messages", [])))
            answer = ANSWER
            if model == "parent" and not any(
                m.get("role") == "tool" for m in request["messages"]
            ):
                payload = {
                    "question": "Return 7",
                    "goal": "Return the integer 7",
                    "background": "Local concurrency verification",
                    "read_scope": [],
                }
                emit(
                    {
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "fixture-fusion",
                                "type": "function",
                                "function": {
                                    "name": "fusion",
                                    "arguments": json.dumps(payload),
                                },
                            }
                        ]
                    },
                    "tool_calls",
                )
            else:
                if model in ("a", "b", "c"):
                    if "\nReview packet:\n" in prompt:
                        packet, _ = json.JSONDecoder().raw_decode(
                            prompt.rsplit("\nReview packet:\n", 1)[1]
                        )
                        answer = json.dumps(
                            {
                                "participant": packet["participant"],
                                "current_answer": ANSWER,
                                "responses": [],
                                "self_review": [
                                    {
                                        "original_quote": ANSWER,
                                        "decision": "maintain",
                                        "updated_position": ANSWER,
                                        "reason": "Unchanged fixture answer",
                                    }
                                ],
                                "mapping_corrections": [],
                                "additions": [],
                            }
                        )
                    else:
                        with lock:
                            arrivals.append(
                                {"model": model, "monotonic": time.monotonic()}
                            )
                        barrier.wait()
                    if model in stalled and "\nReview packet:\n" not in prompt:
                        try:
                            for _ in range(40):
                                self.wfile.write(b": heartbeat\n\n")
                                self.wfile.flush()
                                time.sleep(0.1)
                        except (BrokenPipeError, ConnectionResetError):
                            pass
                        return
                    if args.check_idle:
                        for _ in range(10):
                            emit(
                                {
                                    "reasoning_content": "Continuing the fixture reasoning. "
                                }
                            )
                            time.sleep(0.25)
                elif model == "moderator":
                    if "readiness check" in prompt:
                        answer = "READY"
                    elif "\nIndependent analyses:\n" in prompt:
                        answer = json.dumps(
                            {"consensus": [], "issues": [], "excluded": []}
                        )
                emit({"content": answer})
                # Keep the SSE open while the CLI attaches its read-only wait.
                time.sleep(0.3)
                emit({}, "stop")
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    names = ["long-open", "long-closed", "parent", "moderator", "a", "b", "c"]
    write_json(
        home / "agent/models.json",
        {
            "providers": {
                "fixture": {
                    "api": "openai",
                    "apiKey": "local-fixture",
                    "baseUrl": f"http://127.0.0.1:{server.server_port}/v1",
                    "models": [
                        {"id": name, "contextWindow": 1000000, "maxTokens": 8192}
                        for name in names
                    ],
                }
            }
        },
    )
    (home / "fusion").mkdir()
    settings = {
        "enabled": True,
        "revision": 1,
        "configuration": {
            "version": 2,
            "panel": [
                {"model": "fixture/" + name, "thinking_level": "default"}
                for name in ("a", "b", "c")
            ],
            "moderator": {"model": "fixture/moderator", "thinking_level": "default"},
            "step_timeout_secs": 600,
            "run_timeout_secs": 2700,
        },
    }
    if args.check_idle:
        settings["configuration"].update(
            version=3, step_timeout_secs=0, run_timeout_secs=0, idle_timeout_secs=2
        )
    with sqlite3.connect(home / "fusion/fusion.db") as connection:
        connection.execute(
            "CREATE TABLE fusion_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL)"
        )
        connection.execute(
            "INSERT INTO fusion_meta VALUES('settings',?)", (json.dumps(settings),)
        )
    endpoint = free_endpoint()
    runtime = home / "runtime.json"
    write_json(runtime, {"home": str(home), "endpoint": endpoint})
    client = Future(args.binary, runtime)
    client.env.pop("FUTURE_AGENT_SOCKET", None)
    env = dict(client.env)
    process = None
    evidence = {
        "binary_sha256": digest(Path(client.binary).read_bytes()),
        "verification_source_sha256": digest(Path(__file__).read_bytes()),
    }
    for name in ("verify_runtime.py", "future_runner.py", "datasets.py", "scoring.py"):
        (home / ("source-" + name)).write_bytes((ROOT / name).read_bytes())
    try:
        with (home / "host.log").open("w") as log:
            process = subprocess.Popen(
                [client.binary, "host", "--agent-grpc-addr", endpoint],
                env=env,
                stdout=log,
                stderr=log,
            )
            deadline = time.monotonic() + 60
            while True:
                try:
                    client.call(["fusion", "settings", "--json"], timeout=3)
                    break
                except (RuntimeError, subprocess.TimeoutExpired):
                    if process.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError(
                            "Fixture Host did not become ready; inspect "
                            + str(home / "host.log")
                        )
                    time.sleep(0.2)
            if args.check_idle:
                configured = client.call(["fusion", "settings", "--json"])
                assert configured["configuration"]["step_timeout_secs"] == 0
                assert configured["configuration"]["run_timeout_secs"] == 0
                assert configured["configuration"]["idle_timeout_secs"] == 2

            def run(model, fusion=False, session=None):
                ack = client.call(
                    [
                        "run",
                        "--model",
                        "fixture/" + model,
                        "--no-tools",
                        "--no-title",
                        "--permission",
                        "all",
                        "--mode",
                        "json",
                        "--no-wait",
                        "--cwd",
                        str(home),
                    ]
                    + (["--session", session] if session else [])
                    + [
                        ("/fusion " if fusion else "") + "Return 7",
                    ]
                )
                output = client.wait(ack["sessionId"], ack["runId"], 90)
                assert ANSWER in output["text"], output
                state = client.call(["session", "status", ack["sessionId"], "--json"])
                assert not state.get("eventJournalError"), state.get(
                    "eventJournalError"
                )
                return ack

            expected = FRAGMENT * CHUNKS
            for model in ("long-open", "long-closed"):
                ack = run(model)
                assert counts[model] == 2, counts
                transcript = client.call(
                    [
                        "session",
                        "transcript",
                        "--session",
                        ack["sessionId"],
                        "--select",
                        "thinking",
                        "--all",
                        "--max-bytes",
                        "0",
                        "--json",
                    ]
                )
                assert expected in list(strings(transcript)), (
                    "Reasoning prefix differs from fixture"
                )
                run(model, session=ack["sessionId"])
                evidence[model] = {
                    "requests": counts[model],
                    "reasoning_bytes": len(expected.encode()),
                    "reasoning_sha256": hashlib.sha256(expected.encode()).hexdigest(),
                    "subsequent_run_completed": True,
                }
            ack = run("parent", fusion=True)
            discussion = client.call(["fusion", "show", ack["sessionId"], "--json"])
            write_json(home / "discussion.json", discussion)
            report = discussion["run"]["report"]
            assert discussion["run"]["status"] == "completed", discussion["run"][
                "status"
            ]
            assert sorted(report["analysis_participants"]) == ["P1", "P2", "P3"], report
            assert sorted(report["review_participants"]) == ["P1", "P2", "P3"], report
            assert len(arrivals) == 3 and {a["model"] for a in arrivals} == {
                "a",
                "b",
                "c",
            }
            evidence["fusion"] = {
                "concurrent_analysis_requests": 3,
                "arrival_spread_seconds": max(a["monotonic"] for a in arrivals)
                - min(a["monotonic"] for a in arrivals),
                "analysis_participants": report["analysis_participants"],
                "review_participants": report["review_participants"],
                "status": discussion["run"]["status"],
                "entry_policy_present": entry_policies,
            }
            # Run the shipping benchmark caller too, binding its routing role
            # to this local fixture rather than any real paid provider.
            with (
                patch("future_runner.BASE_MODEL", "fixture/parent"),
                patch("future_runner.BASE_THINKING", "default"),
            ):
                harness_result = client.run_case(
                    {
                        "id": "fixture:7",
                        "dataset": "fixture",
                        "scorer": "integer",
                        "question": "Return 7",
                        "answer": "7",
                    },
                    "fusion",
                    home / "harness-case",
                    90,
                )
            assert harness_result["status"] == "completed", harness_result.get("error")
            assert harness_result["grade"]["correct"]
            assert harness_result["fusion_invocations"] == 1
            assert harness_result["completed_consultations"] == 1
            evidence["daily_harness"] = {
                key: harness_result[key]
                for key in (
                    "primary_endpoint",
                    "fusion_invocations",
                    "completed_consultations",
                    "routing_outcome",
                )
            }
            if args.check_idle:
                c_requests = counts["c"]
                stalled.add("c")
                stopped = run("parent", fusion=True)
                stopped_discussion = client.call(
                    ["fusion", "show", stopped["sessionId"], "--json"]
                )
                failures = [
                    step
                    for step in stopped_discussion["steps"]
                    if step.get("failure_kind") == "idle_timeout"
                ]
                assert len(failures) == 1 and failures[0]["participant"] == "P3", (
                    failures
                )
                assert counts["c"] == c_requests + 1, "Idle timeouts must not retry"
                assert sorted(
                    stopped_discussion["run"]["report"]["analysis_participants"]
                ) == ["P1", "P2"]
                write_json(home / "heartbeat-idle-discussion.json", stopped_discussion)
                evidence["idle_policy"] = {
                    "fixture_threshold_seconds": 2,
                    "continuous_reasoning_seconds_per_request": 2.5,
                    "continuous_output_completed": True,
                    "heartbeat_only_failure_kind": failures[0]["failure_kind"],
                    "heartbeat_only_retries": 0,
                }
            evidence["fusion"]["automatic_tool_choice"] = all(
                choice in (None, "auto") for choice in entry_tool_choices
            )
            write_json(home / "verification.json", evidence)
            assert entry_policies and all(
                present == (args.expect_entry_policy == "present")
                for present in entry_policies
            ), entry_policies
            assert evidence["fusion"]["automatic_tool_choice"], entry_tool_choices
            print(
                json.dumps(
                    {"evidence": str(home / "verification.json"), "results": evidence},
                    ensure_ascii=False,
                ),
                flush=True,
            )
    finally:
        if process is not None and process.poll() is None:
            process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=20)
        server.shutdown()
        for filename in ("auth.json", "models.json", ".future-models-cache.json"):
            (home / "agent" / filename).unlink(missing_ok=True)


if __name__ == "__main__":
    main()
