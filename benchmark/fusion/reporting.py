"""Paired reports, with development results explicitly separated from claims."""

import csv
import math
import random
import shutil
import statistics
from collections import Counter
from pathlib import Path

from datasets import ROOT, digest, read_json, write_json
from scoring import sum_usage


def interval(correct, count):
    if not count:
        return [0.0, 1.0]
    z = 1.959963984540054
    p = correct / count
    denominator = 1 + z * z / count
    midpoint = (p + z * z / (2 * count)) / denominator
    half = (
        z * math.sqrt(p * (1 - p) / count + z * z / (4 * count * count)) / denominator
    )
    return [max(0, midpoint - half), min(1, midpoint + half)]


def paired_interval(by_case):
    # Bootstrap whole questions, keeping repeats together; repeats are not new questions.
    means = [statistics.mean(values) for values in by_case.values()]
    # A three-case smoke suite can otherwise misleadingly return [0, 0] for
    # ties. Suppress this interval for very small samples; this is an operational
    # guard, not a claim that 20 questions confer adequate statistical power.
    if len(means) < 20:
        return None
    rng = random.Random(20261104)
    samples = sorted(
        statistics.mean(rng.choices(means, k=len(means))) for _ in range(10000)
    )
    return [samples[249], samples[9749]]


def generate(directory, public_out=None):
    directory = Path(directory)
    manifest = read_json(directory / "manifest.json")
    (directory / "harness").mkdir(exist_ok=True)
    (directory / "harness/reporting-used.py").write_bytes(Path(__file__).read_bytes())
    rows = []
    for path in sorted(directory.glob("r*-*/result.json")):
        row = read_json(path)
        row["repeat"] = int(path.parent.name.split("-")[0][1:])
        row["_directory"] = path.parent
        rows.append(row)
    expected = len(manifest["suite"]["cases"]) * manifest["repeats"]
    daily = manifest.get("execution_profile") == "daily"
    fusion_rows = [row for row in rows if row["arm"] == "fusion"]
    fusion_usage = {
        "recorded": len(fusion_rows),
        "invoked_cases": sum(
            row.get("fusion_invocations", int("fusion_status" in row)) > 0
            for row in fusion_rows
        ),
        "completed_consultation_cases": sum(
            row.get(
                "completed_consultations", int(row.get("fusion_status") == "completed")
            )
            > 0
            for row in fusion_rows
        ),
        "correct_with_completed_consultation": sum(
            row["status"] == "completed"
            and row["grade"]["correct"]
            and row.get(
                "completed_consultations", int(row.get("fusion_status") == "completed")
            )
            > 0
            for row in fusion_rows
        ),
        "not_invoked_cases": sum(
            row.get("fusion_invocations") == 0
            or row.get("error_category") == "fusion_not_invoked"
            for row in fusion_rows
        ),
        "step_attempt_failure_kinds": dict(
            sum(
                (
                    Counter(row.get("step_attempt_failure_kinds", {}))
                    for row in fusion_rows
                ),
                Counter(),
            )
        ),
    }
    groups = {}
    for arm in ("fusion", "single"):
        arm_rows = [row for row in rows if row["arm"] == arm]
        correct = sum(
            row["grade"]["correct"] and row["status"] == "completed" for row in arm_rows
        )
        parent_usage = sum_usage(
            [
                row.get(
                    "parent_usage",
                    sum_usage(
                        [
                            item.get("usage")
                            for item in row.get("parent_usage_ledger", {}).get(
                                "ledger", []
                            )
                        ]
                    ),
                )
                for row in arm_rows
            ]
        )
        kernel_usage = sum_usage([row.get("kernel_usage") for row in arm_rows])
        combined_usage = sum_usage([parent_usage, kernel_usage])
        usage_incomplete = (
            any(row["status"] != "completed" for row in arm_rows)
            or len(arm_rows) < expected
            or any(row.get("step_attempt_failure_kinds") for row in arm_rows)
            or (
                arm == "fusion"
                and any(
                    row.get("completeness") in ("partial", "single") for row in arm_rows
                )
            )
        )
        # An available kernel cost is not a complete end-to-end cost if the
        # parent ledger omitted it. Preserve partial values only in components.
        for key in list(combined_usage):
            if "cost" in key.lower() and (
                usage_incomplete
                or key not in parent_usage
                or (arm == "fusion" and key not in kernel_usage)
            ):
                combined_usage.pop(key)
        groups[arm] = {
            "correct": correct,
            "scheduled": expected,
            "recorded": len(arm_rows),
            "missing_results": expected - len(arm_rows),
            "accuracy": correct / expected,
            "wilson_95": interval(correct, expected)
            if manifest["repeats"] == 1
            else None,
            "errors": sum(row["status"] != "completed" for row in arm_rows),
            "error_categories": dict(
                Counter(
                    row.get("error_category") or "unclassified"
                    for row in arm_rows
                    if row["status"] != "completed"
                )
            ),
            "by_dataset": {
                name: {
                    "scheduled": sum(
                        case["dataset"] == name for case in manifest["suite"]["cases"]
                    )
                    * manifest["repeats"],
                    "recorded": sum(row["dataset"] == name for row in arm_rows),
                    "correct": sum(
                        row["dataset"] == name
                        and row["status"] == "completed"
                        and row["grade"]["correct"]
                        for row in arm_rows
                    ),
                    "errors": sum(
                        row["dataset"] == name and row["status"] != "completed"
                        for row in arm_rows
                    ),
                }
                for name in sorted(
                    {case["dataset"] for case in manifest["suite"]["cases"]}
                )
            },
            "parse_failures": sum(not row["grade"].get("parsed") for row in arm_rows),
            "mean_answer_seconds": statistics.mean(
                row.get("answer_wall_seconds", row["elapsed_seconds"])
                for row in arm_rows
            )
            if arm_rows
            else None,
            "median_answer_seconds": statistics.median(
                row.get("answer_wall_seconds", row["elapsed_seconds"])
                for row in arm_rows
            )
            if arm_rows
            else None,
            "parent_usage": parent_usage,
            "kernel_usage": kernel_usage,
            "usage_incomplete_possible": usage_incomplete,
            "end_to_end_usage": combined_usage,
        }
    pairs = {}
    for row in rows:
        pairs.setdefault((row["case_id"], row["repeat"]), {})[row["arm"]] = row
    outcomes = {
        "both_correct": 0,
        "fusion_only": 0,
        "single_only": 0,
        "neither_passed": 0,
    }
    by_case = {}
    diagnostics = {
        "corrected": 0,
        "corrupted": 0,
        "review_pairs": 0,
        "minority_correct_cases": 0,
        "minority_preserved": 0,
        "all_first_wrong_cases": 0,
        "all_first_wrong_final_correct": 0,
    }
    for (case_id, _), pair in pairs.items():
        if set(pair) != {"fusion", "single"}:
            continue
        f, s = [
            int(pair[arm]["status"] == "completed" and pair[arm]["grade"]["correct"])
            for arm in ("fusion", "single")
        ]
        key = (
            "both_correct"
            if f and s
            else "fusion_only"
            if f
            else "single_only"
            if s
            else "neither_passed"
        )
        outcomes[key] += 1
        by_case.setdefault(case_id, []).append(f - s)
        stages = pair["fusion"].get("stage_grades", [])
        kernel_grade = pair["fusion"].get("kernel_grade")
        diagnostic_final = (
            int(kernel_grade["correct"]) if isinstance(kernel_grade, dict) else f
        )
        # Latest successful attempt per role/phase; failed outputs are not votes.
        latest = {}
        for stage in sorted(stages, key=lambda item: item["attempt"]):
            if stage["status"] == "completed":
                latest[(stage["participant"], stage["phase"])] = stage["grade"]
        diagnostics["stage_answers"] = diagnostics.get("stage_answers", 0) + len(latest)
        diagnostics["stage_unparsed"] = diagnostics.get("stage_unparsed", 0) + sum(
            not grade["parsed"] for grade in latest.values()
        )
        initial = [
            grade for (_, phase), grade in latest.items() if phase == "analyzing"
        ]
        for (participant, phase), before in latest.items():
            after = latest.get((participant, "reviewing"))
            if (
                phase != "analyzing"
                or not after
                or not before["parsed"]
                or not after["parsed"]
            ):
                continue
            diagnostics["review_pairs"] += 1
            diagnostics["corrected"] += int(not before["correct"] and after["correct"])
            diagnostics["corrupted"] += int(before["correct"] and not after["correct"])
        expected_panel = len(
            manifest["configuration"]["fusion"]["configuration"].get("panel", [])
        )
        if (
            expected_panel
            and len(initial) == expected_panel
            and all(item["parsed"] for item in initial)
        ):
            diagnostics["fully_scored_initial_cases"] = (
                diagnostics.get("fully_scored_initial_cases", 0) + 1
            )
            count = sum(item["correct"] for item in initial)
            if 0 < count < len(initial) / 2:
                diagnostics["minority_correct_cases"] += 1
                diagnostics["minority_preserved"] += diagnostic_final
            if count == 0:
                diagnostics["all_first_wrong_cases"] += 1
                diagnostics["all_first_wrong_final_correct"] += diagnostic_final
    summary = {
        "purpose": "development pilot; not publication-ready",
        "primary_endpoint": manifest.get("primary_endpoint", "kernel_conclusion"),
        "fusion_usage": fusion_usage,
        "manifest_sha256": digest(manifest),
        "arms": groups,
        "paired": outcomes,
        "paired_delta_95_question_bootstrap": paired_interval(by_case),
        "paired_interval_note": "suppressed below 20 distinct questions; formal sample size still requires power analysis",
        "diagnostics": diagnostics,
        "complete": len(rows) == 2 * expected,
        "invalid_exercise": (directory / "INVALID.md").exists(),
        "report_generator_sha256": digest(Path(__file__).read_bytes()),
    }
    write_json(directory / "summary.json", summary)
    with (directory / "results.csv").open("w", newline="", encoding="utf-8") as stream:
        writer = csv.writer(stream)
        writer.writerow(
            [
                "case_id",
                "dataset",
                "repeat",
                "arm",
                "status",
                "correct",
                "parsed",
                "answer_seconds",
                "completeness",
                "error",
            ]
        )
        for row in rows:
            writer.writerow(
                [
                    row["case_id"],
                    row["dataset"],
                    row["repeat"],
                    row["arm"],
                    row["status"],
                    int(row["grade"]["correct"] and row["status"] == "completed"),
                    row["grade"].get("parsed"),
                    row.get("answer_wall_seconds"),
                    row.get("completeness"),
                    row.get("error"),
                ]
            )
    lines = [
        "# Fusion development pilot",
        "",
        "Development smoke run. This sample does not support a general capability or marketing claim.",
        "",
        "Status: complete."
        if summary["complete"]
        else f"Status: INCOMPLETE ({len(rows)}/{2 * expected} model arms recorded). Pending results are not final failures; all aggregates below are provisional.",
        "",
        f"Frozen suite: `{digest(manifest['suite'])}`; repeats: {manifest['repeats']}.",
        "",
        "Primary: the ordinary final answer in each arm. Fusion is enabled with /fusion and selected naturally by the model; skipped consultations remain visible. A correct answer without consultation is not evidence of Fusion-assisted reasoning."
        if daily
        else "Primary: Fusion kernel conclusion vs single-model final answer. Parent answer is retained separately.",
        "",
        "| Arm | Correct / scheduled | Task pass rate | Errors | Parse failures | Mean observed wall seconds |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for arm, values in groups.items():
        latency = values["mean_answer_seconds"]
        lines.append(
            f"| {arm} | {values['correct']}/{expected} | {values['accuracy']:.1%} | {values['errors']} | {values['parse_failures']} | {latency:.1f} |"
            if latency is not None
            else f"| {arm} | 0/{expected} | 0% | - | - | - |"
        )
    lines += [
        "",
        "Errors, absent results and unparseable answers count as incorrect in the scheduled denominator.",
        "Task pass rate includes execution failures. It is not a pure reasoning score when a run fails before producing a final answer. Failed-run wall time is not successful-answer latency.",
        f"Error categories: fusion `{groups['fusion']['error_categories']}`; single `{groups['single']['error_categories']}`.",
        f"Fusion activation/coverage: `{fusion_usage}`. These are observed associations, not a causal estimate of the tool's benefit.",
        "",
        "| Dataset | Fusion correct / scheduled | DeepSeek Flash max correct / scheduled |",
        "| --- | --- | --- |",
    ]
    for name, values in groups["fusion"]["by_dataset"].items():
        single = groups["single"]["by_dataset"][name]
        lines.append(
            f"| {name} | {values['correct']}/{values['scheduled']} | {single['correct']}/{single['scheduled']} |"
        )
    lines += [
        "",
        "| Case | Fusion | DeepSeek Flash max |",
        "| --- | --- | --- |",
    ]
    for (case_id, repeat), pair in sorted(pairs.items()):
        cells = []
        for arm in ("fusion", "single"):
            row = pair.get(arm)
            cells.append(
                "missing"
                if row is None
                else "error"
                if row["status"] != "completed"
                else "correct"
                if row["grade"]["correct"]
                else "wrong"
                if row["grade"]["parsed"]
                else "unparsed"
            )
        lines.append(f"| {case_id}, repeat {repeat + 1} | {cells[0]} | {cells[1]} |")
    lines += [
        "",
        f"Paired outcomes: `{outcomes}`.",
        "",
        f"Stage diagnostics: `{diagnostics}`.",
        "",
        "Runtime-reported token/cache/cost fields are retained in summary.json. Missing cost fields mean unknown, never free. costCny is the runtime field name: Future billing may originate in provider credit_cost; no USD/CNY conversion is asserted. Kernel totals include recorded attempts. End-to-end latency includes the outer routing model; no equal-budget claim is made.",
        "",
        "Configuration and source versions: manifest.json. Public reports should include aggregate data and IDs, not licensed question text, solutions, private session IDs or authentication files.",
    ]
    (directory / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"Report: {directory / 'report.md'}", flush=True)
    if public_out is not None:
        if not summary["complete"] or summary["invalid_exercise"]:
            raise ValueError("Refusing to export an incomplete or invalid exercise")
        prefix = Path(public_out)
        repository = ROOT.parent.parent.resolve()
        destination = prefix.resolve()
        if destination.is_relative_to(repository) and not destination.is_relative_to(
            (ROOT / "reports").resolve()
        ):
            raise ValueError(
                "Repository exports must stay under ignored benchmark/fusion/reports; alternatively choose a path outside the repository"
            )
        if any(
            prefix.with_suffix(suffix).exists() for suffix in (".json", ".csv", ".md")
        ):
            raise ValueError("Public export already exists; use a new prefix")
        safe = {
            "schema": 1,
            "purpose": summary["purpose"],
            "started_at_utc": manifest["started_at_utc"],
            "git_commit": manifest["git_commit"],
            "python": manifest.get("python"),
            "platform": manifest.get("platform"),
            "stored_agent_retry_settings": manifest.get("agent_runtime_policy", {}).get(
                "retry"
            ),
            "binary_version": manifest["configuration"]["binary"],
            "harness_sha256": manifest.get("harness_sha256"),
            "implementation_sha256": manifest.get("implementation_sha256"),
            "binary_sha256": manifest.get("binary_sha256"),
            "report_generator_sha256": summary["report_generator_sha256"],
            "fusion_configuration": manifest["configuration"]["fusion"][
                "configuration"
            ],
            "baseline": manifest["configuration"]["baseline"],
            "declared_model_capabilities": read_json(directory / "audit.json").get(
                "declared_model_capabilities"
            )
            if (directory / "audit.json").exists()
            else None,
            "suite": manifest["suite"],
            "repeats": manifest["repeats"],
            "protocol": manifest["protocol"],
            "primary_endpoint": summary["primary_endpoint"],
            "execution_profile": manifest.get("execution_profile", "legacy"),
            "sources": [
                item
                for item in manifest["data_manifest"]
                if item["dataset"]
                in {case["dataset"] for case in manifest["suite"]["cases"]}
            ],
            "summary": summary,
            "evidence_hashes": [
                {
                    "id": row["case_id"],
                    "repeat": row["repeat"],
                    "arm": row["arm"],
                    "files": {
                        name: digest((row["_directory"] / filename).read_bytes())
                        for name, filename in [
                            ("result", "result.json"),
                            ("observed_result", "result-observed.json"),
                            ("output", "output.json"),
                            ("discussion", "discussion-audited.json"),
                        ]
                        if (row["_directory"] / filename).exists()
                    },
                }
                for row in rows
            ],
            "cases": [
                {
                    "id": row["case_id"],
                    "dataset": row["dataset"],
                    "repeat": row["repeat"],
                    "arm": row["arm"],
                    "status": row["status"],
                    "correct": row["grade"]["correct"] and row["status"] == "completed",
                    "parsed": row["grade"].get("parsed", False),
                    "answer_seconds": row.get("answer_wall_seconds"),
                    "completeness": row.get("completeness"),
                    "error_category": row.get("error_category"),
                    "fusion_invocations": row.get("fusion_invocations"),
                    "completed_consultations": row.get("completed_consultations"),
                    "routing_outcome": row.get("routing_outcome"),
                }
                for row in rows
            ],
        }
        write_json(prefix.with_suffix(".json"), safe)
        public_harness = prefix.parent / (prefix.name + "-harness")
        public_harness.mkdir(exist_ok=False)
        for path in (directory / "harness").iterdir():
            if path.name.endswith((".py", ".json")) or path.name == "requirements.txt":
                shutil.copyfile(path, public_harness / path.name)
        write_json(public_harness / "suite.json", manifest["suite"])
        if manifest.get("implementation_sha256"):
            public_implementation = prefix.parent / (prefix.name + "-implementation")
            public_implementation.mkdir(exist_ok=False)
            for name, expected_hash in manifest["implementation_sha256"].items():
                source = directory / "implementation" / name
                if digest(source.read_bytes()) != expected_hash:
                    raise ValueError("Implementation snapshot hash mismatch: " + name)
                destination = public_implementation / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, destination)
        with prefix.with_suffix(".csv").open(
            "w", newline="", encoding="utf-8"
        ) as stream:
            writer = csv.DictWriter(stream, fieldnames=list(safe["cases"][0]))
            writer.writeheader()
            writer.writerows(safe["cases"])
        prefix.with_suffix(".md").write_text(
            "\n".join(lines)
            + f"\n\nReproduction metadata and safe aggregates: [{prefix.name}.json]({prefix.name}.json).\n"
            + f"\nExecution source snapshot: `{public_harness.name}/` (copy into benchmark/fusion in a clean checkout to reproduce).\n",
            encoding="utf-8",
        )
        print(f"Sanitized public pilot export: {prefix}", flush=True)
