"""Read-only collection of terminal evidence; never submit or regenerate a run."""

import datetime
import shutil
from collections import Counter
from pathlib import Path

from datasets import digest, read_json, write_json
from future_runner import BASE_MODEL, BASE_THINKING, Future, tool_calls
from scoring import sum_usage


def collect(directory, binary, runtime=None):
    directory = Path(directory)
    manifest = read_json(directory / "manifest.json")
    client = Future(binary, runtime)
    capabilities = []
    if runtime:
        cache = Path(read_json(runtime)["home"]) / "agent/.future-models-cache.json"
        wanted = {
            item["model"].split("/")[-1]
            for item in manifest["configuration"]["fusion"]["configuration"]["panel"]
            + [manifest["configuration"]["fusion"]["configuration"]["moderator"]]
        }

        def project(value):
            if isinstance(value, dict):
                if value.get("id") in wanted and "MaxTokens" in value:
                    # The cache contains APIKey/headers: export only this explicit
                    # non-secret projection, never the complete cached entry.
                    capabilities.append(
                        {
                            key: value.get(key)
                            for key in (
                                "id",
                                "ContextWindow",
                                "MaxTokens",
                                "ThinkingLevels",
                            )
                        }
                    )
                for item in value.values():
                    project(item)
            elif isinstance(value, list):
                for item in value:
                    project(item)

        if cache.exists():
            project(read_json(cache))
    daily = manifest.get("execution_profile") == "daily"
    records = []
    for path in sorted(directory.glob("r*-*/result.json")):
        result = read_json(path)
        if result.get("reused_baseline"):
            original = path.parent / "baseline-result-original.json"
            if digest(original.read_bytes()) != result["baseline_result_sha256"]:
                raise ValueError("Reused baseline evidence changed")
            records.append(
                {
                    "case": result["case_id"],
                    "arm": result["arm"],
                    "audited": False,
                    "source": "frozen historical baseline",
                }
            )
            continue
        session = result.get("session_id")
        if not session:
            records.append(
                {"case": result["case_id"], "arm": result["arm"], "audited": False}
            )
            continue
        original_grade = digest(result["grade"])
        original = path.with_name("result-observed.json")
        if not original.exists():
            shutil.copyfile(path, original)
        state = client.call(["session", "status", session, "--json"])
        write_json(path.parent / "parent-state-audited.json", state)
        if state.get("eventJournalError"):
            result.setdefault("outer_error", result.get("error"))
            result["runtime_storage_error"] = state["eventJournalError"]
            result["error_category"] = "persistence_degraded"
            result["error"] = state["eventJournalError"]
        if (
            state.get("thinkingLevel") != BASE_THINKING
            or state.get("model", "").split("/")[-1] != BASE_MODEL.split("/")[-1]
        ):
            raise ValueError(
                "Effective outer model/effort differs from the frozen comparison"
            )
        transcript = client.transcript(session)
        write_json(path.parent / "parent-transcript-audited.json", transcript)
        calls = tool_calls(transcript)
        names = [call.get("name") for call in calls]
        if daily:
            allowed = (
                all(name == "fusion" for name in names)
                if result["arm"] == "fusion"
                else not names
            )
            if not allowed:
                raise ValueError(f"External tool calls in daily reasoning run: {names}")
            if (
                state.get("customSystemPrompt")
                or state.get("appendSystemPrompt")
                or state.get("contextFilesEnabled") is not False
            ):
                raise ValueError(
                    "Daily run did not use the ordinary prompt with project context disabled"
                )
        elif names != (["fusion"] if result["arm"] == "fusion" else []):
            if (
                result["arm"] == "fusion"
                and not names
                and result["status"] == "error"
                and not result["grade"]["correct"]
            ):
                result.setdefault("error_category", "fusion_not_invoked")
                result["protocol_valid"] = False
            else:
                raise ValueError(f"Unexpected tool calls: {names}")
        if result["arm"] == "fusion":
            result["fusion_invocations"] = len(names)
        result["parent_usage"] = state.get("usage", {})
        result["effective_parent_model"] = state.get("model")
        result["effective_parent_thinking"] = state.get("thinkingLevel")
        output = path.with_name("output.json")
        if output.exists():
            result["wait_reconnects"] = read_json(output).get(
                "benchmark_wait_reconnects"
            )
        if result["arm"] == "fusion":
            retained = {
                item["tool_call_id"]: item for item in result.get("consultations", [])
            }
            consulted = []
            for index, call in enumerate(calls):
                selector = (
                    ["--run", result["run_id"], "--tool", call["toolCallId"]]
                    if daily
                    else []
                )
                discussion = client.call(
                    ["fusion", "show", session] + selector + ["--json"]
                )
                write_json(path.parent / f"discussion-audited-{index}.json", discussion)
                if len(calls) == 1:
                    write_json(path.parent / "discussion-audited.json", discussion)
                run = discussion["run"]
                if digest(run["configuration"]) != digest(
                    manifest["configuration"]["fusion"]["configuration"]
                ):
                    raise ValueError(
                        "Actual discussion configuration differs from manifest"
                    )
                if run["input"].get("read_scope"):
                    raise ValueError("Nonempty read_scope in actual discussion")
                item = dict(retained.get(call["toolCallId"], {}))
                item.update(
                    id=run["id"],
                    tool_call_id=call["toolCallId"],
                    status=run["status"],
                    completeness=run["completeness"],
                    configuration=run["configuration"],
                    wall_seconds=(run["updated_at"] - run["created_at"]) / 1000,
                    usage=sum_usage(
                        [step.get("usage") for step in discussion["steps"]]
                    ),
                    report=run.get("report"),
                    kernel_text=(run.get("result") or {}).get("conclusion", ""),
                    step_attempt_failure_kinds=dict(
                        Counter(
                            step.get("failure_kind") or "unclassified"
                            for step in discussion["steps"]
                            if step["status"] == "failed"
                        )
                    ),
                )
                consulted.append(item)
                if len(calls) == 1:
                    result.update(
                        fusion_status=item["status"],
                        completeness=item["completeness"],
                        fusion_configuration=item["configuration"],
                        kernel_wall_seconds=item["wall_seconds"],
                        execution_report=item["report"],
                    )
                if not daily and run["status"] != "completed":
                    if result["grade"]["correct"]:
                        raise ValueError(
                            "Failed Fusion was incorrectly scored as a correct answer"
                        )
                    result.setdefault("outer_error", result.get("error"))
                    result["error"] = (
                        run.get("error") or "Fusion returned no conclusion"
                    )
                    result["error_category"] = "fusion_no_conclusion"
                for step in discussion["steps"]:
                    if not step.get("session_id"):
                        continue
                    child = client.transcript(step["session_id"])
                    write_json(path.parent / f"child-audited-{step['id']}.json", child)
                    if tool_calls(child):
                        raise ValueError(
                            "External tool found in an actual Fusion child"
                        )
            result["completed_consultations"] = sum(
                item["status"] == "completed" and bool(item["kernel_text"])
                for item in consulted
            )
            result["kernel_usage"] = sum_usage([item["usage"] for item in consulted])
            result["step_attempt_failure_kinds"] = dict(
                sum(
                    (Counter(item["step_attempt_failure_kinds"]) for item in consulted),
                    Counter(),
                )
            )
            result["routing_outcome"] = (
                "not_invoked"
                if not calls
                else "completed"
                if result["completed_consultations"]
                else "no_conclusion"
            )
            if daily:
                result["consultations"] = consulted
        if digest(result["grade"]) != original_grade:
            raise ValueError("Evidence collection must never change an answer grade")
        result["evidence_collection"] = (
            "Read-only get_state/transcript and get_fusion when invoked, after terminal; grade unchanged; original in result-observed.json"
        )
        write_json(path, result)
        records.append(
            {
                "case": result["case_id"],
                "arm": result["arm"],
                "audited": True,
                "grade_sha256": original_grade,
                "observed_result_sha256": digest(original.read_bytes()),
            }
        )
    write_json(
        directory / "audit.json",
        {
            "collected_at_utc": datetime.datetime.now(
                datetime.timezone.utc
            ).isoformat(),
            "collector_sha256": digest(Path(__file__).read_bytes()),
            "declared_model_capabilities": capabilities,
            "records": records,
        },
    )
    shutil.copyfile(__file__, directory / "harness/audit-used.py")
    print(
        f"Read-only evidence audited for {len(records)} recorded results; no model calls"
    )
