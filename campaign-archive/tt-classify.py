#!/usr/bin/env python3
"""Classify every documented `cargo test/llvm-cov ... --test-threads` invocation as
PARSED or SWALLOWED, using libtest's actual rule.

The rule (verified 2026-10-17 with an invalid-value control, see GOTCHAS):
  cargo splits its own args from the binary's at the FIRST `--`. libtest then stops
  option parsing at the FIRST `--` it sees. So if the binary's args contain another
  `--` before `--test-threads`, the flag is NOT an option - it becomes a test-name
  filter, and the harness runs at the default concurrency.

Usage: tt-classify.py [root ...]
"""
import re, sys, os

ROOTS = sys.argv[1:] or ['docs', '.future/cov100']
LINE = re.compile(r'(cargo\s+(?:llvm-cov|test)\b.*--test-threads\S*)')

rows = []
for root in ROOTS:
    if os.path.isfile(root):
        files = [root]
    else:
        files = []
        for dirpath, _, names in os.walk(root):
            for n in names:
                if n.endswith(('.md', '.sh')):
                    files.append(os.path.join(dirpath, n))
    for path in files:
        try:
            text = open(path, encoding='utf-8', errors='replace').read().splitlines()
        except OSError:
            continue
        for i, line in enumerate(text, 1):
            for m in LINE.finditer(line):
                cmd = m.group(1)
                # the binary's args = everything after cargo's first `--`
                parts = cmd.split(' -- ', 1)
                if len(parts) < 2:
                    continue
                binargs = parts[1]
                tt = binargs.find('--test-threads')
                if tt < 0:
                    continue
                # is there a further `--` before the flag inside the binary args?
                inner = binargs.find(' -- ', 0, tt)
                verdict = 'SWALLOWED' if inner >= 0 else 'parsed'
                rows.append((verdict, f"{path}:{i}", cmd.strip()[:96]))

rows.sort()
n_bad = sum(1 for v, _, _ in rows if v == 'SWALLOWED')
for v, loc, cmd in rows:
    flag = 'XX' if v == 'SWALLOWED' else 'ok'
    print(f"{flag} {loc}\n     {cmd}")
print(f"\n{len(rows)} invocations: {n_bad} SWALLOWED, {len(rows) - n_bad} parsed")
