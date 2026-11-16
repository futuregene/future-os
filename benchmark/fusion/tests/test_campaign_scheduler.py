"""CI-only checks for bounded rolling campaign scheduling."""

import sys
import threading
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from campaign import case_outcome, case_status, run_window  # noqa: E402


class CampaignSchedulerTests(unittest.TestCase):
    def test_ambiguous_failure_is_retained_ungraded_without_replay(self):
        result = {
            "status": "error",
            "submission_uncertain": True,
            "error": "connection lost after submission",
            "grade": {"correct": False, "parsed": False},
        }
        self.assertEqual(case_status(result), "needs_review")
        outcome = case_outcome([result], 4)
        self.assertFalse(outcome["graded"])
        self.assertFalse(outcome["retry_allowed"])
        self.assertIsNone(outcome["passed"])

    def test_ready_makeup_uses_a_slot_before_a_slow_peer_finishes(self):
        slow_started = threading.Event()
        retry_started = threading.Event()
        lock = threading.Lock()
        events = []
        active = 0
        peak = 0

        def worker(item, lane):
            nonlocal active, peak
            _, name, attempt = item
            with lock:
                active += 1
                peak = max(peak, active)
                events.append((name, attempt, "start"))
            try:
                if name == "slow":
                    slow_started.set()
                    self.assertTrue(retry_started.wait(10))
                elif attempt == 1:
                    self.assertTrue(slow_started.wait(10))
                else:
                    retry_started.set()
                return item, "technical" if name == "fast" and attempt == 1 else "done"
            finally:
                with lock:
                    events.append((name, attempt, "end"))
                    active -= 1

        def record(item, outcome):
            return (item[0], item[1], item[2] + 1) if outcome == "technical" else None

        run_window([("g", "slow", 1), ("g", "fast", 1)], 2, None, {}, worker, record)
        starts = [event for event in events if event[2] == "start"]
        self.assertEqual(
            set(starts),
            {("slow", 1, "start"), ("fast", 1, "start"), ("fast", 2, "start")},
        )
        self.assertEqual(len(starts), 3)
        self.assertLess(
            events.index(("fast", 2, "start")), events.index(("slow", 1, "end"))
        )
        self.assertLessEqual(peak, 2)


if __name__ == "__main__":
    unittest.main()
