"""Compare a primary cargo-mutants sample with its survivor re-check.

Usage: python mutation/compare-runs.py <primary/mutants.out> <recheck/mutants.out>

Prints the mutants whose primary verdict was MISSED and whose re-check verdict is
CAUGHT (witness-set artifacts), the ones still MISSED under the wider witness set,
and anything no re-check covered.
"""
import pathlib
import sys


def names(path, fname):
    f = pathlib.Path(path) / fname
    return {l.strip() for l in f.read_text(encoding="utf-8").splitlines() if l.strip()}


def main() -> int:
    pri, rc = sys.argv[1], sys.argv[2]
    pm, pc = names(pri, "missed.txt"), names(pri, "caught.txt")
    rm, rcc = names(rc, "missed.txt"), names(rc, "caught.txt")
    print(f"primary: {len(pc)} caught / {len(pm)} missed")
    print(f"recheck: {len(rcc)} caught / {len(rm)} missed")
    conv = sorted(pm & rcc)
    still = sorted(pm & rm)
    cover = sorted(pm & (rcc | rm))
    print(f"\nCONVERTED by the wider witness set (primary MISSED -> recheck CAUGHT): {len(conv)}")
    for c in conv:
        print("  +", c)
    print(f"\nSTILL MISSED under the wider witness set: {len(still)}")
    for c in still:
        print("  -", c)
    print(f"\nnot re-tested: {len(set(pm) - set(cover))}")
    for c in sorted(set(pm) - set(cover)):
        print("  ?", c)
    return 0


if __name__ == "__main__":
    sys.exit(main())
