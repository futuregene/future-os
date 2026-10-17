#!/usr/bin/env python3
"""Per-line UNION of two lcov reports (one per feature flavour).

The campaign recipe renders ONE lcov from both flavours' profraw; on this host that
render lost a whole flavour's late-run data in the seg-4 run (evidence in the module
record). Rendering each flavour right after its own leg, then unioning here, is
equivalent for the `DA:0` metric: a line is uncovered iff NEITHER flavour covered it.

usage: lcov-union.py OUT.INFO IN1.INFO IN2.INFO [IN3.INFO ...]
       lcov-union.py --report OUT.INFO IN... --under <suffix> [--under ...]
"""
import sys, re

def load(path):
    """{file: (order, {line: count})}"""
    out, order, cur = {}, [], None
    for line in open(path, encoding="utf-8", errors="replace"):
        if line.startswith("SF:"):
            cur = line.rstrip("\n")[3:]
            out.setdefault(cur, {})
            if cur not in order:
                order.append(cur)
        elif line.startswith("DA:") and cur:
            p = line.rstrip("\n")[3:].split(",")
            ln, cnt = int(p[0]), int(p[1])
            prev = out[cur].get(ln)
            out[cur][ln] = cnt if prev is None else max(prev, cnt)
    return {f: out[f] for f in order}

def main():
    argv = sys.argv[1:]
    unders = [argv[i + 1] for i, a in enumerate(argv) if a == "--under"]
    out_path = argv[argv.index("--out") + 1] if "--out" in argv else None
    drop = set(unders) | ({"--out", out_path} if out_path else set())
    args = [a for a in argv if a not in drop]
    merged = {}
    for p in args:
        for f, lines in load(p).items():
            merged.setdefault(f, {})
            for ln, c in lines.items():
                prev = merged[f].get(ln)
                merged[f][ln] = c if prev is None else max(prev, c)
    if out_path:
        with open(out_path, "w", encoding="utf-8") as fh:
            for f, lines in merged.items():
                fh.write(f"SF:{f}\n")
                for ln in sorted(lines):
                    fh.write(f"DA:{ln},{lines[ln]}\n")
                fh.write("end_of_record\n")
        print(f"wrote {out_path}: {len(merged)} files")
        return
    tot = 0
    for f in sorted(merged):
        if unders and not any(f.endswith(u) for u in unders):
            continue
        z = sum(1 for c in merged[f].values() if c == 0)
        tot += z
        print(f"{z:5d} / {len(merged[f]):5d}  {f}")
    print("TOTAL uncovered =", tot)

main()
