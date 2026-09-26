"""Dump the failing-test list for selected mutants of a cargo-mutants run.

Usage: python mutation/dump-fails.py <mutants.out> <substring> [<substring> ...]

Prints one block per matching mutant: verdict, name, then every test that
actually failed in that mutant's own log (from outcomes.json's log_path map).
"""
import json
import pathlib
import re
import sys

out = pathlib.Path(sys.argv[1])
needles = sys.argv[2:]
doc = json.loads((out / "outcomes.json").read_text(encoding="utf-8"))
F = re.compile(r"^test (\S+) \.\.\. FAILED", re.M)
for e in doc["outcomes"]:
    s = e["scenario"]
    if "Mutant" not in s:
        continue
    name = s["Mutant"]["name"]
    if not any(n in name for n in needles):
        continue
    log = (out / e["log_path"]).read_text(encoding="utf-8", errors="replace")
    fails: list[str] = []
    for m in F.finditer(log):
        if m.group(1) not in fails:
            fails.append(m.group(1))
    print(f"{e['summary']}  {name}")
    for t in fails:
        print(f"    - {t}")
    if not fails:
        print("    (no failing test in the log)")
