"""Offline regression tests for scripts/check-docs.py.

Covers the three defects fixed after the documentation reorg landed:

1. `--scope` filtered findings by their printed text, so a bilingual-pairing
   finding (whose text starts with "bilingual pair missing:", not with a path)
   was dropped and a scoped run passed when it should have failed.
2. `--strict-pending` only checked the debt list's internal hygiene, so a file
   registered as pending with no counterpart still reported success.
3. Pair scope was an allowlist of known directories, so a new docs/
   subdirectory (docs/verification/ was the real case) escaped the bilingual
   requirement entirely.

The end-to-end cases run the real script in a throwaway git repository so exit
codes, not just return values, are asserted.
"""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "check-docs.py"

spec = importlib.util.spec_from_file_location("check_docs", SCRIPT)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

# Every whitelisted path must exist, or the checker reports it missing.
WHITELISTED_FILES = [
    "CLAUDE.md",
    "README.md",
    "README.zh-CN.md",
    "SECURITY.md",
    "SECURITY.zh-CN.md",
    "THIRD_PARTY_NOTICES.md",
    "THIRD_PARTY_NOTICES.zh-CN.md",
    "desktop/CLAUDE.md",
    "orchestration/loop/UPSTREAM.md",
    "orchestration/loop/UPSTREAM.zh-CN.md",
    "scripts/compaction_experiment/README.md",
    "scripts/compaction_experiment/CLOSED_BOOK_PROTOCOL.md",
    "scripts/compaction_experiment/OPEN_BOOK_PROTOCOL.md",
    "docs/README.md",
    "docs/README.zh-CN.md",
]


def build_tree(extra):
    """Create a minimal valid docs tree plus `extra` {path: content}."""
    root = Path(tempfile.mkdtemp(prefix="check-docs-"))
    (root / "scripts").mkdir(parents=True)
    (root / "scripts" / "check-docs.py").write_text(SCRIPT.read_text())
    files = {path: "" for path in WHITELISTED_FILES}
    files.update(extra)
    for rel, content in files.items():
        target = root / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
    for command in (
        ["git", "init", "-q"],
        ["git", "add", "-A"],
        ["git", "-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "fixture"],
    ):
        subprocess.run(command, cwd=root, check=True)
    return root


def run(root, *args):
    return subprocess.run(
        ["python3", "scripts/check-docs.py", *args],
        cwd=root, capture_output=True, text=True,
    )


class ScopedFilteringTests(unittest.TestCase):
    """Defect 1: --scope must not hide findings based on their wording."""

    def test_scoped_keeps_an_in_scope_bilingual_finding(self):
        item = checker.diagnostic(
            "docs/guide/probe.md",
            "bilingual pair missing: docs/guide/probe.md (expected docs/guide/probe.zh-CN.md)",
        )
        self.assertEqual([str(x) for x in checker.scoped([item], ["docs/guide"])], [str(item)])

    def test_scoped_uses_the_affected_path_not_the_message(self):
        inside = checker.diagnostic("docs/guide/a.md", "some wording with no path prefix")
        outside = checker.diagnostic("docs/architecture/b.md", "some wording with no path prefix")
        kept = checker.scoped([inside, outside], ["docs/guide"])
        self.assertEqual([x.path for x in kept], ["docs/guide/a.md"])

    def test_scoped_accepts_a_file_prefix_and_rejects_a_sibling(self):
        item = checker.diagnostic("docs/guide.md", "x")
        sibling = checker.diagnostic("docs/guide-extra/a.md", "x")
        self.assertEqual([x.path for x in checker.scoped([item], ["docs/guide"])], [])
        self.assertEqual([x.path for x in checker.scoped([sibling], ["docs/guide"])], [])

    def test_no_scope_returns_everything(self):
        items = [checker.diagnostic("docs/a.md", "x"), checker.diagnostic("docs/b.md", "y")]
        self.assertEqual(checker.scoped(items, []), items)

    def test_end_to_end_scoped_run_reports_a_missing_pair(self):
        root = build_tree({"docs/guide/only-en.md": "# only\n"})
        result = run(root, "--scope", "docs/guide")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("docs/guide/only-en.md", result.stderr)


class StrictPendingTests(unittest.TestCase):
    """Defect 2: --strict-pending must mean 'bilingualization is complete'."""

    def test_strict_fails_while_debt_remains(self):
        docs = ["docs/guide/x.md"]
        original = checker.BILINGUAL_PENDING
        try:
            checker.BILINGUAL_PENDING = {"docs/guide/x.md"}
            errors, _ = checker.check_bilingual_pairs(docs, strict_pending=True)
            self.assertTrue(any("bilingual debt remains" in str(e) for e in errors), errors)
        finally:
            checker.BILINGUAL_PENDING = original

    def test_non_strict_run_reports_no_error_but_warns_on_stale_entries(self):
        docs = ["docs/guide/x.md", "docs/guide/x.zh-CN.md"]
        original = checker.BILINGUAL_PENDING
        try:
            checker.BILINGUAL_PENDING = {"docs/guide/x.md"}
            errors, warnings = checker.check_bilingual_pairs(docs, strict_pending=False)
            self.assertEqual(errors, [])
            self.assertTrue(any("stale" in str(w) for w in warnings), warnings)
        finally:
            checker.BILINGUAL_PENDING = original

    def test_strict_fails_on_an_entry_that_no_longer_exists(self):
        original = checker.BILINGUAL_PENDING
        try:
            checker.BILINGUAL_PENDING = {"docs/guide/gone.md"}
            errors, _ = checker.check_bilingual_pairs([], strict_pending=True)
            self.assertTrue(any("does not exist" in str(e) for e in errors), errors)
        finally:
            checker.BILINGUAL_PENDING = original

    def test_end_to_end_strict_run_fails_on_debt(self):
        root = build_tree({"docs/guide/only-en.md": "# only\n"})
        script = root / "scripts" / "check-docs.py"
        text = script.read_text().replace(
            "BILINGUAL_PENDING = {",
            'BILINGUAL_PENDING = {\n    "docs/guide/only-en.md",',
            1,
        )
        script.write_text(text)
        subprocess.run(["git", "add", "-A"], cwd=root, check=True)
        normal = run(root)
        strict = run(root, "--strict-pending")
        self.assertEqual(normal.returncode, 0, normal.stdout + normal.stderr)
        self.assertEqual(strict.returncode, 1, strict.stdout + strict.stderr)
        self.assertIn("bilingual debt remains", strict.stderr)


class PairScopeTests(unittest.TestCase):
    """Defect 3: any docs/ subdirectory must be pair-checked."""

    def test_new_and_existing_subdirectories_are_in_scope(self):
        for rel in (
            "docs/guide/a.md",
            "docs/architecture/a.md",
            "docs/internals/a.md",
            "docs/archives/a.md",
            "docs/audits/a.md",
            "docs/verification/a.md",
            "docs/brand-new-directory/a.md",
            "docs/README.md",
        ):
            with self.subTest(rel=rel):
                self.assertTrue(checker.in_pair_scope(rel))

    def test_wiki_and_dist_pair_by_their_own_rules(self):
        self.assertFalse(checker.in_pair_scope("docs/wiki/en/Home.md"))
        self.assertFalse(checker.in_pair_scope("docs/dist/readme-macos.md"))

    def test_non_markdown_and_root_paths_are_out_of_scope(self):
        self.assertFalse(checker.in_pair_scope("docs/dist/readme-macos.txt"))
        self.assertFalse(checker.in_pair_scope("README.md"))
        self.assertFalse(checker.in_pair_scope("scripts/check-docs.py"))

    def test_extra_paths_are_scoped_in_both_directions(self):
        self.assertTrue(checker.in_pair_scope("SECURITY.md"))
        self.assertTrue(checker.in_pair_scope("SECURITY.zh-CN.md"))
        self.assertTrue(checker.in_pair_scope("orchestration/loop/UPSTREAM.md"))
        self.assertTrue(checker.in_pair_scope("orchestration/loop/UPSTREAM.zh-CN.md"))

    def test_md_pair_round_trips(self):
        for rel in ("docs/guide/a.md", "orchestration/loop/UPSTREAM.zh-CN.md"):
            self.assertEqual(checker.md_pair(checker.md_pair(rel)), rel)

    def test_end_to_end_new_directory_gap_is_reported(self):
        root = build_tree({"docs/verification/report.md": "# report\n"})
        result = run(root)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("docs/verification/report.md", result.stderr)


class NegativeControlTests(unittest.TestCase):
    """A checker that never fails is worthless: prove it still catches regressions."""

    def test_clean_fixture_passes(self):
        result = run(build_tree({}))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_stray_markdown_outside_docs_fails(self):
        result = run(build_tree({"stray.md": "# stray\n"}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("outside docs/", result.stderr)

    def test_broken_local_link_fails(self):
        result = run(build_tree({"docs/guide/a.md": "# a\n\n[dead](missing.md)\n",
                                 "docs/guide/a.zh-CN.md": "# a\n"}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing local target", result.stderr)

    def test_unpaired_wiki_page_fails(self):
        result = run(build_tree({"docs/wiki/en/Only.md": "# only\n"}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("wiki bilingual page missing", result.stderr)

    def test_missing_english_packaging_readme_fails(self):
        result = run(build_tree({"docs/dist/readme-linux.txt": "zh\n"}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("packaging bilingual readme missing", result.stderr)

    def test_removing_a_pair_fails(self):
        result = run(build_tree({"docs/guide/a.md": "# a\n"}))
        self.assertEqual(result.returncode, 1)
        self.assertIn("bilingual pair missing", result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
