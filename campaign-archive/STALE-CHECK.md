# The per-line tripwire's STALE half — state after todo_83897cf1c4d3 (2026-10-16 21:20, attempt 4)

`reconcile.py` has two halves. MISSING asks "is every uncovered line cited in the
ledger?". STALE asks the opposite: "does the ledger cite a line the report no longer
shows as uncovered?" This file records what its STALE half could not see, what now
blocks that, what the STALE half reports once it works, and what is left for the docs.

**Current state (attempt 4).** `pairings.json` holds **22 entries**; the gate is GREEN on
every mode — `--gate` 0, `--gate --strict` 0, `--gate --require-named` 0, `--md` 0,
`--stamp` 0, `--verify-stamp` 0 — with **DRIFT 0, failing 0, NOT stated 0** and
`selftest.py` at **70 checks**. Nothing needed repairing this pass: the five re-points
and one split of attempts 2–3 have settled. The remaining blocker is **scheduling, not
tooling** — see the two sections below, and read "Attempt 4's own head-of-file note" if
you are deciding whether to run a fifth attempt.

## Attempt 4 (this pass): the gate needed no repair, and the tree is *provably* not frozen

**The gate work was zero this time** — `--gate --strict` exit 0, `--gate --require-named`
exit 0, DRIFT rows 0, failing 0, NOT stated 0, 22 pairings — i.e. nothing had drifted and
no doc had moved to a new report since attempt 3. That is itself the signal the pairings
have settled; the earlier attempts' churn (five re-points, one split) has stopped.

**A sharper measurement of the precondition.** Attempt 3 argued "not frozen" from doc
mtimes. This pass found a better instrument, and the answer is unambiguous:

```
$ date '+now: %H:%M:%S'                     -> 21:17:47
$ git log -1 --format='%ci'                -> 2026-10-16 21:17:32   (HEAD, 15 s earlier)
$ git status --short | wc -l               -> 0 dirty paths
$ find . -newermt '-5 minutes' -type f     -> desktop/src-tauri/target/**  (a unit is
                                              compiling right now)
```

So **0 dirty paths is not stability** — it is the gap between two of a peer's automated
safety-snapshot commits while that peer's build runs (HEAD moved `2c9aa504` 21:11:55 →
`64353c34` 21:17:32, and the tauri unit's `target/` tree is being written inside the
5-minute window). Add 21 open todos and this is the fourth measured "not frozen".

Anyone tempted to treat "clean working tree" as the freeze signal now has the
counter-example: check for *recent writes anywhere in the tree*, not for a clean status.
The snapshot's hash-based verify is what survives this condition; a rev-based check would
fail on every one of these commits.

## Item (2)'s destination is outside this unit's write set — asked, not skipped

The task asks the frozen pass to write `coverage/final/pairing-gate-snapshot.json`. That
path is **outside** this unit's declared write set (`.future/cov100`), as is
`coverage/final/FINAL-REPORT.md`:

```
INSIDE  .future/cov100/gate-snapshot.json        <- written this pass
OUTSIDE coverage/final/pairing-gate-snapshot.json
OUTSIDE coverage/final/FINAL-REPORT.md
```

`measure-final.sh` hard-codes `OUT="$ROOT/coverage/final"`, so running it as instructed
writes outside the contract. The task's own rule is "if additional writes are needed,
stop and ask the supervisor to revise the plan", so that is what happened: a `user_gate`
carries the decision (grant the scope, or accept an in-scope snapshot path).

What is **not** deferred is the mechanism: the in-scope snapshot is written
(`.future/cov100/gate-snapshot.json`, 22 pairings, exit 0) and verifies clean, and
`.measure-probe.sh` already proves the harness's gate+snapshot steps end to end with
`OUT` redirected — including the red-gate path. So the moment the scope is granted (or the
harness is run by its owner, for whom `coverage/` is in scope), the frozen pass is one
command, not new work.

## `module-agent-llm.md`: verified to need a re-measure, by tool not assertion

Of the 9 unpaired docs, this is the one that could plausibly have become pairable (it has
106 claims across four sections and is not prose). Probed with `--suggest` against every
`/tmp/*.info` and `/tmp/*.json`:

```
reports the doc names:
  MISSING coverage/agent-ag-llm-report.json   (four citations)
  MISSING coverage/llvm-cov-full.json
$ staletable.py --suggest --pairings <probe>
  agent-llm-probe: not clean ... 
      no clean candidate on disk
```

So no on-disk report matches its ledger: it needs a **re-measure**, not a restatement.
That is the same conclusion as attempt 3, now established by the tool rather than by
inspection.

## One more vacuity case pinned: a report file that does not exist

Same class as the empty-scope hole, checked deliberately: a pairing whose `--report` names
a nonexistent file must fail with a reason, not measure an empty set. It does
(`FAIL` / `report missing: <path>`, exit 1), and `selftest.py` now pins it — 70 checks.

## Attempt 4's own head-of-file note

The frozen pass the title asks for **could not be performed, for the fourth consecutive
attempt**, with the process-level evidence above. Since the gate needed no repair this
time, the honest reading is that the tooling side is finished and the remaining blocker is
scheduling: a worker turn cannot freeze peer units, so further identical attempts would
re-report the same measurement at increasing cost. The loop stops here; the
`user_gate` is the next step, and its question is the freeze/write-scope decision rather
than another audit.


`reconcile.py` has two halves. MISSING asks "is every uncovered line cited in the
ledger?". STALE asks the opposite: "does the ledger cite a line the report no longer
shows as uncovered?" This file records what its STALE half could not see, what now
blocks that, what the STALE half reports once it works, and what is left for the docs.

**Read this first.** `pairings.json` (**22 entries**) records each invocation; the gate
checks it; the closeout harness reports it and stamps a revision snapshot.

**The frozen pass the task title asks for could not be performed, for the third turn
running, and this is the measured reason**: 19 open todos (P1 weight), and during this
40-minute pass three docs were edited under me (`module-agent-core` 21:08:38,
`module-tauri-shell` 21:07:27, `module-cli` 20:55, `module-tauri-remote` 21:03:20) with
new reports appearing at 20:56 and 21:03. Two docs changed *while the pairing for them
was being repaired*. What this pass delivers instead is the durable part: the
**vacuity guard** (below — the most important finding), the **`--suggest`** mode that
automates the repair I had been doing by hand five times, two more pairings, and a green
stamp/verify pair. The frozen run remains the successor's job.

## The vacuity hole: this gate could be made GREEN by a report with nothing in scope

Found by `--suggest` (then confirmed deliberately) and fixed this pass. Both halves of
the check are computed **from the report**, so a report that supports neither comparison
satisfies both trivially:

* `missing` iterates the report's uncovered lines — none, so 0;
* `covered` asks whether the doc's claims are in that uncovered set — none are, so 0.

Measured, before the fix:

```
$ # the `goal` pairing, report repointed to a desktop-scoped zeroed report
| goal | authoritative | module-goal.md | 92 | 0 | 0 | 0 | 0 | PASS |
GATE OK: every covered row is attributed to a justified section      (exit 0)
```

So the campaign's own acceptance gate could be turned green by handing it a
wrong-scope or zeroed report — the exact failure mode `covlines.py` exits 3 on and
`reconcile.py`'s `main()` guards with exit 3. `reconcile.py` was never wrong here; the
gate called the loader directly and inherited a green instead of the guard. Two guards
now, both failing the pairing:

* **`per_file` empty** → `NOTHING WAS CHECKED: <report> has no record for any of this
  pairing's N scopes` (the measured hole above);
* **records present but 0 uncovered while the ledger cites lines** → `REPORT CANNOT
  SUPPORT THE CHECK`, since that reads either as a zeroed/truncated report or as a unit
  genuinely at 100 % whose ledger should be cleared — both worth a failure, neither a
  PASS.

Pinned in `selftest.py` item 9: the two zeroed reports FAIL with the reason, and no live
pairing trips the guard. `--suggest` also refuses vacuous candidates (item 10), which is
how the hole was found: its first run offered `/tmp/zeroed.info` as a "clean candidate".

## `--suggest`: the repair I had been doing by hand, now one command

Five times this campaign a unit re-measured and its doc's numbering moved to a new
report, leaving the pairing red; each time the diagnosis was a hand-run of the
measurement against every candidate on disk. `--suggest` does it: for each
non-clean pairing it scans the candidate reports, prints those that WOULD make it clean
(missing 0, covered 0, and the ledger's claims actually resolving into the candidate's
uncovered set), prefers ones the doc itself names, and sorts newest-first. It is a
suggestion to verify by eye, not a rewrite.

First real use, this pass: `agent-core` had moved to its unit's pass-5 report, and
`--suggest` named `/tmp/agb-p6.info` as the clean candidate the doc also names
(213 uncovered, 213 claims, 0 missing, 0 covered). Re-pointed on that basis.

## Two pairings repaired this pass, both because a doc moved mid-pass

| pairing | what happened | resolution |
|---|---|---|
| `tauri-shell` → split into `tauri-shell` + `tauri-shell-seg2` | the TAU-C2 unit added a SECOND ledger for a second revision (21:05:50): sections 1–8 hold 318 rows for the previous revision, the new tail section holds 270 for the current one. One whole-doc pairing mixed them → 4 covered + 58 absent | the doc's own words say §1–§8 are "deliberately left untouched" and the new table "is the authoritative one", so one pairing per revision, each named by its section: 318/0/0/0 and 270/0/0/0, both PASS |
| `agent-core` | its unit published pass 5 (`agb-p6.info`, 21:03:24) and re-pointed §6 at it (21:07:17) | re-pointed; 213/213/0/0/0 |

`tauri-shell` also carries a **finding I did not act on**: the reconcile invocation its
doc prints above the new table (`--scope desktop/src-tauri/src --section "Segment 2"`)
reads **MISSING 1313** when run verbatim, because that scope is the whole `src` tree
(1680 uncovered lines) while the unit's ledger covers its own 12 files. A reviewer who
runs the printed command gets a red book and no explanation. It is that unit's doc, so
it is recorded in the pairing's note rather than edited.

## A live-state assertion I had to remove from my own self-test

`selftest.py` asserted "`--require-named` passes on the live pairings" — a check whose
result depends on whether a peer's doc happens to be clean at that instant. It passed
when written and failed the first time a doc moved (`tauri-shell`, 21:05:15), taking the
selftest red for someone else's edit. Replaced with the **invariant** between the two
modes on the same tree: any unnamed pairing makes `--require-named` exit 1; with none
unnamed its exit equals the plain gate's. That is a statement about the tool, not about
a peer's doc — the same lesson this unit has now had to learn twice, and the first thing
I checked when the selftest came up red at the start of this pass.

## Current state

```
--gate                        exit 0
--gate --strict               exit 0    DRIFT rows 0
--gate --require-named        exit 0    NOT stated 0 (section 17, doc 4, companion 1)
--stamp / --verify-stamp      exit 0    22 pairings, git e6b8f046, 4 dirty paths
selftest.py                   exit 0    69 checks
15 authoritative pairings -> clean 14, justified covered rows 1 (channels-policy), failing 0
9 docs still have no pairing (8 prose-only + module-agent-llm.md, whose report is absent)
```

Snapshot: `.future/cov100/gate-snapshot.json`. The task asked for it at
`coverage/final/pairing-gate-snapshot.json`, which is **outside this unit's declared
write set**; `measure-final.sh` writes it there automatically when the harness runs (the
wiring and the appended verify command were added last pass and are asserted by
`.measure-probe.sh`), so the frozen pass gets it in the right place by running the
harness.

**Why the snapshot verifies on file hashes, not on the revision** — observed twenty
minutes after writing it: HEAD moved from `e6b8f046` to `2c9aa504` (four peer "safety
snapshot" commits, one of which landed the fmt violation below), while
`--verify-stamp` still reported **OK, every doc and report unchanged**. That is the
intended behaviour and the reason the stamp carries per-file `sha256` next to the rev: a
revision-based check would cry wolf on every unrelated commit, whereas the question a
frozen pass must answer is whether *the docs and reports the verdicts came from* are
still the ones on disk. The rev is recorded as context, the hashes are the check.

## A committed fmt violation in a peer's file (reported, not fixed)

`cargo fmt --check` went red **during this pass**, on a file outside my write set:

```
cli/src/browser/safari/safari_manager.rs:793
  -            format!(
  -                "#!/bin/sh\ntouch '{}'\nexec sleep 20\n",
  -                marker.display()
  -            ),
  +            format!("#!/bin/sh\ntouch '{}'\nexec sleep 20\n", marker.display()),
```

It is **committed**, not a working-tree edit: the blob at HEAD fails the same check, and
`git log` attributes it to the CLI-B unit's safety-snapshot commits (`938e434e`,
21:11:55). CI runs `cargo fmt --check`, so this would redden the Rust job. Not mine to
fix (different unit's file, and they are mid-work); recorded here because a red fmt gate
in a committed blob is exactly the kind of thing a closeout should surface. Note the
mechanism too: a *safety snapshot* — the automated commit these units run to preserve
work in a shared worktree — is how an unformatted file reached HEAD, and the same
mechanism swept this unit's own uncommitted edits in an earlier turn.

## The 5-pre column and the snapshot

The 5-pre column (**NOT stated 0**) means every pairing's report is named by its own doc
or a companion record — the property CLOSEOUT §5-pre requires, now measured rather than
assumed. See the dedicated section below for the history and `--require-named`, its
mechanical form.

## The revision snapshot (`--stamp` / `--verify-stamp`) — added in the final pass

The gap this closes: a green gate is a statement about **which docs** it read, and the
recorded `expect` numbers say nothing about which revision produced them. Measured, not
hypothetical — `module-cli.md` was written at 20:54:19, **one second after** a green run,
and three docs moved again during this final pass.

* `--stamp FILE` writes: git revision + dirty-path count, the pairings file's hash, the
gate exit code, and per pairing the doc's and report's `sha256` + the report's size +
the measured counts. It also records `docs_not_covered_by_any_pairing`, because a green
gate over 14 verified docs says nothing about the other 9.
* `--verify-stamp FILE` re-hashes every doc and report the snapshot names and exits 1 if
any moved or vanished, printing both hashes. Verified in both directions by
`selftest.py`: a fresh stamp passes; a stamp with a corrupted hash fails naming the file;
a stamp naming a deleted file fails as MISSING rather than passing.

**This is the closeout precondition** — not the gate (see the decision below).

## The reported-step vs precondition decision (CLOSEOUT 5, and the harness)

**Decision: the gate is a REPORTED STEP inside `measure-final.sh`, and the PRECONDITION
at closeout is `--verify-stamp`.** Rationale, recorded because the opposite choice is
defensible too: the harness runs 1.5–3 h of instrumentation, and aborting it because one
module's ledger disagrees destroys the measurement that would have shown *how* it
disagrees — a red row belongs *in* the report. What a frozen revision can and must
satisfy is the stronger property: this snapshot still describes this tree. One-line
change if the supervisor prefers a hard precondition (the `case $gate_rc` block);
deliberately not made.

## The CLOSEOUT 5-pre column (`--require-named`)

The supervisor added §5-pre: "which report does the ledger section itself name? ... a
check against a report the ledger does not cite manufactures false findings" (after
their own agent-core audit produced phantom covered rows that way — the same failure this
unit hit twice). That property is now **measured per pairing** instead of assumed:

```
pairing stated by the doc (CLOSEOUT 5-pre): section 16, doc 4, companion 1, NOT stated 0
```

**Every one of the 21 pairings is now stated by its own doc** — §5-pre is satisfied
mechanically, not by assertion. It was 1-not-stated until the end of this pass:
`tauri-remote` had been paired against `lcov2` *by matching* (the doc then named only
`lcov`, and against that report 24 of its lines were uncited) and was labelled
reconstructed. The unit then published `/tmp/tauri-tau-remote-lcov4.info` (20:54) and
named it in the doc (21:02); measured against lcov4 the ledger is exactly clean (331
uncovered, 109 claims, MISSING 0, covered 0), so the pairing became a documented one and
the reconstructed count fell to 0. The gate's red on that 21:02 edit is what surfaced the
newer report — the second time in two turns that a heading/verdict change was caught by
the check rather than by reading.

* `section` — the ledger slice names the report (16 pairings).
* `doc` — the doc names it elsewhere (4).
* `companion` — a sibling record names it, via the pairing's `named_in`
  (`channels-policy`: the policy page has no command block; `module-channels.md` §10.7.5
  runs reconcile against it with that report).
* `NOT stated` — 0 at handoff. The shape is still possible and still reported, and
  `tauri-remote`'s history above is the worked example of one.

`--require-named` turns that column into a failure (the mechanical form of §5-pre).
Pinned by `selftest.py`: the same fixture passes without the flag and exits 1 with it,
and the failure now prints its reason. (It did not at first: the flag appended
`("unnamed:"+label, m)` tuples to the failure list, and the detail printer reads
`p['label']` — which never ran, because that entry's problems list was empty. So the
failure printed as a bare count. The self-test caught it by asserting the reason is
present.)

### Finding: CLOSEOUT §5-pre and §5a name a tool that does not exist

§5-pre/§5a cite `strict-claims.py` (with "exits 4") and `threeway.py`. **Neither file is
in `.future/cov100/`** (checked). The obligations may still be right; what is missing is
the mechanism — and a closeout bullet naming a non-existent check reads as satisfied. My
gate now measures the *pairing* half of it (`--require-named`); the per-line set-equality
half ("compare the table's sorted `(path, line)` set against the report's uncovered set,
refuse to guess on ambiguous basenames") is **not implemented by either script**, and I
did not create files named by someone else's plan. Reported for the supervisor to
implement or to reword.

## 4 pairings added in the final pass (13 unpaired docs -> 9)

`--stamp` printed `docs_not_covered_by_any_pairing` and revealed that 13 docs had no
pairing — and that **a doc can parse claims and still never be checked**. Four were
closable immediately, each validated by MISSING 0 like every other pairing:

| new pairing | doc | report | claims / uncovered / missing / covered | provenance |
|---|---|---|---|---|
| `cli-mcp` | module-cli-mcp.md §6 | `/tmp/clamcp-final-lcov.info` | 4 / 4 / 0 / 0 | reconstructed from its §6 rows |
| `tauri-appdata` | module-tauri-appdata.md §S2.4b | `/tmp/appdata-normalized-lcov.info` | 610 / 610 / 0 / 0 | **the doc's own invocation, verbatim** |
| `tauri-remote` | module-tauri-remote.md (whole) | `/tmp/tauri-tau-remote-lcov4.info` | 109 / 331 / 0 / 0 | **documented** (see above — reconstructed against lcov2 until the unit published and named lcov4 mid-pass) |
| `tauri-headless` | module-tauri-headless.md (whole) | `/tmp/tauri-tau2-lcov.info` | 261 / 222 / 0 / 0 | documented (`after-2`, the report §2 tabulates) |

`tauri-appdata` and `tauri-headless` were the two docs whose recorded "0 STALE" I had
called vacuous in an earlier version of this file. Both are now *checked*: appdata
against the exact command its own text prints (610 mentioned / MISSING 0), and headless
against `tau2` — where it is genuinely clean (0 missing, 0 covered) while against the
earlier `tauri-tau` it showed 11 covered rows and 66 uncited lines, which is why its
recorded line was meaningless until it named a report.

`tauri-headless` was also in the "shape 2" table below as unfixed; the shape landed, so
that table's remaining entries are the 8 prose-only docs, not 11.

The 4 pairings whose scope I had to reconstruct were checked the same way: a scope that
does not match the ledger prints MISSING in the hundreds, so MISSING 0 is the criterion.
For `tauri-remote` the whole-doc slice is what makes MISSING 0 — its §4.1 alone leaves
one line uncited (§4.2 cites it), which is the "right rows, wrong slice" case this file
records for agent-rpc.

## DRIFT handled as instructed (`--strict`)

At the start of this pass `--gate --strict` was already 0-DRIFT. During it, one row
drifted and was **re-measured and re-recorded rather than loosened**: `agent-rpc`'s §0.5
claim count moved 296 -> 281 when its unit (AG-A, still open) edited the doc at 20:59:22
— verdict unchanged (uncovered 324, missing 0, covered 0). Recorded in the pairing's
note. Heading mismatches: 0 (the cli §4c `(and S8)` -> `(and S10)` rename of the previous
turn is already absorbed).

## The gate

`pairings.json` holds, per invocation: report, scopes, doc, **the section the doc
itself declares current**, the heading that needle must resolve to, where the
invocation is documented, the expected counts, and — when covered rows are expected —
a justification (the doc sections they come from + verbatim quotes + the reason).
`staletable.py --gate` exits 1 when a covered row is unexplained, when a justification
quote no longer exists in the doc, when a covered row comes from a section nobody
justified, when the recorded expectation drifted (`--strict`), or when the section
needle resolves to the wrong heading. `--md` emits the same verdicts as the markdown
block the closeout report carries.

**The heading check earned itself twice inside this turn**, which is why it exists
alongside the count checks: the cli unit renamed §4c from "(and S8)" to "(and S10)", and
the gate failed on `needle landed on '… S9 (and S10)', not the recorded '… S7 (and S8)'`
— with the counts unchanged (`DRIFT 0`), i.e. the only signal was the heading, and a
count-only assertion would have passed a pairing whose target had moved underneath it.
The same check is what pins each pairing to a real ledger rather than to a section that
merely mentions it.

**In the closeout**: `measure-final.sh` runs the gate as a *reported step* after the four
measurement tracks — it captures the exit code, appends the tool's markdown block
(verdict table + justified rows + any **Failing pairings** list) to `FINAL-REPORT.md`,
and then prints `gate step exit: N`. A red gate does **not** abort the report, because a
red row names some module's ledger and the report is where a reviewer must see it; the
harness keeps its own contract (exit 3 = an invalid *report*). Both paths are exercised
by `.measure-probe.sh` (run from `selftest.py`): a green run yields one gate heading, 17
paired rows and the captured exit code; a run driven by the negative fixture yields
`gate step exit: 1`, a **Failing pairings** section, harness exit 0 and a stderr warning.

The gate is not a rubber stamp — it is pinned in both directions by
`selftest/pairings-negative.json` (three failing pairings, each a distinct failure
mode) and `selftest/pairings-positive.json` (the same fixture with a real quote and a
correct needle), and `selftest.py` asserts both. It earned that on first use: the
first `--gate` run failed on `channels-wide` because the five section names I had
recorded were abbreviated transcriptions of the real headings — a fabricated-name
justification, caught before it reached this record.

One design point the self-test itself forced: it must **not** assert a green verdict on
the live `pairings.json`, because that verdict depends on peer docs that change
mid-run (agent-core went clean → failing → clean on its own unit's edits). The
self-test asserts the gate's mechanism (it runs, it summarises, its failures name a doc
and a reason) plus the fixtures this unit owns; the live verdict is a *finding to read*,
not a test result.

Its second catch was inherited from the previous turn's own probing: `--section "4c"`
on `module-cli.md` resolves to `### Segment 3 (mock-span hygiene) — the three runs
behind §4c`, a cross-reference ~500 lines ABOVE the ledger, so the run reads
`mentioned: 0` for a 122-line ledger. Every pairing now records the heading its needle
must land on, and `selftest/fixture-trap-doc.md` pins that shape permanently.

## The original defect, and this file's co-edit history

`claimed_uncovered()` reads a claim from a markdown row's first cell (or its second
when the first is a bare tag). Its pattern demanded the closing pipe *immediately*
after the line numbers, so the repo's commonest shape — one backtick pair around path
and number together, `` | `src/a.rs:12` | … `` — matched nothing, and `stale` came out
0 for a doc whose rows had never been parsed. A doc with table rows and zero parsed
claims looks exactly like a correct ledger in the output; that is how the blind spot
survived a campaign of "0 STALE" reports.

| when | what | by |
|---|---|---|
| 20:19 | the pattern fixed (backtick before the pipe); `LEDGER_ROW` exposed; `slice_section()`; the vacuity **WARNING** | this unit (todo_7449334225b8) |
| 20:23 | `LEDGER_ROW_TAGGED` and `LEDGER_ROW_SPACED` added | **another agent, same file** |
| 20:30–20:40 | tag class tightened; ABSENT/COVERED split **inline**; section attribution; `counts_for()` (a real bug, below); `istanbul_counts()`; empty-scope exit 3; `patternprobe.py`, `staletable.py`, `staletriage.py`, `blindrework.py`, `selftest.py` | this unit (todo_10ad23544fa1) |
| 20:40–20:50 | `pairings.json` + `--gate`; `sections_matching()`; quote/attribution/needle verification; negative + positive gate fixtures; `measure-final.sh` obligation wording corrected | this unit (todo_ab3b61bf54f4) |

## The bug the self-test found (the most important line in this file)

The first version of the split reported **`COVERED 0` for every pairing** — because
`split_stale()` looked a `key_files()` **needle** (`persistence.rs`) up directly in a
`report_counts()` mapping keyed by **full normalised paths**
(`agent/src/session/persistence.rs`). Every lookup missed, so every claimed line was
classified ABSENT, and the check said "no ledger disagrees with its report" for
exactly the wrong reason. That is the silent-zero class this campaign keeps
re-finding, and it would have gone into a handoff as a clean result.

It was caught by the fixture in `selftest.py`/`selftest/` — a hand-written pairing
whose covered lines *must* come back COVERED — not by reading the code. Fixed in
`counts_for()`. **Conclusion to distrust:** any earlier statement in this campaign
that "COVERED is 0 everywhere" was produced by the broken lookup.

## What the STALE half prints now

```
$ python3 .future/cov100/reconcile.py --label cli --report /tmp/cli-r2s2-final-lcov.info \
    --scope "$PWD/commands" --scope "$PWD/browser" --scope "$PWD/cli/src" \
    --doc docs/testing/module-cli.md
[cli] files with uncovered lines=21 uncovered lines=133
  mentioned in module-cli.md: 133
  MISSING (uncovered, not mentioned): 0
  STALE (doc cites it as uncovered; old doc sections also match unless --section is given): 51
    COVERED (DA>0 - a real disagreement, the actionable half): 38
       browser_tools.rs:2827
       ...
    ABSENT (no `DA:` record -- NOT evidence: ...): 13
       absent configure.rs:48
       ...
    -> the covered rows come from 1 section(s): 4. Waiver ledger — every residual line, with its (38)
       this doc holds 2 sections with claims: if one of them is superseded (a re-measurement), re-run with --section for the authoritative ledger
exit=0
```

Three pieces of information, each of which changes the reviewer's next action:
the **split** (38 disagreements, 13 non-evidence), the **section the rows came from**
(§4, which this doc's own note says §4b supersedes), and a **pointer** to the
authoritative ledger. With `--section 4b` the same run reports **0 STALE**.

Exit codes: `1` = MISSING > 0, `2` = unknown `--section`, `3` = **the scope matched no
report record** (new — see below), `0` otherwise. A `COVERED` line does not change the
exit code: it is a finding for a human, and 8 docs would otherwise redden their own
self-check.

## All pairings, measured (`python3 .future/cov100/staletable.py --cli`)

| pairing | role | doc | claims | uncov | STALE | ABSENT | **COVERED** | exit | verdict |
|---|---|---|---|---|---|---|---|---|---|
| goal | authoritative | module-goal.md | 92 | 92 | 0 | 0 | **0** | 0 | PASS |
| agent-core (§6, p6 report) | authoritative | module-agent-core.md | 213 | 213 | 0 | 0 | **0** | 0 | PASS |
| agent-rpc (§0.5, S7 report) | authoritative | module-agent-rpc.md | 281 | 324 | 0 | 0 | **0** | 0 | PASS |
| agent-rpc-r1 (§6 round-1) | superseded-ledger | module-agent-rpc.md | 169 | 324 | 144 | 45 | **99** | 1 | PASS (justified) |
| channels (§10.4) | authoritative | module-channels.md | 6 | 8 | 0 | 0 | **0** | 0 | PASS |
| channels-wide | wide | module-channels.md | 167 | 8 | 25 | 3 | **22** | 0 | PASS (justified, 5 sections) |
| channels-superseded-report | superseded-report | module-channels.md | 15 | 24 | 5 | 0 | **5** | 1 | PASS (justified) |
| channels-policy | authoritative | channels-test-coverage.md | 13 | 8 | 6 | 0 | **6** | 0 | PASS (justified; report named in a companion) |
| review-rust | authoritative | review-rust.md | 4 | 24 | 0 | 0 | **0** | 0 | PASS |
| desktop (Istanbul JSON) | authoritative | module-desktop.md | 61 | 60 | 0 | 0 | **0** | 0 | PASS |
| mobile | authoritative | module-mobile.md | 17 | 13 | 0 | 0 | **0** | 0 | PASS |
| tauri-shell (previous revision's ledger) | superseded-ledger | module-tauri-shell.md | 318 | 318 | 0 | 0 | **0** | 0 | PASS |
| **tauri-shell-seg2** (current revision) | authoritative | module-tauri-shell.md | 270 | 270 | 0 | 0 | **0** | 0 | PASS |
| cli (§4c, S9 report) | authoritative | module-cli.md | 119 | 123 | 0 | 0 | **0** | 0 | PASS |
| cli-4b (§4b, S5 ledger) | superseded-ledger | module-cli.md | 129 | 123 | 11 | 3 | **8** | 0 | PASS (justified) |
| cli-4 (§4, S3 ledger) | superseded-ledger | module-cli.md | 188 | 123 | 55 | 11 | **44** | 1 | PASS (justified) |
| cli-wide | wide | module-cli.md | 203 | 123 | 62 | 14 | **48** | 0 | PASS (justified, 2 sections) |
| tui (§9.4, its OWN report) | authoritative | module-tui.md | 5 | 13 | 0 | 0 | **0** | 0 | PASS |
| **cli-mcp** (added this pass) | authoritative | module-cli-mcp.md | 4 | 4 | 0 | 0 | **0** | 0 | PASS |
| **tauri-appdata** (added) | authoritative | module-tauri-appdata.md | 610 | 610 | 0 | 0 | **0** | 0 | PASS |
| **tauri-remote** (added, now documented) | authoritative | module-tauri-remote.md | 109 | 331 | 0 | 0 | **0** | 0 | PASS |
| **tauri-headless** (added) | authoritative | module-tauri-headless.md | 261 | 222 | 2 | 2 | **0** | 0 | PASS |

Raw CLI output with exit codes: `stale-check-after.log` (22 runs). DRIFT rows: **0** at
handoff, so every recorded expectation matches its measurement. The four bolded rows in
the earlier list are the previous pass's additions; this pass added `tauri-shell-seg2`
and split `tauri-shell` (see "Two pairings repaired this pass").

## The one real failure, and how it resolved (agent-core — now PASS)

`module-agent-core.md` was restructured by its own unit at 20:44:25: §6's rows were
split per category and their line numbers renumbered. Against **the report §6 cited at
that moment** (`/tmp/agb-final3.info`), 5 new line numbers were shown COVERED and 2 had
no record at all:

```
llm/mod.rs:1355, 1516, 1949   run_loop.rs:2005, 4990    <- DA>0 in agb-final3.info
llm/mod.rs:1607   run_loop.rs:4957                      <- no DA: record
```

I recorded **no justification at the time**, because none existed *for that pairing*.
But the answer was in the same measurement: the unit had published **`/tmp/agb-final6.info`
(20:41:50)** and updated §6 to cite it — and against *that* report the ledger is exactly
clean (**212 uncovered, 212 claims, missing 0, covered 0, absent 0**). So the failure was
"the pairing cited a stale report", not "the rows are wrong", and the fix was one field
in `pairings.json`. That distinction is the whole reason the gate demands a *pairing*
(name the report) rather than a bare count: the same 212-line ledger reads as 5 wrong
rows or as 0 depending only on which report you compare it with.

```
# the measurement that answered it, over every on-disk agent-core report
agb-final3.info    uncovered= 212 missing= 7 covered= 5 absent= 2
agb-final6.info    uncovered= 212 missing= 0 covered= 0 absent= 0   <-- the doc's
```

The same resolution applied to **cli** at 21:00: its §4c is numbered per **S9**
(`/tmp/cli-r2s3-final3-lcov.info`, created 20:45) and the doc says so in words ("the
doc's line numbers are S9's"). Measured against the S7 report the pairing had used, one
claim (`safari_manager.rs:820`) had no `DA:` record — because the comment normalisation
that moved it 819 → 820 happened *between* S7 and S9. Re-paired to S9: 123 uncovered,
119 claims, missing 0, covered 0, absent 0.

**Both failures were caught by the gate rather than by reading**, and both were resolved
by changing a report reference — which is exactly the failure class the harness now
prints as data instead of hiding behind a green sentence.

The same churn explains two other rows changing while this table was written:
`tui` was recorded as unverifiable when its doc's own report was absent, the gate
refused to pass it, and when the unit produced `/tmp/tui-r2-crate.info` at 20:44 the
pairing became clean (13 uncovered, all cited, 0 covered) — the refusal is what
surfaced the change. `agent-rpc`'s ledger grew from 280 to 296 claims between two edits
by its own unit and stayed clean (0 covered, 0 absent, MISSING 0).

**Changes versus the previous table (13 rows, no roles)** — the numbers moved for two
reasons, both the doc's doing, not the tool's:

* **`cli`: 38 COVERED → 0.** The previous row measured the S5 report against the whole
doc, so it compared §4 (S3) and §4b (S5) ledgers with a report §4c's own intro calls
superseded ("Read from the report the reviewer should use:", S7 =
`/tmp/cli-r2s3-final-lcov.info`). The authoritative pairing is now S7 + §4c, which the
doc says is current and which measures MISSING 0 / COVERED 0 / ABSENT 0. The wide and
superseded numbers are kept in full (39 / 7 / 43) instead of being averaged away —
that is the point of recording roles rather than one number per unit.
* **`channels`: 3 COVERED → 0** for the same reason (the authoritative pairing is
§10.4, the ledger the doc names as current); the previous row's §10-wide slice showed
3 and the full-doc figure is 22, now recorded as `channels-wide`.
* `agent-rpc-s6` → `agent-rpc-r1` (same 99, now carrying the justification), and
`agent-rpc-s0` → `agent-rpc` (S7 report, §0.5 — the unit refreshed that ledger twice
while this table was being written: 280 → 296 claims, still clean).
* `tui` moved twice: the doc was rewritten at 20:43 (its old whole-doc ledger became a
§9.4 ledger citing a report that did not exist yet), so the pairing was recorded as
`unverifiable` and the gate refused to pass it; the report appeared at 20:44 and the
pairing went green against it. The `expect` numbers today are 13 uncovered / 5 claims /
0 covered.
* `agent-core` **FAILED first and then resolved as a stale report**: its unit
restructured §6 at 20:44:25 while publishing `/tmp/agb-final6.info` (20:41:50); against
the report the pairing named at that moment, 5 rows were covered. Re-paired to final6
(a one-field change) the ledger is exactly clean — see "The one real failure, and how it
resolved" below. Same pattern for `cli` at 21:00 (§4c is numbered per S9).
* `channels-superseded`, `review-rust-revch`, `cli-wide` keep their measurements;
they are renamed/re-roled, not re-tuned.

`desktop` needed the Istanbul path: an earlier `staletable.py` hard-coded the lcov
reader and printed `uncov 0` for a report with 60 uncovered lines — another silent
zero, fixed by moving the format dispatch into `report_lines()` / `report_counts()`
so every tool reads a report the same way.

## Triage of the COVERED rows (workstream 2's real output)

`staletriage.py` splits each covered row into "an uncovered line sits within ±5"
(line drift) and "standalone", and flags files modified in the worktree. The verdict
per pairing, by inspecting the rows' own text:

| pairing | COVERED | where the rows live | verdict |
|---|---|---|---|
| cli-4 | 44 | §4 "Waiver ledger", whose own note says *"authoritative final ledger is §4b … The line numbers in this table are the ones S3 reported"* | **superseded ledger kept in the same file** (S3). §4c → 0 |
| cli-4b | 8 | §4b, whose heading says *(superseded by §4c)* | **superseded ledger kept in the same file** (S5). Justified |
| agent-rpc-r1 | 99 | §6, and the header blockquote says *"Round 2 (cov100-r2) supersedes §1–§9 below for scope, figures and ledger."* | **superseded ledger** (round 1). §0.5 → 0 |
| channels-wide (10) | 10 | the `attribution-artifact` category table | **the doc declares this category empty** ("now **empty for this crate**") |
| channels-wide (8) / channels-superseded (5) | 8 / 5 | §10.7.2 *"The 16 fallen rows: what covers each line now"*, §10.2 *"What is covered now, and by which test"*, §10.2b | **the row says the line is covered** (each names the new test) |
| channels-wide (3) | 3 | the `unreachable-in-this-environment` category table | same as the first: category declared empty |
| channels-policy | 6 | the table under `### `unreachable-in-this-environment` (0)` — the heading carries the count | **the heading says the group is empty**; the page's intro says those two groups are "now empty for this crate" |

So: **every covered row in this campaign sits in a table that describes history or says
the line is covered now** — no ledger claims a covered line while meaning it. Both
remaining classes are doc-side, and the tool names them rather than hiding them:

1. **superseded ledger in the same file** — the pairing record points at the
authoritative section, and the tool prints which section the covered rows came from,
so this is one command to see (and `pairings.json` keeps the superseded number so its
disappearance is visible too).
2. **rows whose prose says "covered now"** — mechanically indistinguishable from a
stale claim. I deliberately did **not** add prose inference (matching `✔`, `now
covered`, `fallen row` would be guessing, and a wrong guess here hides a real finding).
The honest route, now enforced: a pairing with covered rows must carry a recorded
justification whose quotes exist in the doc; for an *authoritative* pairing the
alternative is to drop or mark those rows.

## The empty-scope guard (exit 3)

One run in this turn printed `files with uncovered lines=0 uncovered lines=0` for a
4.3 MB report that the next identical run read as 347 uncovered lines, and the
reproducibility check passed — i.e. an unexplained zero. Rather than leave that
mistakable for 100 % coverage, `reconcile.py` now exits **3** when **no `SF:` record
matches the `--scope`**. That is unambiguous: a fully covered scope still yields
per-file entries (with empty line sets), so an empty mapping is always a bad scope, a
bad format, or an incomplete report. Both sides are pinned in `selftest.py`.

## The tag-class audit (tightening `LEDGER_ROW_TAGGED`)

The pattern accepted any 1–13 character word as the first cell, so
`| plain prose | `src/a.rs:12` | x |` was a claim — a false-STALE door. Measured
before changing it: all **86 rows** in `docs/**/*.md` that only this pattern accepts
carry a `D<n>` tag (`D1`–`D54`), and substituting the loose class for the tight one
changes the parsed claim set of **0** of **243** docs. The tightening therefore
removes the door with no measured loss, and the residual risk (a future `TAU-C`-style
tag) is the loud kind: 0 claims → the vacuity WARNING.

`patternprobe.py`: **24 cases with expectations, 0 mismatches**, covering the four
accepted shapes, five real rows from these docs that must stay rejected (they are
counts, slashed ratios, or prose), and the tag class. `--contrast` reports **5 flipped
cases** against the pre-tightening class, so the probe is shown to discriminate.

## What is still unverified — 9 docs (see `LEDGER-REWORK.md`)

The final pass reduced this from 13 to **9**, in two distinct classes:

* **Eight module records parse 0 claims**: `module-agent-sandbox.md`, `module-agent.md`,
  `module-cli-tui.md`, `module-agent-session.md`, `module-tauri-bridge.md`,
  `module-tauri-core.md`, `module-tauri.md`, `module-tauri-terminal.md`. Measuring each
  (`blindrework.py`) showed the cause is not a shape: they contain **0 claim-shaped
  rows** and between **0 and 7** `path:LINE` citations in the whole document (mostly
  inside prose), and every one names a `coverage/*.json` report that is **absent from
  this worktree**. For `module-agent-sandbox.md`, the tripwire reads `mentioned 0 of 43`.
  That is a PLAN §5.2 gap to schedule (registered as the supervisor's decision
  `todo_6a850a842666`), not a tooling fix — the detail, the five must-reject
  counterexamples, and the doc-side change are in `LEDGER-REWORK.md`.
* **`module-agent-llm.md`** — a different case: it HAS a parseable ledger (28 rows /
  106 claims) and is not prose, but its report is absent, so no pairing can be sound.
  It needs a re-measure, not a restatement.

`--stamp` reports both classes as `docs_not_covered_by_any_pairing`, so the gap cannot
grow silently: adding a doc to that list is now visible in the closeout artifact.

## Reproduce

```bash
python3 .future/cov100/selftest.py                     # exit code = verdict (69 checks)
python3 .future/cov100/staletable.py --gate            # THE gate: exit 1 on an unexplained
                                                       # covered row, a vanished quote, an
                                                       # unattributed section, a mis-sliced
                                                       # section needle, or a report that
                                                       # cannot support the check (--strict
                                                       # adds drift)
python3 .future/cov100/staletable.py --suggest         # for each red pairing, the reports
                                                       # that WOULD make it clean (verify by eye)
python3 .future/cov100/staletable.py --require-named   # + CLOSEOUT 5-pre's exit-4 condition
python3 .future/cov100/staletable.py --stamp .future/cov100/gate-snapshot.json
python3 .future/cov100/staletable.py --verify-stamp .future/cov100/gate-snapshot.json
                                                       # THE closeout precondition, not the gate
python3 .future/cov100/staletable.py --md              # the markdown block the closeout carries
bash .future/cov100/.measure-probe.sh                  # the harness's gate + snapshot steps,
                                                       # green and red
python3 .future/cov100/patternprobe.py --contrast      # 24 accept/reject cases, 5 flips
python3 .future/cov100/rowscan.py --needle 4c docs/testing/module-cli.md   # ambiguous needle
python3 .future/cov100/blindrework.py                  # the per-doc rework table
```

**Check every exit code with a separate command.** A loop like
`for m in "--gate" "--gate --strict"; do python3 staletable.py $m; echo $?; done` is
wrong under **zsh** (this host's shell), which does not word-split unquoted parameters:
`$m` arrives as one argument containing a space, argparse exits **2**, and the tool looks
broken. Observed here and mistaken for a defect for one command.

Tools: `ledgerrows.py` discovers the shipped patterns (nothing restates them);
`rowscan.py` counts rows/claims per doc and slice; `claimrows.py` shows which row
matched which pattern; `zeroclaims.py` finds rows accepted but turned into 0 claims;
`staledetail.py` prints the split in full; `staletable.py` runs every pairing **and is
the gate**; `pairings.json` is the record it reads; `staletriage.py` classifies the
covered rows; `patternprobe.py` asserts the accept/reject contract; `blindrework.py`
measures the unknown ledgers; `selftest.py` pins all of it (including the gate's own
negative cases in `selftest/pairings-{negative,positive}.json`).

## What changed in the closeout wording (workstream 4)

The obligation in `measure-final.sh` now reads: 0 MISSING for the unit's
**authoritative** report+section, and no COVERED STALE row that `pairings.json` does not
justify, with `staletable.py --gate` exiting 0 as its mechanical form — plus a note that
ABSENT rows are not evidence, and the gate as a reported step (below).

`.future/cov100/CLOSEOUT.md` §5 said "`reconcile.py` 0 MISSING and 0 STALE". The
supervisor had already folded this unit's agent-core finding into §5a ("Neither sees a
row citing the WRONG line — which is exactly what agent-core §6 did"), so §5 was the
last stale sentence. It is now rewritten **with an inline HTML comment marking the
change, its reason and how to revert it**, because that file is `Owner: supervisor` and
live (last updated 20:46:08, i.e. inside this turn's window). The marked edit is the
honest way to correct a shared live doc: the owner sees what changed and can drop the
comment without archaeology. Its §5a requirement — "the pairing (which section against
which report) must be stated" — is exactly what `pairings.json` records and what the
gate checks mechanically.

## The harness had a silent-failure bug of its own (found by wiring the gate in)

`measure-final.sh` used `declare -A RESULT` and tracked invalid tracks as
`RESULT[rust]=invalid`. This host's bash is **3.2**, where `declare -A` is invalid, so
under the script's own `set -u` the assignment itself failed — measured:

```
$ bash -c 'set -uo pipefail; declare -A R; R[x]=1'
declare: -A: invalid option
x: unbound variable            # exit 2
```

An invalid track was therefore **never recorded**: the report kept its bad number and
the harness exited 0 — the silent-pass class this campaign's tools exist to catch, in
the acceptance harness itself. Fixed portably (`INVALID_TRACKS` string), verified in
isolation on bash 3.2 (recorded → exit 3; clean → exit 0) and end-to-end by the probe.
`coverage/final/` does not exist, so this harness has **never been run** on this host;
that is worth knowing before the closeout relies on it.

Two smaller defects the same wiring exposed: a `sed -e '/^$/N;/^\n$/D'` intended to
strip a duplicate heading silently did nothing (the `N` moves the next line into the
pattern space of the preceding blank line, so the address never matched — the report
kept **two** gate headings), so the report now appends the tool's block unfiltered; and
a probe whose copy sat in `/tmp` resolved `ROOT` to `/` because the script derives it
from `$(dirname "$0")/../..` — `mkdir: //target: Read-only file system`. Both are
recorded because both are the kind of thing that reads as "the step ran".

## Limits of this record

* No coverage was re-measured and no instrumented target rebuilt; every number re-reads
  reports already on disk (`/tmp/*-lcov*.info`, `desktop/coverage/*.json`).
* `module-agent-rpc.md` was edited by its own unit during this turn, so its numbers
  are a revision snapshot, not a stable result. `pairings.json` therefore records the
  section *heading* each needle must land on, and the gate fails when it lands
  elsewhere — the same protection against a live doc that a hash would give, without
  breaking on unrelated prose edits.
* `cli`'s authoritative pairing changed report (S5 → S7) and section (whole doc → §4c)
  on the doc's own instructions ("Read from the report the reviewer should use:"), and
  the previous numbers are kept as `cli-4` / `cli-4b` / `cli-wide` rather than
  overwritten. If a reviewer disagrees with that choice, the record shows both.
* The gate's `expect` block is a *recorded* measurement, so it goes stale when a peer
  edits a doc. That is deliberate: `--strict` turns drift into a failure, and the
  default (`--gate`) reports it without failing, per the campaign's contract that only
  an unexplained covered row is a hard failure.
* `reconcile.py` is a **shared, live file** (three authors today). Run `selftest.py`
  after any edit to it; the fixture is the only thing standing between this tool and
  another silent zero.
* Between 20:19 and 20:25 an earlier version of this file said "12 docs still blind …
  all of them a later-cell shape" and later "COVERED is 0 everywhere". Both were
  superseded by measurement — the second one by the fixture. Where this file and the
  tools disagree, the tools are right.
