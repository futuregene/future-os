#!/usr/bin/env python3
"""Per-file uncovered lines from an Istanbul/V8 coverage-final.json (vitest).

Lines with zero statement/branch/function hits are reported; the campaign's JS
metric is "uncovered lines" = lines whose statements all have count 0, matching
the per-line discipline used on the Rust side.

Usage: covlines-js.py <coverage-final.json> [--under PATH]...
"""
import json
import os
import re
import sys

ROOT = "/Users/geilige/future-os"
WORKTREE_RE = re.compile(r"^\.?worktrees/[^/]+/(.*)$")


def relpath(path, root=ROOT):
    """Repository-root-relative path.

    Strips the checkout root AND a leading `.worktrees/<name>/` component. This
    matters twice over: llvm-cov/vitest label `SF:`/keys by the invocation prefix,
    so a run inside a worktree yields
    `/Users/.../.worktrees/cov100-r2/desktop/src/...`, while a supervisor run from
    the main checkout yields `/Users/.../desktop/src/...` for the same file. A
    caller passing `--under desktop/src` (worktree-relative) matched NOTHING and
    the tool used to answer 0/0 = 100% - the exact false-clean this guard exists
    to prevent. See INCIDENTS.md: five instances of this class in one campaign.
    """
    p = path.replace("\\", "/")
    for prefix in (root, os.path.expanduser("~")):  # noqa: B007 - both are stripped
        if p.startswith(prefix.rstrip("/") + "/"):
            p = p[len(prefix.rstrip("/")) + 1:]
    m = WORKTREE_RE.match(p)
    if m:
        p = m.group(1)
    return p


def norm_under(u):
    """Normalise a --under target the same way, and drop a trailing slash."""
    u = relpath(u)
    if u.startswith("./"):
        u = u[2:]
    return u.rstrip("/")


def in_scope(rel_path, unders):
    """Prefix relation either way, so `--under desktop/src` and
    `--under desktop/src/foo.ts` both match the file they name."""
    if not unders:
        return True
    for u in unders:
        if rel_path == u or rel_path.startswith(u + "/") or u.startswith(rel_path + "/"):
            return True
    return False


def main():
    path = sys.argv[1]
    under = [sys.argv[i + 1] for i, a in enumerate(sys.argv) if a == "--under"]
    data = json.load(open(path))

    rows = []
    grand_un = grand_tot = 0
    for f, cov in data.items():
        rel_p = relpath(f)
        und = [norm_under(u) for u in under]
        if not in_scope(rel_p, und):
            continue
        rel = rel_p
        if und and not any(rel == u or rel.startswith(u + "/") for u in und):
            continue
        if "statementMap" not in cov:
            continue
        smap = cov["statementMap"]
        scount = cov["s"]
        # line -> whether any statement on that line was executed
        per_line = {}
        for sid, meta in smap.items():
            ln = meta["start"]["line"]
            hit = scount.get(sid, 0) > 0
            per_line[ln] = per_line.get(ln, False) or hit
        unc = sorted(ln for ln, hit in per_line.items() if not hit)
        tot = len(per_line)
        grand_un += len(unc)
        grand_tot += tot
        if unc:
            rows.append((len(unc), tot, rel, unc))

    rows.sort(reverse=True)
    for n, tot, rel, unc in rows:
        print(f"{n:5d} /{tot:6d}  {rel}")
        print(f"        lines: {','.join(str(x) for x in unc)}")
    pct = ((grand_tot - grand_un) / grand_tot * 100) if grand_tot else 100.0
    print(f"files_with_uncovered={len(rows)} uncovered_lines={grand_un} "
          f"measured_lines={grand_tot} lines%={pct:.4f}")

    # Guard against a report that was read while being written (vitest --coverage
    # reports to coverage-final.json; a concurrent run wipes and rewrites it, and
    # a partial read looks like "everything is uncovered"). Fail loudly rather
    # than let a caller record a false number, in either direction.
    zero_files = sum(1 for cov in data.values()
                     if "statementMap" in cov and cov["statementMap"]
                     and not any(v > 0 for v in cov["s"].values()))
    total_files = sum(1 for cov in data.values() if cov.get("statementMap"))
    if grand_tot == 0 and total_files:
        print("ERROR: report has files but no statement data — wrong file or a "
              "zeroed report (check --under / whether the run actually ran)",
              file=sys.stderr)
        raise SystemExit(3)
    if total_files >= 5 and zero_files / total_files >= 0.9:
        print(f"ERROR: {zero_files}/{total_files} files have zero hits — this is "
              f"a partial or stale report (a coverage run was writing it, or the "
              f"run did not execute your tests). Re-run and re-read.",
              file=sys.stderr)
        raise SystemExit(3)


if __name__ == "__main__":
    main()
