#!/usr/bin/env python3
"""Per-line coverage analysis for the cov100 campaign.

Truth layer (see .future/memory/coverage-llvm-cov.md): per-line LCOV `DA:<n>,0`
is the only source of truth for a missed line. This tool never reads the
`summary` block; it parses DA records.

Usage:
  covlines.py files   <lcov> [--under DIR]...      # per-file DA:0 counts
  covlines.py detail  <lcov> [--under DIR]...      # per-file uncovered line numbers
  covlines.py groups  <lcov> --groups groups.json  # aggregate by work unit
  covlines.py total   <lcov>                       # covered/measured/uncovered
"""
import json
import re
import sys


def parse_lcov(path):
    """-> list of (file, [(line, count), ...])"""
    files = []
    cur = None
    with open(path, "r", encoding="utf-8", errors="replace") as fh:
        for raw in fh:
            line = raw.rstrip("\n")
            if line.startswith("SF:"):
                cur = {"file": line[3:], "da": []}
                files.append(cur)
            elif line.startswith("DA:") and cur is not None:
                body = line[3:]
                n, _, c = body.partition(",")
                # lcov may append a checksum after the count: DA:n,c,ck
                c = c.split(",")[0]
                try:
                    cur["da"].append((int(n), int(c)))
                except ValueError:
                    pass
            elif line == "end_of_record":
                cur = None
    return [(f["file"], f["da"]) for f in files]


WORKTREE_RE = re.compile(r"^\.?worktrees/[^/]+/(.*)$")


def rel(path, root="/Users/geilige/future-os"):
    """Normalise a report path to the form `--under` arguments use.

    Strips the checkout root AND a leading `.worktrees/<name>/` component. The
    second matters because llvm-cov stamps `SF:` with the *invocation* prefix: a
    run from inside a worktree yields
    `/Users/…/.worktrees/cov100-r2/desktop/src-tauri/src/...` while the same run
    from the main checkout yields `/Users/…/desktop/src-tauri/src/...`. Stripping
    only the checkout root left `.worktrees/cov100-r2/desktop/...`, which matches
    no `--under desktop/...` argument - and that reads as "0 uncovered", or as
    100 % if the guard is bypassed. Observed four times in one campaign (twice in
    `reconcile.py`, twice here); the guard now turns it into exit 3, and this
    normalisation removes the trap for the ordinary case.
    """
    p = path
    prefix = root if root.endswith("/") else root + "/"
    if p.startswith(prefix):
        p = p[len(prefix):]
    m = WORKTREE_RE.match(p)
    if m:
        p = m.group(1)
    return p.lstrip("/")


def under_filter(files, under):
    """Match `--under` against each file path, accepting BOTH an absolute path
    and a path relative to the repo root.

    File paths in a report are absolute, but `rel()` strips the repo root for
    display, and the first version of this function compared the stripped path
    against the raw argument — so an absolute `--under` matched nothing and the
    caller got "no per-line records" (which reads as INVALID, or worse as 100 % if
    unchecked). Observed three times in this campaign; accept both forms.
    """
    if not under:
        return files
    norm = [rel(u).rstrip("/") for u in under]
    out = []
    for f, da in files:
        r = rel(f)
        if any(r == u or r.startswith(u + "/") for u in norm):
            out.append((f, da))
    return out


def _guard(files, cov, tot):
    """Fail loudly on an implausible report instead of letting a caller record
    a false number. Two observed failure modes: an instrumented run that dies
    (disk full, an abort) leaves a complete-looking report whose counters are all
    zero; and a scope mismatch (`--under` not matching) leaves no records at all,
    which prints as 100 %."""
    if not files or tot == 0:
        print("ERROR: no per-line records found — wrong file, or --under does "
              "not match this report (a zero denominator reads as 100 %).",
              file=sys.stderr)
        raise SystemExit(3)
    if tot >= 200 and cov / tot < 0.05:
        print(f"ERROR: {cov}/{tot} lines have a nonzero count — this is a zeroed "
              f"or stale report (the instrumented run did not finish). Re-run "
              f"before recording any number.", file=sys.stderr)
        raise SystemExit(3)


def main():
    mode = sys.argv[1]
    argv = sys.argv[2:]
    under = []
    groups_path = None
    lcov = None
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--under":
            i += 1
            under.append(argv[i])
        elif a == "--groups":
            i += 1
            groups_path = argv[i]
        else:
            lcov = a
        i += 1

    files = parse_lcov(lcov)
    files = under_filter(files, under)

    if mode == "total":
        cov = sum(1 for _, da in files for _, c in da if c > 0)
        mis = sum(1 for _, da in files for _, c in da if c == 0)
        tot = cov + mis
        pct = (cov / tot * 100) if tot else 100.0
        print(f"covered={cov} measured={tot} uncovered={mis} lines%={pct:.4f}")
        _guard(files, cov, tot)
        return

    if mode == "files":
        rows = []
        for f, da in files:
            mis = sum(1 for _, c in da if c == 0)
            tot = len(da)
            if mis:
                rows.append((mis, tot, rel(f)))
        rows.sort(reverse=True)
        for mis, tot, name in rows:
            print(f"{mis:5d} /{tot:6d}  {name}")
        print(f"files_with_uncovered={len(rows)} "
              f"uncovered_lines={sum(r[0] for r in rows)}")
        return

    if mode == "detail":
        for f, da in files:
            mis = [n for n, c in da if c == 0]
            if mis:
                print(f"{rel(f)}: {','.join(str(n) for n in mis)}")
        return

    if mode == "groups":
        spec = json.load(open(groups_path))
        allfiles = parse_lcov(lcov)
        print(f"{'group':22s} {'uncovered':>9s} {'measured':>9s} {'files':>6s}")
        grand_mis = grand_tot = 0
        for name, paths in spec.items():
            sel = under_filter(allfiles, paths)
            mis = sum(1 for _, da in sel for _, c in da if c == 0)
            tot = sum(len(da) for _, da in sel)
            nfiles = sum(1 for _, da in sel if any(c == 0 for _, c in da))
            grand_mis += mis
            grand_tot += tot
            print(f"{name:22s} {mis:9d} {tot:9d} {nfiles:6d}")
        pct = ((grand_tot - grand_mis) / grand_tot * 100) if grand_tot else 100.0
        print(f"{'TOTAL':22s} {grand_mis:9d} {grand_tot:9d}   lines%={pct:.4f}")
        return

    raise SystemExit(f"unknown mode {mode}")


if __name__ == "__main__":
    main()
