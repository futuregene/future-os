#!/usr/bin/env python3
"""Count FAILED tests in a libtest log where tracing output is interleaved into the
per-test lines (so `test NAME ... FAILED` may be split across lines).

For each `test NAME ...` line, the verdict is the first bare `FAILED`/`ok` token that
appears before the NEXT `test ` line. Reports names + the count, so a claim like
"5 failures" can be checked against the log.

Usage: failed-count.py <log> [start_line]
"""
import re, sys

path = sys.argv[1]
start = int(sys.argv[2]) if len(sys.argv) > 2 else 1
lines = open(path, encoding='utf-8', errors='replace').read().split('\n')[start - 1:]

VERDICT = re.compile(r'(?<![A-Za-z])(FAILED|ok)(?![A-Za-z])')
test_start = re.compile(r'^test (\S+) \.\.\.')
pending = None
failed, passed = [], 0

for line in lines:
    m = test_start.match(line)
    if m:
        pending = m.group(1)
        # verdict may be on the same line (after interleaved output)
        v = VERDICT.search(line[line.find('...') + 3:])
        if v:
            if v.group(1) == 'FAILED':
                failed.append(pending)
            else:
                passed += 1
            pending = None
        continue
    if pending is not None:
        v = VERDICT.search(line)
        if v:
            if v.group(1) == 'FAILED':
                failed.append(pending)
            else:
                passed += 1
            pending = None

print(f"passed={passed}  failed={len(failed)}  (unresolved at EOF: {pending})")
for n in failed:
    print(f"  FAILED  {n}")
