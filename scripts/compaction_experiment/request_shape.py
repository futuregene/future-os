"""The request shape a real session sends, taken from the Rust code rather than restated.

An exam has to exercise the same prompt and the same tools a session uses, or it measures an
adaptation instead of the product. Earlier scripts drifted twice: they assembled the system
prompt from the examiner's own text, and they renamed the retrieval commands into
`history_search(query=...)` adapters no product exposes.

Both are gone. A session's system prompt is now simply its own base prompt — the runtime no
longer appends anything after a checkpoint — and the retrieval CLI lives behind the ordinary
shell tool, exactly as a session finds it. This module asks the Rust code for both, so a
transcription cannot drift from them because there is no transcription.
"""
import json
import os
from pathlib import Path
import subprocess


class RequestShape:
    """The system prompt and tool definitions production installs, for one session id.

    `base_prompt` is the session's own system prompt. A frozen session's prompt is rebuilt per
    turn from its working directory and is not in the journal, so it cannot be replayed;
    capture a real one with `capture_shape.py` and pass it here.
    """

    def __init__(self, probe, base_prompt, session_id, tools=None):
        self.probe = Path(probe).resolve()
        self.base_prompt = Path(base_prompt).resolve()
        self.session_id = session_id
        self.tools_file = Path(tools).resolve() if tools else None
        self._shape = None

    def _load(self):
        if self._shape is not None:
            return self._shape
        argv = [str(self.probe), "--model", "shape/unused", "--print-request-shape",
                "--base-system-prompt-file", str(self.base_prompt),
                "--session-id", self.session_id]
        if self.tools_file:
            argv += ["--tools-file", str(self.tools_file)]
        raw = subprocess.check_output(argv, text=True)
        shape = json.loads(raw.strip().splitlines()[-1])
        base = self.base_prompt.read_text()
        # Rust's `.len()` on a `String` counts bytes, and a real prompt contains multi-byte
        # characters, so compare bytes rather than Python characters. Fail rather than adapt:
        # a base prompt production would not have used means the caller passed the wrong file.
        assert shape["base_system_prompt_chars"] == len(base.encode()), (
            "base prompt length disagrees between Rust and the file it read: "
            f"rust={shape['base_system_prompt_chars']} bytes={len(base.encode())}")
        for state in ("no_checkpoint", "with_checkpoint"):
            assert shape[state]["system_prompt"] == base, (
                f"{state}: the system prompt must be the session's own, unchanged")
        # A checkpoint must not change the prompt: it used to, and that is what made a
        # session's prefix diverge from its own summary request. Assert the property that
        # replaced it rather than assuming it.
        assert shape["no_checkpoint"] == shape["with_checkpoint"], (
            "committing a checkpoint must not change the system prompt")
        self._shape = shape
        return shape

    def system_prompt(self, has_checkpoint=True):
        """The exact system prompt a session sends, at either point in its life."""
        key = "with_checkpoint" if has_checkpoint else "no_checkpoint"
        return self._load()[key]["system_prompt"]

    def tools(self):
        """The tool definitions production installs, verbatim."""
        return self._load()["with_checkpoint"]["tools"]

    def tool_names(self):
        return [t["function"]["name"] for t in self.tools()]

    def shape(self):
        return self._load()


def guidance_removed(system_prompt):
    """The runtime used to append a recall guidance after a checkpoint. It is gone, and an
    exam should fail loudly if it comes back rather than silently changing what it measures.
    """
    return "## Archived conversation recall" not in system_prompt


def executor_path(env_var="ABC_TOOL_EXECUTOR"):
    """The production tool executor (a build of `production_tool_executor`)."""
    value = os.environ.get(env_var)
    if not value:
        raise RuntimeError(
            f"{env_var} must point at a build of agent/examples/production_tool_executor; "
            "the exam executes tool calls through production handlers, not a re-implementation")
    return Path(value).resolve()
