"""Report top-level items in a Rust test file that are never referenced elsewhere.

Why: gating ONE test to macOS leaves its private helpers unused on Linux, and clippy
`-D warnings` rejects dead code. That cost two CI rounds here (three helpers, then a
struct). This lists every candidate in one pass so the fix is complete rather than
iterative.
"""
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
lines = path.read_text().splitlines()
text = "\n".join(lines)

items = []
for i, line in enumerate(lines):
    m = re.match(r"^(?:pub )?(?:fn|struct|enum|const|static)\s+([A-Za-z_0-9]+)", line)
    if m:
        items.append((m.group(1), i + 1))

unused = []
for name, ln in items:
    uses = len(re.findall(r"\b" + re.escape(name) + r"\b", text))
    if uses <= 1:
        unused.append((name, ln))

print(f"{len(items)} top-level items")
for name, ln in unused:
    print(f"  never used: {name}  (line {ln})")
if not unused:
    print("  none unreferenced")
