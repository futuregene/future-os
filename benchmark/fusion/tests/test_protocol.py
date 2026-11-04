"""Offline protocol regressions. CI only: no network, credentials, or model calls."""

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from datasets import ROOT, load_frozen_questions, read_json, sample_cases, write_json  # noqa: E402
from audit import collect  # noqa: E402
from future_runner import Future, unexecuted_consultation  # noqa: E402
from reporting import generate  # noqa: E402
from scoring import score  # noqa: E402


class ReasoningProtocolTests(unittest.TestCase):
    def test_audit_preserves_legacy_failure_and_daily_answer_when_fusion_is_not_invoked(
        self,
    ):
        for daily in (False, True):
            with self.subTest(daily=daily), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                directory = root / "r0-demo-1-fusion"
                directory.mkdir()
                (root / "harness").mkdir()
                write_json(
                    root / "manifest.json",
                    {"execution_profile": "daily"} if daily else {},
                )
                grade = {"correct": daily, "parsed": daily}
                write_json(
                    directory / "result.json",
                    {
                        "case_id": "demo:1",
                        "arm": "fusion",
                        "session_id": "original",
                        "status": "completed" if daily else "error",
                        "grade": grade,
                        "outer_grade": {"correct": True, "parsed": True},
                    },
                )
                with patch("audit.Future") as factory:
                    client = factory.return_value
                    client.call.return_value = {
                        "model": "future/deepseek-flash",
                        "thinkingLevel": "max",
                        "contextFilesEnabled": False,
                    }
                    client.transcript.return_value = {"entries": []}
                    collect(root, "unused-binary")
                    self.assertEqual(client.call.call_count, 1)
                    self.assertEqual(
                        client.call.call_args.args[0][:2], ["session", "status"]
                    )
                observed = read_json(directory / "result.json")
                self.assertEqual(observed["grade"], grade)
                self.assertEqual(observed["fusion_invocations"], 0)
                if daily:
                    self.assertEqual(observed["routing_outcome"], "not_invoked")
                    self.assertNotIn("error_category", observed)
                else:
                    self.assertEqual(observed["error_category"], "fusion_not_invoked")
                    self.assertFalse(observed["protocol_valid"])
                self.assertTrue((directory / "result-observed.json").exists())

    def test_unexecuted_tool_receipt_does_not_imply_a_completed_consultation(self):
        transcript = {
            "entries": [
                {
                    "blocks": [
                        {
                            "kind": "tool_result",
                            "toolCallId": "truncated",
                            "text": "[Tool execution cancelled — fusion was not executed due to connection interruption (retrying)]",
                        }
                    ]
                }
            ]
        }
        item = unexecuted_consultation({"pending": True}, "truncated", transcript)
        self.assertEqual(item["status"], "not_started")
        self.assertEqual(item["kernel_text"], "")
        self.assertIsNone(item["configuration"])
        result = self.daily_case_with_consultations(1, item)
        self.assertEqual(result["status"], "completed")
        self.assertTrue(result["grade"]["correct"])
        self.assertEqual(result["fusion_invocations"], 1)
        self.assertEqual(result["completed_consultations"], 0)
        self.assertEqual(result["routing_outcome"], "no_conclusion")
        with self.assertRaisesRegex(ValueError, "Missing Fusion record"):
            unexecuted_consultation({"pending": True}, "other-call", transcript)
        with self.assertRaisesRegex(ValueError, "Missing Fusion record"):
            unexecuted_consultation({}, "truncated", transcript)

    def test_audit_keeps_grade_when_a_tool_call_was_cancelled_before_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "r0-demo-1-fusion"
            directory.mkdir()
            (root / "harness").mkdir()
            write_json(root / "manifest.json", {"execution_profile": "daily"})
            grade = {"correct": False, "parsed": False}
            write_json(
                directory / "result.json",
                {
                    "case_id": "demo:1",
                    "arm": "fusion",
                    "session_id": "original",
                    "run_id": "accepted",
                    "status": "error",
                    "error": "'run'",
                    "grade": grade,
                },
            )
            transcript = {
                "entries": [
                    {
                        "blocks": [
                            {
                                "kind": "tool_call",
                                "name": "fusion",
                                "toolCallId": "truncated",
                            },
                            {
                                "kind": "tool_result",
                                "toolCallId": "truncated",
                                "text": "fusion was not executed due to connection interruption",
                            },
                        ]
                    }
                ]
            }

            def response(args, **kwargs):
                if args[:2] == ["session", "status"]:
                    return {
                        "model": "future/deepseek-flash",
                        "thinkingLevel": "max",
                        "contextFilesEnabled": False,
                    }
                if args[:2] == ["fusion", "show"]:
                    return {"pending": True}
                self.fail(f"Unexpected mutation or call: {args}")

            with patch("audit.Future") as factory:
                client = factory.return_value
                client.call.side_effect = response
                client.transcript.return_value = transcript
                collect(root, "unused-binary")
            observed = read_json(directory / "result.json")
            self.assertEqual(observed["grade"], grade)
            self.assertEqual(observed["status"], "error")
            self.assertEqual(observed["error_category"], "evidence_collection")
            self.assertEqual(observed["fusion_invocations"], 1)
            self.assertEqual(observed["completed_consultations"], 0)
            self.assertEqual(observed["consultations"][0]["status"], "not_started")
            self.assertEqual(
                read_json(directory / "result-observed.json")["grade"], grade
            )

    def test_modified_frozen_question_is_rejected_before_generation(self):
        suite = read_json(ROOT / "suites/dev-12.json")
        bundle = read_json(ROOT / "suites/dev-12.cases.json")
        bundle["cases"][0]["question"] += "Altered input"
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "dev-12.json"
            write_json(path, suite)
            write_json(path.with_suffix(".cases.json"), bundle)
            with self.assertRaisesRegex(ValueError, "content mismatch"):
                load_frozen_questions(path, suite)

    def test_grid_reordering_is_not_a_reasoning_error(self):
        case = {
            "scorer": "grid",
            "answer": {
                "header": ["House", "Name"],
                "rows": [["1", "Alice"], ["2", "Bob"]],
            },
        }
        text = (
            'FINAL_ANSWER: {"header":["Name","House"],"rows":[["Bob",2],["Alice",1]]}'
        )
        self.assertTrue(score(case, text)["correct"])
        self.assertFalse(score(case, text.replace('"Bob",2', '"Bob",1'))["correct"])

    def test_unparseable_answer_is_not_recovered_using_the_reference(self):
        result = score(
            {"scorer": "integer", "answer": "42"}, "I think it is 42, possibly 41."
        )
        self.assertFalse(result["correct"])
        self.assertFalse(result["parsed"])

    def test_sampling_does_not_depend_on_answers(self):
        rows = [{"id": str(i), "stratum": str(i % 3), "answer": i} for i in range(15)]
        with patch("datasets.load_cases", return_value=rows):
            first = [row["id"] for row in sample_cases(["fake"], 7, "fixed")]
        changed = [{**row, "answer": "unrelated"} for row in rows]
        with patch("datasets.load_cases", return_value=changed):
            second = [row["id"] for row in sample_cases(["fake"], 7, "fixed")]
        self.assertEqual(first, second)
        self.assertEqual(len(first), len(set(first)))

    def test_idle_reconnect_never_submits_another_generation(self):
        client = Future("future")
        with patch.object(
            client,
            "call",
            side_effect=[
                RuntimeError("Event stream timed out waiting for an event"),
                {"text": "done"},
            ],
        ) as call:
            result = client.wait("session-original", "run-original", 1000)
        self.assertEqual(result["benchmark_wait_reconnects"], 1)
        for attempt in call.call_args_list:
            self.assertEqual(
                attempt.args[0][:5],
                ["session", "wait", "session-original", "--run", "run-original"],
            )

    def test_model_failure_is_not_retried_as_a_transport_reconnect(self):
        client = Future("future")
        with patch.object(
            client, "call", side_effect=RuntimeError("authentication failed")
        ) as call:
            with self.assertRaisesRegex(RuntimeError, "authentication"):
                client.wait("s", "r", 1000)
        self.assertEqual(call.call_count, 1)

    def test_daily_wait_only_reattaches_and_does_not_force_or_abort_the_parent(self):
        client = Future("future")
        responses = [
            RuntimeError("Timed out waiting for run r"),
            {"text": "The ordinary parent response", "finishReason": "answer"},
        ]
        with patch.object(client, "call", side_effect=responses) as call:
            output = client.wait("s", "r", 1000, fusion=True)
        self.assertEqual(output["finishReason"], "answer")
        self.assertEqual(output["benchmark_wait_reconnects"], 1)
        for attempt in call.call_args_list:
            self.assertEqual(
                attempt.args[0][:5], ["session", "wait", "s", "--run", "r"]
            )

    def test_daily_run_preserves_zero_or_multiple_naturally_selected_consultations(
        self,
    ):
        for count in (0, 2):
            with self.subTest(count=count):
                result = self.daily_case_with_consultations(count)
                self.assertEqual(result["status"], "completed")
                self.assertTrue(result["grade"]["correct"])
                self.assertEqual(result["fusion_invocations"], count)
                self.assertEqual(result["completed_consultations"], count)
                self.assertEqual(
                    result["routing_outcome"], "completed" if count else "not_invoked"
                )

    def daily_case_with_consultations(self, count, consultation=None):
        client = Future("unused-binary")

        def response(args, **kwargs):
            if args[:2] == ["session", "new"]:
                return {"sessionId": "daily"}
            if args[0] == "run":
                self.assertNotIn("--system-prompt", args)
                self.assertNotIn("--append-system-prompt", args)
                return {"sessionId": "daily", "runId": "accepted"}
            if args[:2] == ["session", "transcript"]:
                return {"ledger": [], "hasMore": False}
            if args[:2] == ["session", "status"]:
                return {"model": "future/deepseek-flash", "thinkingLevel": "max"}
            if args[:2] == ["session", "set"]:
                return {}
            self.fail(f"Unexpected operation: {args}")

        entries = [
            {
                "blocks": [
                    {"kind": "tool_call", "name": "fusion", "toolCallId": f"call-{i}"}
                    for i in range(count)
                ]
            }
        ]
        consultation = consultation or {
            "status": "completed",
            "kernel_text": "FINAL_ANSWER: 7",
            "usage": {},
            "step_attempt_failure_kinds": {},
            "whole_question_verbatim": True,
            "stage_grades": [],
        }
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(client, "call", side_effect=response),
            patch.object(client, "wait", return_value={"text": "FINAL_ANSWER: 7"}),
            patch.object(client, "transcript", return_value={"entries": entries}),
            patch.object(client, "collect_consultation", return_value=consultation),
        ):
            return client.run_case(
                {
                    "id": "demo:7",
                    "dataset": "demo",
                    "scorer": "integer",
                    "answer": "7",
                    "question": "Return 7",
                },
                "fusion",
                Path(temporary) / "case",
                100,
            )

    def test_unsupported_effort_fails_before_model_execution(self):
        settings = {
            "enabled": True,
            "configuration": {
                "panel": [],
                "moderator": {
                    "model": "future/deepseek-flash",
                    "thinking_level": "max",
                },
            },
        }
        models = {
            "models": {
                "future": [{"id": "deepseek-flash", "thinkingLevels": ["default"]}]
            }
        }
        client = Future("future")
        with patch.object(client, "call", side_effect=[settings, models]) as call:
            with self.assertRaisesRegex(ValueError, "Unsupported"):
                client.preflight()
        self.assertEqual(call.call_count, 2)


class PublicReportTests(unittest.TestCase):
    def fixture(self, root):
        run = root / "run"
        manifest = {
            "suite": {"cases": [{"id": "demo:1", "dataset": "demo"}]},
            "repeats": 1,
            "started_at_utc": "2026-11-04T00:00:00Z",
            "git_commit": "example",
            "configuration": {
                "binary": {"version": "example"},
                "fusion": {"configuration": {}},
                "baseline": {},
            },
            "protocol": "test",
            "data_manifest": [],
        }
        write_json(run / "manifest.json", manifest)
        for arm in ("fusion", "single"):
            result = {
                "case_id": "demo:1",
                "dataset": "demo",
                "arm": arm,
                "status": "completed",
                "grade": {"correct": True, "parsed": True},
                "elapsed_seconds": 1,
                "text": "PRIVATE-ANSWER",
                "session_id": "PRIVATE-SESSION",
                "error": "PRIVATE-PATH",
            }
            write_json(run / f"r0-demo-1-{arm}/result.json", result)
        return run

    def test_public_export_has_no_raw_answers_session_ids_or_errors(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            run = self.fixture(root)
            prefix = root / "public/pilot"
            generate(run, prefix)
            for suffix in (".json", ".csv", ".md"):
                text = prefix.with_suffix(suffix).read_text()
                self.assertNotIn("PRIVATE-", text)
            public = json.loads(prefix.with_suffix(".json").read_text())
            self.assertEqual(public["summary"]["arms"]["fusion"]["correct"], 1)

    def test_invalid_or_missing_results_cannot_be_exported(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            run = self.fixture(root)
            original = (run / "r0-demo-1-single/result.json").read_text()
            (run / "r0-demo-1-single/result.json").unlink()
            with self.assertRaisesRegex(ValueError, "incomplete or invalid"):
                generate(run, root / "public/missing")
            summary = json.loads((run / "summary.json").read_text())
            self.assertEqual(summary["arms"]["single"]["scheduled"], 1)
            self.assertEqual(summary["arms"]["single"]["correct"], 0)
            self.assertFalse((root / "public/missing.json").exists())
            (run / "r0-demo-1-single/result.json").write_text(original)
            (run / "INVALID.md").write_text("invalid setup")
            with self.assertRaisesRegex(ValueError, "incomplete or invalid"):
                generate(run, root / "public/invalid")


if __name__ == "__main__":
    unittest.main()
