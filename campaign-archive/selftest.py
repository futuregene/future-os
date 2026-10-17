#!/usr/bin/env python3
"""Self-test for the STALE half: exit code is the verdict.

Two parts, both run here so one command checks the tool the campaign depends on:

1. `patternprobe.py` -- the accept/reject contract for the `LEDGER_ROW*` patterns
   (24 cases; a false accept is a false-STALE risk, a false reject hides rows).
2. a **fixture pairing** that drives all three report states on purpose
   (`selftest/fixture-doc.md` + `selftest/fixture-lcov.info`): a zero-count line the
   ledger agrees with, two covered lines it still claims, and a line with no record
   at all. The real pairings produce `COVERED 0` everywhere, so without this the
   actionable branch of the split would never execute in the campaign.

Asserts the exact numbers on the printed output, so a change to the split shows up
here rather than in a handoff sentence.

    python3 .future/cov100/selftest.py
"""
import importlib.util
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE_DOC = os.path.join(HERE, "selftest", "fixture-doc.md")
FIXTURE_LCOV = os.path.join(HERE, "selftest", "fixture-lcov.info")
RECONCILE = os.path.join(HERE, "reconcile.py")
PROBE = os.path.join(HERE, "patternprobe.py")

failures = []


def check(ok, what, detail=""):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}"
          + (f" -- {detail}" if detail and not ok else ""))
    if not ok:
        failures.append(what)


def run(args, **kw):
    p = subprocess.run([sys.executable] + args, capture_output=True, text=True, **kw)
    return p


GRADE = os.path.join(HERE, "staletable.py")

sys.path.insert(0, HERE)
import ledgerrows as lr  # noqa: E402  (the shipped pattern/module loader)


def load_staletable():
    """Import the shipped tool as a module so the self-test can stamp/verify in-process.
    Importing (rather than re-implementing) is the point: the checks below then exercise
    the same functions the CLI calls."""
    spec = importlib.util.spec_from_file_location("cov100_staletable", GRADE)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def gate(args=(), expect_rc=0, must_contain=(), must_not_contain=(), why=""):
    p = run([GRADE, "--gate", *args])
    out = p.stdout + p.stderr
    check(p.returncode == expect_rc, f"gate exit {expect_rc} {why}",
          f"rc={p.returncode}")
    for needle in must_contain:
        check(needle in out, f"gate output names: {needle[:52]}", "not found")
    for needle in must_not_contain:
        check(needle not in out, f"gate output does not say: {needle[:52]}", "present")
    return out


def main():
    print("1. pattern contract (patternprobe.py --contrast)")
    p = subprocess.run([sys.executable, PROBE, "--contrast"], capture_output=True,
                       text=True)
    tail = [l for l in p.stdout.splitlines() if l.startswith("cases=")]
    check(p.returncode == 0, "patternprobe exits 0", f"rc={p.returncode}")
    check(bool(tail) and "mismatches=0" in tail[0], "no pattern mismatches",
          tail[0] if tail else "no verdict line")
    check("flipped by the pre-tightening tag class=5" in p.stdout,
          "the contrast run discriminates the tag classes",
          "expected 5 flips")

    print("2. STALE split on the fixture pairing")
    cmd = [sys.executable, RECONCILE, "--label", "selftest",
           "--report", FIXTURE_LCOV,
           "--scope", "/tmp/cov100-selftest/src",
           "--doc", FIXTURE_DOC]
    q = subprocess.run(cmd, capture_output=True, text=True)
    out = q.stdout
    print("    " + out.replace("\n", "\n    ").rstrip())
    want = {
        "uncovered lines=1": "one uncovered line in the fixture scope",
        "mentioned in fixture-doc.md: 1": "the zero-count line is cited",
        "MISSING (uncovered, not mentioned): 0": "nothing missing",
        "STALE (doc cites it as uncovered; old doc sections also match unless "
        "--section is given): 4": "four claimed lines the report disagrees with",
        "COVERED (DA>0 - a real disagreement, the actionable half): 3":
            "the covered lines are split out as COVERED",
        "ABSENT (no `DA:` record -- NOT evidence: a blank line inside a claimed "
        "range, a comment, or a report from another revision): 1":
            "the recordless line is split out as ABSENT",
        "demo.rs:11": "the covered line is listed",
        "gone.rs:5": "F.35: a claim about a file whose uncovered set is EMPTY is still checked",
        "demo.rs:13": "the recordless line is listed as absent",
    }
    for needle, why in want.items():
        check(needle in out, why, f"missing {needle!r}")
    check("demo.rs:10" not in out.split("STALE (")[-1],
          "the agreed line is NOT reported as stale")
    check(q.returncode == 0, "a real disagreement does not fail the gate",
          f"rc={q.returncode}")
    check("WARNING" not in out, "no vacuity warning (rows were parsed)")

    print("3. the empty-scope guard (0/0 must not be mistakable for 100 %)")
    bogus = subprocess.run(
        [sys.executable, RECONCILE, "--label", "selftest-bogus",
         "--report", FIXTURE_LCOV, "--scope", "/definitely/not/here",
         "--doc", FIXTURE_DOC], capture_output=True, text=True)
    check(bogus.returncode == 3, "a scope matching no report record exits 3",
          f"rc={bogus.returncode}")
    check("nothing was measured" in bogus.stderr,
          "the guard says why, on stderr")
    covered_fixture = os.path.join(HERE, "selftest", "fixture-covered.info")
    perfect = subprocess.run(
        [sys.executable, RECONCILE, "--label", "selftest-perfect",
         "--report", covered_fixture, "--scope", "/tmp/cov100-selftest/src",
         "--doc", FIXTURE_DOC], capture_output=True, text=True)
    check(perfect.returncode == 0 and "uncovered lines=0" in perfect.stdout,
          "a genuinely fully-covered scope still exits 0 with 0/0",
          f"rc={perfect.returncode}")

    print("4. the pairing gate (the pairings.json contract)")
    # NOTE: the gate's verdict on the LIVE pairings.json depends on peer docs, which
    # change under this unit (measured: agent-core went clean -> failing because its own
    # unit restructured section 6 mid-run). Asserting a green live gate would make this
    # self-test fail on someone else's edit, so what is asserted here is the gate's
    # *mechanism* (it runs, it summarises, its failures are diagnostic) plus the two
    # fixtures, which this unit owns and which pin both directions.
    live = run([GRADE, "--gate"])
    check(live.returncode in (0, 1), "the gate exits 0 or 1 on the live pairings",
          f"rc={live.returncode}")
    check("authoritative pairings:" in live.stdout,
          "the gate prints its authoritative-pairing summary")
    check(("GATE OK" in live.stdout) != ("GATE FAILED" in live.stdout),
          "the gate says exactly one of GATE OK / GATE FAILED")
    for line in live.stdout.splitlines():
        if line.strip().startswith("FAILING "):
            check("--" in line and ".md" in line,
                  "every live failure names a doc and a reason", line[:80])
    neg = gate(("--pairings", os.path.join(HERE, "selftest", "pairings-negative.json")),
               1,
               ["GATE FAILED: 3 pairing(s)",
                "3 covered rows with no recorded justification",
                "justification quote not found",
                "needle landed on '### Segment 3 — the runs behind §4c'",
                "GATE NOT OK"],
               [],
               "on the three-case negative fixture")
    check("GATE FAILED" in neg, "a failing gate says so in words, not just in the code")
    gate(("--pairings", os.path.join(HERE, "selftest", "pairings-positive.json")), 0,
         ["GATE OK"], ["GATE FAILED"], "on the positive control")

    # drift: a recorded expectation that no longer matches must be visible, and
    # --strict must turn it into a failure (the default contract stays per the todo:
    # only an unjustified covered row fails)
    import tempfile
    data = json.load(open(os.path.join(HERE, "selftest", "pairings-positive.json"),
                          encoding="utf-8"))
    data["pairings"] = data["pairings"][:1]
    data["pairings"][0]["expect"] = {"covered": 99}
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
        json.dump(data, fh)
        drifted = fh.name
    try:
        loose = gate(("--pairings", drifted), 0,
                     ["DRIFT covered: registry recorded 99, measured now 3"],
                     [], "with a drifted expectation")
        check("GATE OK" in loose, "drift alone does not fail the default gate")
        gate(("--pairings", drifted, "--strict"), 1,
             ["GATE FAILED (--strict)", "drifted from"], [], "under --strict")
    finally:
        os.unlink(drifted)

    print("5. the section-needle ambiguity check (rowscan.py --needle)")
    trap = os.path.join(HERE, "selftest", "fixture-trap-doc.md")
    p = subprocess.run([sys.executable, os.path.join(HERE, "rowscan.py"),
                        "--needle", "4c", trap], capture_output=True, text=True)
    out = p.stdout
    check("2 heading(s) contain" in out and "AMBIGUOUS" in out,
          "an ambiguous needle is reported as ambiguous",
          out.strip().splitlines()[0] if out.strip() else "no output")
    p2 = subprocess.run([sys.executable, os.path.join(HERE, "rowscan.py"),
                         "--needle", "4c. Ledger", trap], capture_output=True, text=True)
    check("1 heading(s) contain" in p2.stdout and "AMBIGUOUS" not in p2.stdout,
          "a unique needle is not flagged",
          p2.stdout.strip().splitlines()[0] if p2.stdout.strip() else "no output")

    print("6. the closeout harness's gate step (measure-final.sh)")
    probe = subprocess.run(["bash", os.path.join(HERE, ".measure-probe.sh")],
                           capture_output=True, text=True)
    check(probe.returncode == 0, "the harness probe exits 0",
          f"rc={probe.returncode}: "
          f"{(probe.stderr or probe.stdout)[-200:]}")
    check("HARNESS_EXIT=0" in probe.stdout,
          "the harness exits 0 when every measurement leg is skipped")
    check("captured exit-code line: 1 (want 1)" in probe.stdout,
          "the generated report carries the gate's own exit code")
    check("snapshot file written: 1 (want 1)" in probe.stdout,
          "the harness writes the pairing-gate snapshot")
    check("verify-stamp exit: 0" in probe.stdout,
          "the harness's own snapshot verifies against the tree it was taken from")
    check("gate heading count: 1 (want 1)" in probe.stdout,
          "the report has exactly one gate heading (no sed-filter duplicate)")
    check("HARNESS_EXIT(red gate)=0" in probe.stdout,
          "a RED gate does not block the report (reported step, not precondition)")
    check("gate step exit: 1" in probe.stdout,
          "the red report says exit 1 rather than passing")
    check("FAIL: " not in probe.stdout, "no probe-side check failed")

    print("7. the revision stamp (--stamp / --verify-stamp)")
    # Round-trip on the live pairings: stamp, verify it is clean, then verify a
    # deliberately corrupted copy fails. The corrupt case uses FIXTURE files this unit
    # owns, so no peer doc is touched to test the check.
    stamp_path = os.path.join(HERE, "selftest", "fixture-stamp.json")
    st = load_staletable()
    data = st.load_pairings(os.path.join(HERE, "pairings.json"))
    rec = lr.load_reconcile()
    rows = st.build_rows(rec, data["pairings"])
    stamp = st.build_stamp(rec, data, rows, os.path.join(HERE, "pairings.json"), 0)
    with open(stamp_path, "w", encoding="utf-8") as fh:
        json.dump(stamp, fh)
    ok = subprocess.run([sys.executable, GRADE, "--verify-stamp", stamp_path],
                        capture_output=True, text=True)
    check(ok.returncode == 0 and "OK: every doc and report" in ok.stdout,
          "a fresh stamp verifies against the tree it was taken from",
          f"rc={ok.returncode}")
    check(str(len(stamp["pairings"])) in ok.stdout,
          "the verification echoes how many pairings it covers")

    # a wrong hash must fail, naming the file it moved
    bad = json.loads(json.dumps(stamp))
    bad["pairings"][0]["doc_sha256"] = "deadbeefdeadbeef"
    bad_path = os.path.join(HERE, "selftest", "fixture-stamp-bad.json")
    with open(bad_path, "w", encoding="utf-8") as fh:
        json.dump(bad, fh)
    badr = subprocess.run([sys.executable, GRADE, "--verify-stamp", bad_path],
                          capture_output=True, text=True)
    check(badr.returncode == 1, "a stamp whose doc hash moved exits 1",
          f"rc={badr.returncode}")
    check("MOVED since the snapshot" in badr.stdout and "deadbeef ->" in badr.stdout,
          "the failure names the moved file and both hashes")
    # a named file that no longer exists must fail too, not pass silently
    gone = json.loads(json.dumps(stamp))
    gone["pairings"][0]["doc"] = ".future/cov100/selftest/does-not-exist.md"
    gone_path = os.path.join(HERE, "selftest", "fixture-stamp-gone.json")
    with open(gone_path, "w", encoding="utf-8") as fh:
        json.dump(gone, fh)
    goner = subprocess.run([sys.executable, GRADE, "--verify-stamp", gone_path],
                           capture_output=True, text=True)
    check(goner.returncode == 1 and "MISSING now" in goner.stdout,
          "a stamp naming a vanished file exits 1 rather than passing",
          f"rc={goner.returncode}")
    for p in (stamp_path, bad_path, gone_path):
        if os.path.basename(p) != "fixture-stamp.json":
            os.unlink(p)

    print("8. the CLOSEOUT 5-pre column (--require-named)")
    # The fixtures' docs do not name fixture-lcov.info, so --require-named must fail on
    # them while the identical run without the flag reports the same covered rows and
    # passes: that difference is what proves the flag adds the property rather than
    # replacing the existing checks.
    loose = run([GRADE, "--gate", "--pairings",
                 os.path.join(HERE, "selftest", "pairings-positive.json")])
    check(loose.returncode == 0, "the positive fixture passes without --require-named",
          f"rc={loose.returncode}")
    strict_named = run([GRADE, "--gate", "--require-named", "--pairings",
                        os.path.join(HERE, "selftest", "pairings-positive.json")])
    check(strict_named.returncode == 1,
          "--require-named fails a pairing whose report its doc never names",
          f"rc={strict_named.returncode}")
    check("PAIRING NOT STATED BY THE DOC" in strict_named.stdout
          and "nor in any companion record" in strict_named.stdout,
          "the failure is labelled as an unnamed pairing, with its reason",
          strict_named.stdout[-160:])
    live = run([GRADE])
    check("pairing stated by the doc (CLOSEOUT 5-pre):" in live.stdout,
          "the table reports the 5-pre distribution")
    # 5-pre status is reported, not asserted green for peers: what this checks is that
    # the column exists and parses, and that the two modes agree (item 8's invariant).
    line = next((l for l in live.stdout.splitlines() if "NOT stated" in l), "")
    check("NOT stated" in line,
          "the 5-pre distribution reports an unnamed-pairing count", line)
    check("section " in line and "companion " in line,
          "the distribution separates section / doc / companion / unnamed", line)
    # Consistency, not a green verdict. Asserting "--require-named passes on the live
    # pairings" was a mistake of exactly the kind this unit keeps finding: it makes this
    # self-test's result depend on whether a peer's doc happens to be clean right now, and
    # it failed the first time a doc moved (tauri-shell, 21:05:15 — 19 s before a run).
    # What is asserted is the INVARIANT relating the two modes on the SAME live tree:
    #   * any unnamed pairing makes --require-named exit 1; and
    #   * with none unnamed, --require-named's exit equals the plain gate's exit
    #     (--require-named adds a failure condition, it cannot clear other ones).
    live_named = run([GRADE, "--gate", "--require-named"])
    live_gate = run([GRADE, "--gate"])
    n_unnamed = 0
    if "NOT stated" in line:
        try:
            n_unnamed = int(line.rsplit("NOT stated", 1)[1].strip().strip("*"))
        except ValueError:
            n_unnamed = -1
    check(n_unnamed >= 0, "the 5-pre count parses as a number", f"n={n_unnamed}")
    if n_unnamed > 0:
        check(live_named.returncode == 1,
              "unnamed pairings make --require-named exit 1",
              f"NOT stated {n_unnamed}, rc={live_named.returncode}")
    else:
        check(live_named.returncode == live_gate.returncode,
              "with none unnamed, --require-named matches the plain gate",
              f"gate {live_gate.returncode}, require-named {live_named.returncode}")
    check(live_named.returncode in (0, 1),
          "--require-named exits 0 or 1, never a crash",
          f"rc={live_named.returncode}")

    print("9. the vacuity guards (a report that cannot support the check)")
    # The hole this closes: both halves of the check are computed FROM the report, so a
    # report with no record in the pairing's scope satisfied both trivially. Measured
    # before the fix: `goal` pointed at a desktop-scoped zeroed report -> "92 claims, 0
    # uncovered, PASS, exit 0". These three cases pin it, and the fourth pins that the
    # guard does NOT fire on a legitimately clean pairing.
    base = json.load(open(os.path.join(HERE, "pairings.json")))
    subject = next(p for p in base["pairings"] if p["label"] == "goal")
    for fixture, why in (("/tmp/zeroed.info", "a report with nothing in the pairing's scope"),
                         ("/tmp/zeroed-test.info", "a second such report")):
        if not os.path.exists(fixture):
            continue
        probe = json.loads(json.dumps(subject))
        probe["report"] = fixture
        probe.pop("expect", None)
        path = os.path.join(HERE, "selftest", "pairings-vacuity.json")
        with open(path, "w", encoding="utf-8") as fh:
            json.dump({"version": 1, "pairings": [probe]}, fh)
        r = run([GRADE, "--gate", "--pairings", path])
        check(r.returncode == 1, f"{why} FAILs the gate rather than passing",
              f"rc={r.returncode}")
        check("NOTHING WAS CHECKED" in r.stdout,
              "the failure says nothing was checked, with the reason")
        os.unlink(path)
    # and the guard must not fire on the live, genuinely-clean pairings
    live_gate = run([GRADE, "--gate"])
    check("NOTHING WAS CHECKED" not in live_gate.stdout,
          "no live pairing trips the vacuity guard",
          "a real pairing's report is missing its scope")
    # a report FILE that does not exist is the same class: it must fail loudly with a
    # reason, not measure an empty set
    missing = json.loads(json.dumps(subject))
    missing["report"] = "/tmp/cov100-does-not-exist.info"
    missing.pop("expect", None)
    path = os.path.join(HERE, "selftest", "pairings-missing-report.json")
    with open(path, "w", encoding="utf-8") as fh:
        json.dump({"version": 1, "pairings": [missing]}, fh)
    mr = run([GRADE, "--gate", "--pairings", path])
    check(mr.returncode == 1 and "report missing" in mr.stdout,
          "a nonexistent report file FAILs with the reason",
          f"rc={mr.returncode}")
    os.unlink(path)

    print("10. --suggest (finding the report a moved doc now matches)")
    # Scoped to a single label and only asserted on the mechanism: it must name a
    # candidate the doc itself names, and it must not offer a vacuous (zeroed) report.
    sug = run([GRADE, "--suggest", "--labels", "agent-rpc-r1"])
    check(sug.returncode in (0, 1), "--suggest exits 0 or 1, never a crash",
          f"rc={sug.returncode}")
    check("CANDIDATE" in sug.stdout or "no clean candidate" in sug.stdout,
          "--suggest reports candidates or says none exist", sug.stdout[-160:])
    check("zeroed" not in sug.stdout,
          "--suggest does not offer a zeroed report as a clean candidate",
          sug.stdout[-200:])

    print("11. the section slice is fence-aware (a `#` inside a fence is a comment)")
    # F.70's regression control. A record that pastes example output like
    #     # -> files with uncovered lines=202
    #     #    MISSING (uncovered, not mentioned): 0
    # inside a fence must NOT have its slice terminated by that comment, and a fenced
    # example of a HEADING must not start the slice in the wrong place. Both are
    # asserted here because the previous implementation was blind to fences at BOTH
    # ends, and the failure mode is silent phantom MISSING.
    import importlib.util as _ilu
    _spec = _ilu.spec_from_file_location("reconcile", RECONCILE)
    _rec = _ilu.module_from_spec(_spec)
    _spec.loader.exec_module(_rec)
    _doc = (
        "## 0.5 Residual ledger\n\nHow this is checked:\n\n"
        "```\npython3 reconcile.py --section \"Residual ledger\"\n"
        "# -> files with uncovered lines=43 uncovered lines=202\n"
        "#    MISSING (uncovered, not mentioned): 0\n```\n\n"
        "| row | line |\n|---|---|\n| `mod.rs` 12 | covered |\n"
    )
    _sliced = _rec.slice_section(_doc, "Residual ledger") or ""
    check("mod.rs" in _sliced,
          "a fence-level `#` comment does not truncate the slice",
          f"slice={len(_sliced)} chars")
    _doc2 = (
        "## 1. Real\n\ntext\n\n```\n## 2. Second real section (a fenced EXAMPLE of it)\n```\n\n"
        "## 2. Second real section\n\nbody two\n"
    )
    _s2 = _rec.slice_section(_doc2, "2. Second real section") or ""
    check("body two" in _s2 and "fenced EXAMPLE" not in _s2,
          "a fenced example heading does not start the slice",
          f"slice={len(_s2)} chars")

    print("12. an unpaired fence is reported as a FENCE, not as a missing heading")
    # F.72's control. With fence tracking (section 11 above), one stray ``` makes every
    # later heading invisible; the pre-guard slicer returned None and the caller said
    # "no heading contains X", which blames the wrong thing and hides that the check
    # stopped covering the document.
    _unbal = ("## 0. Intro\n\ntext\n\n```\nnever closed\n\n"
              "## 0.5 Ledger\n\n| a | b |\n|---|---|\n| `x.rs` 1 | 0 |\n")
    try:
        _rec.slice_section(_unbal, "0.5 Ledger")
        check(False, "an odd fence count raises instead of returning None",
              "no exception raised")
    except _rec.DocFenceError as _e:
        check("ODD" in str(_e) and "never closed" in str(_e),
              "an odd fence count raises and names the fence line", str(_e)[:110])
    # and it must NOT raise when the section is reachable BEFORE the stray fence
    _unbal_after = ("## 0.5 Ledger\n\n| a | b |\n|---|---|\n| `x.rs` 1 | 0 |\n\n"
                   "```\nnever closed\n")
    try:
        _s = _rec.slice_section(_unbal_after, "0.5 Ledger") or ""
        check("x.rs" in _s,
              "a stray fence AFTER the section does not raise it (no false alarm)",
              f"slice={len(_s)} chars")
    except _rec.DocFenceError as _e:
        check(False, "a stray fence AFTER the section does not raise it (no false alarm)",
              str(_e)[:110])

    print("13. --suggest --labels narrows BEFORE measuring (F.74)")
    # The filter used to run after build_rows(), which globs and MEASURES every /tmp
    # candidate per pairing - so `--suggest --labels X` cost the same as the full run
    # (measured: still >200 s for a single, already-clean label). Two deterministic
    # assertions on the new path: an unknown label is refused, and a clean one returns
    # without candidates.
    _bad = run([GRADE, "--suggest", "--labels", "no-such-label-exists"])
    check(_bad.returncode == 1 and "no pairing labelled" in _bad.stdout + _bad.stderr,
          "an unknown --labels value is refused (exit 1, named)",
          (_bad.stdout + _bad.stderr).strip()[:110])
    _clean = run([GRADE, "--suggest", "--labels", "desktop"])
    check("clean candidates found: 0" in _clean.stdout,
          "a clean label returns no candidates (its pairing is skipped)",
          _clean.stdout.strip()[-110:])

    print()
    if failures:
        print(f"SELFTEST FAILED: {len(failures)} check(s) -- "
              + "; ".join(failures))
        return 1
    print("SELFTEST OK: pattern contract + all three STALE states + the gate contract, "
          "exit 0")
    return 0


if __name__ == "__main__":
    sys.exit(main())
