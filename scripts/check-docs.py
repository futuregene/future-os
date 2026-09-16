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
2. Bilingual pairing:
   - Pairing-scope directories (docs/ itself, guide/, architecture/,
     internals/, archives/, maintainers/, audits/): name.md must have
     name.zh-CN.md and vice versa. Files whose missing pair is scheduled for
     the bilingualization PR are listed in BILINGUAL_PENDING and do not fail;
     an entry whose pair has landed fails, so the list can only shrink.
   - wiki: en/ and zh/ must contain the same basenames.
   - packaging: each readme must have its -en counterpart and vice versa.
3. Structure: local markdown links resolve, wiki [[...]] targets resolve,
   code fences are closed (kept from the original checker).
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

# Directories (recursively) where name.md <-> name.zh-CN.md pairing applies.
PAIR_SCOPE_DIRS = {"guide", "architecture", "internals", "archives", "maintainers", "audits"}

# Docs whose missing language pair is scheduled for the bilingualization PR.
# Each entry is the path of the existing file (relative to the repo root,
# POSIX separators). Adding a pair without removing the entry is an error.
BILINGUAL_PENDING = {
    # All pending bilingual pairs have landed; keep the set empty so any
    # future scope registers its pending entries here explicitly.
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
            errors.append(f"{relative}:{number}: {error}")
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
                errors.append(f"{relative}:{number}: missing local target {target}")
        if is_wiki:
            for match in WIKI.finditer(line):
                target = match[1].rsplit("|", 1)[-1].split("#", 1)[0]
                if not target or urlsplit(target).scheme:
                    continue
                destination = path.parent / target
                if not destination.suffix:
                    destination = destination.with_suffix(".md")
                if not destination.is_file():
                    errors.append(f"{relative}:{number}: missing wiki page {target}")
    return errors


def md_pair(relative):
    """The bilingual counterpart path of a .md doc, or None if not applicable."""
    if relative.endswith(".zh-CN.md"):
        return relative[: -len(".zh-CN.md")] + ".md"
    return relative.replace(".md", ".zh-CN.md", 1)


def in_pair_scope(relative):
    if relative in EXTRA_PAIR_SCOPED or relative.replace(".zh-CN.md", ".md", 1) in EXTRA_PAIR_SCOPED:
        return True
    if relative.startswith("docs/"):
        rest = relative[len("docs/"):]
        if "/" not in rest:
            return True  # docs/ root itself (README.md etc.)
        return rest.split("/", 1)[0] in PAIR_SCOPE_DIRS
    return False


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
            errors.append(f"packaging bilingual readme missing: {pair.as_posix()}")
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
            continue  # scheduled for the bilingualization PR
        errors.append(f"bilingual pair missing: {rel} (expected {pair})")
    for rel in sorted(BILINGUAL_PENDING):
        # Debt-list hygiene is reported as a warning during the bilingualization
        # work (several workers land pairs concurrently, so the hand-owned list
        # is transiently ahead of the tree) and enforced as a hard failure by
        # the final acceptance run, which passes --strict-pending.
        bucket = errors if strict_pending else warnings
        if rel not in existing:
            bucket.append(f"BILINGUAL_PENDING entry does not exist: {rel}")
        elif md_pair(rel) in existing:
            bucket.append(
                f"BILINGUAL_PENDING entry is stale — pair landed, remove it: {rel}"
            )
    english = {p.name for p in (ROOT / "docs/wiki/en").glob("*.md")}
    chinese = {p.name for p in (ROOT / "docs/wiki/zh").glob("*.md")}
    for name in sorted(english ^ chinese):
        errors.append(f"wiki bilingual page missing: {name}")
    return errors, warnings


def scoped(errors, scope):
    """Keep only diagnostics attributable to `scope` path prefixes.

    Concurrent workers each own a slice of the tree. A global verdict would
    fail every one of them for a peer's in-flight state — and make the
    BILINGUAL_PENDING hygiene check fire for entries outside the slice — so
    slices validate with `--scope` while the unscoped run stays authoritative
    for final acceptance.
    """
    if not scope:
        return errors
    kept = []
    for error in errors:
        subject = error.split(":", 1)[0].split(" ", 1)[0]
        if error.startswith("Checked"):
            continue
        if any(subject == prefix or subject.startswith(prefix.rstrip("/") + "/") for prefix in scope):
            kept.append(error)
    return kept


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--scope",
        help="comma-separated path prefixes; report only diagnostics within them",
    )
    parser.add_argument(
        "--strict-pending",
        action="store_true",
        help="treat BILINGUAL_PENDING hygiene (stale/missing entries) as failures",
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
                errors.append(f"{rel}: documentation .txt outside docs/dist/")
            continue
        if not rel.startswith("docs/"):
            if rel in WHITELIST:
                continue
            errors.append(f"{rel}: markdown file outside docs/ (not whitelisted)")
    for rel in sorted(WHITELIST):
        if not (ROOT / rel).is_file():
            errors.append(f"whitelist entry does not exist: {rel}")
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
        print("\n".join(warnings), file=sys.stderr)
    if errors:
        print("\n".join(errors), file=sys.stderr)
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
