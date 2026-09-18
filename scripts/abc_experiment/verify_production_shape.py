#!/usr/bin/env python3
"""Offline check: the open-book exam's prompt and tools come from the Rust code.

No model calls. Verifies that
  * the system prompt the replay would send is byte-identical to
    `history_recall::system_prompt`, and carries the recall guidance;
  * the tool definitions are production's, identically from both sources
    (the request-shape probe and the tool executor);
  * the recall guidance still names the real `future session history` commands and the
    shell tool, i.e. nothing renamed them into adapters the product does not have;
  * a tool call the model could make is executed by a production handler.
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


def main():
    failures = []

    def check(name, condition, detail=""):
        print(("ok   " if condition else "FAIL ") + name + (("  " + detail) if detail and not condition else ""))
        if not condition:
            failures.append(name)

    sid = "exam-verify-session"
    shape = ps.RequestShape(PROBE, BASE, sid)
    system = shape.system_prompt(has_checkpoint=True)
    base = BASE.read_text()

    # 1. straight from the Rust function, guidance included
    check("system prompt extends the captured base prompt", system.startswith(base))
    check("recall guidance present via the shared helper", ps.has_guidance(system, base))
    check("no-checkpoint form is the base prompt unchanged",
          shape.system_prompt(has_checkpoint=False) == base)

    # 2. it is production's text, not a restatement: the commands the runtime tells the model
    #    to run must be the real ones, and no `history_search`-style adapter may appear.
    for needle in ("## Archived conversation recall",
                   "future session history search --session <current-session-id> --query",
                   "future session history get --session <current-session-id> --entry",
                   "use the existing shell tool",
                   f'"{sid}"'):
        check(f"guidance contains {needle[:48]!r}", needle in system)
    for banned in ("history_search(", "history_get(", "read-only adapters", "offset=0, limit=8192"):
        check(f"guidance has no renamed adapter {banned!r}", banned not in system)

    # 3. tools: production's set, and both sources agree
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
        check("production read handler serves a call", inside.returncode == 0 and
              "inside-marker" in inside.stdout, inside.stderr[:120])
        escaped = execute("read", {"path": str(outside / "outside.txt")})
        check("path boundary refuses a read outside the roots", escaped.returncode != 0 and
              "STUDY_SCOPE_DENIED" in escaped.stderr, escaped.stdout[:120])
        allowed = execute("read", {"path": str(outside / "outside.txt")}, roots=[str(outside)])
        check("an explicit root admits that path", allowed.returncode == 0 and
              "outside-marker" in allowed.stdout)
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
