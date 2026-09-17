#!/usr/bin/env python3
"""End-to-end offline check of the guided open path, no model calls.

Builds the request the guided exam would send for a Future arm and asserts:
  * its system prompt is byte-identical to the Rust one (base + recall guidance);
  * the guidance names the real `future session history` commands and the shell tool;
  * the tools offered are production's, and the recall guidance's tool (shell) is among them;
  * the user turn carries the exam's question, and the system prompt carries nothing of it.
"""
import json
import os
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import guided_open_exam as g  # noqa: E402
import production_shape as ps  # noqa: E402
from recall_guidance import Guidance  # noqa: E402

REPO = HERE.parents[1]
PROBE = Path(os.environ.get("ABC_SHAPE_PROBE", REPO / "target/debug/examples/abc_strategy_probe"))
BASE = Path(os.environ.get("ABC_BASE_PROMPT", Path.home() / "compact-exp/shape/system-prompt.txt"))


def main():
    failures = []

    def check(name, condition, detail=""):
        print(("ok   " if condition else "FAIL ") + name +
              (("  " + detail) if detail and not condition else ""))
        if not condition:
            failures.append(name)

    session_id = "guided-export-0-C3-native-open"
    guides = Guidance(REPO, REPO, REPO,
                      shape_factory=lambda sid: ps.RequestShape(PROBE, BASE, sid))
    shape = ps.RequestShape(PROBE, BASE, session_id)
    text = guides.text("C3", session_id, has_checkpoint=True)
    check("guidance is production's appended text verbatim",
          text == shape.guidance(has_checkpoint=True))
    check("base + guidance reproduces the production system prompt",
          BASE.read_text() + text == shape.system_prompt(has_checkpoint=True))

    # A closed body as the closed run recorded it, amended the way the guided exam does.
    closed_body = {
        "model": "future/deepseek-flash",
        "messages": [
            {"role": "system", "content": "examiner prompt"},
            {"role": "user", "content": "projection\n\nquestion"},
        ],
        "max_tokens": 8192,
    }
    guidance_text = guides.text("C3", session_id)
    amended = g.guided_base(closed_body, guidance_text)
    body = g.a.open_body(amended, g.a.schemas("C3", shape))
    system = body["messages"][0]["content"]

    check("system prompt carries the recall guidance", "## Archived conversation recall" in system)
    check("guidance names the real search command",
          "future session history search --session <current-session-id>" in system)
    check("guidance names the real get command",
          "future session history get --session <current-session-id>" in system)
    check("guidance names the session literally", f'"{session_id}"' in system)
    check("guidance points at the existing shell tool", "existing shell tool" in system)
    check("no renamed adapter in the prompt", "history_search(" not in system)
    check("no renamed adapter in the prompt (get)", "history_get(" not in system)
    # This variant appends the guidance to the closed body's own system prompt, so the
    # examiner text is still there by design; a fully production-shaped open run uses
    # native_open_exam.py with production_shape instead.
    check("the guidance is what this variant adds, and it is production's",
          system == "examiner prompt" + text)
    check("the question stays in the user turn", body["messages"][1]["content"] ==
          "projection\n\nquestion")
    check("tool_choice is autonomous", body.get("tool_choice") == "auto" and bool(body.get("tools")))

    names = [t.get("function", {}).get("name") for t in body["tools"]]
    check("tools are production's set", names == ["read", "write", "edit", "shell"], str(names))
    check("the guidance's tool is offered", "shell" in names)

    print()
    if failures:
        print(f"{len(failures)} failed: {failures}")
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
