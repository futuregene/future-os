#!/usr/bin/env python3
"""Turn coverage/chan/lcov.info into a per-file list of uncovered lines.

Usage: python3 scripts/measure/chan-missed.py [substring-filter]
Writes coverage/chan/missed.txt and prints the grouped summary.
"""
import collections
import sys

needle = sys.argv[1] if len(sys.argv) > 1 else ""
lcov = "coverage/chan/lcov.info"
out = "coverage/chan/missed.txt"

current = None
missed = collections.defaultdict(list)
for raw in open(lcov):
    line = raw.strip()
    if line.startswith("SF:"):
        current = line[3:]
    elif line.startswith("DA:") and current:
        number, _, count = line[3:].partition(",")
        if count.split(",")[0] == "0":
            missed[current].append(int(number))

total = sum(len(value) for value in missed.values())
with open(out, "w") as handle:
    for path in sorted(missed):
        for number in sorted(missed[path]):
            handle.write(f"{path}:{number}\n")
    handle.write("\n")
    for path in sorted(missed):
        handle.write(f"{len(missed[path]):5d}  {path}\n")
    handle.write(f"{total:5d}  TOTAL\n")

print(f"crate uncovered lines: {total} in {len(missed)} file(s)")
for path in sorted(missed, key=lambda name: (-len(missed[name]), name)):
    relative = path.split("/channels/src/", 1)[-1]
    if needle and needle not in relative:
        continue
    numbers = sorted(missed[path])
    print(f"== {relative} ({len(numbers)})")
    print("   ", numbers)
