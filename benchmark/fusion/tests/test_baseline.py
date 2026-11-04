"""CI-only regressions for baseline provenance and outcome selection."""

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from baseline import freeze_challenge, reference_for, regrade_baseline  # noqa: E402
from datasets import ROOT, digest, load_frozen_questions, read_json, write_json  # noqa: E402
from future_runner import BASE_MODEL, BASE_THINKING, Future, prompt_for  # noqa: E402
from reporting import generate  # noqa: E402
from scoring import score  # noqa: E402


class BaselineTests(unittest.TestCase):
    def test_ten_case_challenge_accepts_eight_wrong_and_two_correct(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = self.fixture(root)
            out = root / "challenge-10.json"
            freeze_challenge(baseline, out, "fixed", wrong_count=8, correct_count=2)
            suite = read_json(out)
            _, cases = load_frozen_questions(out, suite)
            self.assertEqual(len(cases), 10)
            self.assertEqual(len(reference_for(baseline, suite, cases)["records"]), 10)

    def fixture(self, root):
        directory = root / "baseline"
        cases = [
            {
                "id": f"omni:{i}",
                "dataset": "omni",
                "stratum": "1",
                "scorer": "integer",
                "question": f"Return {i}",
                "answer": str(i),
            }
            for i in range(30)
        ]
        suite = {
            "schema": 1,
            "seed": "seed",
            "sources_lock_sha256": digest(read_json(ROOT / "sources.lock.json")),
            "cases": [
                {
                    "id": row["id"],
                    "dataset": row["dataset"],
                    "stratum": row["stratum"],
                    "question_sha256": digest(row["question"]),
                    "answer_sha256": digest(row["answer"]),
                }
                for row in cases
            ],
        }
        write_json(
            directory / "frozen-questions.json",
            {
                "schema": 1,
                "suite_sha256": digest(suite),
                "sources_lock_sha256": suite["sources_lock_sha256"],
                "upstream_files": [],
                "cases": cases,
            },
        )
        harness = directory / "harness"
        harness.mkdir()
        for name in ("scoring.py", "future_runner.py"):
            (harness / name).write_bytes((ROOT / name).read_bytes())
        manifest = {
            "arms": ["single"],
            "repeats": 1,
            "execution_profile": "daily",
            "suite": suite,
            "configuration": {
                "baseline": {"model": BASE_MODEL, "thinking_level": BASE_THINKING},
                "fusion": {"configuration": {}},
                "binary": {},
            },
            "harness_sha256": {
                name: digest((ROOT / name).read_bytes())
                for name in ("scoring.py", "future_runner.py")
            },
            "question_bundle_sha256": digest(
                (directory / "frozen-questions.json").read_bytes()
            ),
            "started_at_utc": "2026-11-04T00:00:00Z",
        }
        write_json(directory / "manifest.json", manifest)
        for i, case in enumerate(cases):
            text = f"FINAL_ANSWER: {i if i < 10 else -1}"
            path = directory / f"r0-omni-{i}-single"
            write_json(
                path / "result.json",
                {
                    "case_id": case["id"],
                    "dataset": "omni",
                    "arm": "single",
                    "status": "completed",
                    "text": text,
                    "grade": score(case, text),
                    "effective_parent_model": BASE_MODEL,
                    "effective_parent_thinking": BASE_THINKING,
                    "elapsed_seconds": 1,
                },
            )
            (path / "prompt.txt").write_text(prompt_for(case), encoding="utf-8")
        return directory

    def test_freezes_exact_cohorts_and_rejects_changed_reference(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = self.fixture(root)
            out = root / "challenge.json"
            freeze_challenge(baseline, out, "fixed")
            suite = read_json(out)
            _, cases = load_frozen_questions(out, suite)
            self.assertEqual(len(cases), 20)
            reference = reference_for(baseline, suite, cases)
            self.assertEqual(len(reference["records"]), 20)
            selected = suite["cases"][0]["id"].split(":")[-1]
            result = baseline / f"r0-omni-{selected}-single/result.json"
            row = read_json(result)
            row["elapsed_seconds"] += 1
            write_json(result, row)
            with self.assertRaisesRegex(ValueError, "result/content changed"):
                reference_for(baseline, suite, cases)

    def test_incomplete_baseline_cannot_freeze(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = self.fixture(root)
            (baseline / "r0-omni-0-single/result.json").unlink()
            with self.assertRaisesRegex(ValueError, "incomplete"):
                freeze_challenge(baseline, root / "challenge.json", "fixed")
            self.assertFalse((root / "challenge.json").exists())

    def test_service_failures_do_not_fill_wrong_answer_quota(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = self.fixture(root)
            for i in range(10, 18):
                path = baseline / f"r0-omni-{i}-single/result.json"
                row = read_json(path)
                row.update(status="error", grade={"correct": False, "parsed": False})
                write_json(path, row)
            with self.assertRaisesRegex(ValueError, "Need 14 wrong"):
                freeze_challenge(baseline, root / "challenge.json", "fixed")

    def test_single_arm_report_can_be_complete_without_fusion(self):
        with tempfile.TemporaryDirectory() as temporary:
            baseline = self.fixture(Path(temporary))
            generate(baseline)
            summary = read_json(baseline / "summary.json")
            self.assertTrue(summary["complete"])
            self.assertEqual(summary["arms"]["single"]["correct"], 10)
            self.assertEqual(summary["arms"]["fusion"]["scheduled"], 0)

    def test_resume_keeps_terminal_failures_without_generating_again(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "case"
            original = {"status": "error", "grade": {"correct": False}}
            write_json(path / "result.json", original)
            client = Future("unused")
            with patch.object(client, "call") as call:
                self.assertEqual(
                    client.run_case({}, "single", path, 0, resume=True), original
                )
            call.assert_not_called()

    def test_resume_with_unknown_submission_does_not_resubmit(self):
        with tempfile.TemporaryDirectory() as temporary:
            client = Future("unused")
            with patch.object(client, "call") as call:
                with self.assertRaisesRegex(ValueError, "never resubmit"):
                    client.run_case({}, "single", Path(temporary), 0, resume=True)
            call.assert_not_called()

    def test_regrading_preserves_original_run_and_does_not_generate(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = self.fixture(root)
            original = (source / "r0-omni-0-single/result.json").read_bytes()
            out = root / "regraded"
            with patch.object(Future, "call") as call:
                regrade_baseline(source, out)
            call.assert_not_called()
            self.assertEqual(
                (source / "r0-omni-0-single/result.json").read_bytes(), original
            )
            self.assertEqual(
                (out / "r0-omni-0-single/result-before-regrading.json").read_bytes(),
                original,
            )
            self.assertEqual(
                read_json(out / "manifest.json")["regrading"]["source_manifest_sha256"],
                digest(read_json(source / "manifest.json")),
            )


if __name__ == "__main__":
    unittest.main()
