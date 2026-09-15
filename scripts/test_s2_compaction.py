#!/usr/bin/env python3
"""Local integration smoke: >256K synthetic history -> S2 -> CLI recall -> restart.
Requires a built future binary. No external model, credentials or user DB access.
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import threading
import time

SUMMARY = """## Objective
- Continue the synthetic audit.

## Important Details
- User directives and verified answers are preserved separately.
- Retrieve exact old tool evidence from session history when needed.

## Work State
### Completed
- Earlier file inspections.
### Active
- Verify historical evidence.
### Blocked
- None.

## Next Move
1. Retrieve the requested historical value.

## Relevant Files
- fixture.txt
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    assert binary.name in ("future", "future.exe"), "expected the unified future binary"
    checks, model_requests, errors = [], [], []
    state = {"normal": 0, "summary": 0}
    secret = "MAGIC_HISTORY_TOKEN=delta_731"
    sid = "s2-smoke"
    originals = ["FIRST_USER: preserve the safety constraint", "FIRST_ANSWER: verified output 42",
                 "SECOND_USER: do not replay writes", "SECOND_ANSWER: verify against stored evidence"]

    def check(name, condition):
        if not condition:
            raise AssertionError(name)
        checks.append(name)

    with tempfile.TemporaryDirectory(prefix="future-s2-smoke-") as directory:
        home = Path(directory)
        config = home / ".future" / "agent"
        sessions = config / "sessions"
        sessions.mkdir(parents=True)
        env = os.environ.copy()
        for key in ("FUTURE_HOME", "FUTURE_AGENT_SOCKET", "FUTURE_AGENT_GRPC_ADDR"):
            env.pop(key, None)
        env.update(HOME=str(home), USERPROFILE=str(home), XDG_RUNTIME_DIR=str(home / "runtime"))
        env["PATH"] = str(binary.parent) + os.pathsep + env.get("PATH", "")
        Path(env["XDG_RUNTIME_DIR"]).mkdir()

        def entry(identity, role, content):
            return {"id": identity, "type": role, "role": role,
                    "timestamp": "2026-09-14T00:00:00Z", "content": content}

        records = [entry("info", "session_info", {"cwd": str(home), "model": "smoke/model", "thinking_level": "off"}),
                   entry("u1", "user", originals[0]), entry("a1", "assistant", originals[1])]
        records[0]["role"] = "system"
        expected_outputs = {}
        for index in range(11):
            call_id = f"read-{index}"
            records.append(entry(f"call-{index}", "assistant", [{"type": "tool_call", "id": call_id,
                           "name": "read", "args": {"path": "fixture.txt", "offset": index}}]))
            text = "historical output\n" + "x" * 48_000 + (secret + "\n" if index == 5 else "") + "x" * 48_000
            expected_outputs[f"result-{index}"] = text
            records.append(entry(f"result-{index}", "tool", [{"type": "tool_result", "tool_call_id": call_id,
                                  "content": text, "is_error": False}]))
        records.extend([entry("u2", "user", originals[2]), entry("a2", "assistant", originals[3])])
        (sessions / f"{sid}.jsonl").write_text("".join(json.dumps(row) + "\n" for row in records), encoding="utf-8")

        def text_of(message):
            content = message.get("content") or ""
            return content if isinstance(content, str) else "\n".join(p.get("text", "") for p in content)

        def tool_reply(body):
            text = text_of(next(m for m in reversed(body["messages"]) if m["role"] == "tool"))
            return json.JSONDecoder().raw_decode(text[text.index("{"):])[0]

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                try:
                    body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                    model_requests.append(body)
                    system = "\n".join(text_of(m) for m in body["messages"] if m["role"] == "system")
                    is_summary = "context summarization agent" in system
                    if is_summary:
                        state["summary"] += 1
                        raise AssertionError("default C must not call a summary model")
                    else:
                        state["normal"] += 1
                        step = state["normal"]
                        check(f"normal output cap unchanged {step}", body.get("max_tokens") == 32_000)
                        check(f"one recall guide {step}", system.count("## Archived conversation recall") == 1)
                        check(f"current session in guide {step}", sid in system)
                        history = "\n".join(text_of(m) for m in body["messages"] if m["role"] != "system")
                        for original in originals:
                            check(f"original preserved {step}: {original[:12]}", original in history)
                        if step == 1:
                            check("old tool value absent before recall", secret not in history)
                            command = f"future session history search --session {sid} --query MAGIC_HISTORY_TOKEN --limit 5 --json"
                        elif step == 2:
                            reply = tool_reply(body)
                            check("search supplied exact entry ID", any(m["entryId"] == "result-5" for m in reply["matches"]))
                            offset = next(m["byteOffset"] for m in reply["matches"] if m["entryId"] == "result-5")
                            command = f"future session history get --session {sid} --entry result-5 --offset {offset} --limit 256 --json"
                        elif step == 3:
                            reply = tool_reply(body)
                            check("get supplied exact historical value", secret in "".join(c["text"] for c in reply["chunks"]))
                            check("get stayed bounded", sum(len(c["text"].encode()) for c in reply["chunks"]) <= 256)
                            command = None
                        elif step == 4:
                            check("no summary model after restart", state["summary"] == 0)
                            command = None
                        else:
                            raise AssertionError("unexpected extra model request")
                        if command:
                            delta = {"tool_calls": [{"index": 0, "id": f"recall-{step}", "type": "function",
                                     "function": {"name": "shell", "arguments": json.dumps({"command": command, "timeout": 20})}}]}
                            finish = "tool_calls"
                        else:
                            delta, finish = {"content": "S2 full-plan verified"}, "stop"
                        cost = 0.002
                    chunks = [{"choices": [{"index": 0, "delta": delta, "finish_reason": None}]},
                              {"choices": [{"index": 0, "delta": {}, "finish_reason": finish}]}]
                    # Usage deliberately follows finish: auxiliary accounting must drain it.
                    if is_summary:
                        chunks.append({"choices": [], "usage": {"prompt_tokens": 900, "completion_tokens": 40,
                                                               "total_tokens": 940, "credit_cost": 0.005}})
                    chunks.append({"choices": [], "usage": {"prompt_tokens": 1000 if is_summary else 100,
                                                              "completion_tokens": 50 if is_summary else 10,
                                                              "total_tokens": 1050 if is_summary else 110,
                                                              "credit_cost": cost}})
                    payload = ("".join("data: " + json.dumps(c) + "\n\n" for c in chunks) + "data: [DONE]\n\n").encode()
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Content-Length", str(len(payload)))
                    self.end_headers()
                    self.wfile.write(payload)
                except Exception as error:
                    errors.append(str(error))
                    self.send_response(500)
                    self.end_headers()
                    self.wfile.write(str(error).encode())

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        (config / "models.json").write_text(json.dumps({"providers": {"smoke": {
            "api": "openai-completions", "apiKey": "local-fixture", "baseUrl": f"http://127.0.0.1:{server.server_port}/v1",
            "models": [{"id": "model", "modalities": ["text"], "reasoning": False, "contextWindow": 1_000_000, "maxTokens": 32_000}]}}}), encoding="utf-8")
        (config / "settings.json").write_text(json.dumps({"defaultModel": "smoke/model"}), encoding="utf-8")
        log = open(home / "agent.log", "w+", encoding="utf-8")
        process = None

        def start_agent():
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                port = sock.getsockname()[1]
            env["FUTURE_AGENT_GRPC_ADDR"] = f"127.0.0.1:{port}"
            child = subprocess.Popen([str(binary), "agent", "--grpc-addr", env["FUTURE_AGENT_GRPC_ADDR"]],
                                     env=env, cwd=home, stdout=log, stderr=subprocess.STDOUT)
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                if child.poll() is not None:
                    raise RuntimeError("isolated agent exited")
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=.1):
                        return child
                except OSError:
                    threading.Event().wait(.03)
            stop_agent(child)
            raise TimeoutError("isolated agent startup")

        def stop_agent(child):
            if child is not None and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()  # only the child created by this test
                    child.wait(timeout=5)

        def run(question):
            result = subprocess.run([str(binary), "run", "--session", sid, "--model", "smoke/model", "--thinking", "off",
                                     "--tools", "shell", "--permission", "all", "--mode", "json", question],
                                    env=env, cwd=home, capture_output=True, text=True, timeout=60)
            if result.returncode:
                raise RuntimeError(result.stderr + result.stdout)
            check("normal response completed", "S2 full-plan verified" in result.stdout)

        def read_state():
            with sqlite3.connect((config / "agent.db").as_uri() + "?mode=ro", uri=True) as db:
                checkpoint = json.loads(db.execute("SELECT content_json FROM entries WHERE session_id=? AND entry_type='compaction' ORDER BY position DESC LIMIT 1", (sid,)).fetchone()[0])
                info = json.loads(db.execute("SELECT current_metadata_json FROM sessions WHERE id=?", (sid,)).fetchone()[0])
                for identity, original in expected_outputs.items():
                    stored = db.execute("SELECT b.text FROM entries e JOIN message_blocks b ON b.session_id=e.session_id AND b.entry_position=e.position WHERE e.session_id=? AND e.entry_id=? AND b.kind='tool_result'", (sid, identity)).fetchone()[0]
                    check(f"original DB output unchanged {identity}", stored == original)
                return checkpoint, info

        try:
            process = start_agent()
            run("Find MAGIC_HISTORY_TOKEN in the old tool results using history recall.")
            checkpoint, info = read_state()
            check("actual 256K trigger crossed", checkpoint["tokens_before"] >= 256_000)
            check("compacted input below 32K on fixture", checkpoint["tokens_after"] < 32_000)
            check("S2 schema persisted", checkpoint["schema_version"] == 3)
            check("protected references persisted", {"u1", "a1"}.issubset(checkpoint["protected_entry_ids"]))
            check("C algorithm persisted", checkpoint["algorithm_version"] == "deterministic-s2-evidence-v1")
            check("only normal model usage counted", info["tokens_in"] == 300 and info["tokens_out"] == 30)
            check("no summary model cost", abs(info["total_cost"] - 0.006) < 1e-9)
            stop_agent(process)
            process = start_agent()
            run("Confirm the preserved constraints after restart.")
            _, restored_info = read_state()
            check("usage survives restart", restored_info["tokens_in"] == 400 and restored_info["tokens_out"] == 40)
            check("cost survives restart", abs(restored_info["total_cost"] - 0.008) < 1e-9)
            check("exact model-call count", state == {"normal": 4, "summary": 0} and not errors)
            report = {"ok": True, "checks": checks, "model_calls": state, "external_model_calls": 0,
                      "tokens_before": checkpoint["tokens_before"], "tokens_after": checkpoint["tokens_after"],
                      "schema_version": checkpoint["schema_version"], "protected_entry_ids": checkpoint["protected_entry_ids"],
                      "synthetic_final_usage": {k: restored_info[k] for k in ("tokens_in", "tokens_out", "total_cost")},
                      "original_tool_bytes": sum(len(v.encode()) for v in expected_outputs.values()),
                      "evidence_sha256": hashlib.sha256(expected_outputs["result-5"].encode()).hexdigest()}
            if args.report:
                args.report.parent.mkdir(parents=True, exist_ok=True)
                args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
            print(json.dumps({k: v for k, v in report.items() if k != "checks"}, indent=2))
            print(f"PASS: {len(checks)} checks")
        except Exception:
            log.flush()
            log.seek(0)
            print(log.read()[-12000:])
            print("MODEL ERRORS:", errors)
            raise
        finally:
            stop_agent(process)
            server.shutdown()
            server.server_close()
            log.close()


if __name__ == "__main__":
    main()
