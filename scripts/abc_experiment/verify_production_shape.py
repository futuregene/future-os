#!/usr/bin/env python3
"""Offline check: the exam's prompt and tools are production's, and the CLI still works.

No model calls. Verifies that
  * the system prompt is exactly the session's own, with no appended guidance, and that
    committing a checkpoint does not change it;
  * the tool definitions are production's, identically from both sources (the request-shape
    probe and the tool executor);
  * the retrieval CLI the tools can reach is intact and scoped to one session;
  * a tool call is executed by a production handler, and the harness path boundary holds.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import production_shape as ps  # noqa: E402

REPO = HERE.parents[1]
PROBE = Path(os.environ.get("ABC_SHAPE_PROBE", REPO / "target/debug/examples/abc_strategy_probe"))
BASE = Path(os.environ.get("ABC_BASE_PROMPT", Path.home() / "compact-exp/shape/system-prompt.txt"))
EXECUTOR = Path(os.environ.get("ABC_TOOL_EXECUTOR", REPO / "target/debug/examples/abc_future_shell_probe"))
FUTURE = Path(os.environ.get("ABC_FUTURE", REPO / "target/debug/future"))


def main():
    failures = []

    def check(name, condition, detail=""):
        print(("ok   " if condition else "FAIL ") + name +
              (("  " + detail) if detail and not condition else ""))
        if not condition:
            failures.append(name)

    sid = "exam-verify-session"
    shape = ps.RequestShape(PROBE, BASE, sid)
    base = BASE.read_text()
    system = shape.system_prompt(has_checkpoint=True)

    # 1. the prompt is the session's own, and the guidance the runtime used to append is gone
    check("system prompt is the captured base prompt, byte for byte", system == base)
    check("no recall guidance is appended", ps.guidance_removed(system))
    check("committing a checkpoint does not change the prompt",
          shape.system_prompt(True) == shape.system_prompt(False))

    # 2. tools: production's set, and both sources agree
    tools = shape.tools()
    names = [t["function"]["name"] for t in tools]
    check("tool set is production's coding_tools()", names == ["read", "write", "edit", "shell"],
          f"got {names}")
    described = []
    for name in names:
        out = subprocess.run([str(EXECUTOR)], input=json.dumps({"mode": "describe", "tool": name}),
                             text=True, capture_output=True, check=True)
        described.append(json.loads(out.stdout))
    check("executor and request-shape agree on the definitions", described == tools)

    # 3. the retrieval CLI is retained: it is what the shell tool can reach, and no prompt
    #    mentions it any more, so this is the only thing guaranteeing it is usable.
    help_out = subprocess.run([str(FUTURE), "session", "history", "--help"],
                              text=True, capture_output=True)
    check("`future session history --help` works", help_out.returncode == 0)
    for needle in ("history search", "history get", "--query", "--entry", "--offset"):
        check(f"CLI documents {needle!r}", needle in help_out.stdout)

    # 4. a call the model could make runs through a production handler, and the harness's
    #    path boundary still holds.
    with tempfile.TemporaryDirectory() as directory:
        workspace = Path(directory) / "ws"
        outside = Path(directory) / "outside"
        workspace.mkdir()
        outside.mkdir()
        (workspace / "inside.txt").write_text("inside-marker")
        (outside / "outside.txt").write_text("outside-marker")

        def execute(tool, arguments, roots=None):
            request = {"mode": "execute", "tool": tool, "workspace": str(workspace),
                       "arguments": arguments}
            if roots:
                request["allowed_roots"] = roots
            return subprocess.run([str(EXECUTOR)], input=json.dumps(request), text=True,
                                  capture_output=True)

        inside = execute("read", {"path": "inside.txt"})
        check("production read handler serves a call",
              inside.returncode == 0 and "inside-marker" in inside.stdout, inside.stderr[:120])
        escaped = execute("read", {"path": str(outside / "outside.txt")})
        check("path boundary refuses a read outside the roots",
              escaped.returncode != 0 and "STUDY_SCOPE_DENIED" in escaped.stderr)
        allowed = execute("read", {"path": str(outside / "outside.txt")}, roots=[str(outside)])
        check("an explicit root admits that path",
              allowed.returncode == 0 and "outside-marker" in allowed.stdout)
        unknown = execute("glob", {})
        check("a tool outside production's set is refused",
              unknown.returncode != 0 and "not in production's installed set" in unknown.stderr)

    print()
    if failures:
        print(f"{len(failures)} failed: {failures}")
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
