"""CI-only checks for selective 502 retries and preserved attempt evidence."""

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from datasets import read_json, write_json  # noqa: E402
from retry import run_single  # noqa: E402


class RecordedClient:
    def __init__(self, outcomes):
        self.outcomes = iter(outcomes)
        self.generated = 0

    def run_case(self, case, arm, directory, timeout, resume=False):
        path = Path(directory) / "result.json"
        if path.exists() and resume:
            return read_json(path)
        self.generated += 1
        result = {
            "case_id": case["id"],
            "arm": arm,
            "elapsed_seconds": 1,
            "parent_usage": {"inputTokens": 5},
            **next(self.outcomes),
        }
        write_json(path, result)
        (Path(directory) / "prompt.txt").write_text(
            "unchanged prompt", encoding="utf-8"
        )
        return result


GOOD = {"status": "completed", "grade": {"correct": True, "parsed": True}}
WRONG = {"status": "completed", "grade": {"correct": False, "parsed": True}}
BAD_GATEWAY = {
    "status": "error",
    "http_status": 502,
    "grade": {"correct": False, "parsed": False},
}
OTHER_ERROR = {"status": "error", "grade": {"correct": False, "parsed": False}}
CASE = {"id": "fixture:7"}


class RetryTests(unittest.TestCase):
    def test_502_recovers_and_preserves_both_attempts_and_usage(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "case"
            client = RecordedClient([BAD_GATEWAY, GOOD])
            result = run_single(client, CASE, path, 30, retries=3, retry_delay=0)
            self.assertTrue(result["grade"]["correct"])
            self.assertEqual(client.generated, 2)
            self.assertEqual(result["attempt_count"], 2)
            self.assertEqual(result["parent_usage"]["inputTokens"], 10)
            self.assertTrue(result["usage_incomplete_possible"])
            self.assertEqual(
                read_json(path / "attempts/attempt-1/result.json")["http_status"], 502
            )
            self.assertEqual(
                read_json(path / "attempts/attempt-2/result.json")["status"],
                "completed",
            )

    def test_wrong_answers_and_other_failures_are_not_retried(self):
        for outcome in (WRONG, OTHER_ERROR):
            with (
                self.subTest(outcome=outcome),
                tempfile.TemporaryDirectory() as temporary,
            ):
                client = RecordedClient([outcome])
                result = run_single(
                    client, CASE, Path(temporary) / "case", 30, retries=3, retry_delay=0
                )
                self.assertEqual(client.generated, 1)
                self.assertEqual(result["attempt_count"], 1)

    def test_retry_limit_is_finite_and_terminal_502_is_retained(self):
        with tempfile.TemporaryDirectory() as temporary:
            client = RecordedClient([BAD_GATEWAY, BAD_GATEWAY, BAD_GATEWAY])
            result = run_single(
                client, CASE, Path(temporary) / "case", 30, retries=2, retry_delay=0
            )
            self.assertEqual(client.generated, 3)
            self.assertEqual(result["http_status"], 502)
            self.assertFalse(result["grade"]["correct"])

    def test_resume_does_not_regenerate_terminal_results(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "case"
            original = run_single(RecordedClient([GOOD]), CASE, path, 30, retry_delay=0)
            client = RecordedClient([])
            self.assertEqual(
                run_single(client, CASE, path, 30, retry_delay=0, resume=True), original
            )
            self.assertEqual(client.generated, 0)

    def test_resume_keeps_an_already_failed_502_attempt(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "case"
            first = {
                "case_id": CASE["id"],
                "arm": "single",
                "elapsed_seconds": 1,
                "parent_usage": {},
                **BAD_GATEWAY,
            }
            write_json(path / "attempts/attempt-1/result.json", first)
            client = RecordedClient([GOOD])
            result = run_single(
                client, CASE, path, 30, retries=1, retry_delay=0, resume=True
            )
            self.assertEqual(client.generated, 1)
            self.assertEqual(result["attempt_count"], 2)
            self.assertEqual(read_json(path / "attempts/attempt-1/result.json"), first)

    def test_unknown_submission_is_not_resubmitted(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "case"
            (path / "attempts/attempt-1").mkdir(parents=True)
            client = RecordedClient([])
            with self.assertRaisesRegex(ValueError, "never resubmit"):
                run_single(client, CASE, path, 30, retry_delay=0, resume=True)
            self.assertEqual(client.generated, 0)


if __name__ == "__main__":
    unittest.main()
