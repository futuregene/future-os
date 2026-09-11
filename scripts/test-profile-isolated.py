"""Offline subprocess tests; never starts an agent or reads user credentials."""
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

SCRIPT = Path(__file__).with_name("profile-isolated.py")


class ProfileIsolationTests(unittest.TestCase):
    def test_child_uses_disposable_home_and_preserves_failure_exit(self):
        env = os.environ.copy()
        env.pop("FUTURE_PROFILE_HOME", None)
        env.update(FUTURE_AGENT_SOCKET="do-not-use", FUTURE_AGENT_GRPC_ADDR="do-not-use", XDG_RUNTIME_DIR="do-not-use")
        code = "import os,json,sys; print(json.dumps(dict(os.environ))); sys.exit(7)"
        result = subprocess.run([sys.executable, str(SCRIPT), sys.executable, "-c", code], env=env, text=True, capture_output=True)
        self.assertEqual(result.returncode, 7, result.stderr)
        child = json.loads(result.stdout.splitlines()[-1])
        self.assertNotEqual(child["HOME"], env.get("HOME"))
        self.assertEqual(child["HOME"], child["USERPROFILE"])
        self.assertFalse(Path(child["HOME"]).exists())
        for key in ("FUTURE_AGENT_SOCKET", "FUTURE_AGENT_GRPC_ADDR", "XDG_RUNTIME_DIR"):
            self.assertNotIn(key, child)

    def test_rejects_real_home_override(self):
        env = {**os.environ, "FUTURE_PROFILE_HOME": str(Path.home())}
        result = subprocess.run([sys.executable, str(SCRIPT), "must-not-execute"], env=env, text=True, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("separate profiling home", result.stderr)


if __name__ == "__main__":
    unittest.main()
