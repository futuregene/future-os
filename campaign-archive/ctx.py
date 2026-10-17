#!/usr/bin/env python3
"""Print the source line for each uncovered line in an lcov file.

Usage: python3 ctx.py <lcov> <scope-substring>
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from covlines import parse_lcov  # noqa: E402

lcov, scope = sys.argv[1], sys.argv[2]
WORKTREE = "/Users/geilige/future-os/.worktrees/cov100-r2"
for path, da in parse_lcov(lcov):
    if scope not in path:
        continue
    if path.startswith(WORKTREE):
        local = path
    else:
        local = path.replace("/Users/geilige/future-os/", WORKTREE + "/")
    try:
        with open(local) as fh:
            src = fh.read().splitlines()
    except OSError:
        src = []
    print(f"### {path.split('/future-os/')[-1]}")
    for ln, count in sorted(da):
        if count != 0:
            continue
        text = src[ln - 1] if src and ln <= len(src) else "<no source>"
        print(f"{ln:5d}| {text}")
    print()
