"""Offline regressions; no provider, account, or real session access."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import four_arm_rerun as run
import abc_external_strategies as ext


class BenchmarkTests(unittest.TestCase):
    def test_summary_is_carried_into_codex_next_request(self):
        state = ext.codex_compacted_records([], "PRIOR_SENTINEL")
        self.assertIn("PRIOR_SENTINEL", ext.codex_summary_request(state))
        self.assertIn(ext.CODEX_SUMMARY_PREFIX, ext.plain(state))

    def test_codex_user_budget_is_tokens_not_characters(self):
        user = {"kind": "text", "role": "user", "text": "x" * 70000}
        self.assertIn(user["text"], ext.codex_build([user], "summary"))
        self.assertLessEqual(sum(run.tokens(r["text"]) for r in ext.codex_compacted_records(
            [dict(user, text="x" * 120000)], "summary") if r["kind"] == "text"), 20000)

    def test_opencode_associates_interleaved_parallel_calls(self):
        records = [
            {"kind": "tool_call", "role": "assistant", "call": "a", "position": 1},
            {"kind": "tool_call", "role": "assistant", "call": "b", "position": 1},
            {"kind": "tool_result", "role": "tool", "call": "b", "position": 2, "text": "FAIL", "error": True},
            {"kind": "tool_result", "role": "tool", "call": "a", "position": 3, "text": "ok"},
        ]
        entries = ext.opencode_entries(records)
        self.assertEqual(len(entries), 1)
        self.assertEqual(len(entries[0]["records"]), 4)
        self.assertIn("[Tool error]: FAIL", ext.opencode_serialize(entries[0]))

    def test_common_schedule_covers_every_record(self):
        records = [{"id": str(i), "kind": "text", "role": "user", "text": "x" * 100, "order": i} for i in range(12)]
        ends = run.schedule(records, [4, 8, 12], chunk=50)
        self.assertEqual(ends[-1], 12)
        self.assertTrue(set([4, 8, 12]).issubset(ends))
        self.assertEqual(ends, sorted(set(ends)))

    def test_question_values_are_unique_and_decoys_absent(self):
        records = [{"kind": "text", "role": "assistant", "text": "`src/main.rs` 42 MiB ACK_abcdef"}]
        q = run.questionnaire(records, 0)
        self.assertEqual(len(q["present"]), len(set(q["present"])))
        self.assertTrue(all(v in ext.plain(records) for v in q["present"]))
        self.assertTrue(all(v not in ext.plain(records) for v in q["decoys"]))

    def test_cache_and_budget_admission(self):
        with tempfile.TemporaryDirectory() as tmp:
            calls = run.Calls(Path(tmp), 1, "model", Path("bridge"), "fingerprint")
            with self.assertRaisesRegex(RuntimeError, "BUDGET"):
                calls.execute("x", {}, [], 2, lambda s: ({}, 0))
            self.assertFalse(calls.rows)
            with patch.object(run.subprocess, "run") as process:
                process.return_value.returncode = 0
                process.return_value.stdout = "ok"
                process.return_value.stderr = ""
                first = calls.execute("x", {"prompt": "a"}, ["bridge"], .5, lambda s: ({"text": s}, .1))
                second = calls.execute("x", {"prompt": "a"}, ["bridge"], .5, lambda s: ({"text": s}, .1))
                self.assertEqual(first, second)
                self.assertEqual(process.call_count, 1)
                self.assertAlmostEqual(calls.spent(), .1)

    def test_failed_request_cannot_silently_retry(self):
        with tempfile.TemporaryDirectory() as tmp:
            calls = run.Calls(Path(tmp), 1, "model", Path("bridge"), "fingerprint")
            with patch.object(run.subprocess, "run") as process:
                process.return_value.returncode = 1
                process.return_value.stdout = ""
                process.return_value.stderr = "failed"
                for _ in range(2):
                    with self.assertRaises(RuntimeError):
                        calls.execute("x", {}, ["bridge"], .5, lambda s: ({}, 0))
                self.assertEqual(process.call_count, 1)
                self.assertEqual(calls.spent(), .5)

    def test_common_archive_byte_budget_and_no_future_records(self):
        import four_arm_open as opened
        records = [{"id": "u", "kind": "text", "role": "user", "text": "中文" * 5000}]
        text = opened.archive_call(records, "archive_read", {"id": "0"}, 512)
        self.assertLessEqual(len(text.encode()), 512)
        self.assertIn("content", json.loads(text))
        result = opened.archive_call(records, "archive_search", {"query": "FUTURE_ONLY"}, 512)
        self.assertEqual(json.loads(result)["matches"], [])
        self.assertEqual(opened.archive_call(records, "archive_read", {"id": "0"}, 0), "")

    def test_open_turns_are_not_tool_invocations(self):
        import four_arm_open as opened
        class Fake:
            def model_call(self, *args, **kwargs):
                return {"text": '{"appeared": ["fact"]}', "calls": [], "finish": "stop"}
        q = {"present": ["fact"], "decoys": ["not-fact"], "prompt": "which?"}
        result = opened.open_probe(Fake(), "id", {"text": "fact"}, q, [])
        self.assertEqual(result["model_turns"], 1)
        self.assertEqual(result["tool_invocations"], 0)
        self.assertEqual(result["returned_bytes"], 0)
        self.assertEqual(result["hits"], 1)

    def test_immutable_inputs_reject_mixed_versions(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)/"projection.json"
            run.immutable(p, {"text": "old"})
            with self.assertRaisesRegex(RuntimeError, "mismatch"):
                run.immutable(p, {"text": "new"})
            self.assertEqual(json.loads(p.read_text()), {"text": "old"})


if __name__ == "__main__":
    unittest.main()
