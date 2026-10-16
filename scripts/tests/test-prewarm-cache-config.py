"""Offline regression tests for scripts/ci/check-prewarm-cache.py.

The guard decides whether the caches `ci.yml` reads are written by
`.github/workflows/prewarm.yml` with matching keys, so its own failure modes are
expensive: a checker that stopped detecting drift would let every pull-request
run go cold silently, and one that over-reports would block every PR.

Each case copies the real workflows into a throwaway directory, applies one
mutation, and asserts the *exit code* of the real script (not just an internal
return value), including the unchanged-file positive control.
"""

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ci" / "check-prewarm-cache.py"
WORKFLOWS = (".github/workflows/ci.yml", ".github/workflows/prewarm.yml")


class PrewarmCacheGuard(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="prewarm-cache-guard-"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        for relative in WORKFLOWS:
            target = self.tmp / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / relative, target)

    def run_guard(self) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.tmp)],
            capture_output=True,
            text=True,
            check=False,
        )

    def mutate(self, relative: str, old: str, new: str) -> None:
        path = self.tmp / relative
        content = path.read_text(encoding="utf-8")
        self.assertIn(old, content, f"fixture no longer contains {old!r} — update this test")
        path.write_text(content.replace(old, new, 1), encoding="utf-8")

    def assertPasses(self, result: subprocess.CompletedProcess) -> None:
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def assertFails(self, result: subprocess.CompletedProcess, needle: str) -> None:
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(needle, result.stdout + result.stderr)

    def test_unmodified_workflows_pass(self) -> None:
        """Positive control: the guard must be silent on the real files."""
        self.assertPasses(self.run_guard())

    def test_a_drifted_workspaces_value_fails(self) -> None:
        """The input the prewarm must copy verbatim, or the key hash differs."""
        self.mutate(
            ".github/workflows/prewarm.yml",
            "          workspaces: .\n",
            "          workspaces: |\n            .\n            desktop/src-tauri\n",
        )
        self.assertFails(self.run_guard(), "key 'rust-tests-workspace': cache inputs differ")

    def test_a_pr_writable_cache_step_fails(self) -> None:
        """A PR-scoped save is invisible to every other run and evicts shared
        entries, which is what `save-if` prevents."""
        path = self.tmp / ".github/workflows/ci.yml"
        content = path.read_text(encoding="utf-8")
        content = content.replace("          save-if: ${{ github.ref == 'refs/heads/main' }}\n", "", 1)
        path.write_text(content, encoding="utf-8")
        self.assertFails(self.run_guard(), "`save-if` is '<unset>'")

    def test_a_reader_without_a_writer_fails(self) -> None:
        """Renaming only the ci.yml key leaves the entry unwritten."""
        self.mutate(".github/workflows/ci.yml", "shared-key: rust-quality", "shared-key: rust-lint")
        self.assertFails(self.run_guard(), "written by no prewarm job")

    def test_a_writer_without_a_reader_fails(self) -> None:
        """An orphan prewarm key only consumes the repository cache quota."""
        self.mutate(".github/workflows/prewarm.yml", "shared-key: rust-quality", "shared-key: rust-lint")
        self.assertFails(self.run_guard(), "read by no .github/workflows/ci.yml job")

    def test_a_test_group_the_prewarm_does_not_run_fails(self) -> None:
        """A new crate in a ci.yml group must be added to the prewarm, or the
        group restores an entry that cannot cover it."""
        self.mutate(
            ".github/workflows/ci.yml",
            "          - name: agent\n            filter: rust_agent\n            packages: -p future-agent\n",
            "          - name: agent\n            filter: rust_agent\n            packages: -p future-agent -p future-newcrate\n",
        )
        self.assertFails(self.run_guard(), "runs commands the prewarm does not")

    def test_a_prewarm_only_command_fails(self) -> None:
        """Caching artifacts no reader runs is wasted quota and time."""
        self.mutate(
            ".github/workflows/prewarm.yml",
            "          cargo test -p future-agent\n",
            "          cargo test -p future-agent\n          cargo test -p future-nowhere\n",
        )
        self.assertFails(self.run_guard(), "caches artifacts nothing reads")

    def test_an_os_writer_missing_a_command_fails(self) -> None:
        """A key holds one entry per OS, so every prewarm job writing it — the
        Windows and macOS build jobs included — must run the same commands."""
        self.mutate(
            ".github/workflows/prewarm.yml",
            "      - name: Embedded Agent native lifecycle\n        run: python scripts/ci/check-embedded-agent.py\n\n  build-windows:",
            "  build-windows:",
        )
        self.assertFails(self.run_guard(), "run different commands")

    def test_a_command_only_one_os_job_runs_fails(self) -> None:
        """Sibling writers diverging is the same defect in reverse."""
        self.mutate(
            ".github/workflows/prewarm.yml",
            "      - name: cargo check (workspace)\n        run: cargo check --workspace --manifest-path Cargo.toml\n\n      - name: cargo check (desktop Tauri backend)\n        run: cargo check --manifest-path desktop/src-tauri/Cargo.toml\n\n      - name: cargo check (desktop Tauri backend, no default features)\n        run: cargo check --no-default-features --features headless --manifest-path desktop/src-tauri/Cargo.toml\n\n      - name: Embedded Agent native lifecycle\n        run: python scripts/ci/check-embedded-agent.py\n\n  build-macos:",
            "      - name: Embedded Agent native lifecycle\n        run: python scripts/ci/check-embedded-agent.py\n\n  build-macos:",
        )
        self.assertFails(self.run_guard(), "run different commands")

    def test_a_cache_step_without_shared_key_fails(self) -> None:
        """Without an explicit key the entry is job-id bound and unreachable."""
        self.mutate(".github/workflows/ci.yml", "          shared-key: rust-build\n", "")
        self.assertFails(self.run_guard(), "has no `shared-key`")

    def test_a_structural_change_fails_instead_of_passing_silently(self) -> None:
        """A file the parser cannot read must report, never pass vacuously."""
        path = self.tmp / ".github/workflows/prewarm.yml"
        path.write_text("name: Cache prewarm\non: push\n", encoding="utf-8")
        self.assertFails(self.run_guard(), "no jobs found")

    def test_a_missing_workflow_fails(self) -> None:
        (self.tmp / ".github/workflows/prewarm.yml").unlink()
        self.assertFails(self.run_guard(), "is missing")


if __name__ == "__main__":
    unittest.main(verbosity=2)
