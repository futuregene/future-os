"""The request shape a real session sends, taken from the Rust code rather than restated.

An open-book exam has to exercise the same prompt and the same tools a session uses, or it
measures an adaptation instead of the product. Two things drifted in the earlier scripts:

* the recall guidance was extracted from `history_recall.rs` by regex and then **rewritten** —
  "use the existing shell tool" became "use these read-only adapters of the native Future
  history CLI", and the two `future session history ...` commands became
  `history_search(query=...)` / `history_get(entry_id=...)`, tool names that do not exist in
  the product;
* the system prompt was the examiner's own text (`b.SYSTEM`) rather than the session's, so the
  model never saw the prompt production sends.

This module removes both by asking the Rust code. `--print-request-shape` calls
`history_recall::system_prompt` and returns `coding_tools()`'s definitions, so the text and
the schemas are the ones the runtime installs. A transcription cannot drift from them because
there is no transcription.
"""
import json
import os
from pathlib import Path
import subprocess


class RequestShape:
    """The system prompt and tool definitions production installs, for one session id.

    `base_prompt` is the session's own system prompt without recall guidance. A frozen
    session's prompt is rebuilt per turn from its working directory and is not in the journal,
    so it cannot be replayed; capture a real one with `capture_shape.py` and pass it here.
    Everything appended to it is production's, not the caller's.
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
        # characters, so compare bytes rather than Python characters.
        # Fail rather than adapt: a base prompt production would not have used means the
        # caller passed the wrong file, not that this module should patch it.
        assert shape["base_system_prompt_chars"] == len(base.encode()), (
            "base prompt length disagrees between Rust and the file it read: "
            f"rust={shape['base_system_prompt_chars']} bytes={len(base.encode())}")
        for state in ("no_checkpoint", "with_checkpoint"):
            text = shape[state]["system_prompt"]
            assert text.startswith(base), f"{state} prompt does not extend the base prompt"
        assert shape["with_checkpoint"]["recall_guidance"], (
            "the checkpoint form must carry the recall guidance")
        assert not shape["no_checkpoint"]["recall_guidance"], (
            "the no-checkpoint form must not carry the recall guidance")
        assert shape["no_checkpoint"]["system_prompt"] == base, (
            "the no-checkpoint prompt must be the base prompt unchanged")
        self._shape = shape
        return shape

    def system_prompt(self, has_checkpoint=True):
        """The exact system prompt. A replay starts from a compacted projection, so the
        checkpoint form is the default."""
        shape = self._load()
        key = "with_checkpoint" if has_checkpoint else "no_checkpoint"
        return shape[key]["system_prompt"]

    def guidance(self, has_checkpoint=True):
        """Only the part production appends to the base prompt.

        Callers that already have a base prompt in place need this rather than the whole
        prompt, or they would duplicate it.
        """
        whole = self.system_prompt(has_checkpoint)
        base = self.base_prompt.read_text()
        if not whole.startswith(base):
            raise ValueError("production prompt does not extend the base prompt")
        return whole[len(base):]

    def tools(self):
        """The tool definitions production installs, verbatim."""
        return self._load()["with_checkpoint"]["tools"]

    def tool_names(self):
        return [t["function"]["name"] for t in self.tools()]

    def shape(self):
        return self._load()


def executor_path(env_var="ABC_TOOL_EXECUTOR"):
    """The production tool executor (a build of `abc_future_shell_probe`)."""
    value = os.environ.get(env_var)
    if not value:
        raise RuntimeError(
            f"{env_var} must point at a build of agent/examples/abc_future_shell_probe; "
            "the exam executes tool calls through production handlers, not a re-implementation")
    return Path(value).resolve()


def has_guidance(system_prompt, base_prompt):
    """True when the recall guidance is present. The exam asserts this so a production
    prompt cannot silently degrade into the base prompt."""
    return len(system_prompt) != len(base_prompt) and \
        "## Archived conversation recall" in system_prompt
