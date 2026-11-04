"""Use the shipping CLI and Host, including the real Fusion implementation."""

import json
import os
import re
import subprocess
import time
from collections import Counter
from pathlib import Path

from datasets import ROOT, digest, read_json, write_json
from scoring import score, sum_usage

BASE_MODEL = "future/deepseek-flash"
BASE_THINKING = "max"


class Future:
    def __init__(self, binary, runtime=None):
        self.binary = str(Path(binary).resolve())
        self.env = os.environ.copy()
        if runtime:
            settings = read_json(runtime)
            self.env.update(
                FUTURE_HOME=settings["home"],
                FUTURE_AGENT_GRPC_ADDR=settings["endpoint"],
            )

    def call(self, args, timeout=30, json_output=True):
        proc = subprocess.run(
            [self.binary] + args,
            env=self.env,
            text=True,
            encoding="utf-8",
            capture_output=True,
            timeout=timeout,
        )
        if proc.returncode:
            raise RuntimeError(
                f"future {args[0]} failed ({proc.returncode}): {proc.stderr.strip()}"
            )
        return json.loads(proc.stdout) if json_output else proc.stdout

    def preflight(self, single_only=False):
        settings = self.call(["fusion", "settings", "--json"])
        if not single_only and not settings["enabled"]:
            raise ValueError(
                "Local Fusion is disabled; enable it explicitly before this comparison"
            )
        models = self.call(["models", "--json"])
        found = {
            f"{provider}/{row['id']}": row
            for provider, rows in models["models"].items()
            for row in rows
        }
        profiles = [{"model": BASE_MODEL, "thinking_level": BASE_THINKING}]
        if not single_only:
            profiles += settings["configuration"]["panel"] + [
                settings["configuration"]["moderator"]
            ]
        for profile in profiles:
            model = found.get(profile["model"])
            if (
                model is None
                or profile["thinking_level"] not in model["thinkingLevels"]
            ):
                raise ValueError(
                    f"Unsupported model/thinking binding: {profile}. Refusing silent fallback."
                )
        return {
            "fusion": settings,
            "baseline": {"model": BASE_MODEL, "thinking_level": BASE_THINKING},
            "catalogue": models,
            "binary": self.call(["version", "--json"]),
            "system_prompt": "product default, with project-context files disabled for the closed-text task",
            "fusion_routing_prompt": None,
            "entry": "normal /fusion prompt; automatic tool selection; no benchmark routing instructions",
        }

    def transcript(self, session):
        return self.call(
            [
                "session",
                "transcript",
                "--session",
                session,
                "--select",
                "assistant,tool-call,tool-result",
                "--all",
                "--max-bytes",
                "0",
                "--json",
            ]
        )

    def wait(self, session, run_id, timeout, fusion=False):
        # CLI has a 300s event-idle limit independent of its --timeout. Reattach
        # to the same accepted run on that read-only transport failure; never
        # submit the problem again or spend another generation attempt.
        deadline = time.monotonic() + timeout if timeout else None
        reconnects = 0
        while True:
            remaining = max(1, int(deadline - time.monotonic())) if deadline else 300
            read_timeout = min(60 if fusion else 300, remaining)
            try:
                output = self.call(
                    [
                        "session",
                        "wait",
                        session,
                        "--run",
                        run_id,
                        "--timeout",
                        f"{read_timeout}s",
                        "--mode",
                        "json",
                    ],
                    timeout=read_timeout + 15,
                )
                output["benchmark_wait_reconnects"] = reconnects
                return output
            except RuntimeError as error:
                idle = "Event stream timed out waiting for an event" in str(error)
                read_expired = "Timed out waiting for run" in str(error)
                if not (idle or read_expired) or (
                    deadline and time.monotonic() >= deadline
                ):
                    raise
                reconnects += 1
                # Observe the same daily run through a failed consultation too.
                # Never abort the parent or submit a replacement generation just
                # to make it obey a benchmark-only consultation protocol.

    def run_case(self, case, arm, directory, timeout, resume=False):
        directory = Path(directory)
        existing = directory.exists()
        if existing and not resume:
            raise ValueError("Case directory already exists")
        if existing and (directory / "result.json").exists():
            return read_json(directory / "result.json")
        if existing and not (directory / "ack.json").exists():
            raise ValueError(
                "Interrupted submission without an acknowledgement; inspect manually, never resubmit"
            )
        directory.mkdir(parents=True, exist_ok=resume)
        (directory / ".gitignore").write_text("*\n", encoding="utf-8")
        workspace = directory / "workspace"
        workspace.mkdir(exist_ok=resume)
        prompt = prompt_for(case)
        (directory / "prompt.txt").write_text(
            ("/fusion " if arm == "fusion" else "") + prompt, encoding="utf-8"
        )
        started = time.monotonic()
        result = {
            "case_id": case["id"],
            "dataset": case["dataset"],
            "arm": arm,
            "status": "error",
            "grade": {"correct": False, "parsed": False},
            "primary_endpoint": "daily_final_answer",
        }
        session = None
        try:
            if existing:
                ack = read_json(directory / "ack.json")
            else:
                created = self.call(
                    ["session", "new", "--cwd", str(workspace.resolve()), "--json"]
                )
                session = created["sessionId"]
                self.call(
                    ["session", "set", session, "--context-files", "off", "--json"]
                )
                write_json(directory / "session-created.json", created)
                args = [
                    "run",
                    "--session",
                    session,
                    "--model",
                    BASE_MODEL,
                    "--thinking",
                    BASE_THINKING,
                    "--no-tools",
                    "--no-title",
                    "--permission",
                    "all",
                    "--mode",
                    "json",
                    "--no-wait",
                    "--cwd",
                    str(workspace.resolve()),
                    "--message-file",
                    str((directory / "prompt.txt").resolve()),
                ]
                ack = self.call(args)
                write_json(directory / "ack.json", ack)
            session = ack["sessionId"]
            result.update(session_id=session, run_id=ack["runId"])
            output = self.wait(session, ack["runId"], timeout, fusion=arm == "fusion")
            write_json(directory / "output.json", output)
            result["answer_wall_seconds"] = time.monotonic() - started
            result["text"] = output["text"]
            result["wait_reconnects"] = output.get("benchmark_wait_reconnects", 0)
            result["outer_grade"] = score(case, output["text"])
            parent_transcript = self.transcript(session)
            write_json(directory / "parent-transcript.json", parent_transcript)
            ledger = self.call(
                [
                    "session",
                    "transcript",
                    "--session",
                    session,
                    "--runs",
                    "--all",
                    "--max-bytes",
                    "0",
                    "--json",
                ]
            )
            write_json(directory / "parent-usage.json", ledger)
            result["parent_usage_ledger"] = ledger
            state = self.call(["session", "status", session, "--json"])
            write_json(directory / "parent-state.json", state)
            result["parent_usage"] = state.get("usage", {})
            result["effective_parent_model"] = state.get("model")
            result["effective_parent_thinking"] = state.get("thinkingLevel")
            if (
                state.get("model") != BASE_MODEL
                or state.get("thinkingLevel") != BASE_THINKING
            ):
                raise ValueError(
                    "Effective model/thinking differs from DeepSeek Flash max"
                )
            if ledger.get("hasMore") or parent_transcript.get("hasMore"):
                raise ValueError("Incomplete parent transcript or usage ledger")
            calls = tool_calls(parent_transcript)
            if arm == "single":
                if calls:
                    raise ValueError("Single-model run used tools")
                result.update(status="completed", grade=result["outer_grade"])
            else:
                if any(call.get("name") != "fusion" for call in calls):
                    raise ValueError("Fusion-enabled parent used an external tool")
                result["fusion_invocations"] = len(calls)
                consultations = [
                    self.collect_consultation(
                        case,
                        session,
                        ack["runId"],
                        call["toolCallId"],
                        directory,
                        index,
                    )
                    for index, call in enumerate(calls)
                ]
                result["consultations"] = consultations
                result["completed_consultations"] = sum(
                    item["status"] == "completed" and bool(item["kernel_text"])
                    for item in consultations
                )
                result["routing_outcome"] = (
                    "not_invoked"
                    if not calls
                    else "completed"
                    if result["completed_consultations"]
                    else "no_conclusion"
                )
                result["kernel_usage"] = sum_usage(
                    [item["usage"] for item in consultations]
                )
                result["step_attempt_failure_kinds"] = dict(
                    sum(
                        (
                            Counter(item["step_attempt_failure_kinds"])
                            for item in consultations
                        ),
                        Counter(),
                    )
                )
                # Subquestion answers are not scored against a whole-question
                # reference. Keep their transcripts for qualitative inspection.
                result["stage_grades"] = (
                    consultations[0]["stage_grades"]
                    if len(consultations) == 1
                    and consultations[0]["whole_question_verbatim"]
                    else []
                )
                if len(consultations) == 1:
                    item = consultations[0]
                    result.update(
                        completeness=item["completeness"],
                        fusion_status=item["status"],
                        fusion_configuration=item["configuration"],
                        kernel_wall_seconds=item["wall_seconds"],
                        kernel_text=item["kernel_text"],
                        kernel_grade=item["kernel_grade"],
                    )
                # The daily product endpoint is what the parent actually answers,
                # including a transparently reported skipped/failed consultation.
                result.update(status="completed", grade=result["outer_grade"])
        except (RuntimeError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
            result["error"] = str(error)
            # CLI wait errors can hide the upstream HTTP status. Use the accepted
            # run's terminal ledger, never text from the model, to classify 502s.
            if session and result.get("run_id"):
                try:
                    ledger = self.call(
                        [
                            "session",
                            "transcript",
                            "--session",
                            session,
                            "--runs",
                            "--all",
                            "--max-bytes",
                            "0",
                            "--json",
                        ]
                    )
                    write_json(directory / "failure-ledger.json", ledger)
                    result["parent_usage_ledger"] = ledger
                    result["parent_usage"] = sum_usage(
                        [item.get("usage") for item in ledger.get("ledger", [])]
                    )
                    for item in ledger.get("ledger", []):
                        if (
                            item.get("runId") == result["run_id"]
                            and item.get("status") == "failed"
                            and re.search(r"\bHTTP\s+502\b", item.get("error") or "")
                        ):
                            result.update(
                                http_status=502,
                                error_category="http_502",
                                upstream_error=item["error"],
                            )
                except (
                    RuntimeError,
                    ValueError,
                    subprocess.TimeoutExpired,
                ) as ledger_error:
                    result["failure_evidence_error"] = str(ledger_error)
            if session:
                try:
                    self.call(["session", "abort", session], json_output=False)
                except (RuntimeError, ValueError, subprocess.TimeoutExpired):
                    pass
        finally:
            result["elapsed_seconds"] = time.monotonic() - started
            write_json(directory / "result.json", result)
        return result

    def collect_consultation(
        self, case, session, parent_run, tool_id, directory, index
    ):
        selector = ["--run", parent_run, "--tool", tool_id]
        discussion = self.call(["fusion", "show", session] + selector + ["--json"])
        write_json(directory / f"discussion-{index}.json", discussion)
        if index == 0:
            write_json(directory / "discussion.json", discussion)
        run = discussion["run"]
        if run["input"].get("read_scope"):
            raise ValueError("Fusion received a nonempty read_scope")
        source = run["input"]
        supplied = "\n".join(
            [source.get(key, "") for key in ("question", "goal", "background")]
            + source.get("evidence", [])
        )
        whole_question = case["question"] in supplied
        stages = []
        for step in discussion["steps"]:
            offset, pieces = 0, []
            while step.get("session_id"):
                page = self.call(
                    ["fusion", "show", session]
                    + selector
                    + ["--step", step["id"], "--offset", str(offset), "--json"]
                )
                pieces.append(page.get("transcript", ""))
                next_offset = page.get("next_offset", 0)
                if next_offset >= page.get("total_bytes", 0):
                    break
                if next_offset <= offset:
                    raise ValueError("Fusion transcript paging did not advance")
                offset = next_offset
            transcript = json.loads("".join(pieces)) if pieces else []
            write_json(directory / f"step-{step['id']}.json", transcript)
            if raw_tool_calls(transcript):
                raise ValueError("Fusion child used an external tool")
            text = last_assistant_text(transcript)
            if step["phase"] == "reviewing":
                try:
                    review = json.loads(
                        text.strip().removeprefix("```json").removesuffix("```").strip()
                    )
                    text = review.get("current_answer", "")
                except (ValueError, AttributeError):
                    text = ""
            if whole_question and step["phase"] in ("analyzing", "reviewing"):
                stages.append(
                    {
                        "consultation_id": run["id"],
                        "participant": step["participant"],
                        "phase": step["phase"],
                        "attempt": step["attempt"],
                        "status": step["status"],
                        "grade": score(case, text),
                    }
                )
        conclusion = (run.get("result") or {}).get("conclusion", "")
        return {
            "id": run["id"],
            "tool_call_id": tool_id,
            "status": run["status"],
            "configuration": run["configuration"],
            "completeness": run["completeness"],
            "report": run.get("report"),
            "whole_question_verbatim": whole_question,
            "wall_seconds": (run["updated_at"] - run["created_at"]) / 1000,
            "usage": sum_usage([step.get("usage") for step in discussion["steps"]]),
            "step_attempt_failure_kinds": dict(
                Counter(
                    step.get("failure_kind") or "unclassified"
                    for step in discussion["steps"]
                    if step["status"] == "failed"
                )
            ),
            "kernel_text": conclusion,
            "kernel_grade": score(case, conclusion) if whole_question else None,
            "stage_grades": stages,
        }


def prompt_for(case):
    if case["scorer"] == "grid":
        # Output schema only: header names contain no solution assignments.
        header = json.dumps(case["answer"]["header"])
        fmt = f'a JSON object with "header": {header} and "rows", using the attribute values from the puzzle'
    elif case["scorer"] == "integer":
        fmt = "one integer"
    elif case["scorer"] == "choice":
        fmt = "one option letter"
    else:
        fmt = "the answer in the format requested by the problem"
    return (
        "Use only the supplied problem. Do not browse, access files, or execute code.\n\n"
        + case["question"]
        + f"\n\nSolve the problem. End your answer with FINAL_ANSWER: followed by {fmt}. Do not put commentary after the final answer."
    )


def tool_calls(transcript):
    return [
        block
        for entry in transcript.get("entries", [])
        for block in entry.get("blocks", [])
        if block.get("kind") == "tool_call"
    ]


def raw_tool_calls(transcript):
    return [
        block
        for entry in transcript
        for block in entry.get("content", [])
        if block.get("type") in ("toolCall", "tool_call")
    ]


def last_assistant_text(transcript):
    answers = [
        "\n".join(
            block.get("text", "")
            for block in entry.get("content", [])
            if block.get("type") == "text"
        )
        for entry in transcript
        if entry.get("role") == "assistant"
    ]
    return next((text for text in reversed(answers) if text.strip()), "")


def verify_data():
    for item in read_json(ROOT / "data/manifest.json"):
        path = ROOT / "data" / item["dataset"] / item["file"]
        if digest(path.read_bytes()) != item["sha256"]:
            raise ValueError(f"Dataset changed: {path}")
