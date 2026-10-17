#!/usr/bin/env python3
"""Which lines does the degraded session lose, and do they match the record's 75?

Compares the pinned healthy-session artifact (150) with the degraded-session artifact (217) over the
shell unit's twelve scopes, and prints the set difference in both directions. Purpose: verify the
`host-state-blocked` class actually has 75 members rather than taking the record's word for it - the
whole point of putting the class in the waiver ledger is that a reader can check it.
"""
import sys

SCOPES = ["src/lib.rs", "src/main.rs", "src/menu.rs", "src/launch.rs", "src/instance.rs",
          "src/runtime.rs", "src/host_environment.rs", "src/macos_power.rs",
          "src/desktop_shutdown.rs", "src/desktop_shutdown/", "src/future_login.rs", "src/bin/"]


def rel(sf):
    i = sf.find("/desktop/src-tauri/")
    return sf[i + len("/desktop/src-tauri/"):] if i >= 0 else sf


def in_scope(sf):
    r = rel(sf)
    return any(r == s or r.startswith(s.rstrip("/") + "/") for s in SCOPES)


def uncovered(path):
    out = set()
    cur = None
    keep = False
    for line in open(path, encoding="utf-8", errors="replace"):
        if line.startswith("SF:"):
            cur = line.rstrip()[3:]
            keep = in_scope(cur)
        elif line.startswith("DA:") and keep:
            q = line.rstrip()[3:].split(",")
            if len(q) >= 2 and q[1] == "0":
                out.add((rel(cur), int(q[0])))
    return out


healthy = uncovered(sys.argv[1])
degraded = uncovered(sys.argv[2])
print(f"healthy  ({sys.argv[1].split('/')[-1]}): {len(healthy)} uncovered")
print(f"degraded ({sys.argv[2].split('/')[-1]}): {len(degraded)} uncovered")

lost = degraded - healthy          # covered when healthy, uncovered when degraded
gained = healthy - degraded        # the reverse
print(f"\nlost in the degraded session (the host-state-blocked candidates): {len(lost)}")
by_file = {}
for f, _ in lost:
    by_file[f] = by_file.get(f, 0) + 1
for f, n in sorted(by_file.items(), key=lambda kv: -kv[1]):
    print(f"   {n:4d}  {f}")
print(f"\nuncovered in HEALTHY but covered in degraded (should be 0 if the code is unchanged): "
      f"{len(gained)}")
for f, ln in sorted(gained)[:10]:
    print("   ", f, ln)
