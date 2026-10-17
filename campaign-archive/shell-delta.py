#!/usr/bin/env python3
"""Set-diff the shell scope's uncovered lines across the three artifacts (S8/S9/S10),
to check TAU-C9's G/L arithmetic (9 gained, 76 lost) and its "S10 minus S9 = empty"
claim. Same technique I used for the agent rework's one-line delta.

Usage: shell-delta.py
"""
import re

SCORES = ["/tmp/tauri-shell-lcov-c8union.info",
          "/tmp/tauri-shell-lcov-c9union.info",
          "/tmp/tauri-shell-lcov-c10union.info"]
SCOPE = ('desktop/src-tauri/src/bin/futureos-headless.rs',
         'desktop/src-tauri/src/desktop_shutdown.rs',
         'desktop/src-tauri/src/desktop_shutdown/coordinator.rs',
         'desktop/src-tauri/src/future_login.rs',
         'desktop/src-tauri/src/host_environment.rs',
         'desktop/src-tauri/src/instance.rs',
         'desktop/src-tauri/src/launch.rs',
         'desktop/src-tauri/src/lib.rs',
         'desktop/src-tauri/src/macos_power.rs',
         'desktop/src-tauri/src/main.rs',
         'desktop/src-tauri/src/menu.rs',
         'desktop/src-tauri/src/runtime.rs')


def load(path):
    keep = False
    out = set()
    for line in open(path, errors='replace'):
        if line.startswith('SF:'):
            p = line[3:].strip()
            m = re.search(r'future-os/(?:\.worktrees/[^/]+/)?(.*)$', p)
            rel = m.group(1) if m else p
            keep = any(rel == s or rel.startswith(s + '/') for s in SCOPE)
            continue
        if keep and line.startswith('DA:'):
            n = int(line[3:].split(',')[0])
            c = int(line[3:].split(',')[1].split(',')[0])
            if c == 0:
                out.add((rel, n))
    return out


s8, s9, s10 = (load(p) for p in SCORES)
print(f"uncovered: S8={len(s8)} S9={len(s9)} S10={len(s10)}")
print()
print("S10 minus S9 (should be EMPTY per its claim):")
d = sorted(s10 - s9)
print("   ", d if d else "empty - no line regressed")
print()
print("S9 minus S10 (its claim: runtime.rs 64-66, i.e. 3 regained):")
for f, n in sorted(s9 - s10):
    print(f"    {f}:{n}")
print()
print(f"S10 minus S8  (= {len(s10 - s8)} newly uncovered; its claim: 76 lost):")
by_file = {}
for f, n in s10 - s8:
    by_file[f] = by_file.get(f, 0) + 1
for f, c in sorted(by_file.items(), key=lambda kv: -kv[1]):
    print(f"    {c:4d}  {f}")
print()
print(f"S8 minus S10  (= {len(s8 - s10)} newly covered; its claim: 9 gained):")
by_file = {}
for f, n in s8 - s10:
    by_file.setdefault(f, []).append(n)
for f, ns in sorted(by_file.items()):
    print(f"    {len(ns):4d}  {f}: {sorted(ns)}")
