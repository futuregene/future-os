#!/usr/bin/env python3
"""Print DA:0 lines of one file (or a line window) from an lcov file.

Usage: uncovered-lines.py LCOV SUBSTR [START END]
"""
import sys

lcov, substr = sys.argv[1], sys.argv[2]
start = int(sys.argv[3]) if len(sys.argv) > 3 else 0
end = int(sys.argv[4]) if len(sys.argv) > 4 else 10**9

cur = None
with open(lcov) as fh:
    for line in fh:
        line = line.strip()
        if line.startswith("SF:"):
            cur = line[3:]
        elif line.startswith("DA:") and cur and substr in cur:
            n, count = line[3:].split(",")[:2]
            n = int(n)
            if int(count) == 0 and start <= n <= end:
                print(f"{cur}:{n}")
