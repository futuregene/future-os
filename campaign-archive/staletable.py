#!/usr/bin/env python3
"""Every ledger pairing, measured from `pairings.json`, with a gate.

`pairings.json` records, per invocation: the report, the scopes, the doc, the section
the DOC ITSELF declares current, the heading that needle must resolve to, where the
invocation is documented, and what the measurement is expected to be. This tool reads
that file (no built-in table), measures every pairing, and can act as a gate.

Two things it exists to prevent, both observed in this campaign:

1. **A wide invocation reported as a verdict.** `module-cli.md` keeps three ledgers
   (§4 S3, §4b S5, §4c S7); running the whole doc compares two superseded ledgers with
   one report and prints ~43 "STALE" rows. They are history, and the fix is to name
   the authoritative section -- `reconcile.py` now prints which section the covered
   rows came from.
2. **A needle that resolves to prose, not the ledger.** `--section "4c"` on
   `module-cli.md` matches `### Segment 3 (mock-span hygiene) — the three runs behind
   §4c`, a cross-reference ~500 lines above the ledger: the slice ends up a real
   section that is simply not a ledger, and the run reads `mentioned: 0`. Each
   pairing therefore records the heading its needle must land on, and the gate fails
   when it lands somewhere else.

Gate contract (`--gate`), per pairing:

* COVERED == 0            -> PASS (nothing to justify)
* COVERED > 0, no `covered_justification` recorded
                          -> **FAIL** (a disagreement nobody has explained)
* COVERED > 0, justified  -> PASS only if every quoted fragment is found verbatim in
   the doc *and* every covered row's own section is one of the justified sections.
   A quote that no longer exists, or a covered row in an unexpected section, FAILs.
* `expect` drift          -> reported as DRIFT; a failure only with `--strict`.
* needle lands on the wrong heading
                          -> **FAIL** (the slice is not the ledger it claims to be)

    python3 .future/cov100/staletable.py                 # the table
    python3 .future/cov100/staletable.py --cli           # + run reconcile.py per pairing
    python3 .future/cov100/staletable.py --gate           # exit 0/1, the contract above
    python3 .future/cov100/staletable.py --gate --strict  # ... and fail on drift too
    python3 .future/cov100/staletable.py --pairings FILE  # for self-test fixtures
"""
import argparse
import datetime
import glob
import hashlib
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
import ledgerrows as lr  # noqa: E402

DEFAULT_PAIRINGS = os.path.join(HERE, "pairings.json")


def load_pairings(path):
    with open(path, encoding="utf-8") as fh:
        data = json.load(fh)
    for p in data["pairings"]:
        p["scopes"] = [s.replace("${ROOT}", ROOT) for s in p.get("scopes", [])]
        for key in ("report", "doc"):
            p[key] = p[key].replace("${ROOT}", ROOT)
    return data


def norm(text):
    """Whitespace-collapsed, blockquote-marker-stripped text, for quote matching.

    Ledger quotes in this repo wrap across lines and often sit inside a `>` blockquote
    (`` ...supersedes §1–§9... ``, ``the crate now reads **8 uncovered lines of\n> 25 841
    = ...``). Without stripping the marker, a genuine quote fails to match because a
    `> ` lands in the middle of it after collapsing.
    """
    lines = [re.sub(r"^\s*>\s?", "", l) for l in text.splitlines()]
    return re.sub(r"\s+", " ", " ".join(lines)).strip()


def measure(rec, report, scopes, doc, section):
    text = open(doc, encoding="utf-8", errors="replace").read()
    sliced = rec.slice_section(text, section) if section else text
    if sliced is None:
        return None
    heading = (sliced.splitlines() or [""])[0]
    per_file = rec.key_files(rec.report_lines(report, scopes), scopes)
    counts = rec.report_counts(report, scopes)
    claims = rec.claimed_uncovered(sliced)
    cited = rec.cited_numbers(sliced)
    missing = [(k, ln) for k in per_file for ln in per_file[k]
               if k not in sliced or ln not in cited]
    # ONE implementation of the STALE resolver, shared with reconcile.py. This file
    # carried its own copy, which still had the "a fully-covered file is skipped"
    # blind spot - so the gate and the per-unit tool could disagree about the same doc.
    stale, _counts_all = rec.stale_candidates(sliced, report, scopes)
    covered, absent = rec.split_stale(stale, counts)
    by_sec = rec.claims_by_section(sliced)
    attributed = {}
    for key, ln in covered:
        for head in rec.sections_claiming(by_sec, key, ln):
            attributed[head] = attributed.get(head, 0) + 1
    return {
        "sliced": sliced, "heading": heading, "per_file": per_file,
        "claims": len(claims), "uncovered": sum(len(v) for v in per_file.values()),
        "missing": missing, "covered": covered, "absent": absent,
        "attributed": attributed,
    }


def check_pairing(rec, p):
    """Measure one pairing and return (verdict, details). verdict: PASS|FAIL."""
    problems, notes = [], []
    if not os.path.exists(p["report"]):
        return "FAIL", {"problems": [f"report missing: {p['report']}"], "notes": notes}
    if not os.path.exists(p["doc"]):
        return "FAIL", {"problems": [f"doc missing: {p['doc']}"], "notes": notes}
    try:
        m = measure(rec, p["report"], p["scopes"], p["doc"], p.get("section"))
    except rec.DocFenceError as error:
        # F.72: an unpaired fence is a DOCUMENT defect that makes this check
        # inoperative for the doc. Report that cause rather than "no heading
        # contains X", which sends the reader hunting for a missing heading.
        return "FAIL", {"problems": [str(error)], "notes": notes}
    if m is None:
        return "FAIL", {"problems": [f"no heading contains {p.get('section')!r}"],
                        "notes": notes}
    m["verdict"] = "PASS"

    want_head = p.get("section_heading")
    if want_head and m["heading"] != want_head:
        problems.append(f"needle landed on {m['heading']!r}, not the recorded "
                        f"{want_head!r}")

    # a needle that matches several headings is a standing hazard, even when this
    # run landed on the right one
    if p.get("section"):
        heads = rec.sections_matching(open(p["doc"], encoding="utf-8").read(),
                                      p["section"])
        if len(heads) > 1:
            notes.append(f"needle matches {len(heads)} headings")

    n_cov = len(m["covered"])

    # VACUITY GUARDS (added after `--suggest` offered /tmp/zeroed*.info as a "clean
    # candidate", and a follow-up probe showed the gate itself said PASS for it).
    #
    # Both halves of this check are computed FROM the report, so a report that supports
    # neither comparison satisfies both trivially. Measured: the `goal` pairing pointed at
    # /tmp/zeroed.info (a desktop-scoped report) reported "92 claims, 0 uncovered, PASS,
    # exit 0" -- this gate could be made green by handing it a report with nothing in
    # scope, which is the failure mode the campaign's tools were built to refuse.
    #
    # (A) no record in scope -> nothing was checked. This is the exact condition
    #     `reconcile.py` exits 3 on; this tool called the loader directly and inherited a
    #     green instead of the guard.
    # (B) records exist but the scope reads as fully covered while the ledger cites lines
    #     as uncovered -> the report is zeroed/truncated, or the unit is genuinely at
    #     100 % and its ledger should be emptied. Both are worth a failure, neither a PASS.
    if not m["per_file"]:
        problems.append(
            f"NOTHING WAS CHECKED: {os.path.basename(p['report'])} has no record for any "
            f"of this pairing's {len(p['scopes'])} scopes, so both the MISSING and the "
            f"COVERED comparisons are vacuous. reconcile.py exits 3 on exactly this; a "
            f"report whose scope does not match (or a zeroed/truncated one) must not "
            f"read as a pass.")
    elif m["uncovered"] == 0 and m["claims"]:
        resolving = 0
        for path, ln in sorted(rec.claimed_uncovered(m.get("sliced") or "")):
            if rec.claim_targets(path, ln, m["per_file"]):
                resolving += 1
        if resolving:
            problems.append(
                f"REPORT CANNOT SUPPORT THE CHECK: {os.path.basename(p['report'])} "
                f"shows 0 uncovered lines in scope while the ledger cites {resolving} "
                f"lines as uncovered. A zeroed/truncated report reads that way, and the "
                f"campaign's tools exit 3 on it -- so this pairing cannot be green on "
                f"it. If the unit is genuinely at 100 %, clear its ledger instead.")
    m["claims_resolving"] = None
    if n_cov:
        j = p.get("covered_justification")
        if not j:
            problems.append(f"{n_cov} covered rows with no recorded justification")
        else:
            doc_text = norm(open(p["doc"], encoding="utf-8", errors="replace").read())
            for q in j.get("quotes", []):
                if norm(q) not in doc_text:
                    problems.append(f"justification quote not found in {p['doc']}: "
                                    f"{q[:70]!r}")
            allowed = j.get("sections", [])
            for head, n in sorted(m["attributed"].items(), key=lambda kv: -kv[1]):
                if not any(head.startswith(a) or a.startswith(head)
                           or a in head for a in allowed):
                    problems.append(f"{n} covered rows come from {head!r}, which is "
                                    f"not among the justified sections")
            notes.append(f"{n_cov} covered rows justified by {len(allowed)} section(s)")

    report = p["report"]
    named = None
    try:
        doc_text = open(p["doc"], encoding="utf-8", errors="replace").read()
    except OSError:
        doc_text = ""
    named = named_by(rec, m.get("sliced") or "", doc_text, report, p)
    m["named_by"] = named
    if str(named).startswith("none"):
        notes.append(f"PAIRING NOT STATED BY THE DOC: {os.path.basename(report)} is not "
                     f"named in {os.path.basename(p['doc'])} (sound only because MISSING "
                     f"is {len(m.get('missing', []))}); CLOSEOUT 5-pre makes that an "
                     f"exit-4 condition for its own tool, so either the doc gains the "
                     f"citation or this pairing is labelled reconstructed")
    elif str(named).startswith("via:") and "UNVERIFIED" in str(named):
        notes.append(f"named_in does not actually name {os.path.basename(report)}: "
                     f"{p.get('named_in')}")

    exp = p.get("expect")
    if exp:
        got = {"uncovered": m["uncovered"], "claims": m["claims"],
               "missing": len(m["missing"]), "covered": n_cov, "absent": len(m["absent"])}
        for k, v in exp.items():
            if got.get(k) != v:
                # The word "recorded" means THE REGISTRY's expect block, not the record's claim
                # (F.77). Reading it the other way sends a rework after a record that is already
                # correct while the stale artefact - pairings.json - is left alone. Say so in the line.
                notes.append(f"DRIFT {k}: registry recorded {v}, measured now {got[k]} "
                             f"(a registry/expect lag unless the record itself moved)")
    if p.get("caveat"):
        # a recorded limit on what this pairing proves -- printed in both modes so it
        # cannot be lost behind a green verdict
        notes.append("CAVEAT " + p["caveat"])
    if p.get("doc_cited_report") and not os.path.exists(p["doc_cited_report"]):
        notes.append(f"the doc cites {p['doc_cited_report']}, which is not on disk")
    # A pairing can be provably unverifiable rather than merely failing: the doc's own
    # report is absent. That is a fact about the artifact, so it is checked, not
    # asserted -- and it excuses the row only when the caller opts in, so a missing
    # report cannot silently become a pass.
    blocker = p.get("unverifiable_until")
    m["unverifiable"] = bool(blocker) and not os.path.exists(blocker)
    if m["unverifiable"]:
        notes.append(f"UNVERIFIABLE until {blocker} exists")
    elif blocker:
        problems.append(f"{blocker} now exists: re-measure and drop `unverifiable_until`")
    m["problems"], m["notes"] = problems, notes
    return ("FAIL" if problems else "PASS"), m


def build_rows(rec, pairings):
    rows = []
    for p in pairings:
        verdict, m = check_pairing(rec, p)
        rows.append((p, verdict, m))
    return rows


def print_table(rows):
    w = max(len(p["label"]) for p, _, _ in rows)
    print(f"{'pairing':{w}s} {'role':16s} {'doc':28s} {'claims':>6s} {'uncov':>6s} "
          f"{'STALE':>6s} {'ABSENT':>6s} {'COVERED':>7s} {'named':12s}  verdict")
    for p, verdict, m in rows:
        n_stale = len(m.get("covered", [])) + len(m.get("absent", []))
        named = str(m.get("named_by") or "-")
        if named.startswith("via:"):
            named = "companion"
        print(f"{p['label']:{w}s} {p['role']:16s} {os.path.basename(p['doc']):28s} "
              f"{m.get('claims', 0):6d} {m.get('uncovered', 0):6d} {n_stale:6d} "
              f"{len(m.get('absent', [])):6d} {len(m.get('covered', [])):7d} "
              f"{named:12s}  {verdict}")
        for note in m.get("notes", []):
            print(f"    note: {note}")
        for prob in m.get("problems", []):
            print(f"    PROBLEM: {prob}")
        if p.get("covered_justification") and m.get("covered"):
            j = p["covered_justification"]
            print(f"    justified: {j['reason'].split('. ')[0][:100]}.")
            print(f"    quoted:    {j['quotes'][0][:100]!r}")


def print_markdown(rows, pairings_path, gate_exit, skipped=()):
    """The verdict table as markdown, for the closeout report.

    Written for `measure-final.sh` to append verbatim: a red gate is a REPORTED row,
    not a reason to abort the harness, so the exit code is printed as data."""
    sk = {p["label"] for p, _m in skipped}
    print()
    print("## Pairing gate (`staletable.py --gate`)")
    print()
    print(f"Command: `python3 .future/cov100/staletable.py --gate`  "
          f"(pairings: `{os.path.relpath(pairings_path, ROOT)}`)  "
          f"**exit code: {gate_exit}**")
    print()
    print("A pairing is one (report, scope, doc, section) the STALE half can check. "
          "`COVERED` counts claimed lines the report shows as covered; every one must "
          "be attributed to a section the pairing record justifies, or the gate fails. "
          "`ABSENT` (no `DA:` record at all) is not evidence.")
    print()
    print("| pairing | role | doc | claims | uncovered | STALE | COVERED | ABSENT | verdict |")
    print("|---|---|---|---|---|---|---|---|---|")
    for p, verdict, m in rows:
        stale = len(m.get("covered", [])) + len(m.get("absent", []))
        label = p["label"] + (" (skipped)" if p["label"] in sk else "")
        print(f"| `{label}` | {p['role']} | `{p['doc']}` | {m.get('claims', 0)} | "
              f"{m.get('uncovered', 0)} | {stale} | {len(m.get('covered', []))} | "
              f"{len(m.get('absent', []))} | {verdict} |")
    print()
    fails = [(p, m) for p, v, m in rows if v == "FAIL"]
    if fails:
        print("**Failing pairings** (a ledger that disagrees with the report it cites):")
        print()
        for p, m in fails:
            print(f"* `{p['label']}` — {p['doc']}")
            for prob in m["problems"]:
                print(f"  * {prob}")
        print()
    for p, _v, m in rows:
        if p.get("covered_justification") and m.get("covered"):
            print(f"* `{p['label']}` carries {len(m['covered'])} covered rows, justified "
                  f"in `pairings.json`: {p['covered_justification']['reason']}")
    print()
    if not fails:
        print("Every covered row is attributed to a justified section.")


def cli_run(rec, p, log_fh):
    cmd = [sys.executable, os.path.join(HERE, "reconcile.py"),
           "--label", p["label"], "--report", p["report"]]
    for s in p["scopes"]:
        cmd += ["--scope", s]
    cmd += ["--doc", p["doc"]]
    if p.get("section"):
        cmd += ["--section", p["section"]]
    log_fh.write("$ " + " ".join(cmd) + "\n")
    proc = subprocess.run(cmd, capture_output=True, text=True)
    log_fh.write(proc.stdout)
    if proc.stderr:
        log_fh.write(proc.stderr)
    log_fh.write(f"exit={proc.returncode}\n\n")
    return proc.returncode


class HostStall(Exception):
    """The HOST stalled on a trivial operation (INCIDENTS F.75).

    Measured 2026-10-17: with `/usr/libexec/syspolicyd` wedged at ~1000 % CPU, `import
    hashlib` hung 8 of 8 times (the import loads `libcrypto` and the wedged policy daemon
    stalls that load), and builds stalled outright. This class turns such a stall into a
    named error so the stamp can say "the host could not be checked" instead of hanging
    forever - the campaign's rule for anything that would otherwise fail silently.
    """


# Filled on first successful import; see sha256().
_hashlib = None
STALL_TIMEOUT_SECS = int(os.environ.get("FUTURE_HASH_TIMEOUT", "25"))


def _require_hashlib():
    """Import hashlib under an alarm, so a stalled dylib load becomes HostStall."""
    global _hashlib
    if _hashlib is not None:
        return _hashlib
    import signal as _signal

    def _timeout(signum, frame):  # pragma: no cover - host-fault path
        raise HostStall(
            f"importing hashlib stalled for {STALL_TIMEOUT_SECS}s (it loads libcrypto; a "
            f"wedged syspolicyd makes that load hang - INCIDENTS F.75). The host cannot "
            f"be hashed/verified right now; a machine restart is the usual remedy.")

    previous = _signal.signal(_signal.SIGALRM, _timeout)
    _signal.alarm(STALL_TIMEOUT_SECS)
    try:
        import hashlib as _h
        _hashlib = _h
    finally:
        _signal.alarm(0)
        _signal.signal(_signal.SIGALRM, previous)
    return _hashlib


def sha256(path, timeout=STALL_TIMEOUT_SECS):
    """16-hex digest of a file, or None if it cannot be read.

    Raises HostStall (rather than hanging) if the hasher cannot be loaded in `timeout`
    seconds, and likewise if reading the file itself stalls - both observed on a wedged
    host.
    """
    import signal as _signal
    hasher = _require_hashlib()

    def _timeout(signum, frame):  # pragma: no cover - host-fault path
        raise HostStall(
            f"hashing {path} stalled for {timeout}s - the host is not doing I/O or "
            f"library loads in reasonable time (INCIDENTS F.75).")

    previous = _signal.signal(_signal.SIGALRM, _timeout)
    _signal.alarm(timeout)
    try:
        try:
            with open(path, "rb") as fh:
                return hasher.sha256(fh.read()).hexdigest()[:16]
        except OSError:
            return None
    finally:
        _signal.alarm(0)
        _signal.signal(_signal.SIGALRM, previous)


def git_rev():
    out = {}
    for key, cmd in (("rev", ["git", "rev-parse", "--short", "HEAD"]),
                     ("dirty", ["git", "status", "--porcelain"])):
        try:
            p = subprocess.run(cmd, capture_output=True, text=True, check=False)
            out[key] = p.stdout.strip()
        except OSError:
            out[key] = None
    out["dirty_paths"] = len(out["dirty"].splitlines()) if out.get("dirty") else 0
    return out


def build_stamp(rec, data, rows, pairings_path, gate_exit):
    """A snapshot that says WHICH revision the verdicts describe.

    Without this the gate's green says nothing reproducible: the recorded `expect`
    numbers are compared against docs that a peer can edit a second later (measured:
    `module-cli.md` was written at 20:54:19, one second after a green run), so a reader
    at closeout cannot tell whether the verdict described the frozen tree or an
    intermediate one. The stamp records, per pairing, the doc's and report's sha256 +
    size, plus what was measured; and, at the top level, the git revision, the tree's
    dirtiness, and **which docs no pairing covers at all** — the snapshot's own gap
    statement, since a green gate over verified docs says nothing about the others.
    """
    checked_docs = {os.path.relpath(p["doc"], ROOT) for p in data["pairings"]}
    all_docs = sorted(os.path.relpath(p, ROOT)
                      for p in glob.glob(os.path.join(ROOT, "docs/testing/module-*.md"))
                      + glob.glob(os.path.join(ROOT,
                                               "docs/architecture/channels-test-coverage.md")))
    stamp = {
        "kind": "pairing-gate-snapshot",
        "when": datetime.datetime.now().astimezone().isoformat(timespec="seconds"),
        "git": git_rev(),
        "pairings_file": os.path.relpath(pairings_path, ROOT),
        "pairings_sha256": sha256(pairings_path),
        "gate_exit": gate_exit,
        "docs_not_covered_by_any_pairing": [d for d in all_docs if d not in checked_docs],
        "pairings": [],
    }
    for p, verdict, m in rows:
        try:
            size = os.path.getsize(p["report"])
        except OSError:
            size = None
        stamp["pairings"].append({
            "label": p["label"], "role": p["role"],
            "doc": os.path.relpath(p["doc"], ROOT), "doc_sha256": sha256(p["doc"]),
            "report": (os.path.relpath(p["report"], ROOT)
                       if p["report"].startswith(ROOT) else p["report"]),
            "report_sha256": sha256(p["report"]), "report_size": size,
            "section": p.get("section"), "verdict": verdict,
            "measured": {"uncovered": m.get("uncovered"), "claims": m.get("claims"),
                         "missing": len(m.get("missing") or []),
                         "covered": len(m.get("covered") or []),
                         "absent": len(m.get("absent") or [])},
        })
    return stamp


def verify_stamp(stamp, rec, pairings_by_label):
    """Re-hash everything the stamp names. Returns (moved, missing) lists.

    This is the check a closeout can run to prove the recorded verdicts describe the
    tree on disk now — the thing `expect` blocks cannot do on their own.
    """
    moved, missing = [], []
    def check(kind, label, rel, want):
        path = rel if os.path.isabs(rel) else os.path.join(ROOT, rel)
        now = sha256(path)
        if now is None:
            missing.append(f"{kind} {label}: {rel}")
        elif want and now != want:
            moved.append(f"{kind} {label}: {rel} ({want} -> {now})")
    for entry in stamp["pairings"]:
        check("doc", entry["label"], entry["doc"], entry.get("doc_sha256"))
        check("report", entry["label"], entry["report"], entry.get("report_sha256"))
    check("pairings-file", "-", stamp["pairings_file"], stamp.get("pairings_sha256"))
    del rec, pairings_by_label
    return moved, missing


def named_by(rec, sliced, doc_text, report, p):
    """Does the doc ITSELF name the report this pairing checks it against?

    The supervisor's CLOSEOUT §5-pre requires exactly this ("which report does the ledger
    section itself name? ... a check against a report the ledger does not cite
    manufactures false findings"), after their own agent-core audit produced phantom
    'covered' rows that way — the same failure this unit hit twice (agent-core's §6 was
    compared against `agb-final3.info` while its unit had moved to `final6`; cli's §4c is
    numbered per S9 while the pairing used S7). Both were resolved by pointing the pairing
    at the report the doc's own text names.

    Returns:
      'section'          the ledger slice names it
      'doc'              the doc names it elsewhere
      'via:<file>'       a companion record names it (from the pairing's `named_in`)
      'none'             a RECONSTRUCTED pairing: sound when MISSING is 0, but a pairing
                         the docs do not state — the shape CLOSEOUT 5-pre rejects
    """
    del rec
    base = os.path.basename(report)
    words = [q for q in (p.get("provenance_quotes") or [])] + [base]
    if not any(base in doc_text or w in doc_text for w in words):
        src = p.get("named_in")
        if not src:
            return "none"
        rel = src.split(" ")[0]
        path = rel if os.path.isabs(rel) else os.path.join(ROOT, rel)
        try:
            sibling = open(path, encoding="utf-8", errors="replace").read()
        except OSError:
            sibling = ""
        if base in sibling:
            return f"via:{os.path.basename(path)}"
        return f"via:{os.path.basename(path)} (UNVERIFIED: {base} not found there)"
    # the slice names it (directly, or via a quote the pairing recorded from that slice)
    if base in sliced or any(w and w in sliced for w in words):
        return "section"
    return "doc"


def suggest_reports(rec, p, m, patterns=("/tmp/*.info", "/tmp/*.json"),
                    limit=None):
    """Candidate reports that make this pairing CLEAN (missing 0 and covered 0).

    The recurring cost this removes: when a unit re-measures, its doc's numbering moves
    to a new report and the pairing goes red, and the diagnosis was a hand-run of the
    measurement against every candidate on disk (done that way five times in this
    campaign: agent-core twice, cli, tauri-remote, tauri-shell). One command instead.

    Ranking: newest first, because the doc was edited after the report it cites. The
    result is a SUGGESTION to verify by eye -- a report can be clean by coincidence
    (a smaller scope matching loosely), so the caller still reads the numbers.
    """
    doc = p["doc"]
    text = open(doc, encoding="utf-8", errors="replace").read()
    sliced = rec.slice_section(text, p["section"]) if p.get("section") else text
    if sliced is None:
        return []
    claims = rec.claimed_uncovered(sliced)
    cited = rec.cited_numbers(sliced)
    cands = []
    for pat in patterns:
        cands += glob.glob(pat)
    out = []
    for rep in sorted(set(cands)):
        if rep == p["report"]:
            continue
        try:
            pf = rec.key_files(rec.report_lines(rep, p["scopes"]), p["scopes"])
        except Exception:
            # This scans /tmp, which holds arbitrary JSON (goal event logs, ledger
            # dumps, other units' probes). A non-Istanbul .json raises AttributeError
            # in the Istanbul reader -- caught here on purpose: a SCAN must skip what it
            # cannot read, while the main path stays loud about an explicit --report.
            continue
        if not pf:
            continue
        counts = rec.report_counts(rep, p["scopes"])
        uncovered = {(k, ln) for k in pf for ln in pf[k]}
        missing = [1 for k, ln in uncovered if k not in sliced or ln not in cited]
        stale = []
        for path, ln in sorted(claims):
            t = rec.claim_targets(path, ln, pf)
            if not t:
                continue
            if all(pf[x] and ln not in pf[x] for x in t):
                stale.append((t[0], ln))
        cov, absent = rec.split_stale(stale, counts)
        if not missing and not cov:
            # A clean candidate must also be able to SUPPORT the check: with no record in
            # scope both comparisons are vacuous (the guards in check_pairing exist for
            # this; /tmp/zeroed*.info first appeared here as a "clean candidate").
            if not pf or not uncovered:
                continue
            resolving = 0
            for path, ln in sorted(claims):
                t = rec.claim_targets(path, ln, pf)
                if t and any(ln in pf[x] for x in t):
                    resolving += 1
            if not resolving:
                continue
            out.append({"report": rep, "uncovered": len(uncovered),
                        "claims": len(claims), "resolving": resolving,
                        "absent": len(absent), "mtime": os.path.getmtime(rep),
                        "named": os.path.basename(rep) in text})
    out.sort(key=lambda r: (-r["named"], -r["mtime"]))
    del m
    return out[:limit] if limit else out


def _main_guarded():
    """Run main(), turning a host stall into a clear message and exit 3."""
    try:
        return main()
    except HostStall as error:
        print(f"HOST STALL (INCIDENTS F.75): {error}", file=sys.stderr)
        print("  This is a MACHINE fault, not a repo one: the stamp cannot be produced or "
              "verified on this host state.", file=sys.stderr)
        return 3


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pairings", default=DEFAULT_PAIRINGS)
    ap.add_argument("--md", action="store_true",
                    help="emit the verdict table as markdown for the closeout "
                         "report; the gate verdict is printed as data and the exit "
                         "code still reflects it")
    ap.add_argument("--gate", action="store_true",
                    help="exit 1 when a pairing has unexplained covered rows, a "
                         "vanished justification quote, an unattributed section, or "
                         "a mis-resolved section needle")
    ap.add_argument("--strict", action="store_true",
                    help="with --gate: recorded/measured drift is a failure too")
    ap.add_argument("--skip-unverifiable", action="store_true",
                    help="with --gate: exclude pairings whose doc cites a report that is "
                         "not on disk. They are printed as SKIPPED with the missing "
                         "artifact, never as passes; without this flag they FAIL")
    ap.add_argument("--require-named", action="store_true",
                    help="with --gate: also fail when a pairing's report is not named by "
                         "the doc OR a companion record it points at (CLOSEOUT 5-pre's "
                         "exit-4 condition, applied to this table)")
    ap.add_argument("--suggest", action="store_true",
                    help="for every pairing whose verdict is not clean, scan the "
                         "candidate reports on disk and print those that WOULD make it "
                         "clean (missing 0, covered 0). Suggestion, not a rewrite: "
                         "verify the numbers, then re-point pairings.json")
    ap.add_argument("--labels", default=None,
                    help="with --suggest: comma-separated pairing labels to limit to")
    ap.add_argument("--cli", action="store_true",
                    help="also run reconcile.py per pairing and write the log")
    ap.add_argument("--stamp", metavar="FILE",
                    help="write a snapshot of this run: git revision, tree dirtiness, "
                         "and per pairing the doc's and report's sha256 plus what was "
                         "measured. Use at freeze time so a later reader can prove "
                         "which revision the verdicts describe")
    ap.add_argument("--verify-stamp", metavar="FILE",
                    help="re-hash every doc and report a snapshot names and FAIL if any "
                         "moved. This is the closeout precondition: a green gate says "
                         "nothing about which docs it was green on")
    args = ap.parse_args()

    if args.verify_stamp:
        with open(args.verify_stamp, encoding="utf-8") as fh:
            stamp = json.load(fh)
        rec = lr.load_reconcile()
        moved, missing = verify_stamp(stamp, rec, None)
        print(f"stamp: {args.verify_stamp}  ({stamp.get('when')}, "
              f"git {stamp.get('git', {}).get('rev')}, "
              f"{stamp.get('git', {}).get('dirty_paths')} dirty paths, "
              f"gate exit {stamp.get('gate_exit')}, {len(stamp.get('pairings', []))} pairings)")
        if moved:
            print(f"MOVED since the snapshot ({len(moved)}):")
            for line in moved:
                print(f"    {line}")
        if missing:
            print(f"MISSING now ({len(missing)}):")
            for line in missing:
                print(f"    {line}")
        if not moved and not missing:
            print("OK: every doc and report the snapshot names is unchanged -- the "
                  "recorded verdicts describe this tree.")
        return 1 if (moved or missing) else 0

    data = load_pairings(args.pairings)
    rec = lr.load_reconcile()
    # F.74: with --suggest + --labels, filter BEFORE measuring. `build_rows` globs and
    # MEASURES every candidate for pairing (4-8 s each), so filtering afterwards meant
    # `--suggest --labels X` cost the same as the full run (measured: it still exceeded
    # 200 s for a single, already-clean label). Narrowing only helps if it skips the work.
    _pairings = data["pairings"]
    if args.suggest and args.labels:
        _wanted = {x.strip() for x in args.labels.split(",") if x.strip()}
        _keep = [p for p in _pairings if p.get("label") in _wanted]
        if not _keep:
            print(f"no pairing labelled {', '.join(sorted(_wanted))}", file=sys.stderr)
            return 1
        _pairings = _keep
    rows = build_rows(rec, _pairings)
    if args.stamp:
        fails_st = [(p, m) for p, v, m in rows if v == "FAIL"]
        dirty_st = [n for _p, _v, m in rows for n in m.get("notes", [])
                    if n.startswith("DRIFT")]
        exit_code = 1 if (fails_st or (args.strict and dirty_st)) else 0
        stamp = build_stamp(rec, data, rows, args.pairings, exit_code)
        with open(args.stamp, "w", encoding="utf-8") as fh:
            json.dump(stamp, fh, indent=2, ensure_ascii=False)
        g = stamp["git"]
        print(f"wrote {args.stamp}")
        print(f"  git {g.get('rev')} ({g.get('dirty_paths')} dirty paths), "
              f"gate exit {exit_code}, {len(stamp['pairings'])} pairings")
        print(f"  docs NOT covered by any pairing ({len(stamp['docs_not_covered_by_any_pairing'])}): "
              f"{', '.join(os.path.basename(d) for d in stamp['docs_not_covered_by_any_pairing'])}")
        print(f"  verify later with: python3 .future/cov100/staletable.py "
              f"--verify-stamp {os.path.relpath(args.stamp, ROOT)}")
        return exit_code
    if args.suggest:
        wanted = set(args.labels.split(",")) if args.labels else None
        hits = 0
        for p, verdict, m in rows:
            if wanted and p["label"] not in wanted:
                continue
            clean = (verdict == "PASS" and not m.get("covered")
                     and not m.get("missing"))
            if clean:
                continue
            print(f"{p['label']}: not clean (missing {len(m.get('missing') or [])}, "
                  f"covered {len(m.get('covered') or [])}) against "
                  f"{os.path.basename(p['report'])}")
            # ONE call, not two (F.74). Each call globs /tmp and MEASURES every
            # candidate report - 4-8 s per pairing - and the loop used to call it once
            # for the list and once again for the empty-check, so `--suggest` over the
            # ~18 drifting pairings needed 300+ s and timed out (the endgame's documented
            # repair path). Calling once is also STRICTLY more correct: with two calls a
            # unit writing a new report mid-run could make the printed list non-empty and
            # the second call empty, so the "no clean candidate" line would contradict the
            # candidates printed above it.
            cands = suggest_reports(rec, p, m)
            for c in cands:
                hits += 1
                print(f"    CANDIDATE {c['report']}  uncovered={c['uncovered']} "
                      f"claims={c['claims']} resolving={c['resolving']} "
                      f"absent={c['absent']}"
                      f"{'  (the doc names it)' if c['named'] else ''}"
                      f"  mtime={datetime.datetime.fromtimestamp(c['mtime']).strftime('%H:%M:%S')}")
            if not cands:
                print("    no clean candidate on disk: the unit may not have measured yet, "
                      "or the ledger is genuinely inconsistent")
        print()
        print(f"clean candidates found: {hits}")
        return 0 if hits else 1
    if args.md:
        # machine-readable-for-a-human: no banner noise, just the table + verdicts
        fails_md = [(p, m) for p, v, m in rows if v == "FAIL"]
        skipped_md = ([(p, m) for p, _v, m in rows if m.get("unverifiable")]
                      if args.skip_unverifiable else [])
        exit_code = 1 if (fails_md or (args.strict and any(
            n.startswith("DRIFT") for _p, _v, m in rows for n in m.get("notes", [])))) else 0
        print_markdown(rows, args.pairings, exit_code, skipped_md)
        return exit_code
    print(f"pairings: {args.pairings}  ({len(rows)} entries, "
          f"roles: {', '.join(sorted({p['role'] for p, _, _ in rows}))})")
    print_table(rows)

    fails = [(p, m) for p, v, m in rows if v == "FAIL"]
    drifts = [(p, m) for p, _, m in rows if any(n.startswith("DRIFT") for n in m.get("notes", []))]
    unnamed = [(p, m) for p, _, m in rows if str(m.get("named_by") or "").startswith("none")]
    if args.require_named:
        # Fold the 5-pre condition into the rows' own problem lists rather than adding a
        # parallel failure list: the first version appended ("unnamed:"+label, m) tuples
        # to `fails`, and the detail printer reads p['label'] -- which never ran, because
        # that entry's problems list was empty, so the failure printed as a bare count
        # with no reason. Found by selftest.py asserting the reason is present.
        for p, m in unnamed:
            m.setdefault("problems", []).append(
                f"PAIRING NOT STATED BY THE DOC (CLOSEOUT 5-pre): "
                f"{os.path.basename(p['report'])} is not named in "
                f"{os.path.basename(p['doc'])} nor in any companion record it points at")
        rows = [(p, "FAIL" if m.get("problems") else v, m) for p, v, m in rows]
        fails = [(p, m) for p, v, m in rows if v == "FAIL"]
    skipped = [(p, m) for p, _v, m in rows if m.get("unverifiable")] if args.skip_unverifiable else []
    if skipped:
        fails = [(p, m) for p, m in fails if not m.get("unverifiable")]
        print()
        print(f"SKIPPED as unverifiable ({len(skipped)}) -- the doc cites an artifact that "
              f"is not on disk, so these rows are NOT verdicts:")
        for p, m in skipped:
            print(f"    {p['label']}: waiting on {p['unverifiable_until']}"
                  f" ({len(m.get('covered', []))} covered, {len(m.get('missing', []))} "
                  f"uncited lines against the substitute report)")
    if args.cli:
        log = os.path.join(HERE, "stale-check-after.log")
        with open(log, "w", encoding="utf-8") as fh:
            fh.write("# reconcile.py, every pairing in pairings.json, with exit codes.\n"
                     "# Regenerate: python3 .future/cov100/staletable.py --cli\n\n")
            for p, _v, _m in rows:
                cli_run(rec, p, fh)
        print(f"\nwrote {log}")

    n_auth = [p for p, _v, _m in rows if p["role"] == "authoritative"]
    clean = [p for p in n_auth if not next(m for pp, _v, m in rows if pp is p).get("covered")]
    justified = [p for p in n_auth
                 if next(m for pp, _v, m in rows if pp is p).get("covered")
                 and p.get("covered_justification")]
    unver = [p for p in n_auth if next(m for pp, _v, m in rows if pp is p).get("unverifiable")]
    failing = [p for p in n_auth if next(m for pp, _v, m in rows if pp is p).get("problems")]
    print()
    print(f"authoritative pairings: {len(n_auth)} -> clean {len(clean)}, "
          f"justified covered rows {len(justified)}, "
          f"unverifiable {len(unver)}, **failing {len(failing)}**")
    for p in failing:
        m = next(m for pp, _v, m in rows if pp is p)
        print(f"    FAILING {p['label']}: {p['doc']} -- "
              f"{'; '.join(m['problems'])[:150]}")
    print(f"pairings needing a justification: "
          f"{sum(1 for _p, _v, m in rows if m.get('covered'))}, all provided: "
          f"{all(p.get('covered_justification') for p, _v, m in rows if m.get('covered'))}")
    dist = {}
    for _p, _v, m in rows:
        k = str(m.get("named_by") or "-").split(":")[0]
        dist[k] = dist.get(k, 0) + 1
    print(f"pairing stated by the doc (CLOSEOUT 5-pre): section {dist.get('section', 0)}, "
          f"doc {dist.get('doc', 0)}, companion {dist.get('via', 0)}, "
          f"**NOT stated {dist.get('none', 0)}**")
    for p, m in unnamed:
        print(f"    not stated: {p['label']} -- {os.path.basename(p['report'])} is not "
              f"named in {os.path.basename(p['doc'])}")
    print(f"DRIFT rows: {len(drifts)}")
    for p, m in drifts:
        for note in m["notes"]:
            if note.startswith("DRIFT"):
                print(f"    {p['label']}: {note}")

    if args.gate or args.strict:
        print()
        if fails:
            print(f"GATE FAILED: {len(fails)} pairing(s)")
            for p, m in fails:
                for prob in m["problems"]:
                    print(f"    {p['label']}: {prob}")
        if args.strict and drifts:
            print(f"GATE FAILED (--strict): {len(drifts)} pairing(s) drifted from "
                  f"pairings.json; re-measure and update the recorded expectation")
            for p, m in drifts:
                for note in m["notes"]:
                    if note.startswith("DRIFT"):
                        print(f"    {p['label']}: {note}")
        ok = not fails and not (args.strict and drifts)
        print("GATE OK: every covered row is attributed to a justified section"
              + (f" ({len(skipped)} unverifiable pairing(s) excluded, listed above)"
                 if skipped else "") if ok else "GATE NOT OK")
        return 0 if ok else 1
    return 0


if __name__ == "__main__":
    sys.exit(_main_guarded())
