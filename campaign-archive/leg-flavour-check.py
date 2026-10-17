#!/usr/bin/env python3
"""Is each lcov leg the FEATURE FLAVOUR it claims to be?

WHY (F.81, measured 2026-10-17): the tauri coverage recipe renders one lcov per flavour and unions
them. If the target dir is not cleaned between legs, leg 2 accumulates leg 1's profraw and the
"headless" report silently becomes gui+headless - I measured exactly that: a report claiming to be
headless carried SF=170 files including gui-only `commands/agent.rs` and `menu.rs`, while the true
headless build carries SF=136 and NO gui-only file.

That defect is invisible from a count: the union still produced the right number (34810 keys, 386
uncovered, from either leg), and the leg read ALONE looks *better* (386 vs 251 uncovered) because a
bigger file set means more covered lines. The only reliable test is a FILE-SET IDENTITY test, which
is what this does: a gui report must contain gui-only markers and no headless-only ones, and vice
versa. Counting cannot substitute - a count tells you how many, never which.

usage: leg-flavour-check.py gui.info headless.info
       (name the files as the flavours they are SUPPOSED to be; exit 1 on any mismatch)
"""
import re
import sys

# Files that exist in exactly one flavour. Verified in this repo: the gui-only pair are reached only
# from the gui entry point, and the headless-only triple from the headless binary. If a future
# Cargo.toml change moves one, the tool reports it rather than silently passing.
GUI_ONLY = ["/src/commands/agent.rs", "/src/menu.rs"]
HEADLESS_ONLY = ["/src/no_gui.rs", "/src/headless/mod.rs", "/src/bin/futureos-headless.rs"]

# A leg restricted to one flavour also carries at most this many files for this crate; used as a
# sanity bound that catches "leg 2 is a union" without hard-coding the exact count.
GUI_FLOOR = 150
HEADLESS_CEILING = 145


def sfs(path):
    out = []
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            if line.startswith("SF:"):
                out.append(line.rstrip("\n")[3:])
    return out


def has(files, suffix):
    return any(f.endswith(suffix) for f in files)


def main():
    gui_path, hl_path = sys.argv[1], sys.argv[2]
    gui, hl = sfs(gui_path), sfs(hl_path)
    fails = []

    print(f"gui leg      {gui_path}: {len(gui)} files")
    print(f"headless leg {hl_path}: {len(hl)} files")
    print()

    for f in GUI_ONLY:
        ok = has(gui, f)
        print(f"  {'ok  ' if ok else 'FAIL'} gui leg CONTAINS gui-only {f}")
        if not ok:
            fails.append(f"gui leg missing gui-only marker {f} (is it really a gui build?)")

    for f in GUI_ONLY:
        bad = has(hl, f)
        print(f"  {'FAIL' if bad else 'ok  '} headless leg EXCLUDES gui-only {f}")
        if bad:
            fails.append(f"headless leg contains gui-only {f} -> it is a UNION, not a headless build "
                         f"(most likely: the target dir was not cleaned between legs)")

    for f in HEADLESS_ONLY:
        ok = has(hl, f)
        print(f"  {'ok  ' if ok else 'FAIL'} headless leg CONTAINS headless-only {f}")
        if not ok:
            fails.append(f"headless leg missing headless-only marker {f}")

    print(f"  {'ok  ' if len(gui) >= GUI_FLOOR else 'FAIL'} gui leg has >= {GUI_FLOOR} files "
          f"(got {len(gui)})")
    print(f"  {'FAIL' if len(hl) > HEADLESS_CEILING else 'ok  '} headless leg has <= "
          f"{HEADLESS_CEILING} files (got {len(hl)})")
    if len(hl) > HEADLESS_CEILING:
        fails.append(f"headless leg has {len(hl)} files, above the {HEADLESS_CEILING} ceiling - "
                     f"the shape of a leg that kept the other flavour's profraw")

    print()
    if fails:
        print("LEG IDENTITY: FAIL")
        for f in fails:
            print("  !", f)
        return 1
    print("LEG IDENTITY: ok - each leg matches the flavour it claims")
    return 0


if __name__ == "__main__":
    sys.exit(main())
