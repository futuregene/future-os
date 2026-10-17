#!/usr/bin/env python3
"""Apply the pre-classified pairing repairs from CLOSEOUT 2b, measuring before writing.

WHY: the freeze step is "repair 7 pairings", the CLOSEOUT calls a wrong hand-edit there "a corruption of
the gate's MEANING rather than a number", and I have already made two reader errors in this family (F.79
invented six drifts; F.87 copied a record's headline number without checking its metric). So each repair
here is:

  1. DECLARED as data (new report / new scopes / new section / new expect);
  2. VERIFIED before writing: the report must exist, and the new `expect` is not something I type - it is
     MEASURED from the report over the new scopes, and the script asserts the measured value equals the
     value declared in the plan. If they differ, the plan is stale and nothing is written;
  3. APPLIED atomically per pairing, with the before/after printed;
  4. IDEMPOTENT: a second run reports "already repaired" and changes nothing.

Dry-run by default. `--apply` writes.

usage: pairing-repair.py [--apply] [--only label1,label2]
"""
import json
import subprocess
import os
import re
import sys

REG = ".future/cov100/pairings.json"

# Each entry: what CLOSEOUT 2b says the repair is, plus the measured expectation that must hold.
# `expect_uncovered` is ASSERTED against a measurement - never trusted as typed.
PLAN = {
    "desktop": {
        "why": "doc's own '59 = 55 waived + 4 OPEN' is right; report valid (333 files, 0 all-zero)",
        "expect_uncovered": 59,
        "section": "2. Before / after",
    },
    "agent-rpc": {
        "why": "the DOC moved to 202 (AG-A closed cli.rs:590), and the registry still points at the S7 report that measures 324 - so BOTH the report and the section must move",
        "report": "/tmp/agent-svc-rework-norm.info",
        "expect_uncovered": 202,
        "section": "0.5 Residual ledger",
    },
    "cli": {
        "why": "needle landed on '4c. Ledger - measurement S12'; that section's report is /tmp/cli-r2s4-final-lcov.info and measures 52, while the registry still points at the S3 report measuring 123",
        "report": "/tmp/cli-r2s4-final-lcov.info",
        "expect_uncovered": 52,
        "section": "4c. Ledger",
    },
    "tauri-appdata": {
        "why": "record is now 378 = 368 + 10 (S23); last measured, so repair LAST",
        "report": "/tmp/appdata-taub-s23-normalized-lcov.info",
        "expect_uncovered": 378,
        "section": "S18.5",
    },
    "tauri-headless": {
        "why": "record's 359 over TWELVE scopes, verified per file; registry had 2 and a report path that no longer exists",
        "report": ".future/cov100/tauri-tau-after2-lcov.info",
        "scopes": [
            "${ROOT}/desktop/src-tauri/src/terminal/server.rs",
            "${ROOT}/desktop/src-tauri/src/terminal/pty.rs",
            "${ROOT}/desktop/src-tauri/src/terminal/session.rs",
            "${ROOT}/desktop/src-tauri/src/terminal/manager.rs",
            "${ROOT}/desktop/src-tauri/src/terminal/shell.rs",
            "${ROOT}/desktop/src-tauri/src/terminal/cwd.rs",
            "${ROOT}/desktop/src-tauri/src/headless/mod.rs",
            "${ROOT}/desktop/src-tauri/src/headless/agent.rs",
            "${ROOT}/desktop/src-tauri/src/cli_tool.rs",
            "${ROOT}/desktop/src-tauri/src/agent_host.rs",
            "${ROOT}/desktop/src-tauri/src/agent_supervisor.rs",
            "${ROOT}/desktop/src-tauri/src/skills_bootstrap.rs",
        ],
        "expect_uncovered": 359,
        "section": "2. Before / after",
    },
    "cli-mcp": {
        "why": "record's 4, reproduced on both reports it names (3 + 1)",
        "expect_uncovered": 4,
        "section": "2. Result",
    },
}


def rel(s, root):
    """Reduce a path to repo-relative, stripping BOTH checkout prefixes.

    MEASURED (2026-10-17): reports in this campaign use two conventions - a run inside the worktree labels
    `SF:/Users/.../.worktrees/cov100-r2/desktop/src/...`, while a supervisor run from the main checkout
    labels the SAME file `SF:/Users/.../desktop/src/...`. Stripping only the worktree form made this script
    measure **0** for the app-data report (its SF: lines are main-checkout paths) against scopes that are
    worktree paths - and 0 would have been written as `expect: 0`, a false-clean of 378 lines. covlines-js.py
    documents the identical trap; this is its Rust-side twin.
    """
    s = s.replace("${ROOT}", root)
    m = re.match(r"^/.*?/\.worktrees/[^/]+/(.*)$", s)
    if m:
        return m.group(1)
    m = re.match(r"^/(?:Users|home)/[^/]+/(?:future-os|future-os-[^/]+)/(.*)$", s)
    if m:
        return m.group(1)
    return s.lstrip("./")


def measure(report, scopes, root):
    """Uncovered count over `scopes`, one pass. Returns None when the report is absent.

    Dispatches on file type: the `desktop` pairing's report is an Istanbul coverage-final.json, and reading
    it with this lcov parser answers 0 uncovered - a false-clean, and the same bug I fixed in
    freeze-matrix.py (F.79). That is why the 0 was refused rather than written.
    """
    if not os.path.exists(report):
        return None
    if report.endswith(".json"):
        args = ["python3", ".future/cov100/covlines-js.py", report]
        for s in scopes:
            args += ["--under", rel(s, root)]
        out = subprocess.run(args, capture_output=True, text=True)
        m = re.search(r"\buncovered_lines=(\d+)", out.stdout)
        return int(m.group(1)) if m else None
    want = [rel(s, root) for s in scopes]
    unc = 0
    cur = None
    keep = False
    for line in open(report, encoding="utf-8", errors="replace"):
        if line.startswith("SF:"):
            cur = rel(line.rstrip("\n")[3:], root)
            keep = any(cur == w or cur.startswith(w.rstrip("/") + "/") for w in want)
        elif line.startswith("DA:") and cur is not None and keep:
            p = line.rstrip("\n")[3:].split(",")
            if len(p) >= 2 and p[1] == "0":
                unc += 1
    return unc


def main():
    apply = "--apply" in sys.argv
    only = None
    for i, a in enumerate(sys.argv):
        if a == "--only" and i + 1 < len(sys.argv):
            only = set(sys.argv[i + 1].split(","))
    reg = json.load(open(REG))
    root = os.path.abspath(".")
    pairs = {p["label"]: p for p in reg["pairings"]}
    changed = 0
    failed = []

    for label, plan in PLAN.items():
        if only and label not in only:
            continue
        p = pairs.get(label)
        print(f"=== {label} ===")
        if p is None:
            print("  !! not in the registry - plan is stale")
            failed.append(label)
            continue
        report = plan.get("report") or p["report"]
        scopes = plan.get("scopes") or p["scopes"]
        measured = measure(report, scopes, root)
        print(f"  report   {report}")
        print(f"  scopes   {len(scopes)}")
        if measured is None:
            print("  !! report missing - cannot verify the expectation; NOT writing")
            failed.append(label)
            continue
        ok = measured == plan["expect_uncovered"]
        print(f"  measured {measured}  declared {plan['expect_uncovered']}  "
              f"{'ok' if ok else '!! MISMATCH - plan is stale, NOT writing'}")
        if not ok:
            failed.append(label)
            continue
        # what would change?
        delta = []
        if report != p["report"]:
            delta.append(f"report {p['report']} -> {report}")
        if scopes != p["scopes"]:
            delta.append(f"scopes {len(p['scopes'])} -> {len(scopes)}")
        if plan.get("section") and plan["section"] != p.get("section"):
            delta.append(f"section {p.get('section')!r} -> {plan['section']!r}")
        if plan["expect_uncovered"] != (p.get("expect") or {}).get("uncovered"):
            delta.append(f"expect.uncovered {(p.get('expect') or {}).get('uncovered')} -> {plan['expect_uncovered']}")
        if not delta:
            print("  already repaired")
            continue
        for d in delta:
            print(f"    would change: {d}")
        if apply:
            p["report"] = report
            p["scopes"] = scopes
            if plan.get("section"):
                p["section"] = plan["section"]
            p.setdefault("expect", {})
            p["expect"]["uncovered"] = plan["expect_uncovered"]
            p["expect"]["claims"] = plan["expect_uncovered"]
            changed += 1

    if apply and changed:
        json.dump(reg, open(REG, "w"), indent=2, ensure_ascii=False)
        open(REG, "a").write("\n")
        print(f"\nwrote {REG}: {changed} pairing(s) repaired")
    elif not apply:
        print("\nDRY RUN - re-run with --apply")
    if failed:
        print(f"\nNOT REPAIRED (plan stale or report missing): {', '.join(failed)}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
