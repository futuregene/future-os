#!/usr/bin/env python3
"""Check repository documentation links, wiki targets, fences and language parity.

Run from any directory: python3 scripts/check-docs.py
Uses git's file inventory (including new, non-ignored docs), not the skills
submodule's third-party references. Does not fetch remote URLs or check anchors.
This is a structural check, not a substitute for source-based factual review.
"""

from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import unquote, urlsplit


ROOT = Path(__file__).resolve().parent.parent
LINK = re.compile(r"!?\[[^\]\n]*\]\((<[^>\n]+>|[^)\n]+)\)")
WIKI = re.compile(r"\[\[([^]\n]+)\]\]")
FENCE = re.compile(r"^\s*(?:>\s*)*(`{3,}|~{3,})(.*)$")
INLINE_CODE = re.compile(r"(`+).*?\1")


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


def check_file(root, relative):
    errors = []
    path = root / relative
    is_wiki = relative.parts[:2] == ("docs", "wiki")
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
            destination = path.parent / unquote(parsed.path)
            if not destination.exists():
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


def main():
    inventory = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=ROOT,
    ).decode().split("\0")
    files = sorted({
        Path(name) for name in inventory
        if name.endswith((".md", ".mdx", ".rst"))
        or (name.startswith("docs/dist/") and name.endswith(".txt"))
    })
    errors = []
    for path in files:
        if (ROOT / path).is_file():
            errors.extend(check_file(ROOT, path))
    english = {p.name for p in (ROOT / "docs/wiki/en").glob("*.md")}
    chinese = {p.name for p in (ROOT / "docs/wiki/zh").glob("*.md")}
    for name in sorted(english ^ chinese):
        errors.append(f"wiki bilingual page missing: {name}")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Checked {len(files)} docs: local/wiki links, fences and bilingual page inventory OK.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
