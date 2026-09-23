#!/usr/bin/env python3
"""Check repository documentation: placement, bilingual pairing, links,
wiki targets and fences.

Run from any directory: python3 scripts/check-docs.py
Uses git's file inventory (including new, non-ignored docs), not the skills
submodule's third-party references. Does not fetch remote URLs or check anchors.
This is a structural check, not a substitute for source-based factual review.

Rules
1. Placement: every .md/.mdx/.rst file must live under docs/, except the
   root-level files in WHITELIST (repo entry points and legal files, plus
   desktop/CLAUDE.md). The only documentation .txt files are the
   release-package readmes under docs/dist/ (path is frozen: the signed and
   portable release workflows copy these files verbatim).
2. Bilingual pairing. Every .md under docs/ needs both languages, plus the
   extra paths in EXTRA_PAIR_SCOPED:
   - default rule: name.md <-> name.zh-CN.md.
   - docs/wiki/: en/ and zh/ must contain the same basenames.
   - docs/dist/: each readme needs its -en counterpart and vice versa.
   The rule is "all of docs/" rather than a list of known directories, so a
   newly added docs/ subdirectory inherits the bilingual requirement instead
   of silently escaping it.
   Files whose pair is still being written are listed in BILINGUAL_PENDING.
   That list is temporary debt: an entry is a warning in a normal run and a
   failure under --strict-pending, which additionally requires the list to be
   empty — i.e. bilingualization fully complete.
3. Structure: local markdown links resolve, wiki [[...]] targets resolve,
   code fences are closed (kept from the original checker).

Each finding carries the path it is *about* (`Diagnostic.path`), so --scope
filters by affected file. Filtering on the printed message instead would drop
findings whose text does not begin with a path (bilingual-pairing diagnostics
did), silently passing a scoped run that should have failed.
"""

from pathlib import Path
import argparse
import re
import subprocess
import sys
from urllib.parse import unquote, urlsplit


ROOT = Path(__file__).resolve().parent.parent
LINK = re.compile(r"!?\[[^\]\n]*\]\((<[^>\n]+>|[^)\n]+)\)")
WIKI = re.compile(r"\[\[([^]\n]+)\]\]")
FENCE = re.compile(r"^\s*(?:>\s*)*(`{3,}|~{3,})(.*)$")
INLINE_CODE = re.compile(r"(`+).*?\1")


class Diagnostic:
    """One finding: `path` is the affected repo-relative file (used by
    --scope), `message` is the line to print."""

    __slots__ = ("path", "message")

    def __init__(self, path, message):
        self.path = path
        self.message = message

    def __str__(self):
        return self.message

    def __repr__(self):
        return f"Diagnostic({self.path!r}, {self.message!r})"


def diagnostic(path, message):
    return Diagnostic(path, message)


# Files allowed to live outside docs/ (repo-root level, plus desktop/CLAUDE.md).
WHITELIST = {
    "CLAUDE.md",
    "README.md",
    "README.zh-CN.md",
    "SECURITY.md",
    "SECURITY.zh-CN.md",
    "THIRD_PARTY_NOTICES.md",
    "THIRD_PARTY_NOTICES.zh-CN.md",
    "desktop/CLAUDE.md",
    # Release-pipeline dependency: build-macos-signed.yml, build-windows-signed.yml
    # and build-linux.yaml copy this provenance notice into the shipped
    # licenses/loop/ directory. It cannot move under docs/ without editing those
    # workflows, so it is whitelisted for placement only — bilingual pairing is
    # still enforced through EXTRA_PAIR_SCOPED below.
    "orchestration/loop/UPSTREAM.md",
    "orchestration/loop/UPSTREAM.zh-CN.md",
    # Experiment companion docs. They sit beside the scripts they describe because a reader
    # reproducing a run needs the commands and the flags together with the code, and they
    # must move with it. The findings themselves are under docs/internals/compaction/.
    "scripts/compaction_experiment/README.md",
    "scripts/compaction_experiment/CLOSED_BOOK_PROTOCOL.md",
    "scripts/compaction_experiment/OPEN_BOOK_PROTOCOL.md",
}

# Bilingual pairing also applies to these paths outside docs/.
EXTRA_PAIR_SCOPED = {
    "orchestration/loop/UPSTREAM.md",
    # Root documents that stay at the repo root by convention (GitHub surfaces,
    # legal notices) but are still user-facing documents and must be bilingual.
    # CLAUDE.md / desktop/CLAUDE.md are deliberately excluded: they are
    # agent-harness instruction files, not documentation read by users.
    "SECURITY.md",
    "THIRD_PARTY_NOTICES.md",
}

# docs/ subtrees that enforce bilingual coverage by a different rule and are
# therefore excluded from the name.md <-> name.zh-CN.md check.
PAIR_BY_OTHER_RULE = ("docs/wiki/", "docs/dist/")

# Docs whose missing language pair is scheduled for the bilingualization PR.
# Each entry is the path of the existing file (relative to the repo root,
# POSIX separators). Adding a pair without removing the entry is an error.
BILINGUAL_PENDING = {
    # The auto-approval design is intentionally being reviewed in Chinese
    # before its English translation is produced.
    "docs/internals/desktop/AUTO_APPROVAL.zh-CN.md",
}


def prose_lines(text):
    """Yield line numbers/prose, plus a final unclosed-fence diagnostic if any."""
    text = re.sub(
        r"<!--.*?-->", lambda match: "\n" * match[0].count("\n"), text, flags=re.S
    )
    opened = None
    for number, line in enumerate(text.splitlines(), 1):
        match = FENCE.match(line)
        if match:
            marker, suffix = match.groups()
            if opened is None:
                opened = (marker[0], len(marker), number)
            elif marker[0] == opened[0] and len(marker) >= opened[1] and not suffix.strip():
                opened = None
            continue
        if opened is None:
            yield number, INLINE_CODE.sub("", line), None
    if opened:
        yield opened[2], "", "unclosed code fence"


def load_submodule_paths(root):
    """Directories declared as git submodules — their contents are not in the
    index and may not be checked out, so link existence cannot be verified."""
    gitmodules = root / ".gitmodules"
    if not gitmodules.is_file():
        return []
    paths = []
    for line in gitmodules.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line.startswith("path = "):
            paths.append(root / Path(line[len("path = "):]))
    return paths


def in_submodule(destination, submodule_paths):
    return any(
        destination == sub or sub in destination.parents
        for sub in submodule_paths
    )


def check_file(root, relative, submodule_paths):
    errors = []
    path = root / relative
    is_wiki = Path(relative).parts[:2] == ("docs", "wiki")
    for number, line, error in prose_lines(path.read_text(encoding="utf-8")):
        if error:
            errors.append(diagnostic(relative, f"{relative}:{number}: {error}"))
        for match in LINK.finditer(line):
            target = match[1].strip()
            if target.startswith("<"):
                target = target[1:-1]
            else:
                target = re.split(r'\s+[\"\']', target, maxsplit=1)[0]
            parsed = urlsplit(target)
            if parsed.scheme or parsed.netloc or not parsed.path:
                continue
            destination = (path.parent / unquote(parsed.path)).resolve()
            if not destination.exists():
                if in_submodule(destination, submodule_paths):
                    continue
                errors.append(
                    diagnostic(relative, f"{relative}:{number}: missing local target {target}")
                )
        if is_wiki:
            for match in WIKI.finditer(line):
                target = match[1].rsplit("|", 1)[-1].split("#", 1)[0]
                if not target or urlsplit(target).scheme:
                    continue
                destination = path.parent / target
                if not destination.suffix:
                    destination = destination.with_suffix(".md")
                if not destination.is_file():
                    errors.append(
                        diagnostic(relative, f"{relative}:{number}: missing wiki page {target}")
                    )
    return errors


def md_pair(relative):
    """The bilingual counterpart path of a .md doc, or None if not applicable."""
    if relative.endswith(".zh-CN.md"):
        return relative[: -len(".zh-CN.md")] + ".md"
    return relative.replace(".md", ".zh-CN.md", 1)


def in_pair_scope(relative):
    """Does the name.md <-> name.zh-CN.md rule apply to this file?

    Deliberately "all of docs/" rather than an allowlist of directories: a new
    docs/ subdirectory must inherit the requirement. An allowlist let
    docs/verification/ escape the check entirely.
    """
    if not relative.endswith(".md"):
        return False
    if relative in EXTRA_PAIR_SCOPED or md_pair(relative) in EXTRA_PAIR_SCOPED:
        return True
    if not relative.startswith("docs/"):
        return False
    return not relative.startswith(PAIR_BY_OTHER_RULE)


def check_packaging_pairs(docs):
    errors = []
    seen = set()
    for rel in docs:
        if not rel.startswith("docs/dist/") or not rel.endswith(".txt"):
            continue
        seen.add(rel)
        name = Path(rel).name
        if name.endswith("-en.txt"):
            pair = Path(rel).with_name(name[: -len("-en.txt")] + ".txt")
        else:
            pair = Path(rel).with_name(Path(name).stem + "-en.txt")
        if not (ROOT / pair).is_file():
            errors.append(
                diagnostic(pair.as_posix(), f"packaging bilingual readme missing: {pair.as_posix()}")
            )
    return errors


def check_bilingual_pairs(docs, strict_pending=False):
    errors = []
    warnings = []
    existing = {rel for rel in docs if rel.endswith(".md")}
    for rel in sorted(existing):
        if not in_pair_scope(rel):
            continue
        pair = md_pair(rel)
        if pair in existing:
            continue
        if rel in BILINGUAL_PENDING:
            continue  # debt recorded below (warning, or failure under --strict-pending)
        errors.append(
            diagnostic(rel, f"bilingual pair missing: {rel} (expected {pair})")
        )
    for rel in sorted(BILINGUAL_PENDING):
        # Debt-list hygiene: reported as a warning during the bilingualization
        # work (several workers land pairs concurrently, so the hand-owned list
        # is transiently ahead of the tree) and enforced as a hard failure by
        # the final acceptance run.
        bucket = errors if strict_pending else warnings
        if rel not in existing:
            bucket.append(
                diagnostic(rel, f"BILINGUAL_PENDING entry does not exist: {rel}")
            )
        elif md_pair(rel) in existing:
            bucket.append(
                diagnostic(
                    rel,
                    f"BILINGUAL_PENDING entry is stale — pair landed, remove it: {rel}",
                )
            )
    if strict_pending:
        # --strict-pending means "this repository's documentation is complete":
        # any outstanding debt is a failure even though the list is self-
        # consistent. Without this, a scoped list could hold a missing pair
        # indefinitely and still report success.
        for rel in sorted(BILINGUAL_PENDING):
            if rel in existing and md_pair(rel) not in existing:
                errors.append(
                    diagnostic(
                        rel,
                        f"bilingual debt remains: {rel} still has no {md_pair(rel)}",
                    )
                )
    english = {p.name for p in (ROOT / "docs/wiki/en").glob("*.md")}
    chinese = {p.name for p in (ROOT / "docs/wiki/zh").glob("*.md")}
    for name in sorted(english ^ chinese):
        page = f"docs/wiki/{'en' if name in english else 'zh'}/{name}"
        errors.append(diagnostic(page, f"wiki bilingual page missing: {page}"))
    return errors, warnings


def scoped(diagnostics, scope):
    """Keep only findings about files inside `scope` path prefixes.

    Concurrent workers each own a slice of the tree. A global verdict would
    fail every one of them for a peer's in-flight state, so slices validate
    with --scope while the unscoped run stays authoritative for final
    acceptance. Filtering uses the finding's affected path, not its printed
    text.
    """
    if not scope:
        return diagnostics

    def in_scope(path):
        return any(
            path == prefix or path.startswith(prefix.rstrip("/") + "/")
            for prefix in scope
        )

    return [item for item in diagnostics if in_scope(item.path)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--scope",
        help="comma-separated path prefixes; report only diagnostics within them",
    )
    parser.add_argument(
        "--strict-pending",
        action="store_true",
        help=(
            "require complete bilingual coverage: BILINGUAL_PENDING must be "
            "empty and every entry valid (final-acceptance mode)"
        ),
    )
    args = parser.parse_args()
    scope = [part for part in (args.scope or "").split(",") if part]
    inventory = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=ROOT,
    ).decode().split("\0")
    docs = sorted({
        name for name in inventory
        if name.endswith((".md", ".mdx", ".rst"))
        or (name.startswith("docs/dist/") and name.endswith(".txt"))
        or (name.startswith("docs/") and name.endswith(".txt"))
    })
    errors = []
    for rel in docs:
        if rel.endswith(".txt"):
            if rel.startswith("docs/") and not rel.startswith("docs/dist/"):
                errors.append(
                    diagnostic(rel, f"{rel}: documentation .txt outside docs/dist/")
                )
            continue
        if not rel.startswith("docs/"):
            if rel in WHITELIST:
                continue
            errors.append(
                diagnostic(rel, f"{rel}: markdown file outside docs/ (not whitelisted)")
            )
    for rel in sorted(WHITELIST):
        if not (ROOT / rel).is_file():
            errors.append(diagnostic(rel, f"whitelist entry does not exist: {rel}"))
    submodule_paths = load_submodule_paths(ROOT)
    for rel in docs:
        if (ROOT / rel).is_file():
            errors.extend(check_file(ROOT, rel, submodule_paths))
    bilingual_errors, warnings = check_bilingual_pairs(docs, args.strict_pending)
    errors.extend(bilingual_errors)
    errors.extend(check_packaging_pairs(docs))
    errors = scoped(errors, scope)
    warnings = scoped(warnings, scope)
    if warnings:
        print("warnings:", file=sys.stderr)
        print("\n".join(str(item) for item in warnings), file=sys.stderr)
    if errors:
        print("\n".join(str(item) for item in errors), file=sys.stderr)
        return 1
    if scope:
        print(
            f"Checked {len(docs)} docs (scope: {','.join(scope)}): no diagnostics in "
            "the requested slice."
        )
        return 0
    print(
        f"Checked {len(docs)} docs: placement, bilingual pairing, local/wiki "
        f"links and fences OK ({len(BILINGUAL_PENDING)} pairs pending bilingualization)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
