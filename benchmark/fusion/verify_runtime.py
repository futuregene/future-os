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

from datasets import ROOT, digest, write_json
from future_runner import Future

FRAGMENT = "思考🙂" * 2048
CHUNKS = 64
ANSWER = "FINAL_ANSWER: 7"
PANEL_TIMEOUT_SECS = 5
PANEL_TIMEOUT_SUPPORT = {
    model: f"Fixture participant {model} derives 7 from the supplied request."
    for model in ("a", "b")
}


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
    parser.add_argument(
        "--check-panel-timeout",
        action="store_true",
        help="run only the panel timeout check with a continuously reasoning participant",
    )
    parser.add_argument(
        "--check-evidence",
        action="store_true",
        help="exercise source receipts and repair fabricated verified_fact references",
    )
    parser.add_argument(
        "--check-decisions",
        action="store_true",
        help="exercise explicit critical objection disposition and omitted-item repair",
    )
    parser.add_argument(
        "--check-local-repair",
        action="store_true",
        help="repair multiple incorrect disposition quotes together",
    )
    parser.add_argument(
        "--check-recovery-progress",
        action="store_true",
        help="real-clock recovery reasoning for over 300 seconds; no paid calls",
    )
    parser.add_argument(
        "--expect-recovery-progress",
        choices=("preserved", "absolute-timeout"),
        default="preserved",
    )
    parser.add_argument(
        "--artifact-root", help="Directory for retained local verification records"
    )
    args = parser.parse_args()
    if args.check_recovery_progress:
        args.check_idle = True
    home = Path(args.artifact_root or ROOT / ".local") / (
        "verification-" + uuid.uuid4().hex[:12]
    )
    (home / "agent").mkdir(parents=True, mode=0o700)
    home.chmod(0o700)
    counts = Counter()
    lock = threading.Lock()
    arrivals = []
    entry_policies = []
    entry_tool_choices = []
    delivery_requests = []
    configured_requests = []
    repeat_after_failure = set()
    stalled = set()
    exhausted = set()
    report_recovery_requests = []
    panel_timeout_probe = {"enabled": False, "reasoning_chunks": 0}
    source_probe = {"enabled": False, "reviews": []}
    decision_probe = {"enabled": False, "attempts": []}
    lineage_probe = {"previous_run_id": None}
    local_repair = {"enabled": False, "feedback": []}
    recovery_progress = {"enabled": False, "chunks": 0}
    barrier = threading.Barrier(3, timeout=15)

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            model = request["model"]
            if model in ("parent", "moderator", "always-moderator", "a", "b", "c"):
                configured_requests.append(request)
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
                if any(m.get("role") == "tool" for m in request["messages"]):
                    delivery_requests.append(request)
                else:
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
            if model in exhausted and "Closed answer protocol:" in prompt:
                if "The working response exhausted its output budget" not in prompt:
                    emit(
                        {
                            "reasoning_content": "The supported result is already in the supplied reports."
                        },
                        "length",
                    )
                    self.wfile.write(b"data: [DONE]\n\n")
                    self.wfile.flush()
                    return
                report_recovery_requests.append(request)
            needs_consultation = not any(
                m.get("role") == "tool" for m in request["messages"]
            )
            if model == "parent" and lineage_probe["previous_run_id"]:
                last_user = max(
                    i
                    for i, m in enumerate(request["messages"])
                    if m.get("role") == "user"
                )
                needs_consultation = not any(
                    m.get("role") == "tool"
                    for m in request["messages"][last_user + 1 :]
                )
            if model == "parent" and (
                model in repeat_after_failure or needs_consultation
            ):
                payload = {
                    "question": "Return 7",
                    "goal": "Return the integer 7",
                    "background": "Local concurrency verification",
                    "read_scope": [],
                }
                if lineage_probe["previous_run_id"]:
                    payload["revisit"] = {
                        "previous_run_id": lineage_probe["previous_run_id"],
                        "kind": "corrected_inference",
                        "reason": "Recheck whether the prior premise is a literal request rather than an inference from agreement",
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
                        additions = []
                        if source_probe["enabled"]:
                            manifest = packet["source_manifest"]
                            assert manifest["supplied_evidence"] == "not_supplied", (
                                manifest
                            )
                            assert manifest["file_scope"] == "not_requested", manifest
                            repaired = "Validation feedback:" in prompt
                            reference = (
                                [
                                    {
                                        "source_id": "request",
                                        "sha256": manifest["inline_sources"][0][
                                            "sha256"
                                        ],
                                        "quote": "Return 7",
                                    }
                                ]
                                if repaired
                                else ["fabricated-source-id"]
                            )
                            additions = [
                                {
                                    "kind": "verified_fact",
                                    "content": "The request asks to return 7",
                                    "evidence": reference,
                                }
                            ]
                            with lock:
                                source_probe["reviews"].append(
                                    {
                                        "participant": packet["participant"],
                                        "repaired": repaired,
                                        "source_manifest": manifest,
                                    }
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
                                "additions": additions,
                                "objections": (
                                    [
                                        {
                                            "participant": packet["participant"],
                                            "quote": ANSWER,
                                            "reason": "Check whether 7 is actually requested rather than merely compatible",
                                            "effect": "An unsupported value must not be delivered",
                                        }
                                    ]
                                    if decision_probe["enabled"] and model == "c"
                                    else []
                                ),
                            }
                        )
                    else:
                        with lock:
                            arrivals.append(
                                {"model": model, "monotonic": time.monotonic()}
                            )
                        barrier.wait()
                        if panel_timeout_probe["enabled"] and model == "c":
                            try:
                                for _ in range(8 * PANEL_TIMEOUT_SECS):
                                    emit(
                                        {
                                            "reasoning_content": "Still working on the fixture."
                                        }
                                    )
                                    panel_timeout_probe["reasoning_chunks"] += 1
                                    time.sleep(0.25)
                            except (BrokenPipeError, ConnectionResetError):
                                return
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
                if (
                    "Closed answer protocol:" in prompt
                    and "\nReview packet:\n" not in prompt
                ):
                    answer = json.dumps(
                        {
                            "answer": ANSWER,
                            "support": (
                                PANEL_TIMEOUT_SUPPORT[model]
                                if panel_timeout_probe["enabled"]
                                and model in PANEL_TIMEOUT_SUPPORT
                                else "The supplied request asks to return 7."
                            ),
                            "unresolved": (
                                ["Check the supplied source before finalizing"]
                                if (
                                    source_probe["enabled"] or decision_probe["enabled"]
                                )
                                and model == "c"
                                else []
                            ),
                            "checks": (
                                [
                                    {
                                        "claim": "The requested value is 7",
                                        "quote": "Return 7",
                                        "assessment": "supports",
                                        "effect": "Deliver the requested value",
                                    }
                                ]
                                if model.endswith("moderator")
                                else []
                            ),
                        }
                    )
                if model == "moderator" and "\nDecision audit packet:\n" in prompt:
                    packet, _ = json.JSONDecoder().raw_decode(
                        prompt.rsplit("\nDecision audit packet:\n", 1)[1]
                    )
                    repaired = "Validation feedback:" in prompt
                    value = json.loads(answer)
                    value["decision_audit"] = {
                        "resolution": "settled",
                        "decision_update": (
                            {
                                "decision": "maintain",
                                "reason": "The literal request still supplies 7; the challenged inference does not change the answer",
                            }
                            if repaired and packet["lineage"]
                            else None
                        ),
                        "dispositions": (
                            [
                                {
                                    "id": key,
                                    "decision": "rejected",
                                    "reason": "The literal value is supplied by the request",
                                    "effect": "Deliver 7 without inventing an unstated premise",
                                    "basis": [
                                        {"source_id": "request", "quote": "Return 7"}
                                    ],
                                }
                                for key in packet["objections"]
                            ]
                            if repaired
                            else []
                        ),
                    }
                    if panel_timeout_probe["enabled"]:
                        value["decision_audit"]["dispositions"] = [
                            {
                                "id": key,
                                "decision": "accepted",
                                "reason": "The review confirms the requested value.",
                                "effect": "Deliver 7.",
                                "basis": [
                                    {
                                        "source_id": objection["report_source"],
                                        "source_path": "/current_answer",
                                    }
                                ],
                            }
                            for key, objection in packet["objections"].items()
                        ]
                    if local_repair["enabled"]:
                        if repaired:
                            assert (
                                "basis[0].quote" in prompt
                                and "basis[1].quote" in prompt
                            ), prompt[-3000:]
                            assert (
                                "Original source excerpts for local quote repair"
                                in prompt
                            )
                            assert "Return 7" in prompt
                            local_repair["feedback"].append(prompt)
                            for disposition in value["decision_audit"]["dispositions"]:
                                disposition["basis"] = [
                                    {"source_id": "request", "quote": "Return 7"},
                                    {"source_id": "request", "quote": "7"},
                                ]
                        else:
                            value["decision_audit"]["dispositions"] = [
                                {
                                    "id": key,
                                    "decision": "rejected",
                                    "reason": "The request states the value",
                                    "effect": "Return 7",
                                    "basis": [
                                        {
                                            "source_id": "request",
                                            "quote": "Return the 7",
                                        },
                                        {
                                            "source_id": "request",
                                            "quote": "Return number 7",
                                        },
                                    ],
                                }
                                for key in packet["objections"]
                            ]
                    answer = json.dumps(value)
                    decision_probe["attempts"].append(
                        {"repaired": repaired, "objections": list(packet["objections"])}
                    )
                if (
                    recovery_progress["enabled"]
                    and model == "parent"
                    and any(m.get("role") == "tool" for m in request["messages"])
                ):
                    try:
                        for _ in range(62):
                            emit(
                                {
                                    "reasoning_content": "Checking the existing failed-discussion material. "
                                }
                            )
                            recovery_progress["chunks"] += 1
                            time.sleep(5)
                    except (BrokenPipeError, ConnectionResetError):
                        return
                emit({"content": answer})
                # Keep the SSE open while the CLI attaches its read-only wait.
                time.sleep(0.3)
                emit({}, "stop")
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    names = [
        "long-open",
        "long-closed",
        "parent",
        "bypass-parent",
        "moderator",
        "always-moderator",
        "a",
        "b",
        "c",
    ]
    write_json(
        home / "agent/models.json",
        {
            "providers": {
                "fixture": {
                    "api": "openai",
                    "apiKey": "local-fixture",
                    "baseUrl": f"http://127.0.0.1:{server.server_port}/v1",
                    "models": [
                        {
                            "id": name,
                            "reasoning": True,
                            "thinkingLevels": ["off", "low", "high", "default"]
                            if name != "always-moderator"
                            else ["low", "high", "default"],
                            "compat": {
                                "thinkingFormat": "deepseek",
                                "alwaysThinking": name == "always-moderator",
                            },
                            "contextWindow": 1000000,
                            "maxTokens": 65536,
                        }
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
                {"model": "fixture/" + name, "thinking_level": "high"}
                for name in ("a", "b", "c")
            ],
            "moderator": {"model": "fixture/moderator", "thinking_level": "high"},
            "step_timeout_secs": 600,
            "run_timeout_secs": 2700,
        },
    }
    if args.check_idle or args.check_panel_timeout:
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
            if args.check_idle or args.check_panel_timeout:
                configured = client.call(["fusion", "settings", "--json"])
                assert configured["configuration"]["step_timeout_secs"] == 0
                assert configured["configuration"]["run_timeout_secs"] == 0
                assert configured["configuration"]["idle_timeout_secs"] == 2

            def run(model, fusion=False, session=None, timeout=90):
                ack = client.call(
                    [
                        "run",
                        "--model",
                        "fixture/" + model,
                        "--thinking",
                        "high",
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
                output = client.wait(ack["sessionId"], ack["runId"], timeout)
                assert ANSWER in output["text"], output
                state = client.call(["session", "status", ack["sessionId"], "--json"])
                assert not state.get("eventJournalError"), state.get(
                    "eventJournalError"
                )
                return ack

            if args.check_panel_timeout:
                configured = client.call(
                    [
                        "fusion",
                        "settings",
                        "--panel-timeout",
                        str(PANEL_TIMEOUT_SECS),
                        "--json",
                    ]
                )
                assert (
                    configured["configuration"]["panel_timeout_secs"]
                    == PANEL_TIMEOUT_SECS
                ), configured
                c_requests = counts["c"]
                request_count = len(configured_requests)
                panel_timeout_probe["enabled"] = True
                try:
                    advanced = run("parent", fusion=True)
                finally:
                    panel_timeout_probe["enabled"] = False
                client.call(["fusion", "settings", "--panel-timeout", "0", "--json"])
                discussion = client.call(
                    ["fusion", "show", advanced["sessionId"], "--json"]
                )
                write_json(home / "panel-timeout-discussion.json", discussion)
                panel_run = discussion["run"]
                report = panel_run["report"]
                assert panel_run["status"] == "completed", panel_run
                assert panel_run["result"]["final_answer"] == ANSWER, panel_run
                assert sorted(report["analysis_participants"]) == ["P1", "P2"], report
                assert "synthesizing" in report["completed_phases"], report
                failed = [
                    step for step in discussion["steps"] if step["status"] == "failed"
                ]
                assert [
                    (step["participant"], step["phase"], step.get("failure_kind"))
                    for step in failed
                ] == [("P3", "analyzing", "panel_timeout")], failed
                assert counts["c"] == c_requests + 1, counts
                assert panel_timeout_probe["reasoning_chunks"] > 1, panel_timeout_probe
                moderator_prompts = [
                    "\n".join(strings(request["messages"]))
                    for request in configured_requests[request_count:]
                    if request["model"] == "moderator"
                ]
                assert any(
                    "\nIndependent analyses:\n" in prompt
                    and all(
                        support in prompt for support in PANEL_TIMEOUT_SUPPORT.values()
                    )
                    for prompt in moderator_prompts
                ), moderator_prompts
                evidence["panel_timeout"] = {
                    "phase_timeout_seconds": PANEL_TIMEOUT_SECS,
                    "reasoning_chunks": panel_timeout_probe["reasoning_chunks"],
                    "timed_out_participant": "P3",
                    "analysis_participants": report["analysis_participants"],
                    "final_answer": panel_run["result"]["final_answer"],
                }
                write_json(home / "verification.json", evidence)
                print(
                    json.dumps(
                        {
                            "evidence": str(home / "verification.json"),
                            "results": evidence,
                        }
                    ),
                    flush=True,
                )
                return

            bypass = client.call(
                [
                    "run",
                    "--model",
                    "fixture/bypass-parent",
                    "--no-tools",
                    "--no-title",
                    "--permission",
                    "all",
                    "--mode",
                    "json",
                    "--no-wait",
                    "--cwd",
                    str(home),
                    "/fusion Return 7",
                ]
            )
            try:
                client.wait(bypass["sessionId"], bypass["runId"], 30)
                raise AssertionError("Unconsulted answer was accepted")
            except RuntimeError:
                ledger = client.run_ledger(
                    bypass["sessionId"], bypass["runId"], wait_terminal=30
                )
                write_json(home / "bypass-ledger.json", ledger)
                rejected = next(
                    row for row in ledger["ledger"] if row["runId"] == bypass["runId"]
                )
                assert (
                    rejected["status"] == "failed"
                    and "FUSION_ROUTING_FAILED" in rejected["error"]
                ), rejected
            evidence["unconsulted_answer_rejected"] = True
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
            assert discussion["run"]["input"]["parent_request"] == "Return 7", (
                discussion["run"]["input"]
            )
            report = discussion["run"]["report"]
            assert discussion["run"]["status"] == "completed", discussion["run"][
                "status"
            ]
            assert sorted(report["analysis_participants"]) == ["P1", "P2", "P3"], report
            assert report["review_participants"] == [], report
            assert report["strategy"] == "direct_consensus", report
            assert discussion["run"]["input"]["answer_only"] is True
            assert discussion["run"]["result"]["final_answer"] == ANSWER
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
            harness_client = Future(
                args.binary, runtime, model="fixture/parent", thinking="high"
            )
            harness_result = harness_client.run_case(
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
            exhausted.add("moderator")
            recovered = run("parent", fusion=True)
            recovered_discussion = client.call(
                ["fusion", "show", recovered["sessionId"], "--json"]
            )
            exhausted.clear()
            write_json(
                home / "exhausted-moderator-discussion.json", recovered_discussion
            )
            assert recovered_discussion["run"]["status"] == "completed", (
                recovered_discussion
            )
            recovered_step = next(
                step
                for step in recovered_discussion["steps"]
                if step["phase"] == "synthesizing"
            )
            assert recovered_step["settings"]["report_revision"] == 1, recovered_step
            assert recovered_step["settings"]["thinking_level"] == "high"
            assert "report_thinking_level" not in recovered_step["settings"]
            assert len(report_recovery_requests) == 1
            assert report_recovery_requests[0]["reasoning_effort"] == "high"
            assert recovered_discussion["run"]["result"]["final_answer"] == ANSWER
            evidence["moderator_output_limit"] = {
                "recovered": True,
                "report_only_requests": 1,
                "original_thinking_level": "high",
                "report_thinking_level": "high",
                "user_settings_preserved": True,
            }
            client.call(
                [
                    "fusion",
                    "settings",
                    "--main",
                    "fixture/always-moderator",
                    "--thinking",
                    "high",
                    "--json",
                ]
            )
            exhausted.add("always-moderator")
            low_report = run("parent", fusion=True)
            low_discussion = client.call(
                ["fusion", "show", low_report["sessionId"], "--json"]
            )
            exhausted.clear()
            write_json(
                home / "always-thinking-moderator-discussion.json", low_discussion
            )
            assert low_discussion["run"]["status"] == "completed", low_discussion
            low_step = next(
                step
                for step in low_discussion["steps"]
                if step["phase"] == "synthesizing"
            )
            assert low_step["settings"]["thinking_level"] == "high"
            assert "report_thinking_level" not in low_step["settings"]
            assert low_step["settings"]["report_revision"] == 1
            assert len(report_recovery_requests) == 2
            assert report_recovery_requests[-1]["reasoning_effort"] == "high"
            evidence["always_thinking_report"] = {
                "original_level": "high",
                "report_level": "high",
                "report_only_requests": 1,
                "completed": True,
            }
            client.call(
                [
                    "fusion",
                    "settings",
                    "--main",
                    "fixture/moderator",
                    "--thinking",
                    "high",
                    "--json",
                ]
            )
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
            evidence["fusion"]["required_fusion_tool_choice"] = all(
                choice == {"type": "function", "function": {"name": "fusion"}}
                for choice in entry_tool_choices
            )
            write_json(home / "verification.json", evidence)
            assert entry_policies and all(
                present == (args.expect_entry_policy == "present")
                for present in entry_policies
            ), entry_policies
            assert (
                entry_tool_choices and evidence["fusion"]["required_fusion_tool_choice"]
            ), entry_tool_choices
            assert not delivery_requests, (
                "A sealed completed answer must not invoke the parent model again"
            )
            evidence["fusion"]["additional_parent_delivery_requests"] = 0
            evidence["fusion"]["typed_answer_delivered"] = True
            assert all(
                request.get("reasoning_effort") == "high"
                for request in configured_requests
            ), [(r["model"], r.get("reasoning_effort")) for r in configured_requests]
            assert all(
                request.get("max_tokens") == 65536 for request in configured_requests
            ), [(r["model"], r.get("max_tokens")) for r in configured_requests]
            evidence["user_settings"] = {
                "thinking_level": "high",
                "max_tokens": 65536,
                "all_requests_preserved": True,
            }
            if args.check_idle:
                stalled.update(["a", "b", "c"])
                recovery_before = len(delivery_requests)
                failed = harness_client.run_case(
                    {
                        "id": "fixture:failed",
                        "dataset": "fixture",
                        "scorer": "integer",
                        "question": "Return 7",
                        "answer": "7",
                    },
                    "fusion",
                    home / "failed-discussion-case",
                    90,
                )
                assert (
                    failed["fusion_invocations"] == 1
                    and failed["completed_consultations"] == 0
                ), failed
                assert len(delivery_requests) == recovery_before + 1
                assert not delivery_requests[-1].get("tools"), delivery_requests[
                    -1
                ].get("tools")
                repeat_after_failure.add("parent")
                blocked = harness_client.run_case(
                    {
                        "id": "fixture:blocked",
                        "dataset": "fixture",
                        "scorer": "integer",
                        "question": "Return 7",
                        "answer": "7",
                    },
                    "fusion",
                    home / "blocked-repeat-case",
                    90,
                )
                repeat_after_failure.clear()
                assert blocked["status"] == "error" and "FUSION_REPEAT_BLOCKED" in str(
                    blocked.get("upstream_error") or blocked.get("error")
                ), blocked
                with sqlite3.connect(home / "fusion/fusion.db") as connection:
                    actual_runs = connection.execute(
                        "SELECT COUNT(*) FROM fusion_runs WHERE parent_session=?",
                        (blocked["session_id"],),
                    ).fetchone()[0]
                assert actual_runs == 1, actual_runs
                assert all(
                    request.get("reasoning_effort") == "high"
                    and request.get("max_tokens") == 65536
                    for request in configured_requests
                )
                evidence["failed_discussion_recovery"] = {
                    "fusion_invocations": 1,
                    "tools_removed": True,
                    "unexpected_repeat_blocked": True,
                }
            if args.check_evidence:
                stalled.clear()
                source_probe["enabled"] = True
                sourced = run("parent", fusion=True)
                source_probe["enabled"] = False
                source_discussion = client.call(
                    ["fusion", "show", sourced["sessionId"], "--json"]
                )
                write_json(
                    home / "source-validation-discussion.json", source_discussion
                )
                reviews = [
                    step
                    for step in source_discussion["steps"]
                    if step["phase"] == "reviewing"
                ]
                assert len(reviews) == 3, reviews
                assert all(
                    step["settings"]["report_revision"] == 1 for step in reviews
                ), reviews
                assert all(not step.get("report_quality") for step in reviews), reviews
                assert len(source_probe["reviews"]) == 6, source_probe
                assert sum(item["repaired"] for item in source_probe["reviews"]) == 3
                for step in reviews:
                    report = json.loads(step["output"])
                    reference = report["additions"][0]["evidence"][0]
                    assert reference["source_id"] == "request"
                    assert (
                        reference["sha256"] == hashlib.sha256(b"Return 7").hexdigest()
                    )
                assert source_discussion["run"]["result"]["final_answer"] == ANSWER
                assert all(
                    request.get("reasoning_effort") == "high"
                    and request.get("max_tokens") == 65536
                    for request in configured_requests
                )
                evidence["source_validation"] = {
                    "fabricated_references_rejected": 3,
                    "same_session_repairs": 3,
                    "repaired_references_validated": True,
                    "missing_evidence_distinct_from_empty_query": True,
                    "self_contained_task_completed_without_external_evidence": True,
                    "user_settings_preserved": True,
                }
            if args.check_decisions:
                decision_probe["attempts"].clear()
                exhausted.clear()
                stalled.clear()
                decision_probe["enabled"] = True
                decided = run("parent", fusion=True)
                decision_probe["enabled"] = False
                discussion = client.call(
                    ["fusion", "show", decided["sessionId"], "--json"]
                )
                write_json(home / "critical-objections-discussion.json", discussion)
                result = discussion["run"]["result"]
                assert result["final_answer"] == ANSWER, result
                audit = result["decision_audit"]
                assert audit["resolution"] == "settled", audit
                assert len(audit["dispositions"]) == 1, audit
                assert audit["dispositions"][0]["id"] == "OP3-1", audit
                synth = next(
                    step
                    for step in discussion["steps"]
                    if step["phase"] == "synthesizing"
                )
                assert synth["settings"]["report_revision"] == 1, synth
                assert not synth.get("report_quality"), synth
                assert len(decision_probe["attempts"]) == 2, decision_probe
                assert [
                    attempt["repaired"] for attempt in decision_probe["attempts"]
                ] == [False, True]
                assert all(
                    request.get("reasoning_effort") == "high"
                    and request.get("max_tokens") == 65536
                    for request in configured_requests
                )
                evidence["critical_objections"] = {
                    "omitted_disposition_rejected": True,
                    "same_session_repair": True,
                    "disposition_persisted": True,
                    "user_settings_preserved": True,
                }
                lineage_probe["previous_run_id"] = discussion["run"]["id"]
                reopened = run("parent", fusion=True, session=decided["sessionId"])
                lineage_probe["previous_run_id"] = None
                inherited = client.call(
                    ["fusion", "show", reopened["sessionId"], "--json"]
                )
                write_json(home / "inherited-decision-discussion.json", inherited)
                lineage = inherited["run"]["lineage"]
                assert (
                    lineage["revisit"]["previous_run_id"] == discussion["run"]["id"]
                ), lineage
                assert lineage["previous_result"]["final_answer"] == ANSWER, lineage
                assert (
                    inherited["run"]["result"]["decision_audit"]["decision_update"][
                        "decision"
                    ]
                    == "maintain"
                ), inherited
                assert (
                    inherited["run"]["configuration"]
                    == discussion["run"]["configuration"]
                ), inherited
                assert all(
                    request.get("reasoning_effort") == "high"
                    and request.get("max_tokens") == 65536
                    for request in configured_requests
                )
                evidence["decision_inheritance"] = {
                    "previous_result_frozen": True,
                    "maintained_answer": True,
                    "user_settings_preserved": True,
                }
            if args.check_local_repair:
                exhausted.clear()
                stalled.clear()
                decision_probe["enabled"] = True
                local_repair["enabled"] = True
                repaired = run("parent", fusion=True)
                decision_probe["enabled"] = False
                local_repair["enabled"] = False
                discussion = client.call(
                    ["fusion", "show", repaired["sessionId"], "--json"]
                )
                write_json(home / "local-quote-repair-discussion.json", discussion)
                write_json(
                    home / "local-quote-repair-feedback.json", local_repair["feedback"]
                )
                assert discussion["run"]["result"]["final_answer"] == ANSWER
                assert len(local_repair["feedback"]) == 1
                evidence["local_quote_repair"] = {
                    "all_bad_fields_reported": True,
                    "original_excerpts_provided": True,
                    "same_session_repair": True,
                    "answer_preserved": True,
                }
            if args.check_recovery_progress:
                exhausted.clear()
                stalled.update(("a", "b", "c"))
                recovery_progress["enabled"] = True
                started = time.monotonic()
                try:
                    recovered = run("parent", fusion=True, timeout=400)
                except RuntimeError as error:
                    if args.expect_recovery_progress != "absolute-timeout":
                        raise
                    assert "FUSION_STAGE_TIMEOUT" in str(error), str(error)
                    evidence["recovery_progress"] = {
                        "expected_old_failure_reproduced": True,
                        "elapsed_seconds": time.monotonic() - started,
                        "reasoning_chunks": recovery_progress["chunks"],
                    }
                else:
                    assert args.expect_recovery_progress == "preserved", (
                        "Old absolute timeout unexpectedly passed"
                    )
                    assert time.monotonic() - started > 300
                    transcript = client.transcript(recovered["sessionId"])
                    write_json(home / "recovery-progress-transcript.json", transcript)
                    evidence["recovery_progress"] = {
                        "continuous_reasoning_over_300_seconds_completed": True,
                        "elapsed_seconds": time.monotonic() - started,
                        "reasoning_chunks": recovery_progress["chunks"],
                    }
                recovery_progress["enabled"] = False
                assert all(
                    request.get("reasoning_effort") == "high"
                    and request.get("max_tokens") == 65536
                    for request in configured_requests
                )
                evidence["recovery_progress"]["user_settings_preserved"] = True
            write_json(home / "verification.json", evidence)
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
        write_json(home / "fixture-requests.json", configured_requests)
        for filename in ("auth.json", "models.json", ".future-models-cache.json"):
            (home / "agent" / filename).unlink(missing_ok=True)


if __name__ == "__main__":
    main()
