"""Find unix-only API uses in desktop test code whose enclosing fn is NOT cfg-gated.

Windows CI fails to COMPILE such a test (`E0433`/`E0599`), and a macOS-local check
never sees it. Two CI rounds were spent on `FdCapture` alone, so this scans the
whole family and derives the answer from the code.

A use counts as gated when EITHER
  * the enclosing fn carries a cfg gate in its attribute block, OR
  * the innermost `#[cfg(...)]`-attributed block/item that contains the use is gated.

The second form is how most of this crate does it (`#[cfg(unix)] { ... }` inside a
portable function), so a scanner that only looks at fn attributes reports dozens of
false positives - which is what my first version did.
"""
import pathlib
import re

PATTERNS = [
    r"std::os::unix",
    r"libc::",
    r"PermissionsExt",
    r"MetadataExt",
]
ROOT = pathlib.Path("desktop/src-tauri/src")
FN = re.compile(r"\n([ \t]*)(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([a-z_0-9]+)\(")
CFG_LINE = re.compile(r"^\s*#\[cfg\((.*)\)\]")


def lines_before(text, pos, want=6):
    """The `want` source lines immediately preceding `pos` (exclusive)."""
    start = text.rfind("\n", 0, pos)
    out = []
    i = start
    while i > 0 and len(out) < want:
        j = text.rfind("\n", 0, i)
        out.append(text[j + 1:i])
        i = j
    return out


def gated_use(text, pos):
    """Is the use at `pos` inside something cfg-gated?"""
    for line in lines_before(text, pos, 12):
        m = CFG_LINE.match(line)
        if m:
            return not any(x in m.group(1) for x in ("not(unix)", "not(target_os"))
        if line.strip() and not line.strip().startswith(("//", "#[")):
            # a plain statement/brace with no cfg in between: stop looking only if
            # it clearly closes an unrelated item
            if line.strip() in ("}", "};"):
                continue
    # fall back: walk back over the enclosing fn's attribute block
    fns = list(FN.finditer(text, 0, pos))
    if fns:
        fpos = fns[-1].start()
        for line in lines_before(text, fpos + 1, 8):
            m = CFG_LINE.match(line)
            if m:
                return not any(x in m.group(1) for x in ("not(unix)", "not(target_os"))
            if line.strip() and not line.strip().startswith(("//", "#[", "///")):
                break
    return False


def main():
    findings = []
    for path in sorted(ROOT.rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        for pat in PATTERNS:
            for m in re.finditer(pat, text):
                if gated_use(text, m.start()):
                    continue
                fns = list(FN.finditer(text, 0, m.start()))
                fn = fns[-1].group(2) if fns else "<top>"
                line = text.count("\n", 0, m.start()) + 1
                findings.append((str(path), fn, line, pat))
    if not findings:
        print("clean: every unix-only use is cfg-gated")
        return 0
    print(f"{len(findings)} ungated use(s):")
    for path, fn, line, pat in findings:
        print(f"  {path}:{line}  fn {fn}  ({pat})")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
