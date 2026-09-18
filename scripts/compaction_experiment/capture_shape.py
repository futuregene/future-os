#!/usr/bin/env python3
"""Capture the request shape a real turn sends, for the compaction driver.

The compaction strategies receive the session's system prompt and the real tool
definitions; the driver cannot guess either. A session's own prompt is rebuilt each turn
from its working directory and project context and is not stored in the journal, so no
experiment can replay a frozen session's original prompt. What it can do is capture a real
one — this script runs an isolated Agent against a local stub provider, records the first
request, and writes the system prompt and tool definitions it contained.

Usage:
  python3 scripts/compaction_experiment/capture_shape.py --binary target/debug/future \
      --out ~/compact-exp/shape

Writes <out>/system-prompt.txt, <out>/tools.json and <out>/shape.json. The captured text
is a real production prompt, not the frozen sessions' own; report it as such.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time

# Tools a session may hold. `shell` matters: production only appends the archived-recall
# guidance when recall is allowed *and* the session can run a shell.
DEFAULT_TOOLS = ["shell", "read", "write", "edit", "glob", "grep"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--tools", default=",".join(DEFAULT_TOOLS))
    parser.add_argument("--cwd", default=None, help="working directory of the captured session")
    args = parser.parse_args()
    binary = args.binary.resolve()
    captured = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            try:
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                if not captured:
                    captured.append(body)
                text = "\n".join(
                    (m.get("content") if isinstance(m.get("content"), str)
                     else "\n".join(p.get("text", "") for p in m.get("content") or []))
                    for m in body.get("messages", [])
                    if m.get("role") == "system"
                )
                chunks = [
                    {"choices": [{"index": 0, "delta": {"content": "ok"}, "finish_reason": None}]},
                    {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]},
                    {"choices": [], "usage": {"prompt_tokens": 100, "completion_tokens": 5,
                                              "total_tokens": 105, "credit_cost": 0.0}},
                ]
                payload = ("".join("data: " + json.dumps(c) + "\n\n" for c in chunks)
                           + "data: [DONE]\n\n").encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
                del text
            except Exception as error:  # surfaced through the exit status
                captured.append({"error": str(error)})
                self.send_response(500)
                self.end_headers()

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()

    args.out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="future-shape-") as directory:
        home = Path(directory)
        config = home / ".future" / "agent"
        config.mkdir(parents=True)
        env = os.environ.copy()
        for key in ("FUTURE_HOME", "FUTURE_AGENT_SOCKET", "FUTURE_AGENT_GRPC_ADDR"):
            env.pop(key, None)
        env.update(HOME=str(home), USERPROFILE=str(home), XDG_RUNTIME_DIR=str(home / "runtime"))
        env["PATH"] = str(binary.parent) + os.pathsep + env.get("PATH", "")
        Path(env["XDG_RUNTIME_DIR"]).mkdir()
        (config / "models.json").write_text(json.dumps({"providers": {"shape": {
            "api": "openai-completions", "apiKey": "local-fixture",
            "baseUrl": f"http://127.0.0.1:{server.server_port}/v1",
            "models": [{"id": "model", "modalities": ["text"], "reasoning": False,
                        "contextWindow": 1_000_000, "maxTokens": 384_000}]}}}), encoding="utf-8")
        (config / "settings.json").write_text(
            json.dumps({"defaultModel": "shape/model"}), encoding="utf-8")
        # `future run --session` needs an existing session; a one-entry journal is enough,
        # and its `cwd` is what the base prompt is built from.
        cwd = Path(args.cwd).resolve() if args.cwd else home
        sessions = config / "sessions"
        sessions.mkdir(parents=True, exist_ok=True)
        (sessions / "shape.jsonl").write_text("".join(json.dumps(row) + "\n" for row in [
            {"id": "info", "type": "session_info", "role": "system",
             "timestamp": "2026-09-14T00:00:00Z",
             "content": {"cwd": str(cwd), "model": "shape/model", "thinking_level": "off"}},
            {"id": "u1", "type": "user", "role": "user",
             "timestamp": "2026-09-14T00:00:01Z", "content": "Reply with the single word: ok"},
        ]), encoding="utf-8")

        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        env["FUTURE_AGENT_GRPC_ADDR"] = f"127.0.0.1:{port}"
        agent = subprocess.Popen([str(binary), "agent", "--grpc-addr", env["FUTURE_AGENT_GRPC_ADDR"]],
                                 env=env, cwd=cwd, stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL)
        try:
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                if agent.poll() is not None:
                    raise SystemExit("isolated agent exited before it served a request")
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=.1):
                        break
                except OSError:
                    threading.Event().wait(.03)
            else:
                raise SystemExit("isolated agent did not start")

            result = subprocess.run(
                [str(binary), "run", "--session", "shape", "--model", "shape/model",
                 "--thinking", "off", "--tools", args.tools, "--permission", "all",
                 "--mode", "json", "Reply with the single word: ok"],
                env=env, cwd=cwd, capture_output=True, text=True, timeout=120)
            if result.returncode:
                raise SystemExit(f"capture turn failed:\n{result.stdout}\n{result.stderr}")
        finally:
            agent.terminate()
            try:
                agent.wait(timeout=10)
            except subprocess.TimeoutExpired:
                agent.kill()
                agent.wait(timeout=5)
            server.shutdown()
            server.server_close()

    if not captured or "error" in captured[0]:
        raise SystemExit(f"captured nothing usable: {captured!r}")
    body = captured[0]
    system = "\n".join(
        (m.get("content") if isinstance(m.get("content"), str)
         else "\n".join(p.get("text", "") for p in m.get("content") or []))
        for m in body.get("messages", [])
        if m.get("role") == "system"
    )
    tools = body.get("tools") or []
    if not system.strip():
        raise SystemExit("captured a request with an empty system prompt")
    if not tools:
        raise SystemExit("captured a request with no tool definitions")
    (args.out / "system-prompt.txt").write_text(system, encoding="utf-8")
    (args.out / "tools.json").write_text(
        json.dumps(tools, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (args.out / "shape.json").write_text(json.dumps({
        "captured_from": "a real session turn against a local stub provider",
        "cwd": str(cwd),
        "requested_tools": args.tools,
        "system_prompt_chars": len(system),
        # The runtime used to append a recall guidance after a checkpoint; it no longer does.
        # This records whether the captured prompt carries one, so a capture taken from an
        # older build is detectable rather than silently used.
        "system_prompt_has_recall_guidance": "## Archived conversation recall" in system,
        "tool_count": len(tools),
        "tool_names": [t.get("function", {}).get("name") for t in tools],
        "model": "shape/model",
    }, indent=2) + "\n", encoding="utf-8")
    guidance = "## Archived conversation recall" in system
    print(f"system prompt: {len(system)} chars"
          + (" [WARNING: carries the retired recall guidance; rebuild before using]"
             if guidance else " (no recall guidance, as the runtime sends it)"))
    print(f"tools: {len(tools)} -> {[t.get('function', {}).get('name') for t in tools]}")
    print(f"written to {args.out}")


if __name__ == "__main__":
    main()
