#!/usr/bin/env python3
"""Freeze-readiness matrix: one cheap row per pairing in .future/cov100/pairings.json.

WHY: at freeze time I must know, for every module, (a) which report describes it, (b) whether the
ledger still agrees with that report, (c) whether the doc section still resolves, and (d) whether
*gates* and *review verdicts* exist. Doing that by hand under time pressure is how a number gets
published against the wrong report (INCIDENTS F.48/F.58/F.77).

CHEAP BY DESIGN: it reads ONLY the report each pairing names (no /tmp globbing, no re-measurement).
`staletable.py --suggest` is the expensive repair tool; this is the status view. A report that is
missing or a count that has drifted is a REPAIR TASK, not an error here.

Environment: ROOT defaults to the worktree; ${ROOT} in the registry is substituted.
"""
import hashlib
import json
import os
import re
import subprocess
import sys

REG = ".future/cov100/pairings.json"
ROOT = os.path.abspath(os.environ.get("COV_ROOT") or os.getcwd())


def sha(path, n=12):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:n]


def _scope_match(sf, scope):
    """A registry scope may name a DIRECTORY or a single FILE (the tui pairings list files),
    and the report may label paths absolutely or worktree-relative. Compare on the
    repo-relative tail, and accept equality as well as a '/' boundary -- appending '/' to
    every scope (my first version) silently matched nothing for file scopes and would have
    been reported as 'DRIFT n -> 0', i.e. a false alarm of the exact class
    covlines-js.py's docstring warns about."""
    def rel(p):
        p = p.replace("${ROOT}", ROOT)
        m = re.match(r"^/.*?/\.worktrees/[^/]+/(.*)$", p)
        if m:
            return m.group(1)
        m = re.match(r"^/.*?/(future-os)/(.*)$", p)
        if m:
            return m.group(2)
        return p.lstrip("./")

    a, b = rel(sf), rel(scope)
    return a == b or a.startswith(b.rstrip("/") + "/")


def measure(report, scopes):
    """Uncovered-line count over `scopes` in `report`. Returns (uncovered, files) or (None, None).

    Dispatches on the file type: Istanbul `coverage-final.json` is NOT lcov, and reading it
    with the lcov parser answers 0 files / 0 uncovered -- a false 100%, the campaign's
    recurring failure mode.
    """
    if not os.path.exists(report):
        return None, None
    if report.endswith(".json"):
        args = ["python3", ".future/cov100/covlines-js.py", report]
        for s in scopes:
            args += ["--under", s.replace("${ROOT}/", "")]
        out = subprocess.run(args, capture_output=True, text=True)
        if out.returncode not in (0,):
            return None, None
        # The summary line is `files_with_uncovered=33 uncovered_lines=59 measured_lines=12800`.
        # Matching bare `uncovered` picks up "files_with_uncovered" first and reports the FILE
        # count as the line count (my first version did exactly that: it claimed a drift 60 -> 33
        # that did not exist). Match the full key.
        m = re.search(r"\buncovered_lines=(\d+)", out.stdout)
        f = re.search(r"\bfiles_with_uncovered=(\d+)", out.stdout)
        if not m:
            print("   (warning: could not parse covlines-js output)", file=sys.stderr)
            return None, None
        return int(m.group(1)), int(f.group(1)) if f else None

    uncovered = 0
    files = 0
    seen = set()
    cur = None
    with open(report, "r", errors="replace") as fh:
        for line in fh:
            if line.startswith("SF:"):
                cur = line[3:].strip()
                if any(_scope_match(cur, w) for w in scopes):
                    files += 1
                    seen.add(cur)
            elif line.startswith("DA:") and cur in seen:
                parts = line[3:].split(",")
                if len(parts) > 1 and parts[1].strip() == "0":
                    uncovered += 1
    return uncovered, files


def section_resolves(doc, needle):
    if not os.path.exists(doc):
        return "DOC MISSING"
    if not needle:
        return "NO SECTION DECLARED"
    with open(doc, "r", errors="replace") as fh:
        text = fh.read()
    # fences are opaque (F.70): a '#' inside a fence is not a heading.
    inside = False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("```"):
            inside = not inside
            continue
        if inside:
            continue
        if line.startswith("#") and needle in line:
            return "ok"
    return "SECTION NOT FOUND"


def main():
    reg = json.load(open(REG))
    rows = []
    for p in reg["pairings"]:
        report = p.get("report") or ""
        scopes = p.get("scopes") or []
        exp = p.get("expect") or {}
        get = measure(report, scopes)
        cur = None
        fhash = ""
        if not os.path.exists(report):
            state = "REPORT MISSING"
        else:
            cur = get[0]
            fhash = sha(report)
            if exp.get("uncovered") is None:
                state = "no expect"
            elif cur == exp["uncovered"]:
                state = "IN SYNC"
            else:
                state = f"DRIFT {exp['uncovered']} -> {cur}"
        rows.append({
            "label": p["label"],
            "role": p.get("role", ""),
            "state": state,
            "report": report,
            "hash": fhash,
            "doc": p.get("doc", ""),
            "section": p.get("section", ""),
            "sec": section_resolves(p.get("doc", ""), p.get("section", "")) if p.get("doc") else "n/a",
            "files": get[1],
        })

    bad = [r for r in rows if r["state"] not in ("IN SYNC",)]
    secbad = [r for r in rows if r["sec"] not in ("ok", "n/a")]
    print(f"pairings: {len(rows)}   in-sync: {len(rows) - len(bad)}   drifting/missing: {len(bad)}"
          f"   section problems: {len(secbad)}")
    print()
    w = max(len(r["label"]) for r in rows)
    for r in sorted(rows, key=lambda x: (x["state"] == "IN SYNC", x["label"])):
        mark = "  " if r["state"] == "IN SYNC" else "!!"
        print(f"{mark} {r['label']:<{w}}  {r['state']:<28} {r['sec']:<16} "
              f"files={r['files']} {r['hash']}")
    print()
    if bad:
        print("REPAIR BEFORE STAMPING (use: staletable.py --suggest --labels <label>):")
        for r in bad:
            print(f"  {r['label']:<{w}}  {r['state']}   report={r['report']}")
    if secbad:
        print("SECTION HEADINGS THAT NO LONGER RESOLVE (the ledger moved or was renamed):")
        for r in secbad:
            print(f"  {r['label']:<{w}}  {r['sec']}  doc={r['doc']}  section={r['section']!r}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
