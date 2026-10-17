# cov100 round 2 — campaign brief (every worker must read this file first)

Worktree: `/Users/geilige/future-os/.worktrees/cov100-r2` (branch `test/cov100-r2`).
This is your cwd. Baseline revision: `8449acf6` (= `origin/main` at campaign start).

## 0. Objective

Raise test coverage of this repository to **100 % of coverable lines** again, after
739 commits of drift since the previous campaign (2026-08-27). Every line in your
scope is either executed by a test you add, or registered as unreachable with a
concrete mechanism in the module record. Nothing else is a result.

## 1. The metric (round 2 口径 A)

**Per-line LCOV `DA:<line>,0`** — never the llvm-cov `summary` block, never
"statements", never regions. A line counts as uncovered iff its `DA` counter is 0.

```bash
# per-file counts, worst first, plus the residual total
python3 .future/cov100/covlines.py files <lcov>
# the exact uncovered line numbers per file (the work list)
python3 .future/cov100/covlines.py detail <lcov> --under <your scope>
python3 .future/cov100/covlines.py total <lcov> --under <your scope>
```

For the JS tracks the equivalent tool is `.future/cov100/covlines-js.py` over
`desktop/coverage/coverage-final.json` (line executed iff any statement on it
executed).

Baseline numbers at 8449acf6 (measured by the supervisor, lcov kept in
`.future/cov100/baseline/`):

| unit | scope | uncovered lines | measured | tool |
|---|---|---|---|---|
| desktop frontend | `desktop/src` | 292 | 12 800 | vitest |
| mobile | `mobile/src` | 131 | 9 859 | jest |
| channels | `channels/src` | 85 | 25 531 | llvm-cov |
| packages + tasks + goal | `packages/*/src`, `orchestration/{tasks,goal}/src` | 219 | 31 295 | llvm-cov |
| cli | `cli/src` | 731 | 37 188 | llvm-cov |
| tui | `tui/src` | 270 | 59 849 | llvm-cov |
| agent | `agent/src` | 913 | 63 723 | llvm-cov |

## 2. Environment rules (every shell you run)

### 2.0-bis If a build/exec STALLS, **WAIT — do not retry** (host fault, measured 2026-10-17)

Symptom: `cargo` or a test binary sits with **cargo's own CPU near zero** (waiting, not computing),
`ps` shows no `rustc`, and `/usr/libexec/syspolicyd` is above ~300 % CPU. Measured cause: macOS
**per-INODE code-signature verification** under a saturated policy daemon — and **killing a stalled
attempt RESTARTS that verification from the beginning** (measured: five short attempts made no progress;
one uninterrupted attempt succeeded in 315 s).

So: **once an attempt has started, let it finish.** A timeout-with-retry loop is strictly worse than
patience here, because the work a retry discards is the work it needs. If a `measure-final.sh` track
exhausts its budget on this fault, re-run it **from scratch in a clear window** rather than retrying in
place, and say in the record that the track was lost to the host rather than failing a test.
Check `bash .future/cov100/endgame.sh --check`'s `build:` field before starting anything expensive;
when the field is HIGH, do desk work and take measurements when it clears (it is intermittent).

### 2.0 `--test-threads=1` must come IMMEDIATELY after the single `--`

**Read this before you type any test command** (measured 2026-10-17, FLAKES §8c).
`cargo` splits its own args from the test binary's at the **first** `--`, and **libtest stops
parsing options at the first `--` it then sees**. So this pair behaves completely
differently:

```bash
cargo test --lib -j 8 -- cli_tool::tests -- --test-threads=1   # WRONG: the flag is
                                                              # swallowed as a test-name
                                                              # filter -> 32-way parallel
cargo test --lib -j 8 -- --test-threads=1 cli_tool::tests      # RIGHT: the flag is parsed
```

A filter never goes before the flag. Consequence of getting it wrong: your "serial" run is
actually the default concurrency (the core count), which is exactly the condition the
`--test-threads=1` rule exists to avoid - and it makes wall-clock budgets fail and process-global
`PATH` swaps visible to concurrently running tests.

**How to check any option that "must be in force"**: give it a value that can only fail when
parsed. `--test-threads=ABC` errors ("must be a number > 0") when the flag is parsed and runs
happily when it is not - a two-second proof, and better than timing two slow tests.

```bash
ulimit -n 10240                                   # or dozens of unrelated agent tests fail with EMFILE
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home                 # the REAL ~/.future leaks into tests otherwise
export CARGO_TARGET_DIR=target/cov-<your-unit>    # YOUR private dir — never the default, never another unit's
mkdir -p "$HOME"
```

Eight workers share this host, so keep builds modest: `-j 4` (not `-j 16`) on
every `cargo llvm-cov` / `cargo test`.

Why: Homebrew's `rust`/`cargo` shadows rustup and produces "never executed"
unit-hash mismatches; a shared target dir interleaves instrumentation and can
report stale line attributions as "uncovered".

## 2a. What the declared write scope IS (corrected 2026-10-16)

**It is conflict detection and attribution, NOT a sandbox.** The guard's own doc says it: registered
agents declare the path set they write into, and *claiming a todo while a peer holds a live lease in an
**overlapping** workspace* is refused (or degraded to serial). There is no write enforcement anywhere in
`workspace_guard.rs` - its API is `paths_overlap` / `live_workspace_conflicts` /
`todo_workspace_conflicts`.

Consequences to hold onto:
* a worker **can** write any file in the worktree and nothing stops it;
* if two units declare disjoint scopes and one writes outside its scope into the other's files, the guard
  does **not** see that conflict either, because the conflict is computed from the DECLARED sets;
* therefore "one writer per file" rests on **voluntary compliance plus the supervisor's review** (which
  compares each unit's actual diff against its declared scope at acceptance). Do not describe the scope as
  a sandbox, and do not treat "no conflict reported" as evidence that no collision happened;
* an empty scope makes a unit invisible to the conflict computation entirely, which is why
  `audit-declarations.py` matters.

Evidence: `INCIDENTS.md` F.43 (a unit wrote a file that was in neither its todo scope nor its agent
workspace, and it was accepted; the case was benign, the model was wrong).

## 2b. Disk and formatting rules (read before any instrumented build)

An instrumented target dir is **3–12 GB** and this checkout's `target/` reached
83 GB with 13 units running (the volume hit 98 % full, which makes instrumented
runs fail in the middle and emit garbage). Rules:

1. **One instrumented dir per unit, alive as briefly as possible.** Measure,
   copy the lcov to `/tmp/<unit>-lcov.info`, then `rm -rf target/cov-<unit>`
   immediately. The lcov is the artifact; the target dir is not.
2. **Re-measure by rebuilding, never by keeping the dir warm.** If you need a
   second number, use the *uninstrumented* suite (`cargo test -p <crate>`) for
   the inner loop; rebuild for the final number.
3. **Flake work needs no instrumentation at all** — use plain `cargo test` in the
   shared `target/debug` (see `.future/cov100/FLAKES.md`).
4. **Check `df -h .` before a heavy build.** Under 40 GB free, delete your own
   target dir first — do not wait, and do not delete a peer's dir: only the unit
   that owns a directory deletes it. The supervisor made that mistake once (it
   reclaimed dirs no live process happened to match for an instant while their
   owners were still active, and a unit paid a ~13 min rebuild); if free space is
   under 20 GB, say so in a report instead of reclaiming anything that is not
   yours.
5. **Never run `cargo fmt` across a crate.** It reformatted a peer's in-flight
   files and the peer was mid-run. Format per file, and only when that file is
   actually dirty (`cargo fmt --check -- <file>`).


## 3. Git rules — read twice

* **Never run a git command that changes state**: no `add`, `commit`, `checkout`,
  `restore`, `stash`, `reset`, `switch`, `worktree`, no `git clean`. The
  supervisor commits. Peer workers share this checkout, so a `checkout`/`stash`
  destroys their in-flight work.
* Read-only git (`status`, `diff`, `log`, `show`) is fine.
* Write **only** inside your declared scope. Other units own other paths.
* Do not `rm` or rewrite files outside your scope to "make a measurement clean".

## 4. What counts as a result

For **each** uncovered line in your scope, one of:

1. **Covered** — a test you add makes it execute. The test must assert the
   behaviour that line implements, not just touch it (a test that calls the line
   without asserting its effect is not a result; cf. `docs/testing/weak-test-audit.md`).
2. **Waived** — the line is unreachable, registered in your module record under one
   of the repo's four categories (see `docs/testing/waiver-ledger.md`) *with the
   mechanism*, e.g. `unreachable-by-construction` needs the invariant that makes
   every input avoid it; `attribution-artifact` needs the neighbouring covered
   lines as evidence; `unreachable-in-this-environment` needs the host/fixture
   that cannot be provided here; `platform-unmeasured` needs the OS that does
   measure it.

Forbidden (these are the failure modes the reviewers hunt):

* deleting or weakening a production guard so its line stops existing;
* weakening/removing an assertion to get a green suite;
* `#[ignore]`, `#[cfg(test)]`-ing a test away, or `--skip`ping a test in the
  measurement command;
* counting a line as covered because *another* unit's test covers it (report it,
  don't claim it);
* widening a parameter to `#[allow(dead_code)]` / `let _ =` to silence a branch.

Real bugs you find on the way **count** — record them in your module record with
the evidence (a failing test that proves it, then the fix). Do not silently fix
production behaviour without saying so in your report.

## 4b. Production seams: propose, never invent; the supervisor decides

**AND: a cross-scope write that is REQUIRED TO UNBLOCK MEASUREMENT is a separate case, and it is allowed
when all four hold** (added 2026-10-17 after the tauri-bridge unit made one and flagged it, `module-tauri-bridge.md:72`).
The case was: `desktop/src-tauri/src/terminal/session.rs`'s two test helpers were called from two TEST modules
while `terminal/mod.rs` gates the helper module with `#[cfg(test)]`, so the non-test lib build did not compile —
blocking tau-core, tau-remote and tau-bridge alike, in a file static for 90+ minutes with no other agent editing it.
The unit applied two `#[cfg(test)]` attributes outside its write set and flagged it with the revert consequence.
The four conditions:

1. **the tree does not compile or no measurement can run without it** — i.e. the cost of NOT writing is that
   nobody in the crate can measure at all;
2. **the change is behaviour-preserving and test-side**: no production line's behaviour changes (here both
   callers were `#[cfg(test)] mod tests`, so gating the helpers cannot affect the shipped binary);
3. **no other writer holds the file** (check the live leases — another unit's in-flight work is not yours to
   edit, however stuck it looks);
4. **it is FLAGGED in the record with the revert consequence spelled out** — "reverting this re-breaks the
   crate" is the sentence a later reader needs.

If those four hold, write it and report it; if any fails, stop and route it. **I ratified this one** (the tree
compiles again and the attributes are on test-only helpers). The distinction to keep: this is not a *seam*
(condition 2 is what separates them) — a seam changes production behaviour to buy coverage and still needs my
explicit authorisation.

A line may need a **seam** to become coverable. The rule this campaign settled on:

1. **Propose, do not take.** A seam changes production code, so it is the supervisor's
   call. Record it in the unit's module doc with the lines it would make coverable and the
   smallest change that does it.
2. It is **approved only if** (a) it makes a line coverable rather than merely moving it,
   (b) it is **behaviour-identical** with the equivalence argument stated, (c) it follows a
   pattern **already present in the same crate** (e.g. an existing injectable side effect or
   an extraction the neighbouring function already does), and (d) it is the smallest change
   that achieves (a).
3. **Never** delete or widen a guard to make a line disappear, and never invent a seam for a
   genuine W5/W7 residue (a wrapper that re-execs, an `AppHandle<Wry>`-only command): those
   stay waived with the mechanism named.
4. A production change alters the compiled crate, so **peers' earlier numbers are from an
   earlier revision** — the final measurement is re-taken on the frozen revision; do not
   re-measure another unit's scope.

## 5. Deliverables per unit

### 5.1 Acceptance gates the unit must run itself (the supervisor re-runs them)

A green test suite is not a green PR. Run, in your own directory, and report each exit
code — the supervisor's acceptance is now gated on these:

* **JS units** (`desktop`, `mobile`): `npx tsc --noEmit` and `npx eslint ...` **in addition
  to** the test run. Jest/vitest strip types, so a suite can be perfectly green while `tsc`
  is red, and CI runs typecheck and lint as separate steps. Measured in this campaign: one
  unit shipped 6 `tsc` errors in four files that CI's Mobile job would have caught, and the
  failure was found only because a *flake* unit happened to run the gate.
* **Rust units**: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` for your
  crate (CI fails on both), plus your test target.
* If a gate cannot pass for a reason outside your scope, say so explicitly with the command
  and the output rather than omitting it.

### 5.2 Mechanical self-check before you hand off (cheap, catches silence)

```bash
python3 .future/cov100/reconcile.py --report <your lcov|coverage-final.json> \
    --scope <scope prefix used above> --doc docs/testing/module-<x>.md \
    --section "<the heading of YOUR round-2 section>" --label <unit>
```

It prints `mentioned` / `MISSING` / `STALE` for every uncovered line in your
scope:

* `MISSING` = an uncovered line your round-2 section never mentions at all. This
  must be `0` before you hand off; it cannot check the *mechanism*, only that you
  did not silently skip a line.
* `STALE` = a line your section cites as uncovered that the report now shows
  covered (line drift, or a doc left behind). Should also be `0` for the section
  you measured.
  Both checks are per-line and mechanical, so they are the cheapest way to catch
  the failure this campaign has already seen once (a handoff whose ledger was
  PARTIAL while its prose sounded complete).

**Never write evidence as a table whose first cell is a `path:line`** (added 2026-10-17 after this
bit the unit AND the supervisor twice in three segments). `reconcile.py` reads `| \`path:line\` |` at
the start of a table row as a CLAIM THAT THE LINE IS UNCOVERED — that is how the ledger format is
defined, so a table is not free prose. Two measured instances:

* a row announcing a line as **CLOSED** (`| \`shadow_review/snapshot.rs:192\` | — | CLOSED |`) was read
  as claiming 192 uncovered while the report shows it covered → a false `STALE`;
* a row citing a **count** for an already-covered line (`commands/skills.rs:98` at count 1) produced
  the same false `STALE` (STALE is any cited line the report shows covered, and `DA:n` with n>0 is
  covered).

So: put residual-ledger rows in the `path:line`-first-cell table (that is the format), and write
everything else — closed rows, corroboration, comparisons between two runs — as **prose or a table
whose first cell is not a line reference**. If your handoff reports `STALE` and you cannot point at a
line that is genuinely cited as uncovered, read your own tables before believing the tool is wrong.

**A HEADING inside (or above) your ledger's level TRUNCATES the slice** (added 2026-10-17, found by the
app-data unit and verified against `slice_section`'s own rule). `slice_section` takes the section up to
"the next heading of the same or higher level", so a generator that emits `### OPEN` / `### WAIVED`
lines under a level-4 ledger heading ends the slice at the first of them: the run reported
**"mentioned: 2, MISSING: 384"** for a table that lists all 386 lines. `reconcile.py` exits 3 rather than
3 for a doc whose rows it never parsed — that guard is what makes the failure loud instead of silent.
**The symptom to recognise is a suspiciously SMALL `mentioned` count**; demote those lines to bold text
(or raise the ledger's own heading level) and re-run.

**And a ledger row writes the colon INSIDE the backticks** — `` | `src/a.rs:12-15` | ... | `` — so a
parser expecting `` `path` : lines `` parses **0 of 43 rows and reports every line MISSING** (the
app-data chain measured this; a false alarm exactly the size of a real finding). If a doc you know has
a ledger reports every line missing, suspect your row pattern before the doc.

1. The tests (inside your scope's test modules / `tests/` dir).
2. `docs/testing/module-<x>.md` updated in place: run identity (revision, host,
   exact command), before/after uncovered-line counts, and one row per residual
   uncovered line (path:line — category — mechanism). Keep existing section
   numbers stable (production source comments cite them).
3. A report to the supervisor containing:
   * the lcov path + the exact `covlines.py` command and its `total` output
     before and after;
   * the list of test names added (grouped by file);
   * every residual uncovered line with its category;
   * anything you found that is broken (with the reproduction);
   * the commands a reviewer can run to reproduce all of the above.

## 5.3 Is a red run's number quotable? (refined 2026-10-16 — two deliveries were judged inconsistently)

Two denominations, two rules. Stated once here because it decides several units' published numbers:

* **Frozen/final figure** — published in `FINAL-REPORT.md`: requires an **all-green** run, **no
  exceptions**. `measure-final.sh` enforces this (`PARTIAL_TRACKS`).
* **Segment figure** — recorded in a module record while other units are still writing: a red run's
  figure **may** be recorded iff ALL FOUR hold, each stated in the record:
  1. every failing test is **named**, with its cause;
  2. **the segment's own new tests RAN and PASSED** in that run (this is the condition that protects the
     number: your fixture must be shown to have executed, which is what the `ok` line in the run log is for);
  3. the **direction argument** is stated — a red run executes *less* code, never more, so the residual
     is an **upper bound** on the true residual and **no line can be falsely reported covered**;
  4. the frozen sweep re-measures it (the frozen number is what the final report publishes).

  **Corrected 2026-10-17 (INCIDENTS F.69).** Condition 2 used to read "every failing test lies *outside*
  the measured scope". That wording governed the wrong thing and I had nonetheless accepted three
  deliveries against it (`tauri-appdata` segments 8, 9 and 10 each had one failure INSIDE a scope:
  `tasks::tests::the_gui_loop_runs_a_queued_request_it_is_woken_for`, and segment 10 also
  `commands/update.rs:497`) — a rule whose wording contradicts three consecutive accepted deliveries is
  not a rule. The substantively right test is the one now written: a failure anywhere can only LOSE
  coverage, so the direction argument is what makes the figure safe; what a failure inside a scope can
  invalidate is *your own fixture having run*, which is a different question and is answered directly by
  the run log. A failure inside the scope must still be **named** and, if it is not a documented flake,
  treated as the segment's work; and the record should say that the residual may include lines its own
  fixture would have closed, which the frozen pass then re-measures.

If **any** failure is inside the measured scope and the segment's own new tests did **not** all pass, the
figure is **not quotable at either denomination**.

**A revision diff does NOT prove the tree is unchanged** (measured 2026-10-17, fourth instance). Every
module record tells the next segment to reuse the number when `git diff <rev>..HEAD` over the scopes is
empty. That check sees **committed** work only, so an **uncommitted peer edit is invisible to it** — and
the app-data slice hit exactly that: the diff was empty while `desktop/src-tauri/src/tasks.rs` was dirty
with another session's in-flight test, found only by the tree check (`git status --short` plus an mtime
that is stable across two samples). A segment that reused the number on the diff alone would have
published a figure that did not describe the tree it was standing in. **STEP 0 is both probes:** the
revision diff AND the tree check; reuse the number only when BOTH are clean, and say which one found the
exception when they disagree.

Worked examples, kept for consistency:
* `tauri-appdata` §Segment 4 (`526`): instrumented gui run green; the headless target lost four
  `remote_peer` tests to the documented `8022` bind contention → quotable under (b).
* `tauri-shell` §Segment 3 (`255`): gui `2028 passed/1 failed`, headless `1617/7` — all eight failures are
  `remote`/`remote_peer`/`agent_bridge` tests failing at `bind port 8022 (EADDRINUSE)`, every one of them
  **outside** the twelve measured shell files, and every shell-scope test passed → quotable under (b),
  with the reds and their cause recorded.
* `tauri-appdata` §Segment 10 (`400`): gui `2068 passed/6 failed`, headless `1651/1`; six failures are the
  documented `8022` and CLI-probe families, and ONE is inside the ten scopes
  (`commands/update.rs:497`) — quotable because the segment's OWN new test
  (`store::threads::tests::a_create_that_loses_the_race_returns_the_winners_row`) reads `ok` in both the
  instrumented run and the gate run, the direction argument holds, and the frozen pass re-measures.
  This is the case that exposed the old wording.

## 6. Measurement commands per unit

```bash
# Rust unit (example: channels) — build + run + report in ONE process
cargo llvm-cov -p future-channel --no-fail-fast --lcov --output-path /tmp/<unit>-lcov.info -j 8
# If a test target fails, cargo-llvm-cov emits no report; the profraw is still on
# disk, so render it separately:
cargo llvm-cov report --lcov --output-path /tmp/<unit>-lcov.info

# fast inner loop (uninstrumented, shared worktree target dir)
cargo test -p future-channel

# JS units
cd desktop && npx vitest run --coverage      # -> desktop/coverage/coverage-final.json
cd mobile  && npm run gen-version && timeout 300 npx jest --runInBand --coverage; echo "exit=$?"
```

`mobile`: a leaked timer makes jest hang after a green run — the exit code is the
judgement (`124` = hang), see `.future/memory/testing-gotchas.md`.

### 6a-bis. Clearing stale profiles: use `cargo llvm-cov clean`, NEVER `rm -rf` the target dir

Measured 2026-10-17 (agent unit, segment 21; its finding F-47): the earlier wording of this
rule said to `rm -rf target/cov-<unit>/llvm-cov-target` before measuring, so that a failed run
cannot leave stale `.profraw` files for `cargo llvm-cov report` to merge into a previous
segment's number. **That directory also holds the BUILD**, so the literal form forces a
from-scratch rebuild of every dependency — **8m53s under this machine's peer load** — and two
attempts died on the 560 s cap with no target completed and no lcov at all. The invariant is
about the PROFILES, not the tree, and the profiles live directly in that directory:

```bash
cargo llvm-cov clean                       # preferred: both forms below are equivalent
cargo llvm-cov clean --workspace           # what measure-final.sh uses for the whole tree
# or, if you must target the directory by hand:
find target/cov-<unit>/llvm-cov-target -maxdepth 1 -name '*.profraw' -delete
```

`rm -rf target/cov-<unit>` remains correct for **disposal after** you have copied the lcov to
`/tmp/` (PLAN §5 rule 1: the lcov is the artifact, the target dir is not) — the distinction is
whether you are about to measure again (clear profiles, keep the build) or are done (drop the
directory). The cost of getting this wrong is not a wrong number; it is **no number**, plus
~20 minutes per attempt.

### 6a-ter. Two-flavour crates: render per flavour, then union

`desktop/src-tauri` needs two feature flavours. Do NOT run both legs and then one `cargo llvm-cov
report`: with a warm target dir the first flavour's profraw can fail to map to the binary the second
flavour rebuilt, and its data is silently dropped from the merge (measured: a gui-only report reading
626 with `scheduler` at 175 uncovered instead of 23, FLAKES item 30). Render each flavour immediately
after its own leg and union:

```bash
cargo llvm-cov clean
cargo llvm-cov --lcov --no-fail-fast -j 6 --output-path /tmp/tau-gui.info -- --test-threads=1
cargo llvm-cov clean
cargo llvm-cov --lcov --no-fail-fast -j 6 --no-default-features --features headless \
    --output-path /tmp/tau-headless.info -- --test-threads=1
python3 .future/cov100/lcov-union.py --out /tmp/tau-union.info /tmp/tau-gui.info /tmp/tau-headless.info
```

A line is uncovered in the union iff NEITHER flavour covered it, which is the `DA:0` metric.

**Two failure modes, and the second one defeats the obvious check** (both measured):

1. *Profiless merge* — a flavour's data fails to map and is dropped, so a `cfg(feature = "gui")`-only file
   reads uncovered or a covered count collapses. The prescription above (render per flavour) is the fix.
2. *A killed test binary writes no profraw* (measured 2026-10-17, TAU-B segment 11): an external SIGTERM
   killed the gui lib test binary (`process didn't exit successfully: … futureos_lib-… (signal: 15,
   SIGTERM)`), so the gui flavour contributed **nothing** — and the resulting report looked plausible:
   the **`DA:` line count was IDENTICAL** to the previous segment's (34 751) because the same lines are
   still measured, they simply **read 0**. So the check this section used to prescribe — "a flavour-only
   file HAS `DA` records" — **cannot detect it: it was measuring presence, not hits.**

The sharpened check, which does catch both: **a flavour-only file must have at least one NONZERO hit in
that flavour's own report.** All-zero records for the files that only that flavour compiles means the
flavour's binary died, whatever the exit code said. Run it with:

```bash
python3 .future/cov100/flavor-alive.py /tmp/tau-gui.info /tmp/tau-headless.info
```

which prints each flavour's covered/uncovered totals and FAILS (exit 1) if a flavour's report has no
nonzero hit at all.

**Its limit, stated so the check is not over-trusted** (found by the shell unit the same day): it catches a
flavour that is **ENTIRELY** dead. A flavour whose **one target** was signal-killed still has live targets
(main.rs, the bins, integration tests), so it passes this check while having lost that target's data —
measured: the shell unit's headless leg had its lib target terminated by `signal: 15, SIGTERM` (a killed
target prints no `test result:` summary at all, unlike a plain failure which prints `test result: FAILED.`
plus counts), and the flavour was only PARTIALLY dead. The per-leg defence is the truncation guard
(summaries < `Running` lines) plus the signalled-target rule below; for a two-flavour crate, check both.

**The stronger technique, used by the app-data unit at segment 15 and worth copying** (it answers the limit
above): pick 3–4 lines that only ONE flavour compiles and check their **HIT COUNTS**, not their presence —
e.g. `commands/update.rs:19 = 4` with the headless side ABSENT, `no_gui.rs:3 = 16` with the gui side ABSENT,
and one line reading `0` on the gui side and ABSENT headless as a control. A partial loss moves a hit count;
a fully dead flavour moves it to zero. And **`grep -c 'signal: 15' <leg log>` should be 0** — a signalled
target contributes no data however healthy the rest of the leg looks. Also: **a test binary that dies by signal reports nothing**, so when a leg's `cargo
llvm-cov` output contains `signal: 15`/`signal: 9`, treat that leg as producing NO data for that target
regardless of how the report reads, and say so in the record.

### 6a. If you attach a `--verify` validator (optional, and easy to get wrong)

A validator is a **floor, not the verification** (the supervisor re-runs the real
check at acceptance). Three properties it must have, each of which a unit got
wrong in this campaign:

1. **Paths must match the leg's cwd.** The validator runs via `sh -c` from the
   *goal cwd* (the worktree root), but a leading `cd desktop &&` changes the cwd
   for the rest of the chain — so `desktop/coverage/coverage-final.json` then
   resolves to `desktop/desktop/...` and dies with `FileNotFoundError`.
2. **It must finish inside 120 s** (`orchestration/goal/src/validator.rs`
   `DEFAULT_TIMEOUT`), unless `FUTURE_GOAL_VALIDATOR_TIMEOUT_SECS` is set in the
   worker's environment. A validator that runs a multi-minute suite is killed
   mid-run every time — and killing `vitest --coverage` mid-write is one way to
   leave a complete-looking, near-all-zero `coverage-final.json` (the zeroed
   report).
3. **It must be meaningful**, i.e. fail on a bad report. The campaign tools exit
   `3` on a zeroed/partial report and on a scope mismatch, so they are safe to
   use as the metric leg. Reading the tool's *stdout* while ignoring its exit code
   is how a zeroed report turns into "everything uncovered".

A correct, fast example (desktop frontend), run from the goal cwd:

```bash
--verify "cd desktop && npx tsc --noEmit && cd .. && python3 .future/cov100/covlines-js.py desktop/coverage/coverage-final.json --under .worktrees/<name>/desktop/src"
```

A failing validator does **not** block completion (it records an error and the
todo still completes) — which is why a permanently-red validator is noise, not a
gate. Fix the string rather than living with it.


Known-host failures you are allowed to hit (do not "fix" them, do not treat them
as your regression): `desktop/src-tauri` lib suite deadlocks in parallel
(`--test-threads=1` needed) — not your scope anyway. `agent/tests/cli_smoke.rs`
profile-timer races and `cli` `safari_manager` first-launch window flakes are
pre-existing (`.future/memory/testing-gotchas.md`).


## 7. Reporting progress

```bash
future goal report --goal <G> --agent-id <your-id> --message "…"     # durable mid-run note
```

If you are blocked (>10 minutes without progress, or a decision belongs to a
human), stop and report **blocked** with the exact obstacle instead of guessing.
An honest partial result with the residual lines classified beats a false 100 %.

### 7.1 Supervisor dispatch policy — the two criteria that HOLD a unit (added 2026-10-17)

The supervisor does not fill idle seats mechanically. Two conditions block a dispatch, and both are
checkable rather than remembered:

1. **A RATE-MEASURING unit is running** (`future goal lease status` shows a claim on a ticket whose work is
   a before/after flake rate). Adding a second cargo-heavy unit changes the load the first one is measuring,
   and a rate is only a rate for the load it was taken at. **Wait for that lease to clear.** Document-only
   work (records, ledgers, gate repairs) is always allowed — it costs no CPU.
2. **The host is saturated or wedged.** `bash .future/cov100/endgame.sh --check` reports `load:` and
   `build:`. If either is HIGH/BLOCKED, do desk work: a measurement taken there is worth nothing, and a
   stall should be waited out rather than retried (PLAN §2.0-bis).

Everything else is "dispatch now": a free lease on a unit whose module is frozen. If neither criterion holds
and a seat is idle, leaving it idle is the mistake.

**RE-EVALUATE AFTER EACH DISPATCH, NOT ONCE PER BATCH** (INCIDENTS F.92, the same day this section was
written): a dispatch can itself create the condition that blocks the next one. On 2026-10-17 I checked both
criteria, saw `live claims: 0`, dispatched F-AGENT-2 - correct, it was alone - and then dispatched a review
unit in the same breath, which compiles its own checkout and probes. Eight rustc processes and a rising load
later, the rate-measuring unit I had just started was measuring beside a builder. **A check that gates a
SEQUENCE must be run per element, or it is testing the empty case.** So: dispatch one, re-check, dispatch the
next - and when a pair has already gone out together, steer both with the fact rather than pretending the
host is quiet.

## 8. Closeout (the supervisor runs this, not the units)

```bash
.future/cov100/measure-final.sh            # all four tracks -> coverage/final/FINAL-REPORT.md
```

It refuses to print a figure for a report the tools reject (a track shows
`INVALID`), so the acceptance run cannot silently publish a zeroed report. Expect
1.5–3 h. Per-unit residual obligations (tripwire green, waiver verdict, flake
rate) are listed at the end of the generated report — a number alone is not a
claim.


### §7.1c Dispatch must use the ticket's OWNER id

Measured 2026-10-17: I dispatched REAL-NET (`todo_f5453bf671b4`) as a NEW agent id
`worker-realnet`. The kernel auto-registered the agent, then gave it nothing:
`decision=wait ... open advancement(s) unavailable to this agent (blocked, leased to other
agents, or owner-scoped)`, the session was discarded ("no turn ran"), and the ticket's owner
was the whole time `worker-tauri-remote`. The kernel is owner-scoped: an agent that is not the
ticket's `owner` cannot claim it, no matter what the dispatch note says.

**Before every `dispatch.sh <agent>`, read the ticket's owner from the projection**
(`future goal frontier show`, which prints `todo <id> owner=<agent>`), and dispatch THAT id.
If the ticket has no owner, or the owner is wrong, fix the owner first (`todo update --owner`)
— do not invent an agent id and hope. A wasted dispatch costs ~70 s of timeout plus a spurious
registered agent that then shows up in every later owner list.
