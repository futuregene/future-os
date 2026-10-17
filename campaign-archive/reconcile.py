#!/usr/bin/env python3
"""Reconcile a module record against a coverage report.

Purpose: a cheap tripwire for the supervisor's per-delivery review and the final
sweep. It cannot prove a waiver is *correct* (only an adversarial reviewer can),
but it catches the two mechanical failures that matter:

  MISSING  an uncovered line the module record never mentions at all;
  STALE    a line the record cites as uncovered that the report now shows covered
           (line drift, or a doc left behind by later work).

Usage:
  reconcile.py --report <lcov|coverage-final.json> --scope <path prefix>
               --doc <module doc> [--lines-first]

Matching is deliberately dumb: for every uncovered file, the file's basename must
appear in the doc, and each of its uncovered line numbers must appear somewhere in
the doc. That is enough to refuse "the doc says nothing about this line"; the
mechanism still has to be read by a human.
"""
import argparse
import json
import os
import re
import sys


ROOTS = ("/Users/geilige/future-os/",)
WORKTREE_RE = re.compile(r"^\.?worktrees/[^/]+/(.*)$")


def norm_path(p):
    """Normalise a path from a report or a `--scope` argument to a common form.

    Reports reach this tool in several shapes depending on how the unit produced
    them: absolute (`/Users/…/worktrees/x/agent/src/a.rs`), repo-relative with the
    worktree component (`worktrees/x/agent/src/a.rs`), or repo-relative with the
    worktree stripped by a `sed` normalisation the units commonly run
    (`agent/src/a.rs` — this campaign's agent units do exactly that). A scope
    argument can be any of those too, and comparing raw prefixes therefore matched
    nothing, which reads as "no records" (INVALID) or, without the guard, as 100 %.

    Strip the checkout root, then any leading `.worktrees/<name>/` or
    `worktrees/<name>/`, then `./` — on BOTH sides. Observed cost of getting this
    wrong: three separate silent zeros in one afternoon.
    """
    s = p.strip()
    for r in ROOTS:
        if s.startswith(r):
            s = s[len(r):]
    while s.startswith("./"):
        s = s[2:]
    m = WORKTREE_RE.match(s)
    if m:
        s = m.group(1)
    return s.lstrip("/")


def in_scope(path_norm, want):
    """Whether a normalised report path is inside a normalised scope.

    After norm_path() both sides look like `agent/src/rpc[/x.rs]`, so the relation
    is: the scope is a PREFIX of the path (its descendants), or the path still
    carries components the scope does not (the scope is a path SEGMENT of it). Both
    clauses are needed — the first for `agent/src/rpc` vs `agent/src/rpc/a.rs`, the
    second for a report that kept a leading component a `sed` normalisation dropped
    on the scope side. This is a filter, not a claim: over-matching costs one extra
    row, under-matching costs a silent zero (observed three times).
    """
    if not want:
        return True
    p = path_norm.rstrip("/")
    for w in want:
        if p == w or p.startswith(w + "/") or (("/" + w + "/") in ("/" + p + "/")):
            return True
    return False


def lcov_lines(path, scopes):
    """{source path: {uncovered line numbers}} for files under `scopes`."""
    per_file = {}
    cur = None
    want = [norm_path(s).rstrip("/") for s in scopes]
    for raw in open(path, encoding="utf-8", errors="replace"):
        line = raw.rstrip("\n")
        if line.startswith("SF:"):
            sf = line[3:]
            cur = sf if in_scope(norm_path(sf), want) else None
            if cur:
                per_file.setdefault(cur, set())
        elif line.startswith("DA:") and cur:
            m, _, c = line[3:].partition(",")
            try:
                if int(c.split(",")[0]) == 0:
                    per_file[cur].add(int(m))
            except ValueError:
                pass
    return per_file


def istanbul_lines(path, scopes):
    data = json.load(open(path))
    per_file = {}
    want = [norm_path(s).rstrip("/") for s in scopes]
    for f, cov in data.items():
        if not in_scope(norm_path(f), want):
            continue
        smap, scount = cov.get("statementMap") or {}, cov.get("s") or {}
        per_line = {}
        for sid, meta in smap.items():
            ln = meta["start"]["line"]
            per_line[ln] = per_line.get(ln, False) or scount.get(sid, 0) > 0
        unc = {ln for ln, hit in per_line.items() if not hit}
        if unc:
            per_file[f] = unc
    return per_file


def istanbul_counts(path, scopes):
    """Istanbul counterpart of raw_counts(): {file: {line: hits}}.

    A JS report has no per-line `DA:` record; it has a statement map. So "the report
    has a record for this line" means "some statement STARTS on this line", and the
    count is the largest of those. Lines with no statement at all (blank, comment,
    JSX-only text) are the ABSENT class there, exactly as in lcov.
    """
    data = json.load(open(path))
    out = {}
    want = [norm_path(s).rstrip("/") for s in scopes]
    for f, cov in data.items():
        if not in_scope(norm_path(f), want):
            continue
        smap, scount = cov.get("statementMap") or {}, cov.get("s") or {}
        per_line = out.setdefault(f, {})
        for sid, meta in smap.items():
            ln = meta["start"]["line"]
            per_line[ln] = max(per_line.get(ln, 0), scount.get(sid, 0))
    return out


def report_lines(path, scopes):
    """Uncovered lines per file, dispatching on the report's format.

    Both halves of the check must agree about the format; a tool that hard-codes the
    lcov reader silently reports `uncovered=0` for an Istanbul report (observed while
    building staletable.py: desktop's 60 uncovered lines read as 0).
    """
    if path.endswith(".json"):
        return istanbul_lines(path, scopes)
    return lcov_lines(path, scopes)


def report_counts(path, scopes):
    """Every line the report has a RECORD for, with its hit count (see
    raw_counts() for the three states this distinguishes)."""
    if path.endswith(".json"):
        return istanbul_counts(path, scopes)
    return raw_counts(path, scopes)


def cited_numbers(doc):
    """All line numbers a doc cites, with `a-b` ranges expanded.

    Ledgers in this repo cite uncovered lines both individually (`62,67,69`) and
    as ranges (`144-148`), so a bare-number scan reports phantom gaps for every
    line inside a range. A false MISSING is the harmful direction of this
    tripwire (it sends a worker chasing a line the doc did name), so ranges are
    expanded generously; false *mentions* are tolerated.
    """
    cited = set()
    for m in re.finditer(r"(?<![\d.\-])(\d{1,7})-(\d{1,7})(?![\d.])", doc):
        a, b = int(m.group(1)), int(m.group(2))
        if 0 < a <= b and b - a <= 2000:
            cited.update(range(a, b + 1))
    for m in re.finditer(r"(?<![\d.])(\d{1,7})(?![\d.])", doc):
        cited.add(int(m.group(1)))
    return cited


def key_files(per_file, scope):
    """Give each file a needle that is UNIQUE within the report, so two files
    sharing a basename (`feishu/mod.rs` vs `dingtalk/mod.rs`, or `compaction/mod.rs`
    vs `config/mod.rs`) stay separate. A peer hit this collision and worked around
    it by hand; a naive scope-relative suffix is not enough, because `mod.rs` stays
    `mod.rs` under two different scopes — the first implementation of this lost 29
    lines that way, so the needle grows until it is unique.
    """
    def needle(full, depth):
        parts = full.split("/")
        return "/".join(parts[-depth:])

    needles = {}
    for depth in range(1, 8):
        seen = {}
        for full in per_file:
            seen.setdefault(needle(full, depth), []).append(full)
        for full in per_file:
            if full in needles:
                continue
            cand = needle(full, depth)
            if len(seen[cand]) == 1:
                needles[full] = cand
        if len(needles) == len(per_file):
            break
    return {needles[f]: lines for f, lines in per_file.items()}


# The ledger-row pattern: a markdown row whose FIRST cell is `path:lines` (see
# claimed_uncovered). Module-level so the diagnostic tools in this directory count
# rows with the shipped pattern instead of a copy of it that can drift.
LEDGER_ROW = re.compile(
    r"^\s*\|\s*`?([A-Za-z0-9_./-]+\.(?:tsx|ts|rs))`?\s*:\s*`?([0-9,\-\s]+?)\s*`?\s*\|")

# The same claim in a TAG-FIRST ledger: `| D12 | `path:12` | category | mechanism |`
# (module-desktop.md's shape). Accepted only when the first cell is a bare TAG, so
# prose citations are still rejected. Without this, a tag-first ledger parsed as
# ZERO claims and its STALE half was silently vacuous - the same false-clean
# direction as the backtick bug, found the same way (the vacuity WARNING fired on
# module-desktop.md with 314 rows and 0 claims).
#
# The tag class is deliberately TIGHT: `[A-Za-z]{1,2}[0-9]{1,3}` = `D1`..`D999`,
# `W12`, `T7`. A looser `[A-Za-z][A-Za-z0-9 _.-]{0,12}` accepted any short word,
# which made a PROSE first cell plus a covered `` `path:line` `` in the second cell
# a claim - i.e. a false STALE, the harmful direction. Measured before tightening:
# all 86 rows in docs/**/*.md that only this pattern accepts carry a `D<n>` tag
# (`D1`-`D54`), so nothing is lost; and if a future ledger uses another tag, the
# failure is the LOUD one (0 claims -> the vacuity WARNING), not a silent false
# claim. Probed by `patternprobe.py`.
LEDGER_ROW_TAGGED = re.compile(
    r"^\s*\|\s*`?[A-Za-z]{1,2}[0-9]{1,3}`?\s*\|\s*"
    r"`?([A-Za-z0-9_./-]+\.(?:tsx|ts|rs))`?\s*:\s*`?([0-9,\-\s]+?)\s*`?\s*\|")


# A third shipped shape: `| `path` 1111, 1115 | category | mechanism |` - the path in
# backticks, then a SPACE and the line list, no colon (module-channels.md,
# module-tauri-{headless,remote,shell}.md). The cell must contain ONLY numbers and
# separators after the path, so `| `foo.rs` 2 tests | ... |` is still rejected.
# Measured: 11 docs were "blind" (claims 0) until this pattern existed, so their STALE
# half had never run - the third variant of the same false-clean finding.
LEDGER_ROW_SPACED = re.compile(
    r"^\s*\|\s*`?([A-Za-z0-9_./-]+\.(?:tsx|ts|rs))`?\s+([0-9][0-9,\-\s]*?)\s*`?\s*\|")


def claimed_uncovered(doc):
    """Line numbers the doc claims are UNCOVERED.

    A claim in this repository's ledgers is a markdown table ROW carrying
    ``file:line`` in its first cell, or in its second cell when the first cell is a
    bare tag:

        | `src/a.rs:12,15` | 2 | unreachable-by-construction | … |
        | `src/a.rs:12` | …           |
        | src/a.rs:12 | …             |
        | D12 | `src/a.rs:12` | unreachable-by-construction | … |
        | `src/a.rs` 12, 15 | unreachable-by-construction | … |

    Nothing else counts (not prose, not a later cell). Scanning the whole document
    instead produced three classes of false STALE in this campaign: a line cited as
    *evidence* for another row's waiver (`auth.rs:530` is the filter that proves
    doctor.rs:140 dead, and it is covered), a line cited inside a `#[ignore]`d test's
    description, and every citation in an earlier campaign's section (whose numbering
    has since drifted). A false STALE sends a reviewer to "fix" a correct row, so the
    rule is narrow.

    The pattern needs whitespace (or nothing) before the closing pipe, so a first
    cell with trailing prose (`| `src/a.rs:12` (see §3) | …`) is still rejected.
    It used to demand the pipe IMMEDIATELY after that whitespace, which silently
    rejected the commonest shape in this repo — one backtick pair around path and
    number together, `` | `src/a.rs:12` | … `` — so `stale` came out 0 for a doc
    whose rows the scan had never parsed at all. `rowscan.py` in this directory
    reports, per doc, how many rows each pattern accepts; a doc with table rows and
    0 claims now prints a WARNING instead of a silent 0 STALE.
    """
    claims = set()
    row = LEDGER_ROW
    for line in doc.splitlines():
        m = (row.match(line) or LEDGER_ROW_TAGGED.match(line)
             or LEDGER_ROW_SPACED.match(line))
        if not m:
            continue
        # keep the WHOLE path the row gave: several `mod.rs` files exist in one
        # crate, so reducing to a basename made a claim about one file look like a
        # claim about all of them (36 false STALE in one run).
        path = m.group(1).lstrip("./")
        for part in m.group(2).split(","):
            part = part.strip()
            rng = re.match(r"^(\d+)\s*-\s*(\d+)$", part)
            if rng:
                a, b = int(rng.group(1)), int(rng.group(2))
                if 0 < a <= b and b - a <= 2000:
                    claims.update((path, n) for n in range(a, b + 1))
            elif part.isdigit():
                claims.add((path, int(part)))
    return claims


def claim_targets(path, line, per_file):
    """Report keys the row's path refers to, by longest-suffix match.

    A row writes `agent/src/compaction/mod.rs`; the report key is
    `compaction/mod.rs`. Matching a *suffix* keeps the two apart from
    `config/mod.rs`, which a basename match would not.
    """
    parts = path.split("/")
    for depth in range(len(parts), 0, -1):
        suffix = "/".join(parts[-depth:])
        hits = [k for k in per_file if k == suffix or k.endswith("/" + suffix)]
        if hits:
            # shortest matching key wins (most specific scope available)
            hits.sort(key=len)
            return hits
    return []


def fence_parity(doc):
    """(count, first_unclosed_line_or_None) for ``` markers, counting only lines whose
    first non-space characters open or close a fence.

    Returns the number of markers and, when that number is ODD, the 1-based line of
    the marker that leaves the document inside a fence. An odd count is a DOCUMENT
    defect - and a dangerous one, because a fence-aware slicer treats everything after
    it as code, so real headings become invisible (F.72, found by the app-data unit the
    day after my F.70 fix made this slicer fence-aware: my fix removed a silent failure
    for a `#` inside a fence and introduced one for an unpaired fence).
    """
    n = 0
    first_open = None
    in_fence = False
    for i, line in enumerate(doc.split("\n"), 1):
        if line.lstrip().startswith("```"):
            n += 1
            if not in_fence:
                in_fence = True
                if first_open is None:
                    first_open = i
            else:
                in_fence = False
    return n, (first_open if n % 2 else None)


class DocFenceError(Exception):
    """Raised when a document's fence parity is odd AND that is why no heading could
    be found. Callers must report THIS, not 'no heading contains X': the second message
    sends the reader hunting for a missing heading while the real defect is an unpaired
    fence, and the whole check silently stops covering that document."""


def slice_section(doc, needle):
    """The one section whose HEADING contains `needle`, up to the next heading of
    the same or higher level. Returns None when no heading matches (the caller
    decides whether that is fatal).

    Slicing matters for both halves of the check and in opposite directions: it
    keeps an old campaign's citations from satisfying MISSING, and it also means
    the STALE half only sees rows *inside* the slice — a `--section` that does not
    contain the ledger makes the STALE half vacuous (observed: a unit's documented
    invocation sliced its narrative section while its ledger rows sat in a later
    `## 6. Waiver ledger`; the run printed 0 STALE for a slice it had never parsed).

    FENCED CODE BLOCKS ARE SKIPPED (fixed 2026-10-17, INCIDENTS F.70). A `#` at
    column 0 inside a fence is a shell COMMENT, not a heading, but the previous
    implementation matched `^#{1,N} ` anywhere — so a record that pasted example
    output like

        # -> files with uncovered lines=202
        #    MISSING (uncovered, not mentioned): 0

    had its slice TERMINATED by that comment, hiding every ledger row below it.
    The result was phantom MISSING against rows the record does document: measured
    at 13 for one pairing and 7 for a module record, and 283 such lines exist in 30
    docs. The same blindness applied when FINDING the opening heading, so a fenced
    example of a heading could start the slice in the wrong place.

    AN UNPAIRED FENCE RAISES `DocFenceError` (2026-10-17, INCIDENTS F.72): with fence
    tracking, one stray ``` makes every later heading invisible, so this function
    would return None and the caller would blame a missing heading. When no heading
    matched and the parity is odd, the parity is the answer.
    """
    offset = 0
    start = None
    level = 0
    in_fence = False
    for line in doc.split("\n"):
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
        elif not in_fence:      # a `#` inside a fence is a comment, not a heading
            m = re.match(r"^(#{1,6}) ", line)
            if m:
                lvl = len(m.group(1))
                if start is None:
                    if needle in line:
                        start = offset
                        level = lvl
                elif lvl <= level:
                    return doc[start:offset]
        offset += len(line) + 1
    if start is not None:
        return doc[start:]
    # nothing matched: is it because the document's fences are unbalanced?
    n, first_open = fence_parity(doc)
    if first_open is not None:
        raise DocFenceError(
            f"no heading contains {needle!r}, and this document has {n} code-fence "
            f"markers (ODD) - the fence opened at line {first_open} is never closed, "
            f"so every later heading is treated as code. Fix the fence, then re-run.")
    return None


def raw_counts(path, scopes):
    """{norm path: {line: hit count}} for EVERY `DA:` record in scope.

    `lcov_lines()` keeps only the zero-count lines, which is what the metric needs
    but not what the STALE half needs: there, a line the ledger cites as uncovered
    has three possible states in the report, and only the third is a finding.

    * count > 0   -> the ledger disagrees with the report (a real STALE)
    * count == 0  -> agreement
    * NO RECORD   -> nothing at all (a blank line, a comment, a `#[cfg]`-dropped
                     block, or a report from another revision). `claimed_uncovered()`
                     expands `a-b` ranges across every line in them, so a range
                     spanning a blank line manufactures one of these per blank, and a
                     cross-revision pairing manufactures thousands.
    """
    want = [norm_path(s).rstrip("/") for s in scopes]
    out, cur = {}, None
    for raw in open(path, encoding="utf-8", errors="replace"):
        line = raw.rstrip("\n")
        if line.startswith("SF:"):
            sf = line[3:]
            cur = norm_path(sf) if in_scope(norm_path(sf), want) else None
            if cur:
                out.setdefault(cur, {})
        elif line.startswith("DA:") and cur is not None:
            m, _, c = line[3:].partition(",")
            try:
                out[cur][int(m)] = int(c.split(",")[0])
            except ValueError:
                pass
    return out


def counts_for(key, counts):
    """Hit counts for one `key_files()` NEEDLE, from a `report_counts()` mapping.

    The two are keyed differently on purpose: `key_files()` shortens each report path
    to its shortest suffix that is unique within the report (so two `mod.rs` stay
    apart), while `report_counts()` keeps the full normalised path. Looking a needle
    up directly in `counts` therefore misses ALWAYS -- which silently reported every
    claimed line as ABSENT (no record) and `COVERED 0` everywhere, i.e. the exact
    false-clean this file exists to prevent. Found by `selftest.py`'s fixture, whose
    covered lines came back absent.
    """
    if key in counts:
        return counts[key]
    hits = [v for k, v in counts.items() if k.endswith("/" + key)]
    if not hits:
        return {}
    merged = {}
    for h in hits:
        for ln, c in h.items():
            merged[ln] = max(merged.get(ln, 0), c)
    return merged


def split_stale(stale, counts):
    """Split STALE rows into (covered, absent) -- see raw_counts() for the three
    states. `stale` is the list of `(report key, line)` pairs main() collected, and
    the keys are `key_files()` needles (see counts_for())."""
    covered, absent = [], []
    cache = {}
    for key, ln in sorted(set(stale)):
        if key not in cache:
            cache[key] = counts_for(key, counts)
        (covered if cache[key].get(ln, 0) > 0 else absent).append((key, ln))
    return covered, absent


def claims_by_section(doc):
    """{(heading): claim set} for a document, so a STALE count can be attributed to
    the section that produced it.

    Why this matters more than it looks: a record that was re-measured mid-campaign
    keeps BOTH ledgers — `module-cli.md` has §4 (measurement S3, and its own note says
    §4b supersedes it) and §4b (S5); `module-agent-rpc.md` carries a round-1 §6 next to
    its round-2 §0.5. Parsing the whole file therefore compares two revisions' claims
    with one report, and every row the superseding measurement closed reads as STALE.
    The rows are not wrong; the invocation is too wide. Naming the section turns a
    "38 covered rows" scare into "38 rows from §4, the superseded one".
    """
    out = {}
    cur = "(no heading)"
    buf = []

    def flush():
        if buf:
            got = claimed_uncovered("\n".join(buf))
            if got:
                out.setdefault(cur, set()).update(got)
            buf.clear()

    for line in doc.splitlines():
        m = re.match(r"^(#{1,6})\s+(\S.*)$", line)
        if m:
            flush()
            cur = m.group(2).strip()
        else:
            buf.append(line)
    flush()
    return out


def sections_claiming(by_section, key, line):
    """The sections whose rows can be the source of this claim: same line number and a
    path that is a suffix of the report key (rows write full paths, `key_files()`
    shortens them to needles)."""
    hits = []
    for head, claims in by_section.items():
        for path, ln in claims:
            if ln == line and (path == key or path.endswith(key)
                               or key.endswith(path)):
                hits.append(head)
                break
    return hits


def sections_matching(doc, needle):
    """Every HEADING that contains `needle`, as `(level, text)`.

    `slice_section()` takes the FIRST match, and a doc often mentions a section
    number in prose before its own heading exists -- `--section "4c"` on
    `module-cli.md` resolves to `### Segment 3 (mock-span hygiene) — the three runs
    behind §4c` (a cross-reference, ~500 lines above the ledger) instead of
    `## 4c. Ledger after the mock-span pass`. The slice is then a real section that
    simply is not the ledger, and every line reads MISSING -- measured, after a
    `--section 4c` run printed `mentioned: 0` for a 122-line ledger.

    So an invocation is only as precise as its needle: pass a fragment that is
    unique to the intended heading, and check it with this function (or with
    `rowscan.py --sections`, which lists them per doc).
    """
    out = []
    for line in doc.splitlines():
        if not line.startswith("#"):
            continue
        level = len(line) - len(line.lstrip("#"))
        text = line[level:].strip()
        if needle in text:
            out.append((level, text))
    return out


def stale_candidates(sliced_doc, report, scopes):
    """`(cov_candidates, counts)` for the STALE half: the ledger rows that do NOT agree
    with the report, resolved against every file the report MEASURED.

    ONE implementation, called by both `reconcile.py` and `staletable.py`: those two
    carried separate copies of this loop and the copy in `staletable.py` still had the
    blind spot described in `main()` (a file whose uncovered set has become EMPTY was
    treated as "cannot tell" and skipped), so the authoritative gate and the per-unit
    tool could disagree about the same doc. Duplicate checks do not add coverage, they
    add a second hiding place for one bug.
    """
    counts_all = report_counts(report, scopes)
    measured = key_files(counts_all, scopes)   # {needle: {line: count}}, ALL files
    stale = []
    for path, ln in sorted(claimed_uncovered(sliced_doc)):
        targets = claim_targets(path, ln, measured)
        if not targets:
            continue
        # stale unless EVERY file the row can name reports this line as uncovered
        if all(measured[t].get(ln, None) != 0 for t in targets):
            stale.append((targets[0], ln))
    return stale, counts_all


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", required=True)
    ap.add_argument("--scope", required=True, action="append",
                    help="path prefix(es) of the unit's own scope; repeatable")
    ap.add_argument("--doc", required=True)
    ap.add_argument("--label", default="unit")
    ap.add_argument("--section", default=None,
                    help="restrict doc matching to one section (heading substring)")
    args = ap.parse_args()

    # format dispatch lives in report_lines()/report_counts(): the uncovered half, the
    # STALE half and the standalone tools must all read the same report the same way
    per_file = key_files(report_lines(args.report, args.scope), args.scope)
    if not per_file:
        # No SF: record matched the scope. A fully covered unit still yields entries
        # (with empty line sets), so an EMPTY mapping is never a coverage result -- it
        # is a wrong scope, a wrong format, or a truncated/copied report. Without this
        # guard the run prints "uncovered lines=0", which reads exactly like 100 %
        # coverage. Observed once in this campaign (a run of the agent-rpc pairing
        # printed 0/0 for a 4.3 MB report that the next identical run read as 347
        # uncovered lines) and cost a re-measurement to disbelieve; the campaign's own
        # tools exit 3 on a scope mismatch, so this does too.
        print("[ERROR] no `SF:` record in the report matches the --scope; nothing was "
              "measured. Check the scope paths, the report file (is it complete?) and "
              "its format; the campaign's other tools exit 3 here.", file=sys.stderr)
        sys.exit(3)
    doc = open(args.doc, encoding="utf-8", errors="replace").read()
    if args.section:
        try:
            sliced = slice_section(doc, args.section)
        except DocFenceError as error:
            # F.72: an unpaired fence is a DOCUMENT defect that makes the check
            # inoperative for this doc. Report that cause (exit 3 = "nothing was
            # measured"), not "no heading contains X", which sends the reader
            # hunting for a missing heading.
            print(f"ERROR: {error}", file=sys.stderr)
            print(f"       (doc: {args.doc})", file=sys.stderr)
            sys.exit(3)
        if sliced is None:
            print(f"ERROR: no heading containing {args.section!r} in {args.doc}",
                  file=sys.stderr)
            sys.exit(2)
        doc = sliced
    else:
        _n, _open = fence_parity(doc)
        if _open is not None:
            print(f"WARNING: {args.doc} has {_n} code-fence markers (ODD); the fence "
                  f"opened at line {_open} is never closed, so headings after it are "
                  f"invisible to this check (F.72)", file=sys.stderr)
    total = sum(len(v) for v in per_file.values())
    files_with_hits = sum(1 for v in per_file.values() if v)

    missing, stale, ok = [], [], 0
    cited = cited_numbers(doc)
    for base, lines in sorted(per_file.items()):
        if base not in doc:
            missing.extend((base, ln) for ln in sorted(lines))
            continue
        for ln in sorted(lines):
            if ln in cited:
                ok += 1
            else:
                missing.append((base, ln))

    # STALE: the doc claims `file:LINE` is uncovered in a ledger ROW, but the
    # report shows it covered (or the line no longer exists). Only first cells of
    # table rows count — see claimed_uncovered() for the false positives that rule
    # avoids. `claims` is kept so the count can be reported: a check that parsed no
    # row at all is not a check that passed (see the WARNING below).
    # STALE half. Candidates come from EVERY file the report has a `DA:` record for,
    # not from the uncovered-only map: a file whose uncovered set has become EMPTY is
    # exactly the state a coverage campaign produces, and the previous guard
    # (`per_file[t] and ln not in per_file[t]`) treated that empty set as "cannot
    # tell" - so every ledger row about a fully-covered file was SKIPPED and the run
    # printed no STALE block at all. Found by the AG-A unit at segment 16
    # (`session/history/paging.rs` reached zero uncovered while its row still claimed
    # seven such lines; the unit removed the row by hand and reported the blind spot).
    # That is a FALSE CLEAN in the half of the check whose whole job is to catch a
    # record overstating its residual. A file the report never measured (a peer's path,
    # a wrong pairing) is still skipped: that is a scope question, not a row question.
    # See `stale_candidates()`, which now holds the only copy of this logic.
    claims = claimed_uncovered(doc)
    stale, counts_all = stale_candidates(doc, args.report, args.scope)

    print(f"[{args.label}] files with uncovered lines={files_with_hits} "
          f"uncovered lines={total}")
    print(f"  mentioned in {os.path.basename(args.doc)}: {ok}")
    print(f"  MISSING (uncovered, not mentioned): {len(missing)}")
    for base, ln in missing[:40]:
        print(f"     {base}:{ln}")
    if len(missing) > 40:
        print(f"     ... and {len(missing) - 40} more")
    # AMBIGUOUS-BASENAME HINT (found by the TAU-B unit at segment 6): `key_files()
    # gives a file the shortest suffix that is UNIQUE within the report, so with both
    # `commands/tasks.rs` and `src/tasks.rs` in scope the top-level file's needle is
    # `src/tasks.rs`. A ledger that cites the bare `tasks.rs` then fails `base not in
    # doc` for every one of its lines and reports ~98 MISSING rows, which reads as
    # "you never mentioned these lines" rather than "spell the path out". The unit
    # hit exactly that and worked around it by hand. Detect the shape and say what to
    # do: a large MISSING set whose BASENAMES are all in the doc is a citation problem,
    # not a documentation gap.
    if missing:
        hintable = []
        for base, _ln in missing:
            stem = base.rsplit("/", 1)[-1]
            if base not in doc and stem in doc:
                hintable.append(base)
        if len(hintable) >= max(3, len(missing) // 4):
            print(f"  HINT: {len(missing)} MISSING row(s) name a needle whose bare "
                  f"basename DOES appear in the doc - e.g. "
                  f"{', '.join(sorted(set(hintable))[:3])}. This is an AMBIGUOUS "
                  f"BASENAME, not a documentation gap: another file with the same "
                  f"basename is also in scope, so the needle keeps a directory "
                  f"(that is what keeps two `mod.rs`/`tasks.rs` apart). Cite the "
                  f"spelled-out path in the ledger row and re-run.")
    if stale:
        uniq = sorted(set(stale))
        # Split before printing: "not in the uncovered set" is NOT the same claim as
        # "covered". A line with no `DA:` record at all is not evidence of anything,
        # and ranges expanded across blank lines (or a report from another revision)
        # produce those in bulk. Without the split, a reviewer is sent to "fix"
        # correct rows -- the failure direction claimed_uncovered() warns about, and
        # the reason 51 cli rows looked STALE while 0 of them disagreed.
        covered, absent = split_stale(uniq, counts_all)
        print(f"  STALE (doc cites it as uncovered; old doc sections also match "
              f"unless --section is given): {len(uniq)}")
        print(f"    COVERED (DA>0 - a real disagreement, the actionable half): "
              f"{len(covered)}")
        for base, ln in covered[:20]:
            print(f"       {base}:{ln}")
        if len(covered) > 20:
            print(f"       ... and {len(covered) - 20} more")
        print(f"    ABSENT (no `DA:` record -- NOT evidence: a blank line inside a "
              f"claimed range, a comment, or a report from another revision): "
              f"{len(absent)}")
        for base, ln in absent[:10]:
            print(f"       absent {base}:{ln}")
        if len(absent) > 10:
            print(f"       ... and {len(absent) - 10} more (list them all with "
                  f"python3 .future/cov100/staledetail.py --all)")
        if not covered:
            print(f"    -> nothing to fix: no claimed line disagrees with this "
                  f"report. If the absent ones surprise you, the ledger and the "
                  f"report describe different revisions.")
        elif args.section is None:
            # Which section produced the covered rows? A wide invocation over a record
            # that was re-measured mid-campaign mixes two revisions' ledgers.
            by_sec = claims_by_section(doc)
            hit_counts = {}
            for key, ln in covered:
                for head in sections_claiming(by_sec, key, ln):
                    hit_counts[head] = hit_counts.get(head, 0) + 1
            if hit_counts:
                top = sorted(hit_counts.items(), key=lambda kv: -kv[1])
                shown = ", ".join(f"{h[:48]} ({n})" for h, n in top[:3])
                print(f"    -> the covered rows come from {len(hit_counts)} section(s): "
                      f"{shown}")
                if len(by_sec) > 1:
                    print(f"       this doc holds {len(by_sec)} sections with claims: if "
                          f"one of them is superseded (a re-measurement), re-run with "
                          f"--section for the authoritative ledger")
    # Loud when the STALE half had nothing to look at: 0 rows parsed is not the
    # same as 0 stale rows, and the two printed the same thing until this warning
    # existed (a whole unit shipped on a vacuous 0). Not fatal: some ledgers are
    # prose or bullet lists, so an error would redden correct docs.
    if not claims:
        n_rows = sum(1 for line in doc.splitlines() if re.match(r"^\s*\|\s*", line))
        if n_rows:
            print(f"  WARNING: {n_rows} table rows in the doc slice, 0 parsed as "
                  f"ledger claims -- the STALE check did NOT run "
                  f"(python3 .future/cov100/rowscan.py --samples shows which rows).")
    sys.exit(1 if missing else 0)


if __name__ == "__main__":
    main()
