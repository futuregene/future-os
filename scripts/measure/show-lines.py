#!/usr/bin/env python3
"""Show source lines by number: python3 scripts/measure/show-lines.py FILE N [N...]"""
import sys

path = sys.argv[1]
lines = open(path).read().split("\n")
for raw in sys.argv[2:]:
    number = int(raw)
    text = lines[number - 1] if 0 < number <= len(lines) else "<out of range>"
    print(f"{number:5d}: {text}")
