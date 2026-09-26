# Desktop shell (`d-shell`) coverage and waivers

Scope: `desktop/src/components/`, `desktop/src/app/`, `desktop/src/main.tsx`
(the `group:d-shell` slice of `desktop/src`). This is the waiver/evidence doc the
`js-module` gate resolves for that group.

> **Recovery note.** This document was accidentally truncated to zero bytes while
> appending a section (a `str.index` returned 0 instead of the offset, and the
> prefix written back was empty). It has been rebuilt from the measurement data
> (`coverage-final.json` / `coverage-summary.json`, both still on disk and
> unchanged), from the gate output, and from an early draft preserved in the
> snapshot at `.future/cov100/tau-m/docs/testing/desktop-shell.md`. Every number,
> line number and arm classification below was re-derived from the report rather
> than from memory; the *prose* is therefore not byte-identical to the lost
> revision, and the figures are.

## 0. The `<agent-id>` verification string cannot exit 0 — measured, not inferred

Two non-destructive experiments (both re-runnable) pin this down exactly:

```
# A. what argv does cmd actually hand to python?  (an EXISTING file stands in for the
#    redirect target, so nothing is created anywhere)
cmd /c 'python -c "import sys;print(sys.argv)" desktop/coverage/<README.md'
  -> ['-c', 'desktop/coverage/']          # the token before `<` becomes the 4th argument

# B. and can verify.py cope with that argument?
python .future/cov100/verify.py js-module group:d-shell 99.99 docs/testing/desktop-shell.md desktop/coverage/
  -> PermissionError: [Errno 13] Permission denied: '...\desktop\coverage'   (exit 1)
```

Mechanism: cmd.exe consumes `<agent-id>/coverage-summary.json` as an **input
redirect**, leaving argv4 = `desktop/coverage/` — a directory. `cmd_js_module`
does `p = ROOT / summary` then `p.read_text()`, so a directory raises
`PermissionError` and the gate exits 1.

Two consequences, both proven rather than assumed:

* Creating a file named `agent-id` at the repo root **cannot** make the gate pass
  — it only gets python as far as experiment B, which still fails. (That
  workaround was already refused on safety grounds; this shows it is also
  useless, and that it would additionally redirect stdout to `/\coverage-summary.json`,
  outside the worktree.)
* No filename can contain `<` or `>`, so the placeholder cannot be satisfied
  literally on Windows.
* Note the asymmetry: `cmd_rust_module` resolves an `own` report from
  `RUST_GROUP_REPORTS` and would ignore a bad 4th argument; **`cmd_js_module` has
  no such override.**

### The exact cmd mechanism, isolated

Measured with a probe file and a probe tail, **input redirection only** (no `>`):

```
cmd /c 'python -c "import sys;print(sys.argv)" pre/probe.txt/tail'      # control: no redirect
  -> ['-c', 'pre/probe.txt/tail']
cmd /c 'python -c "import sys;print(sys.argv)" pre/<probe.txt/tail'     # input redirect
  -> The system cannot find the path specified.    (exit 1)
```

So cmd takes **everything after `<`** as the redirect target path — here
`probe.txt/tail` relative to cwd, which does not exist, so python is never launched.
Applied to the real string `desktop/coverage/<agent-id>/coverage-summary.json`:

* the redirect target is the whole path **`agent-id/coverage-summary.json`**;
* argument 4 becomes **`desktop/coverage/`** — a directory.

The second half is why the obvious workaround fails: **creating a file named
`agent-id` or a directory containing `coverage-summary.json` still cannot pass**,
because the surviving argument is a directory and `cmd_js_module` does
`p.read_text()` on it (`PermissionError`, exit 1 — measured earlier as experiment B).

### A safety slip made while measuring this — recorded, not hidden

Writing the probe I typed `<probe.txt>` **with a closing `>`**, which cmd reads as a
second redirect — **stdout** — to `/tail`, i.e. a path at the drive root, outside
the worktree. Two 8- and 16-byte files appeared at `D:\`: `tail` and
`coverage-summary.json`, containing my own `sys.argv` dumps (`['-c']`,
`['-c', 'pre/']`). I verified their content matched my diagnostics, then removed
both with a literal-path guard, and confirmed they are gone (`Test-Path` false).

Nothing of the user's was touched, but this is exactly the hazard the repo's safety
rules exist for, and it *empirically demonstrates* the redirect-out risk that this
section had only argued: **`<path>` with a stray `>` writes outside the worktree.**
Anyone probing cmd's redirect behaviour should use `<file` with no closing bracket,
and check the drive root afterwards.

### Landmine: the gate's DEFAULT report path is stale, and shows the old baseline

`cmd_js_module`'s fourth argument defaults to **`desktop/coverage/coverage-summary.json`**.
That file exists, and it is **not** this group's report:

| report | written | group lines |
|---|---|---|
| `desktop/coverage/coverage-summary.json` (the default) | **01:54:37** — five minutes *before* this session started (01:59:04) | **52.3557 % (989/1889)** |
| `desktop/coverage/d-shell/coverage-summary.json` (this task's) | 15:25:31 | **99.4177 % (1878/1889)** |

So invoking the gate with only three arguments —

```
python .future/cov100/verify.py js-module group:d-shell 99.99 docs/testing/desktop-shell.md
```

— reads the stale shared file and prints:

```
FAIL: 27 undocumented uncovered file(s) in group:d-shell
group:d-shell: 52.3557% lines (989/1889) ... 900 uncovered line(s) in 33 file(s)
```

Those are **exactly the numbers quoted in this todo's own task description**
(*"当前 52.36% = 989/1889 行，33 个文件未达 100%"*), because that description was
written from this same pre-session report. A reviewer who omits the fourth argument
therefore sees the *original* baseline and a FAIL, with no indication that any work
was done — the number is a snapshot of the checkout before this task existed, not a
measurement of the current tree.

**The report path must be passed explicitly.** This task's report is
`desktop/coverage/d-shell/coverage-summary.json` (also copied to
`desktop/coverage/todo_8cb0c2e99ae7/`), and the gate exits 0 against either. The
stale shared file was **not** overwritten: it sits outside this task's write set and
is the baseline artefact, so it is documented rather than "fixed".

### Which shell failed — the error text says which

The same literal string fails in **three different ways** depending on how it is
invoked, and each signature identifies the shell. Verified by running each form:

| invocation | what python receives | output | exit |
|---|---|---|---|
| **cmd.exe** (`cmd /c …`) | *nothing* — `<` is a stdin redirect and cmd aborts before launching python | `系统找不到指定的文件。` (empty stdout) | 1 |
| **PowerShell** | the literal `desktop/coverage/<agent-id>/coverage-summary.json` — PowerShell passes `<` through as an argument | `FAIL: missing desktop/coverage/<agent-id>/coverage-summary.json — run the module's coverage job first` | 1 |
| **direct exec** (no shell) | the same literal | the same `FAIL: missing …` | 1 |

Two things follow, both useful to whoever owns the harness:

1. **This task's 45 rejections all carry the cmd signature** (mojibake
   *"the system cannot find the file"* with empty stdout), so the gate is being
   invoked through cmd — not PowerShell, whose error text would name the missing
   path. That is why the failures looked like a coverage/validator problem when
   python was in fact never started.
2. **Switching to PowerShell or a no-shell exec would not pass either**, but it
   would fail *legibly*: verify.py would report `missing <the path>`, which points
   at the placeholder immediately. Only substitution (or accepting
   `desktop/coverage/d-shell/`) makes the gate exit 0.

**Fix (harness-side — outside this task's write set):** substitute the
placeholder before invoking, or accept the canonical path. Both of these exit 0:

```
python .future/cov100/verify.py js-module group:d-shell 99.99 docs/testing/desktop-shell.md desktop/coverage/d-shell/coverage-summary.json
python .future/cov100/verify.py js-module group:d-shell 99.99 docs/testing/desktop-shell.md desktop/coverage/todo_8cb0c2e99ae7/coverage-summary.json
```

### Two further repo-level checks this task cannot satisfy

Both are aggregation checks whose inputs live outside the declared write set
(`desktop/src/components`, `desktop/src/app`, `desktop/src/main.tsx`, this file):

* `verify.py weak` → `FAIL: docs/testing/weak-test-audit.md missing`. That is a
  repo-wide doc, so it has to be owned by one worker (or the supervisor), not by
  each subtree. Nothing in this subtree is implicated: `debris` is 0
  scratch-looking files, and §2 records that no `only`/`skip`/snapshot exists here.
* `desktop/coverage/tmp-probe/` holds a report for
  `src/features/artifacts/ArtifactDetailPanel.tsx` — **another worker's**
  subtree, so it is deliberately left untouched, per the coordination rule that
  peers do not tidy each other's artifacts during a shared run.

## 1. Run identity and measurement

| what | value |
|---|---|
| goal todo | `todo_8cb0c2e99ae7` (group `d-shell`) |
| measurement | `cd desktop && npx vitest run --coverage --maxWorkers=2 --retry=2 --coverage.reportsDirectory=coverage/d-shell` |
| report | `desktop/coverage/d-shell/coverage-summary.json` |
| gate | `python .future/cov100/verify.py js-module group:d-shell 99.99 docs/testing/desktop-shell.md desktop/coverage/d-shell/coverage-summary.json` |

**The measurement is the whole-project suite, with no exclusions.** The final run
was the command above over the entire `desktop/` project:
**245 test files / 3152 tests, all passed**, report covering all 62 expected source
files for this group. `--retry=2` only retries a test that failed for
environmental reasons on this shared, heavily loaded checkout (several peers
compile Rust in the same tree); it never weakens an assertion.

> **This project-wide count is a moving target and must not be read as a claim
> about this group.** Peers advance `desktop/src/features/**` in the same checkout,
> so the total moved 3191 → 3168 → 3152 within one afternoon while **this group's
> own figures never changed** (62 files, 1878/1889 lines, 11 uncovered, 19 arms) and
> its suite stayed at **52 files / 643 tests**. The gate is scoped to `group:d-shell`
> and prints only the group's numbers, which is why it stayed green throughout. Any
> project-wide figure quoted elsewhere in this doc is a snapshot taken at that
> moment, not a value to match.
Two things worth recording for anyone reproducing this:

1. **A parse error in *any* test file aborts vitest's coverage writing.** A
   peer's mid-edit file (`src/features/agent/useThreadMessages.events.test.tsx`,
   `PARSE_ERROR: Unexpected token`) produced two runs that finished with **no
   report at all**, and the gate then fails with "missing coverage-summary.json".
   That message means "the measurement never finished", not "coverage is bad".
2. **A run killed by a timeout also writes no report.** Check the report's
   `LastWriteTime` against the newest source edit before believing the gate —
   and re-measure after any mutate/revert cycle, because the reverted files carry
   newer mtimes than the report even when their content is unchanged.

| | lines | branch coverage | files with uncovered lines | uncovered lines |
|---|---|---|---|---|
| before this segment | 55.9555 % (1057/1889) | 69.09 % (968/1401) | 41 | 832 |
| after this group's work | **99.4177 % (1878/1889)** | **98.64 % (1382/1401)** | 6 | **11** |

#### Statement coverage — reported because the contract requires both numbers

The operating contract's *"JS/TS: report BOTH line and statement coverage"* section
is explicit that a JS/TS module must carry both figures, because istanbul (and, in
the general case, a line metric) can credit a line to whichever statement on it
executed — so `if (pending.current) return;` can read as covered while the `return`
never ran. This doc originally reported only lines, which was a **gap against the
contract**, found by re-reading the contract rather than the measurement.

| | statements | uncovered statements | files |
|---|---|---|---|
| this group (`d-shell`) | **99.4557 % (2010/2021)** | **11** | **6** |
| whole project (`total` row, all measured files) | 99.35 % (9334/9395) | — | — |

The two lists are **identical here** — the same 11 items in the same 6 files — which
is the contract's own sanity criterion, enforced by the gate as
`uncovered statements - uncovered lines > max(20, uncovered lines)`: for this group
that is `11 - 11 = 0`, well inside the bound. The contract notes desktop is
generally unaffected because vitest's v8 provider agrees with itself
(`d-agent 15/15`, `d-settings 10/11`, `d-panels 12/14`, `d-pkgs 5/8`); this group
agrees exactly, so **the line list is also the statement list** and §4's ledger
doubles as the statement work list. No hidden untested guard is concealed by the
line metric here.

The independent pass also lists **6 files with fully-uncovered lines**, which is
exactly the 6 files in §4 — so the ledger reconciles with the measurement, not
merely with the summary. (The group's line *percentage* below is measured against
all 62 files; the 11 uncovered lines are the whole of the gap.)

### Branch coverage

Branch coverage closed from 69.09 % to **98.64 % (1382/1401)**. Every file the gate had reported
as "lines 100 % but branches short" is now either closed or waived with a proven
argument:

| file | before | now |
|---|---|---|
| `hooks/useAppStartup.ts` | 66.7 % (4/6) | **100 % (6/6)** |
| `hooks/useLeftPanelWidth.ts` | 75.0 % (12/16) | **100 % (16/16)** |
| `hooks/useWorkspaceDialogs.ts` | 77.3 % (17/22) | **100 % (22/22)** |
| `hooks/useThreadDialogs.ts` | 78.2 % (43/55) | 98.2 % (54/55) |
| `hooks/useFutureAccount.ts` | 85.7 % (30/35) | **100 % (35/35)** |
| `hooks/useAppSettings.ts` | 64.3 % (9/14) | 92.9 % (13/14) |

The gate now prints only two files, both waived in §4:
`ui/overlayStack.ts` 83.3 % — **not** the `stack.length > 0 &&` short-circuit
(that is covered, `[26, 24]`); the uncovered arm is the cleanup's defensive
`if (index !== -1)` — and
`layout/ActivityRailSelectionToolbar.tsx` 87.5 % (`if (checkboxRef.current)` —
React attaches refs before effects run — `unreachable-by-construction`).

> Correction: an earlier revision of this doc attributed the `overlayStack.ts`
> gap to the `&&` in `isTop()`. Reading the branch map (`b["1"]` has a zero arm)
> shows the `&&` arms are both taken and the real gap is the cleanup guard. The
> §4 row and the `ui/overlayStack.ts` entry below are the corrected attribution.

The gate prints only the two *worst* files, so it **substantially under-reports the
branch picture**. Dumping every uncovered arm from `coverage-final.json` gives
**19 uncovered branch arms across 12 files**, all of them dead by construction
(see Appendix A for the full inventory and each reason). The gate's own output
labels this informational — *"the gate is on lines, but uncovered branches are
untested boundary cases and belong in your dimensions evidence"* — so Appendix A
is that dimensions evidence.

#### Complete branch-arm ledger for `layout/ActivityRail.tsx`

The gate only prints the two *worst* files, so this file's arms were previously
recorded only as six uncovered *lines*. Dumping every uncovered arm from
`coverage-final.json` (statement counts and `branchMap`/`b`) gives six arms, and
each now has its own classification. Counts are illustrative (the **zero** is the
claim):

| arm | expression | category | why |
|---|---|---|---|
| L362 `cond-expr` 0 | `featureItems.length > 0 ? (<div>…) : null` | `unreachable-by-construction` | `featureItems` is a module-level `const featureItems: […] = []` (L92) that is never reassigned, so `.length > 0` is permanently false and the whole map body (L363-369) is dead |
| L413 `cond-expr` 0 | `threadSelectionMode(thread) ? toggleThreadSelection : undefined` | `unreachable-by-construction` | this row is inside the *pinned* section; `threadSelectionMode` is `selectionMode && isThreadInScope(thread)` and `isThreadInScope` returns false for any `pinned` thread (already asserted in `hooksEnv.test.tsx` and `useRailSelection.test.ts`), so the truthy arm cannot be taken. **Guard test added** (round 3, fault 12) |
| L702 `if` 0 | `if (a.status !== b.status)` | `unreachable-by-construction` | the sole call site filters to `status === "active"`, so both compared items always share a status. **Guard test added** (round 3, fault 11) |
| L703 `cond-expr` 0 and 1 | `a.status === "active" ? -1 : 1` | `unreachable-by-construction` | same reason — the enclosing `if` never runs, so the ternary is never even evaluated |
| L711 `binary-expr` 2 | `thread.lastMessageAt ?? thread.updatedAt ?? thread.createdAt` | `unreachable-by-construction` | every thread comes from `list_threads`, and its **producer** guarantees the field: `desktop/src-tauri/src/store/threads.rs:28` declares `pub updated_at: i64` — non-optional, no `skip_serializing_if`, and `THREAD_COLUMNS` always selects it. Only `last_message_at: Option<i64>` is optional (matching `lastMessageAt?: number \| null` in TS), and **that** arm is the one taken. No production code builds a `StoredThread` locally (checked: the only files matching a thread-shaped literal are the store integration and this component, none of which construct one), so a value without `updatedAt` cannot reach `sortThreads` |
| L364/369/658/659/665 statements | the two dead `featureItems.map` bodies | `unreachable-by-construction` | same empty-constant reason as L362 |

Note L711's category is `unreachable-by-construction` with the *type
declaration* as the construction — not `unreachable-in-this-environment`, which
would be about the measurement platform. A test could only reach it by casting a
fixture with `as unknown as StoredThread`, i.e. by asserting behaviour on data
this app cannot produce, which is precisely the coverage-shaped test this task
is meant to remove; it is recorded rather than written.

#### The remaining single arms, and how each was chased

- **`ui/overlayStack.ts` L31** (`if (index !== -1)` in the effect cleanup) —
  `unreachable-by-construction`, and backed by a **failed counterexample attempt**
  rather than reasoning alone. Structurally the module-level `stack` has exactly
  one mutator: the cleanup of the effect that pushed that id. Each cleanup removes
  its own id by `lastIndexOf`, and React runs an effect's cleanup **at most once
  per setup**, so an id cannot be missing when its own cleanup runs — the guard is
  defensive against a double cleanup that React never performs. Empirically, a
  dedicated `ui/overlayStack.test.tsx` exercises the strongest available routes
  and the arm is **still** `[190, 0]`-style zero (counts drift with every run; the
  *zero* is the claim): StrictMode's setup → cleanup → setup double-invoke (the
  closest React gets to a repeated cleanup), three open trees unmounted out of
  registration order, nested overlays closed inner-then-outer, and repeated
  register/unregister cycles. Note the sibling arms *are* taken: the `!open` early
  return and the `&&` in `isTop()` both have non-zero counts, so this is a single
  genuinely-dead arm, not an untested path.

- **`hooks/useAgentStatus.ts` L54** — `commit`'s `if (cancelled) return;`. Reachable
  only if the splash-delay timer fires after teardown, and the effect cleanup
  clears exactly that timer. The *failure* arm at L48 is now covered (see §4),
  which is the one that actually mattered: it is the path the Agent-down status
  page depends on.

- **`hooks/useThreadDialogs.ts` L66** — the `: current` side of the title-generation
  setter when a generation result belongs to a *replaced* dialog. The behaviour is
  asserted (`useThreadDialogs.test.tsx` proves a superseded generation is dropped
  and a newer dialog's in-flight marker survives), but the arm itself needs a
  result to arrive for a dialog whose `generation` no longer matches while the
  newer dialog is still open — the two tests that try it land on the *other* side
  of the ternary (the values are equal), so it is recorded rather than claimed.

- **`hooks/useAppSettings.ts` L131** (`if (sandboxFallbackRef.current)`) —
  `unreachable-by-construction`, and this one is backed by a **failed
  counterexample attempt**, not just reasoning. The chain: (1) the ref is true only
  between `sandboxFallbackRef.current = true` and the pending write's `.finally`;
  (2) `sandboxFallbackRequired` is true only while `approvalTier === "sandbox"`,
  and the same effect body has already called `setAppSettings` with
  `approvalTier: "manual"` optimistically, so every later render sees
  `required === false` and takes the *first* branch (`ref = false`); (3) the only
  other way back to `"sandbox"` is a reload, and `reloadSettings` chains its read
  behind the pending write, so it cannot land before the write resolves — by which
  time `.finally` has cleared the ref. Empirically: mounting under `StrictMode`
  (verified to double-invoke, `effectRuns === 2`) left the arm at `[0, 0]`; then a
  test that holds the fallback write open, changes the probe's code and fires a
  window-focus reload got the arm *evaluated* four times and it still never took
  its true side (`[0, 4]`).

- **`hooks/useContextData.ts` L211** (`if (!cancelled)` inside the minimum-spinner
  `setTimeout`) — `unreachable-by-construction`. The timer is armed only from the
  bootstrap's `.finally`, which itself returns early when `cancelled` is already
  true; and the effect's cleanup does `if (minTimer) clearTimeout(minTimer)`. So
  the callback can only fire while `cancelled` is still false — the guard's false
  side is unreachable, not merely untaken. **Delete-and-tested** (§2b fault 18):
  with the guard removed the *whole* project suite still passes, which is the
  expected signature of an unreachable arm. Note the paired-clear caveat from
  §2b round 9: this guard and `useAgentStatus.ts:54` are unreachable *because*
  something else clears their timer, so they are not redundant in the way
  `OnboardingGate.tsx:211` is.

- **`layout/ContextPanel.tsx` L172** (`if (first)` before `onTabChange(first.value)`) —
  `unreachable-by-construction`. `tabs` is one of three module-level non-empty
  constants (`gitTabs` 3 entries, `fileTabs` 2, `pendingTabs` 2), so `tabs[0]` is
  always defined and the "no tabs to fall back to" branch cannot be entered.

## 1b. Coverage attribution — whose tests produce this number

The figure in §1 is a **group** measurement, not a claim about the tests this task
wrote. Measured the same way each time (per-line, same command, `--retry=0`):

| test population | lines | files with uncovered lines | gate |
|---|---|---|---|
| **all 52 test files in the group** (the measured suite) | **99.4177 % (1878/1889)** | **6** | **passes** |
| the **46** files that remain if the 6 untracked pre-session files are lost | **92.3769 % (1745/1889)** | **12** | **fails** |
| only the **25** files authored by this session | 84.3833 % (1594/1889) | 20 | — |

So **284 of the 1878 covered lines come from the other 27 test files**, and **14
source files reach 100 % only because of tests this task did not write** —
`useThreadStore.ts` (120 of its 120 lines), `useRightPanelWidth.ts` (46),
`useHasProviders.ts` (41), `useUpdateChecker.ts` (17),
`usePendingApprovalCounts.ts` (16), `useRemoteStatus.ts` (8), `useAgentDoneBell.ts`
(6), `AgentStatusGate.tsx` (5), `Badge.tsx` (2), plus partial help on
`ActivityRail.tsx`, `threadTree.ts`, `ThreadListItem.tsx`, `panelGeometry.ts` and
`useAutoUpgradeSkills.ts`. Attribution is by **mtime** (this session id is
`20260926-015904` = 01:59:04; the six pre-session files carry 01:41–01:56).

### The operational consequence — read this before committing

**Six of the 27 non-mine test files are UNTRACKED**, so they are not in git and
would be destroyed by a `git clean` or lost if a checkpoint commit includes only
this task's files:

```
hooks/useAgentConnection.test.tsx     hooks/useRemoteStatus.test.tsx
hooks/useHasProviders.test.tsx        hooks/useRightPanelWidth.test.tsx
hooks/usePendingApprovalCounts.test.tsx   hooks/useUpdateChecker.test.tsx
```

Losing them moves the group from **99.4177 % (gate passes)** to **92.3769 % with 12
uncovered files**, and **six** of those twelve —
`useAgentConnection.ts`, `useHasProviders.ts`, `usePendingApprovalCounts.ts`,
`useRemoteStatus.ts`, `useRightPanelWidth.ts`, `useUpdateChecker.ts` — are **not in
§4's waiver ledger**, because at 52 files they are fully covered and need no waiver.
A commit containing only this task's work would therefore look complete and fail the
gate. **The checkpoint commit must include those six files.**

**This is measured, not predicted.** Running the gate against the 46-file
measurement (the state a commit-without-those-files would reproduce) exits **1**:

```
FAIL: 6 undocumented uncovered file(s) in group:d-shell
group:d-shell: 92.3769% lines (1745/1889) ... 144 uncovered line(s) in 12 file(s)
  docs/testing/desktop-shell.md: 6 file(s) not properly waived
    - .../hooks/useAgentConnection.ts (5 uncovered): not named in the doc
    - .../hooks/useHasProviders.ts (41 uncovered): not named in the doc
    - .../hooks/usePendingApprovalCounts.ts (16 uncovered): not named in the doc
    - .../hooks/useRemoteStatus.ts (8 uncovered): not named in the doc
    - .../hooks/useRightPanelWidth.ts (46 uncovered): not named in the doc
    - .../hooks/useUpdateChecker.ts (17 uncovered): not named in the doc
```

The gate names exactly the six source files this section predicted, and the
remedy it prints is the one to avoid: *"add it under waivers with a category and a
reason, or cover it"* — the right answer here is **neither**; it is to keep the six
test files, which need no waiver at all.

(The 21 remaining non-mine files are tracked and carry no such risk.)

## 2. Tests added in this segment

New files (all under `desktop/src/components/`, plus one under `desktop/src/app/`):

> **Scope of this table.** It lists the tests **this task added** (31 untracked files
> beneath `src/components/` and `src/app/`). It is **not** the group's complete test
> inventory, and a reviewer should not read it as such: **6 further test files are
> present in the group's directories but were authored by an earlier session**,
> not by this task —
> `hooks/useAgentConnection.test.tsx`, `hooks/useHasProviders.test.tsx`,
> `hooks/usePendingApprovalCounts.test.tsx`, `hooks/useRemoteStatus.test.tsx`,
> `hooks/useRightPanelWidth.test.tsx`, `hooks/useUpdateChecker.test.tsx`.
> Attribution is by mtime: those six carry timestamps **01:41–01:56**, while this
> session id is `20260926-015904` (01:59:04) and this task's earliest file is
> `useAgentConnection.race.test.tsx` at **02:12**. They are untracked (the supervisor
> commits at checkpoints), which is why they are easy to miss. **Their tests run in
> the same suite and contribute to the measured coverage below** — so the group's
> 99.4177 % is the result of this task's tests *plus* those six files' tests, and
> §4's waivers are what carry the remainder. Verified with the reverse-direction
> documentation audit in §5.

| file | covers |
|---|---|
| `ui/DiffView.test.tsx` | `DiffView` (64 uncovered lines), every row kind, hunk numbering, header-vs-content disambiguation |
| `ui/FloatingScrollbar.test.tsx` | all four `height > 0` × `visible` arms as an explicit table (was lines 100 % / branches 50 %), the grab cursor, geometry passthrough, drag forwarding |
| `ui/uiExtra.test.tsx` | `TextInput`, `Select`, `Switch`, `Dialog`, `Field`, `EmptyState`, `FileTypeIcon`, `CopyButton`, `CopyablePre`, `SelectMenu`/`SelectMenuItem` |
| `ui/overlayStack.test.tsx` | the overlay layer stack directly (it had no dedicated test file): empty-stack `isTop()`, register/unregister, only the last-opened layer is top, nested restore, StrictMode mount/unmount leak check, three trees unmounted out of order, and an end-to-end *"lets only the topmost Overlay close on Escape"* check plus the non-Escape-keys case |
| `ui/ToastHost.test.tsx` | toast stack, tone styling, per-toast auto-dismiss, timer cleanup |
| `app/App.test.tsx` | the root component (was a waiver): asserts it mounts `AppShell` with no props of its own |
| `layout/AppShell.test.tsx` (stale-edge guard) | the falsification attempt behind `AppShell.tsx:563`'s waiver: a hover edge captured while collapsed, then detached by expanding, must not arm the preview when the panel is collapsed again — with a live edge re-checked so the assertion is about staleness rather than a broken preview. The arm survived (`[0, 8]`), which is what makes that waiver counterexample-backed |
| `layout/AppShell.test.tsx` (divider a11y) | the left resize divider's **ARIA separator contract**: `aria-valuemin ≤ aria-valuenow ≤ aria-valuemax` (the invariant, not just the individual attributes — a `now` outside the range is the classic broken-slider bug), the ceiling tracking the panel hook's computed max rather than a hard-coded number, and `tabIndex === 0` plus a real `focus()` so the arrow-key path is keyboard-reachable. Closes the `aria-valuemax` and `tabIndex` gaps found by the §5 accessibility audit |
| `layout/OnboardingGate.test.tsx` (env-switcher loading) | the dev switcher's **loading sub-state**: while `getFutureEnvironment` is pending the placeholder reads `"..."` (not `"custom"`, which would imply the environment is already known) and the control is disabled; once the probe answers with a non-test/production value it becomes `"custom"` and usable. Closes the state-audit gap in §5; mutation-checked in §2b round 12 (`expected 'custom' to be '...'`) |
| `layout/OnboardingGate.test.tsx` (pre-selected default) | the picker **pre-highlights the first recommended model** — exactly one card carries `bg-accent-soft`, and it is the first — and confirming without touching the picker applies that same model. Locks the invariant that holds `handleStart`'s two fallback arms dead; mutation-checked in §2b round 11 (`expected [] to deeply equal [ 'Model first' ]`) |
| `app/main.test.tsx` | the **entry point** (`src/main.tsx`, was a waiver): imports it for its side effect against a document that owns `#root` and asserts the app lands **in that element** after flushing React's concurrent render; plus the negative case where `#root` is absent and the import must **reject** rather than mount nowhere. Lives in `src/app/` (in the write set) because `main.tsx` is enumerated as a file |
| `layout/ContextPanel.test.tsx` | the whole panel (105 lines): tab set per thread kind + capability pending, tab fallback, loading/no-thread/file-tree states, runs/inspect drill-down, review, artifacts + detail, resize divider, once-per-open tab seeding, all four inspect events; plus the **null-workspace** pass-through to both the file tree (`rootPath`) and the artifacts panel (`workspacePath`), the tool→run lookup when the run has left the snapshot, and the inspect-run event that must *not* re-issue the tab change while already on runs |
| `layout/OnboardingGate.test.tsx` | the whole gate (123 lines): idle/busy/failed login states, BYOK + both cancel paths, auto-login, the three init steps (retry, catalogue failure, skills failure, empty-catalogue deadline, 500 ms floor), the ≥2-recommended picker vs single-default auto-apply, the in-flight guard, the dev environment switcher, and **localized model descriptions** incl. a blank preferred locale falling back to the other |
| `layout/AppShell.test.tsx` | the whole shell (203 lines): the three readiness gates, layout/section/center-mode switching, collapse + hover-preview behaviour, terminal mounting, store error, every rail/workspace/thread handler, the dialog confirmations, and all six event bridges; plus the **no-thread / no-workspace** state (rail gets a null id, approvals scoped to `null`, all four `activeThread?.id ?? …` handlers refresh the whole store), the chat-mode **restore**, the **default-model fallback** for the coach conversation, the **non-connected agent** effect (no skills refresh, no forced revalidation) and the resize divider's **non-arrow key** no-op |
| `layout/ActivityRail.variants.test.tsx` | the rail's variant axes (28 branch arms): floating vs docked skin + toggle label, the remote indicator's four tones, the macOS traffic-light inset as a 3-row table, skill badge/dot/attention, selection mode (scope, tri-state, toolbar actions), signed-out variants, thread ordering incl. pinned hoisting, **and the two scope-invariant guards below** |
| `layout/ActivityRail.variants.test.tsx` (scope-invariant guards) | the two invariants that *hold the rail's last dead branch arms dead*, now falsifiable instead of prose: (a) only `status === "active"` threads are listed, so the sort's `a.status !== b.status` arm cannot run — asserts an archived and a deleted thread are absent; (b) pinned threads are outside batch scope, so the pinned row never offers a selection toggle — asserts a pinned row has no checkbox while the unpinned one does, **and** that select-all over that scope reports exactly `1 selected`, which would read `2 selected` if the pin filter went away. Both guards were mutation-checked (§2b round 3) |
| `layout/ActivityRailSelectionToolbar.test.tsx` | the (selected, total) matrix as a 4-row table (was lines 100 % / branches 62.5 %), the tri-state recomputation, disabled delete, both actions |
| `layout/dialogs.test.tsx` | `RenameDialog`, `ConfirmDeleteDialog`, `AppShellDialogs`, `WorkspaceDialogs`, `LeftPanelTitlebarToggle`; plus the **dismissed-while-editing race** — one `act` that dismisses then fires the late edit, so the updater holds `null` — for the rename input, the batch-delete file toggle and the workspace rename input |
| `layout/ActivityRail.nav.test.tsx` (nav-set guard) | the rail's **complete labelled nav entry set**, in render order (`EXPECTED_NAV_ENTRIES`), asserted exactly rather than by spot-checking individual entries. Round 5's `featureItems` experiment showed a stray entry could appear with 55/55 tests still green; this test fails on such a stray (`+ "Scratch feature"`) and on a dropped entry (`- "Models"`), both mutation-checked in §2b round 6 |
| `layout/ContextPanel.test.tsx` (divider a11y) | the right resize divider is **keyboard-reachable**, not drag-only: `tabIndex === 0` and a real `focus()` landing on it, plus `aria-orientation`. Closes the `tabIndex` gap from the §5 accessibility audit — the arrow-key nudge test only implied focusability before |
| `layout/railMenus.test.tsx` (aria-controls) | the account trigger's `aria-controls` **resolves to the element it controls**: asserted null while closed and, once open, that the id resolves to the real `role="menu"` element. Closes the `aria-controls` gap from the §5 accessibility audit — a dangling reference is a common React defect precisely because the menu is only rendered while open |
| `layout/railMenus.test.tsx` | `ThreadItemMenu`, `WorkspaceHeaderMenu`, `ChatSectionMenu`, `ActivityRailAccountFooter`; plus the **dropdown flip** (`useDropUpMenu` resolving drop-up when the menu would spill past its clipping container — reproduced by stubbing the measured rect, since jsdom reports zero rects and the flip arm was otherwise unreachable in tests) and two footer boundaries: the update dot on the *settings-row* branch (only reachable without an account), and an address starting with `@` so `split("@")[0]` is empty and the `\|\| email` fallback keeps the whole address as the avatar label |
| `layout/ActivityRail.nav.test.tsx` | rail nav entries (expanded + collapsed), section collapse toggles, workspace group actions + header menu, thread actions menu, skill-intro bubble and 4 s attention pulse |
| `layout/ThreadListItem.actions.test.tsx` | context menu, actions trigger, menu items, Escape/outside dismissal + focus restore, selection checkbox, run-status indicators, approval badge |
| `layout/hooks/flows.test.tsx` | `useWorkspaceDialogs` (incl. all five dismissed-dialog/null-updater arms), `useNewConversation`, `useApprovals` |
| `layout/hooks/hooksEnv.test.tsx` | `useWindowWidth`, `useDropUpMenu`, `useCollapsedWorkspaces`, `useLeftPanelWidth` (incl. both sides of both width thresholds, the stored-value edge cases and pointer-identity in the drag), `useRailSelection` |
| `layout/hooks/hooksMisc.test.tsx` | `useAgentStatus` (incl. the probe-failure arm, the failure→recovery tick, and the unmount-before-first-probe teardown that must not re-arm a poll), `useAutoUpgradeSkills`, `useAppStartup` (incl. both cancelled-late arms), `useAppSettings` (incl. the serialized write queue: a superseded success, a superseded failure, a superseded reload, the applied reload, the generic `probe_failed` code, and the single-fallback-write property under a held write) |
| `layout/hooks/useModelSelection.actions.test.ts` | `useModelSelection` persistence + derived state |
| `layout/hooks/useFutureAccount.events.test.ts` | `useFutureAccount` push handlers, balance lifecycle, stale-response guards |
| `layout/hooks/useAgentConnection.race.test.tsx` | `useAgentConnection` generation guards + readiness classification |
| `layout/hooks/useContextData.test.tsx` (extended) | the post-git generation check's **live** side: a refresh whose `ensureWorkspaceGit` probe finishes while it is still the newest one must carry on and fetch |
| `layout/hooks/useRailSelection.test.ts` (extended) | only Escape leaves selection mode; with selection mode off there is no listener at all, so Escape is an inert key |
| `layout/hooks/useThreadStore.hook.test.ts` (extended) | a bootstrap failure that lands **after** unmount is not written into the dead hook; and the pre-existing *"ignores a bootstrap that resolves after unmount"* now asserts the observable parts instead of asserting nothing |
| `ui/useCopyState.test.ts` (repaired) | the pre-existing *"clears a pending reset timer on unmount"* asserted nothing; it now proves the cleanup ran via the pending-timer count (1 before unmount, 0 after), and that assertion is mutation-checked (§2b fault 13) |

Test style note: the repo has no `@testing-library/react` dependency and
`desktop/package.json` is outside this task's write set, so the behavioural
component tests follow the repo's existing harness (`react-dom/client`
`createRoot` + `act`, `desktop/src/test/renderHook.ts`) and assert on rendered
DOM, focus, disabled state, emitted events and call arguments — never on
snapshots.

Weak tests fixed in this segment (**weak-tests-fixed**): the group was audited
for the three shapes the task names.

* **No-assertion tests** — **two found and fixed.** Both were pre-existing tests in
  this subtree whose body ended in a bare `act(...)` with a comment claiming the
  behaviour but no assertion (`useCopyState.test.ts` *"clears a pending reset
  timer on unmount"*, `useThreadStore.hook.test.ts` *"ignores a bootstrap that
  resolves after unmount"*). Neither could fail, so neither was testing anything.
  * `useCopyState` is now **discriminating**: `expect(vi.getTimerCount())` is 1
    after the copy and 0 after unmount. Mutation-checked (fault 13 in §2b):
    replacing the cleanup with a no-op made it fail `expected 1 to be +0`.
  * `useThreadStore` could **not** be made discriminating, and the doc says so
    rather than pretending otherwise: React 18+ silently drops a post-unmount
    `setState`, so removing the `if (cancelled)` guard is invisible to any
    assertion. It now asserts the observable parts — no `console.error`, and the
    consumer's last snapshot is unchanged — and carries an in-file note that it
    is a regression guard against the late path *throwing*, not proof that the
    guard prevents the write.
  * A scan for this shape across all 52 test files in the subtree now reports no
    genuine no-assertion block (the only remaining hits are regex artifacts from
    multi-line titles, each of which does contain `expect`).

  Cross-checked with the **harness's own criteria** rather than only my own scan:
  `verify.py`'s `WEAK_PATTERNS` and `DISABLED_PATTERNS` applied to all 52 test
  files here produce **0 hits each**. That matters because those two checks
  (`verify.py weak`, `verify.py skipped`) are the repo's definition of a weak or
  disabled test; my subtree clears them on the harness's terms.
* **Tautologies** — two removed. A `expect(props.size).toBe("xs")` type-smoke in
  `uiExtra` (it asserted the prop it had just passed, exercising nothing), and a
  StrictMode case whose stated claim the branch counts did not support
  (`[0, 3]`: the guard's true arm never ran, so the test looked stronger than it
  was). The latter's *finding* is kept in §4 as a counterexample, which is the
  part worth having.
* **Snapshots instead of behaviour** — none; there is no snapshot in this subtree.
* **Order-dependent tests** — **one found and fixed**, and it is the subtlest of the
  weak-test shapes because it *passes*.
  `AppShell.test.tsx` *"creates a workspace and refreshes the store onto the active
  thread"* read `children.newConversation` — a module-level child-prop capture —
  **without first mounting that child**. It therefore asserted against a props
  object left behind by whichever earlier test had rendered `NewConversation`, so
  it passed for the wrong reason and exercised none of its own arrange. Found by
  running the subtree with `--sequence.shuffle`: under seed `1790401261161` it fails
  with
  `TypeError: Cannot read properties of null (reading 'onAddWorkspace')`.
  Two fixes, because the test was only half the problem:
  1. the missing arrange (`act(() => rail().onNewChat())`, so the composer is really
     mounted) — making the test exercise its own render;
  2. a **structural guard**: `beforeEach` now clears every capture on `children`, so
     the whole *class* of defect fails loudly instead of passing by luck. All 55
     tests in the file still pass with the reset, which proves this was the only
     borrower and that the other readers already arranged properly.
  Verified by re-running the exact failing seed (green) plus three further shuffled
  orders (green).
* **Dead imports** — removed from `useThreadDialogs.test.tsx` and
  `useUnreadThreads.test.tsx`.
* **`act` hygiene** — three `useAppSettings` write-queue tests called
  `changeSettings` outside `act`, so React logged "An update to Probe inside a
  test was not wrapped in act(...)" for each. The calls are now made inside
  `act`; the file runs warning-free. (The assertions were correct before too, but
  a warning-free run is what makes a failure signal unambiguous.)

### 2b. Mutation sample — the tests fail when the code is wrong

Coverage proves a line ran; it does not prove a test would notice if the line
were *wrong*. So deliberate faults were injected into production code in this
subtree, one per high-value behaviour, and the affected suites were run:

| # | file | injected fault | killed by |
|---|---|---|---|
| 1 | `hooks/useLeftPanelWidth.ts` | `>= 1280` → `> 1280` (off-by-one at the width threshold) | **6** tests, incl. the boundary table's *"at the 1280 threshold"* row |
| 2 | `hooks/useAgentStatus.ts` | probe failure yields `phase: "ready"` instead of `"unavailable"` | *"keeps polling after a probe failure so a later recovery is picked up"* |
| 3 | `layout/ActivityRailSelectionToolbar.tsx` | `if (checkboxRef.current)` → `if (!checkboxRef.current)` (tri-state never set) | **3** tests, incl. the `2 of 3 ⇒ indeterminate=true` table row |
| 4 | `hooks/useAppSettings.ts` | write-success generation guard `===` → `!==` (stale snapshot applied) | **6** tests, incl. *"does not apply a superseded write's snapshot over the newer edit"* |
| 5 | `ui/DiffView.tsx` | added-line counter `newLine += 1` → `+= 2` | *"numbers added rows by the new file and deleted rows by the old file"* |

**Result: 5 of 5 mutants killed — 16 failing tests across 4 files, 0 survivors.**
Fault 1 is the most reassuring: the *boundary* assertion built for the threshold
is exactly what caught the off-by-one, and fault 4 is caught by the write-queue
test added in this segment — so the two most "coverage-shaped" tests are
demonstrably load-bearing rather than line-touching.

#### Round 2 — the three big components (the largest block of new coverage)

Round 1 covered hooks and small widgets; the three components that hold almost
all of the new lines (`AppShell` 202/203, `OnboardingGate` 122/123,
`ContextPanel` 105/105) had not been mutation-tested. A second round targeted
their *decision* points rather than their arithmetic:

| # | file | injected fault | killed by (assertion as reported) |
|---|---|---|---|
| 6 | `layout/AppShell.tsx` | readiness gate `phase !== "ready"` → `=== "ready"` (shell renders while not ready) | `expected 'ready' to be 'starting'` — the AgentStatusGate test |
| 7 | `layout/AppShell.tsx` | `thread.mode === "workspace" ? "workspace" : "chat"` → branches swapped | `expected 'workspace' to be 'chat'` — *"selecting a thread switches to its section"* |
| 8 | `layout/OnboardingGate.tsx` | auto-apply threshold `recommendedModels.length >= 2` → `>= 3` (2 recommended models silently auto-applied, picker never shown) | `expected '中文English…' to contain 'Choose your AI model'` — 2 tests |
| 9 | `layout/ContextPanel.tsx` | `\|\| workspaceKindPending \|\| seededForOpenRef.current` → `&&` (re-seeds on every effect run) | `expected "vi.fn()" to be called 1 times, but got 2 times` — the once-per-open seeding test |
| 10 | `layout/ContextPanel.tsx` | `gitReviews.branch?.files.length ?? 0` → `?? 1` (review tab counts data that is not there) | `expected 'Context panel viewFilesRuns' to contain 'Loading context...'` — the no-git-data hidden/loading boundary test |

**Result: 5 of 5 mutants killed — all three suites failed, 0 survivors.**
Mutants 8 and 10 are the ones worth noting: 8 is the *boundary* between "pick a
model" and "just use the only one", and 10 is the *empty-state* branch (`?? 0` vs
`?? 1`) — both are exactly the kind of condition a render-only test would leave
unverified.

#### Round 3 — mutating the *invariants* that hold the remaining dead arms dead

A waiver that says "this arm is dead because X always holds" is only as good as
X holding. The rail's last two dead arms rest on two in-component invariants, and
those were previously asserted only in this document. Round 3 removes each
invariant and checks that the new guard tests notice:

| # | file | injected fault | killed by (assertion as reported) |
|---|---|---|---|
| 11 | `layout/ActivityRail.tsx` | drop the call-site filter: `sortThreads(threads.filter(t => t.status === "active"))` → `sortThreads(threads)` | `expected [ 'Collapse sidebar', … ] to not include 'archived-one'` — plus the sibling orphan-row assertion |
| 12 | `layout/ActivityRail.tsx` | make pinned threads scoped: `return selectionMode && isThreadInScope(thread)` → `return selectionMode` | `expected [ 'Select pinned-one', … ] to not include 'Select pinned-one'` — plus `expected 3 to be 2` on the two pre-existing scope-count tests |

**Result: 2 of 2 killed, 0 survivors** (3 failing tests each). This is the round
that makes the two remaining branch waivers *falsifiable*: if a future change
removes either filter, a test fails rather than the waiver silently becoming
wrong — which is exactly how the earlier `useAgentStatus:48` waiver was found to
be wrong.

Every mutant in every round was reverted and verified byte-exact: `git diff` over
the touched files is empty, and the suites return to green. Through round 4:
**13 injected faults, 13 killed, 0 survivors.**

#### Round 4 — killing a *fixed* weak test's new assertion (fault 13)

The two no-assertion tests repaired in §2 were checked the same way as everything
else, so the repair is not taken on faith:

| # | file | injected fault | killed by (assertion as reported) |
|---|---|---|---|
| 13 | `ui/useCopyState.ts` | remove the unmount cleanup: `useEffect(() => clearTimer, …)` → `useEffect(() => () => {}, …)` | `expected 1 to be +0` — the pending-timer count after unmount |

#### Round 5 — falsification attempts on the remaining dead arms

A "this arm is dead because X cannot happen" waiver is the class that has already
been wrong once (`useAgentStatus:48`), so the highest-value remaining arms are
attacked rather than argued. A **failed** attempt is itself the evidence: it
converts the waiver from reasoning into something that was tested and survived.

| # | arm | attempted route | outcome |
|---|---|---|---|
| 14 | `AppShell.tsx:563` `if (showLeftPanel) return;` | the only route that could reach it: deliver a hover to an edge node **detached** by the layout flipping back to expanded. Test captures the edge while collapsed, expands (detaching it, asserted via `edge.isConnected === false`), dispatches `mouseover` on the detached node, collapses again, and asserts no preview appears — with a live edge re-checked afterwards so the assertion is about *staleness*, not a broken preview | **survived** — arm still `[0, 8]`; React's delegated event system drops events for unmounted fibres. Test kept (it encodes real behaviour) and the waiver is now counterexample-backed |
| — | `ActivityRailMenus.tsx:255` `if (items.length === 0)` | read the source for a way to empty the item query | **settled by reading**: the file contains **no `disabled` attribute at all**, so `[role=menuitem]:not(:disabled)` cannot filter, and all three menus render their items unconditionally (`ChatSectionMenu` exactly 1, `ThreadItemMenu` ≥2, `WorkspaceHeaderMenu` ≥4) |
| — | `ActivityRailSelectionToolbar.tsx:23` `if (checkboxRef.current)` | read the source for a conditional render of the checkbox | **settled by reading**: the checkbox is rendered unconditionally (L34-42) with no conditional anywhere in the 81-line file, so the ref is always attached when the effect runs |

| — | `ActivityRail.tsx` `featureItems` (L364/369/658/659/665) | **populate the array** and see whether anything notices | **two results, both worth having.** (1) *Reachability*: with one entry injected, L364/658/659 become covered, so the render path is real working code that "add them back to restore" (the source comment at L88-92) can re-enable — the line is dead only while the array is empty. (2) *Gap found*: **55 of 55 rail tests still passed**, i.e. a stray navigation entry could appear with no test noticing. Fixed — see the guard test below |

Rounds 1-3 injected faults and killed them; round 4 did the same for a repaired
weak test; round 5 attacked waivers and they survived — except the `featureItems`
row, which is the one that exposed a missing test rather than confirming a
waiver. The two "settled by reading" rows are weaker evidence and are labelled as
such rather than promoted to counterexamples.

#### Round 6 — the test that round 5 proved was missing (faults 15-16)

Round 5's `featureItems` experiment showed the rail never asserted its own nav
entry set, so `ActivityRail.nav.test.tsx` now does:

```
expect(labels).toEqual(EXPECTED_NAV_ENTRIES)   // the complete labelled-button set, in order
```

Derived from a deliberate mismatch (the first draft expected `[]` and the failure
diff printed the real six), so the expectation is measured rather than guessed.
It catches both a dropped entry and a stray one, and it is mutation-checked:

| # | file | injected fault | killed by (assertion as reported) |
|---|---|---|---|
| 15 | `layout/ActivityRail.tsx` | add one `featureItems` entry (the "restore" path the source comment describes) | `expected [ Array(7) ] to deeply equal [ Array(6) ]` — `+ "Scratch feature"`, i.e. exactly the stray entry that previously passed unnoticed |
| 16 | `layout/ActivityRail.tsx` | remove the `Models` entry from the nav | the same test, on the missing side |

**Result: 2 of 2 killed.** This is the round where attacking a waiver produced a
test rather than an excuse, which is the outcome the task's "strengthen real
tests" bar is actually asking for.

#### Round 7 — producer-side evidence for a "field is always present" waiver

Several arms are waived with the argument *"this fallback cannot be reached
because the field is always set"*. That argument is only as good as its source,
and the consumer's TypeScript type is the **weakest** possible source — it is an
assumption about the wire, not a guarantee. For `ActivityRail.tsx:711`
(`lastMessageAt ?? updatedAt ?? createdAt`) the claim was therefore re-derived
from the **producer**:

1. `desktop/src-tauri/src/store/threads.rs:28` declares `pub updated_at: i64` —
   non-optional (`created_at: i64` beside it, and only the genuinely nullable
   columns are `Option<i64>`: `last_message_at`, `last_opened_at`,
   `archived_at`, `deleted_at`, `agent_session_id`, …), with no
   `skip_serializing_if` on it.
2. `THREAD_COLUMNS` — the single column list every `SELECT` in that module uses —
   includes `updated_at`, so it is always on the wire.
3. Nothing in production builds a `StoredThread` locally (checked across
   `desktop/src`: the only files even matching a thread-shaped object literal are
   the store integration layer and `types.ts`, and the latter's `createdAt` hits
   are unrelated types — `StoredRunEvent`, `StoredToolCall`, `StoredToolOutput`).

So the arm is dead on a **producer** guarantee rather than a consumer type. This
is the same *kind* of upgrade round 5 gave `featureItems`: the waiver's wording
was already right, but the evidence behind it was weaker than it looked. The
carry-over is a rule for the remaining rows — when a waiver says "the field is
always present", cite the Rust struct or the `SELECT`, not the TS interface.

#### Round 8 — proving a guard redundant by deleting it

`OnboardingGate.tsx:210` (`if (starting) return;`) was waived on the argument that
React refuses to activate a `disabled` button, so the guard cannot run. That is a
claim about *reachability*, and it has a cheap decisive test: **delete the guard
and see whether anything notices**.

| # | file | injected fault | outcome |
|---|---|---|---|
| 17 | `layout/OnboardingGate.tsx` | delete `if (starting) return;` entirely | **0 tests fail** — the gate suite stays green (24/24 when recorded; **re-verified at the current 26/26** — the count grew as tests were added, the conclusion did not change) |

A surviving mutant is normally bad news, but here it is the *evidence*: the guard
is unreachable, and moreover **redundant** — nothing the component does today
depends on it. That is a stronger statement than the reachability argument it
replaces (and stronger than the other construction-only waivers, which have not
been deleted-and-tested). Two consequences recorded in §4 and §6:

* The guard's only caller is the Get-started button, which is
  `disabled={starting \|\| selectedModelId == null}` — so it would become
  load-bearing again the moment that `disabled` clause were relaxed (e.g. to let
  a user retry a failed start), which is exactly why it is waived rather than
  deleted.
* The test named *"applies only once even if Get started is activated again in
  flight"* passes with the guard gone, so it measures the **`disabled`
  mechanism**, not the guard. Its name is still accurate about behaviour (the
  second activation is ignored) — but the mechanism is now documented, so nobody
  reads it as coverage of line 210.

**Tally across all twelve rounds: 23 injected faults — 19 killed by a failing test,
and 4 deliberately left surviving (faults 17-20).**

#### The ledger itself was re-verified, not just written

A mutation ledger is only evidence if its entries are reproducible, so **six were
re-injected and checked against what this document claims — every killed entry
sampled, and all four survivors**:

| fault | ledger says | re-injection observed | verdict |
|---|---|---|---|
| 1 · `useLeftPanelWidth` `>= 1280` → `> 1280` | **6** tests fail, incl. the *"at the 1280 threshold"* row | **6** failed, and that named row is among them (`expected 256 to be 288`) | accurate |
| 21 · `DiffView` remove `oldLineNumber = oldLine;` | **3** tests fail, e.g. `expected [ '', '', '', '1', '', '2', '3' ] to deeply equal [ '', '', '', '1', '2', '2', '3' ]` | **3** failed, first assertion **character-for-character identical** to the quoted text | accurate |
| 17 · `OnboardingGate` delete `if (starting) return;` | **0 tests fail** (a deliberate survivor) | **0** failed — the guard is still redundant | accurate |
| 18 · `useContextData` L211 guard deleted | **0 tests fail** (survivor) | **0** failed | accurate |
| 19 · `useAgentStatus` L53 guard deleted | **0 tests fail** (survivor) | **0** failed | accurate |
| 20 · `ContextPanel` `if (first)` guard deleted | **0 tests fail** (survivor) | **0** failed | accurate |

All reverts were byte-exact (`git diff` empty). Two **stale figures** were found
and fixed in the process: fault 17's row quoted `24/24` from when it was recorded
(the gate suite has since grown to **26**), and fault 20's quoted `39/39`
(`ContextPanel` has since grown to **40**). Both rows now carry the recorded figure
*and* the re-verified one, since the *conclusion* in each case is unchanged while
the count moved — the drift a ledger accumulates if nobody re-reads it.

A survivor is normally a gap;
here each one answers a *reachability* question that a killed mutant cannot — and
the four are not interchangeable:

| survivor | what it proved |
|---|---|
| 17 `OnboardingGate:210` | the guard is **redundant** — deleting it is provably behaviour-neutral |
| 18 `useContextData:211`, 19 `useAgentStatus:53` | unreachable **because of a paired clear**; deleting the paired `clearTimeout` would make them load-bearing, so they are *not* safe to delete |
| 20 `ContextPanel:172` | unreachable today but **protective** — deleting it would throw on an empty `tabs` list |

And the killed ones are not all equal either: faults 1-16 killed faults *in code*,
while **fault 21 killed a mutation to the invariant that holds another waiver dead**
(`DiffView:102`'s assignment), upgrading that waiver from argued to test-guarded.

#### Round 9 — delete-and-test on two hook guards (a survivor that is *not* redundancy)

Round 8's technique was applied to the two remaining hook guards, where a **failing**
test would have falsified the waiver outright:

| # | file | injected fault | outcome |
|---|---|---|---|
| 18 | `hooks/useContextData.ts` | L211 `if (!cancelled) setLoading(false);` → `setLoading(false);` (guard deleted) | **0 tests fail** — the full project suite passes with the mutant in place |
| 19 | `hooks/useAgentStatus.ts` | L53 `if (cancelled) return;` deleted from `commit` | **0 tests fail** — same |

Both were run against the **whole** project (`npx vitest run --no-coverage`, 245
files / 3158 tests), not just the two owning files, because a guard could be
exercised indirectly through another component. Nothing anywhere distinguishes the
guard's presence from its absence, which is what an unreachable arm predicts.

**But this survivor is a *different* result from round 8's, and the difference
matters.** For `OnboardingGate:210` the survival proved the guard *redundant* — I
could point at its single caller and say nothing depends on it. Here the guards are
**unreachable but not redundant**: each is unreachable only because of a paired
mechanism elsewhere in the same effect — `useContextData`'s cleanup does
`if (minTimer) clearTimeout(minTimer)`, and `useAgentStatus` clears the very timer
that would re-enter `commit`. Delete the *paired* clearTimeout and these guards
become load-bearing again. So the correct characterization is:

* `OnboardingGate:210` — unreachable **and** provably redundant (safe to delete).
* `useContextData:211` / `useAgentStatus:53` — unreachable **because of a paired
  clear**, and deleting the guard would be a silent trap for whoever later removes
  that clear.

That distinction is now carried in §4 and §6 rather than left as "both are dead
guards". The mutation result also has a second use: it shows **no test in the
project covers the post-cancellation path** for either hook, which is exactly the
kind of gap a coverage percentage cannot show (both files are at 100 % lines).

#### Round 10 — the last two construction-only arms, one survived and one killed

The two remaining arms whose waiver rested on nothing but an argued invariant were
tested the same way. They produced **opposite** results, and both are useful:

| # | file | injected fault | outcome |
|---|---|---|---|
| 20 | `layout/ContextPanel.tsx` | delete the `if (first)` guard: `const first = tabs[0]; if (first) onTabChange(first.value)` → `onTabChange(tabs[0]!.value)` | **0 tests fail** — 39/39 `ContextPanel` tests green when recorded, **re-verified against the whole project suite (3191 tests, all green)**; the file has since grown to 40 tests and the conclusion is unchanged. Confirms `tabs` is never empty |
| 21 | `ui/DiffView.tsx` | remove the `oldLineNumber = oldLine;` assignment from the `delete` branch | **killed by 3 tests**, e.g. `expected [ '', '', '', '1', '', '2', '3' ] to deeply equal [ '', '', '', '1', '2', '2', '3' ]` |

Fault 21 is the more valuable of the two. The waiver for `DiffView.tsx:69`
(`return oldLineNumber ?? ""` for delete rows) said the fallback is dead because
the row builder always assigns `oldLineNumber` — a construction argument. The
mutation shows the **stronger** property: removing that assignment fails three
named line-numbering tests, so the invariant is *locked by tests*, and the `?? ""`
fallback is only reachable by breaking it — which the suite catches. That is an
upgrade from "argued" to "guarded", the same category as round 3's invariant
guards.

Fault 20 came back a survivor, and the right reading is the one from round 9 rather
than round 8: `ContextPanel:172`'s guard is **unreachable but protective**, not
redundant. Deleting it would not change today's behaviour (hence 0 failures), but
`tabs[0]!.value` on an empty list would *throw* — so the guard is a real safety net
the moment a fourth tab source could ever produce zero tabs. Contrast with
`OnboardingGate:210`, whose deletion is provably inert.

One process note, stated because it was unexplained rather than because it matters:
the first full-suite run with fault 20 in place reported `1 failed | 244 passed`
with **zero** test failures (a suite-level error), and the immediate re-run passed
cleanly including `ContextPanel`'s own 39 tests. I could not reproduce it, and it
does not affect the conclusion — but a once-off file-level failure with no failing
test is the signature of a parse/transform error in *some* file, so it is recorded
here rather than quietly dropped.

#### Round 11 — breaking an invariant found a second unguarded behaviour

`OnboardingGate.tsx:212` held the last multi-arm cluster
(`find(…) ?? recommendedModels[0] ?? null`, counts `[2, 0, 0]` — only the `find`
result is ever used). The waiver argued the list is frozen at ≥2 entries while
`selecting` is true and the selection is always a key taken from that same list.
Round 10's technique applies directly, because the invariant is *held by a line*:

| # | file | injected fault | outcome |
|---|---|---|---|
| 22 | `layout/OnboardingGate.tsx` | seed a selection that is **not** in the recommended list: `setSelectedModelId(modelKey(recommendedModels[0]!))` → `setSelectedModelId("not-a-real-key")` | **0 tests fail** (24/24 green) — the invariant was real but *unguarded* — while coverage shows `L212 arm 1` going `0 → 1`, proving the arm is reachable exactly when the invariant breaks |

Two conclusions from one mutation:

1. **The waiver is correct.** The arm is dead only because of the seed on L240,
   not by construction — the same characterisation as the `featureItems` cluster.
2. **A real test gap existed.** A key not in the list leaves the picker with **no
   card highlighted** (the active card is styled `bg-accent-soft`), and confirming
   silently falls through to `recommendedModels[0]`. No test covered the
   pre-selected default before this round.

So `OnboardingGate.test.tsx` gained *"pre-selects the first recommended model, and
confirms it when the user does not choose"*, which asserts exactly one card carries
`bg-accent-soft` (and it is the first) and that confirming applies `future/first`.
Re-running fault 22 against the **new** test kills it with
`expected [] to deeply equal [ 'Model first' ]` — the empty highlighted set. That
makes this the second round (after round 6's nav set) where attacking a waiver
yielded a test instead of an excuse.

#### Round 12 — auditing the plan's named state dimensions found one more

Two of the plan's named component-test assertion targets had not been audited:
**loading/empty/error states** and **error boundaries**. Both were swept the same
way as accessibility (render-vs-assert).

| # | file | injected fault | outcome |
|---|---|---|---|
| 23 | `layout/OnboardingGate.tsx` | collapse the loading placeholder: `{env.loading ? "..." : "custom"}` → `{env.loading ? "custom" : "custom"}` | **killed** by the new test — `expected 'custom' to be '...'` |

The audit's other results, kept because negative findings are evidence too:

* **Error boundaries: none exist** in `src/components` or `src/app` (no
  `ErrorBoundary`, `componentDidCatch` or `getDerivedStateFromError`), so that
  named target is `N/A` here — recorded rather than silently omitted, the same way
  the `#[cfg]` blind spot is handled on the Rust side.
* Four of five loading/empty/error flags were **comments, not states**
  (`ActivityRail`'s skills note, `useAppSettings`' error prose, `useThreadStore`'s
  comment, and `ThreadListItem`'s "empty placeholder", whose observable contract
  — no unread indicator for a cancelled run — *is* already asserted). Worth
  stating because a word-matching audit over-reports; each was read before being
  believed or dismissed.
* The one real gap was a **sub-state**: the dev-environment switcher's `disabled`
  and its `custom` placeholder were both covered, but the **loading** state was
  not. While `getFutureEnvironment` is pending the placeholder reads `"..."` — not
  `"custom"`, which would wrongly imply the environment is already known — and the
  control is disabled so the user cannot pick against a stale option list. The
  harness needed a one-line change to hold the probe open
  (`getFutureEnvironment: () => mocks.envPending ?? Promise.resolve(mocks.env)`),
  and `envPending` is now reset in `beforeEach` after a first attempt leaked it
  into the neighbouring tests — caught by two existing tests failing, not by
  guessing.

## 3. Dimension evidence (six-dimension bar)

**Conformance checked:** the six row labels below are the harness's own canonical
`DIMENSIONS` (`verify.py`), matched literally — `boundary`, `error-path`,
`concurrency`, `property`, `platform-cfg`, `serialization` — with **no extra rows
and none missing** (verified by parsing this table and diffing against the
constant). `verify.py dimensions` itself is repo-level: it needs
`docs/testing/dimension-matrix.md`, which is outside this task's write set, so this
section is this subtree's contribution to that matrix in the harness's vocabulary.
The plan's */component-test assertion targets* (interaction results, error
boundaries, loading/empty/error states, accessibility) are a separate axis and are
audited in §5 rather than folded into the six dimensions.

**Every sub-topic the contract names, adjudicated.** The contract defines each
dimension by an explicit list (e.g. `error-path` = *"IO failure, parse failure,
network error, timeout, permission denied, malformed peer input"*) and requires
each to be either evidenced or an honest `N/A` with a concrete reason — *"fabricating
a row is worse than an honest N/A"*. Every sub-topic is therefore closed out below,
including the ones this subtree legitimately cannot exercise:

| dimension | sub-topic | disposition in this subtree |
|---|---|---|
| boundary | empty | `DiffView` empty diff (single placeholder row); `AppShell` no-thread/no-workspace with `threads: []`, `workspaces: []`; `ContextPanel` no-thread state |
| boundary | single element | `OnboardingGate` single recommended model auto-applies (no picker); one-thread workspace; `ActivityRailSelectionToolbar` `1 of 1` |
| boundary | max | left panel clamped at the 480 ceiling; `ActivityRailSelectionToolbar` `3 of 3` (select-all ⇒ deselect-all) |
| boundary | overflow / very large input | 512 KB-plus diff and a 4 KB-plus line (deferred layout) |
| boundary | Unicode + CJK | CJK/emoji passthrough in `DiffView`; 28-character CJK title truncated by **code points**, not code units |
| boundary | off-by-one | width thresholds asserted on **both sides** — *"at the 1280 threshold"* and *"one pixel below 1280"*, plus the 768 edge |
| error-path | IO failure | storage read/write failure survived (`useLeftPanelWidth`); settings write failure toasts and re-reads |
| error-path | parse failure | corrupt / non-array JSON ⇒ empty (`useCollapsedWorkspaces`) |
| error-path | network error | agent probe rejection ⇒ `unavailable`; IPC/`invoke` rejection paths |
| error-path | timeout | a probe stuck in `checking` past the budget ⇒ `startup_timeout` |
| error-path | permission denied | **`N/A`** — jsdom has no filesystem permissions and this subtree reads no files. The nearest real analogues are covered elsewhere: the sandbox approval tier (`useAppSettings`) and the storage-failure cases above |
| error-path | malformed peer input | a malformed skill-sync result is logged and swallowed; a probe returning an unexpected shape falls back to `probe_failed` |
| concurrency | concurrent access | double-invoked effects (StrictMode) and two overlapping `useAgentConnection` ticks |
| concurrency | cancellation | unmount before a probe/import/fetch resolves never re-arms or writes |
| concurrency | ordering / races | superseded writes, generations, reloads and balance fetches all asserted **not** to clobber newer state |
| concurrency | lock behaviour | **`N/A`** — the subtree contains no locks, mutexes or shared mutable state; coordination between overlapping operations is by generation identity, which *is* tested |
| concurrency | shutdown | unmount cleanup: timers cleared, window listeners removed (asserted, incl. that no further poll is armed) |
| property | invariants | `DiffView` line numbering (`add` advances only the new file, `delete` only the old, context both); the divider's `min ≤ now ≤ max` |
| property | round-trips | `localStorage` write → read for collapsed workspaces, left-panel width, last-used model/thinking |
| property | table-driven over an input space | **9** `it.each` tables, incl. width thresholds, all nine `FileKind`s, the (`selected`, `total`) matrix, the scrollbar `height × visible` matrix |
| property | randomized | **`N/A` by design** — the repo's style is deterministic table-driven tests and no property-based-testing library is in this project's dependency set (`desktop/package.json` is outside this task's write set) |
| platform-cfg | OS `#[cfg]` branches | **`N/A`** for cfg-gating (jsdom-only TypeScript). The one platform-dependent UI decision is covered across the `isMacOS × fullscreen` axes |
| platform-cfg | path separators | **`N/A`** — paths here are opaque strings passed through to children and backends; separator normalisation lives in `integrations/storage/*` and the Rust side, outside this subtree |
| platform-cfg | case sensitivity | **`N/A`** — same reason; no path is compared or matched in this subtree |
| serialization | encode/decode round-trip | `localStorage` round-trips listed above; `DiffView` parses serialized diff text into its row model |
| serialization | legacy payloads | a stored model/thinking pair is re-resolved against the **live** catalogue; a legacy selection is qualified before reaching the send pipeline |
| serialization | schema evolution | **`N/A` for these payloads** — the persisted values are a string array (`useCollapsedWorkspaces`) and scalars, not versioned object schemas |
| serialization | unknown fields | **`N/A`** — extra object fields cannot occur in a string array or scalar; the *malformed entry* cases that can occur (non-string entries, corrupt JSON) **are** covered |

Four sub-topics are `N/A` with a stated reason and the rest carry named evidence, so
no row is fabricated and none is left unexplained.

| dimension | evidence (file :: case) |
|---|---|
| `boundary` | `AppShell` :: **no active thread and no active workspace** (the fresh-install / just-deleted state) — shell still renders, rail gets a null id, approvals scope to `null` not a stale id, and all four `activeThread?.id ?? …` handlers (add workspace, pin workspace group, approval decision, agent activity) refresh the whole store with `undefined`; also the resize divider's **non-arrow key** (no nudge) and a **non-connected** agent connection (no skills refresh, no forced revalidation); `ContextPanel` :: `activeWorkspace: null` passes `rootPath: null` to the file tree and `workspacePath: null` to the artifacts panel; `dialogs` :: **dismissed while an edit is in flight** (dismiss + late change in one `act`) for the rename input, the batch-delete file toggle and the workspace rename input; `OnboardingGate` :: **localized model descriptions** incl. a blank preferred locale falling back to the other, and a model with none at all rendering without a caption; `DiffView` :: empty diff (single placeholder row), blank context line, 4 KB-plus line (deferred layout), 512 KB-plus diff, CJK/emoji passthrough, two-hunk numbering; `useRailSelection` :: empty selection makes delete a no-op and keeps selection mode open, pinned rows excluded from select-all, **non-Escape keys** leave selection mode running; `ActivityRailAccountFooter` :: unknown balance renders `—`, empty local-part falls back, address without `@`; `useNewConversation` :: 28-character CJK title truncation counted in code points; `useLeftPanelWidth` :: stored width clamped at the 480 ceiling and at the window floor; `ActivityRail.nav` :: chat/workspace sections collapse independently with one row of each |
| `error-path` | `flows.test` :: rename/delete write failures keep the dialog open with the server message; unreadable image toast + thread never created; thread-create failure toast + rethrow; approval reload failure keeps the previous queue; `hooksMisc` :: malformed skill-sync result is swallowed; failed settings write toasts and reloads the authoritative snapshot; provider-config read failure ⇒ `useAppStartup` `failed`; auth verification failure retained without failing startup; `useFutureAccount.events` :: balance fetch failure clears, marks unavailable and re-verifies; **`useAgentStatus` :: a probe that fails once and then answers `ready` reports `unavailable` on the failing tick and is still admitted on a later one**; `useThreadStore.hook` :: a bootstrap failure that lands **after** unmount is not written into the dead hook |
| `concurrency` | `useAgentConnection.race` :: slow older tick cannot overwrite a newer catalog (two generation guards), superseded empty-catalog readiness probe dropped, newer failure keeps the last good catalog, unchanged catalog keeps array identity; `flows.test` :: a **second workspace-rename confirm while the first is in flight is ignored** (exactly one store call) — this was previously misattributed here to `useModelSelection`, which has no in-flight guard at all; `flows.test` :: stale rename/generate/cleanup results and a stale pending-prompt id are dropped; `useFutureAccount.events` :: balance success *and* balance failure both dropped after a credential change; `ThreadListItem.actions` :: outside pointerdown and Escape ordering; `useWindowWidth`/`useLeftPanelWidth` :: listener and drag cleanup on unmount; `overlayStack` :: only the top layer answers `isTop()`; `useContextData` :: a refresh superseded while parked in its git probe is abandoned, **and** one that is still current carries on; `useAgentStatus` :: unmounting before the first probe settles must not re-arm the poll (asserted by spying on `setTimeout` and requiring zero further schedules); `OnboardingGate` :: the init flow is entered exactly once across re-renders, the second Get-started activation during an in-flight write is rejected, the finalize effect does not re-pick after a catalogue blip; `AppShell` :: stop-remote fires once per signed-out transition, reauthentication retries the persisted pairing only for `checking → authenticated` with a pair id; `useThreadDialogs` :: a generation **failure** landing after the dialog was dismissed must not resurrect it; `useAppStartup` :: the `agentReady` flip mid-flight is asserted through the settled snapshot rather than the effect's internal cancelled flag; `ToastHost` :: per-toast timers stay independent; `useAppSettings` :: two edits in quick succession where the older write's snapshot arrives *after* the newer one (and separately, where the older write *fails* then) must not roll the newer optimistic state back or refetch; a window-focus reload superseded by an edit must not apply, while an unsuperseded reload must; and the sandbox fallback must persist exactly once and notify once however many times the effect re-runs while its write is held open |
| `property` | `DiffView` :: table-driven row kinds and line-numbering invariants (`add` advances only the new file, `delete` only the old, context advances both); `FileTypeIcon` :: table over all nine `FileKind` values; `uiExtra` :: `Switch` reports the negation of the current state, `Dialog` is inert while closed; `FloatingScrollbar` :: the four `height > 0` × `visible` combinations as a table; `ActivityRailSelectionToolbar` :: the (`selected`, `total`) matrix incl. `0 of 0` reading as "nothing selected" rather than "all selected"; `useModelSelection.actions` :: `modelsEmptyReason` is exactly one of `undefined`/`all_disabled`/`no_models`; `ThreadListItem.actions` :: run-indicator precedence (local status wins over streaming, cancelled never reads as unread) |
| `platform-cfg` | `N/A` for OS branching — this subtree is jsdom-only TypeScript. The one platform-dependent UI decision is covered explicitly: `LeftPanelTitlebarToggle` renders the macOS traffic-light inset as a 2×2 `isMacOS` × fullscreen table (mocked at the `lib/platform` boundary), and `ActivityRail` is rendered across the macOS/other × windowed/fullscreen axes with `useIsFullscreen` mocked. The `windows-red-baseline`/`blindspot` checks are Rust-side and do not apply to this group. |
| `serialization` | `useCollapsedWorkspaces` :: corrupt JSON ⇒ empty, non-array JSON ⇒ empty, non-string entries dropped, toggle → `localStorage` round-trip → reload restores; `useLeftPanelWidth` :: stored width parsed and clamped, corrupt value falls back to the viewport default, empty string clamps up to the floor, read/write storage failure survived; `useModelSelection.actions` :: last-used model / thinking level round-trip through `localStorage`; `DiffView` :: serialized diff text → row model (`---`/`+++`/`-- ` classification, repeated identical lines keyed distinctly) |

## 4. Waivers (category + reason, per line)

| file | line(s) | category | reason |
|---|---|---|---|
| `desktop/src/main.tsx` | n/a | **none (covered)** | Covered instead of waived: `desktop/src/app/main.test.tsx` imports the entry for its side effect against a document that owns `#root`, and asserts the app is mounted **into that element** (not a detached node) after flushing React's concurrent render. The earlier waiver claimed this line was un-hostable; that was wrong — a jsdom document can be given a `#root`, which is the only thing the entry needs. The test lives in `src/app/` rather than beside `main.tsx` because the write set covers `desktop/src/app/` while `desktop/src/main.tsx` is enumerated as a *file*, so a sibling `desktop/src/main.test.tsx` would be out of scope and a test one directory down is not. A second case removes `#root` and asserts the import **rejects** rather than mounting nowhere (the entry casts the lookup, so a broken `index.html` would otherwise fail silently). Coverage: `main.tsx` line 7 executes twice; the file is fully covered — this waiver is **gone**. |
| `desktop/src/app/App.tsx` | n/a | none (covered) | Covered instead of waived: `app/App.test.tsx` asserts it mounts `AppShell` with no props of its own — 100 % lines and branches. |
| `desktop/src/components/layout/hooks/useAgentStatus.ts` | n/a | none (covered) | Covered instead of waived: the `catch` arm that synthesizes `phase: "unavailable"` is exercised by *"keeps polling after a probe failure so a later recovery is picked up"* (mock rejected **once**, then resolving). The earlier waiver claimed no assertion could accompany this arm; that was wrong. |
| `desktop/src/components/layout/hooks/useAgentStatus.ts` | 54 | `unreachable-by-construction` | Defensive `if (cancelled) return;` re-check inside `commit`. The only caller that can reach `commit` after cancellation is the splash-delay `setTimeout(commit, …)`, and the effect cleanup clears exactly that timer, so the callback cannot run post-cancel. Kept as protection against a future second scheduler. **Delete-and-tested (§2b fault 19): removing it leaves the whole project suite green, confirming no test can distinguish it — but note this is *unreachable-because-of-a-paired-clear*, not redundancy: it becomes load-bearing again if that `clearTimeout` is ever removed.** |
| `desktop/src/components/layout/hooks/useAppSettings.ts` | 132 | `unreachable-by-construction` | `if (sandboxFallbackRef.current) return;` guard, with a failed counterexample attempt recorded above (`[0, 4]` under a held write plus StrictMode double-invoke). Kept as a re-entrancy guard. |
| `desktop/src/components/layout/ActivityRailMenus.tsx` | 256 | `unreachable-by-construction` | `if (items.length === 0) return;` in `handleMenuKeyDown`. No `[role=menuitem]` is ever `disabled` (so `:not(:disabled)` cannot filter anything out) and all three menus render their items unconditionally — `ChatSectionMenu` exactly one, `ThreadItemMenu` ≥2, `WorkspaceHeaderMenu` ≥4. Kept so a future disabled menu cannot divide-by-zero the modulo focus wrap. |
| `desktop/src/components/layout/ActivityRail.tsx` | 364, 369, 658, 659, 665 | `unreachable-by-construction` | The two `featureItems.map(…)` bodies (expanded and collapsed layouts). `featureItems` is the module-level `const featureItems: Array<…> = []` (line 92) and is never mutated, so `featureItems.length > 0` is always false and neither map body runs. This is scaffolding for future rail sections; deleting it would change the intended shape of the nav. |
| `desktop/src/components/layout/ActivityRail.tsx` | 703 | `unreachable-by-construction` | `sortThreads`'s `return a.status === "active" ? -1 : 1;` — its single call site is `sortThreads(threads.filter(thread => thread.status === "active"))`, so the comparator's `: 1` arm can never be selected. The invariant is now **locked by a guard test** (round 3, fault 11) so a future call-site change fails a test rather than silently rotting this waiver. |
| `desktop/src/components/layout/AppShell.tsx` | 564 | `unreachable-by-construction` | `if (showLeftPanel) return;` in `handlePreviewLeftPanel`. Both call sites (L640, L649/650) sit inside `!showLeftPanel` blocks and the node is removed in the same commit that flips it, so the early return cannot be taken. **Backed by a failed counterexample** (not reasoning alone): the one route that could have reached it is a hover delivered to a node detached by the layout flipping back to expanded, so `AppShell.test.tsx` captures the edge while collapsed, expands (detaching it), dispatches `mouseover` on the detached node, collapses again and asserts no preview appears — and the arm still reads `[0, 8]`. The test is kept because it also encodes real behaviour: a stale edge must not arm the preview for later. |
| `desktop/src/components/layout/OnboardingGate.tsx` | 211 | `unreachable-by-construction` | `if (starting) return;` at the top of `handleStart`. Its **only** caller is the Get-started button (L373, `onClick={() => void handleStart()}`), and that button carries `disabled={starting \|\| selectedModelId == null}` (L371) — so while `starting` is true the activation never dispatches and the guard cannot run. **Proven by mutation, not argument** (§2b round 8): deleting the guard leaves **24/24** `OnboardingGate` tests green, i.e. the guard is not protecting anything the current code reaches. Worth knowing for review: it is therefore *provably redundant today* — and precisely because of that it is the one guard whose removal is behaviour-neutral, while it would become load-bearing again if the `disabled` clause were ever relaxed (e.g. to allow retry). The sibling test that asserts only one `set_default_model` is issued still passes without it, so that test measures the `disabled` mechanism rather than the guard. |
| `desktop/src/components/layout/hooks/useThreadDialogs.ts` | n/a | none (covered, one dead arm) | Line 66's `: current` arm is dead-by-construction as argued in §1; it is a *branch*, not a line, so it does not appear in the line ledger. It is listed in Appendix A. |
| `desktop/src/components/ui/overlayStack.ts` | n/a | none (covered, one dead arm) | Line 31's `index === -1` arm is `unreachable-by-construction` with a failed counterexample (see §1). Branch-only; listed in Appendix A. |
| `desktop/src/components/layout/ActivityRailSelectionToolbar.tsx` | n/a | none (covered, one dead arm) | `if (checkboxRef.current)`'s false arm — React attaches refs before effects run. Branch-only; listed in Appendix A. |

All other files in the group are at 100 % lines, including `ThreadListItem`,
`WorkspaceDialogs`, `AppShellDialogs`, `ActivityRailAccountFooter`,
`ActivityRailMenus` (except the waived guard), `DiffView`, `ToastHost`,
`ContextPanel`, `OnboardingGate`, `App` and every hook under
`desktop/src/components/layout/hooks/`.

### 4b. Completion statement

**Required evidence tokens — present verbatim** (the operating contract states:
*"Every handoff's evidence must contain these three literal tokens"*):

| token | how this document satisfies it |
|---|---|
| **`lines-100-or-waived`** | 11 uncovered lines in 6 files; **every one carries a category and a reason** in §4, reconciled line-by-line against `coverage-final.json` (§5). Nothing is left unevaluated. |
| **`dimensions`** | §3 is the six-dimension matrix using the harness's own `DIMENSIONS` names (`boundary`, `error-path`, `concurrency`, `property`, `platform-cfg`, `serialization`), each row pointing at the specific cases that make it real; §5 additionally audits the four component-test assertion targets the plan names. |
| **`weak-tests-fixed`** | §2 lists the weak tests found and repaired — including one that *passed for the wrong reason* — plus the class-wide sweeps, and §5 records **0 hits** under the harness's own `WEAK_PATTERNS`/`DISABLED_PATTERNS`. |

Nothing in this subtree is left deliberately unwaived. Every one of the 11
uncovered lines is in the table above with a category and a reason, and the
report's file list reconciles exactly with this table (6 files, same line
numbers — verified programmatically against `coverage-final.json`).

Remaining outside this task's control:

1. The literal `<agent-id>` in this todo's verification string (see §0). The gate
   itself exits 0 against the report that exists.
2. `verify.py weak` needs the repo-level `docs/testing/weak-test-audit.md`, which
   is outside the write set.

Item 3 of the previous revision — *"`main.tsx`'s single line needs a sibling
`desktop/src/main.test.tsx`"* — **no longer applies**: it is covered by
`desktop/src/app/main.test.tsx`, which sits inside the write set. See §4.

## 5. Document self-audit — every claim in this file checked against the artefacts

A coverage/waiver doc is only worth what a reviewer can verify, and a rebuilt
document is exactly where an invented claim can hide. So the claims in this file
are *checked*, not asserted. Each audit is a small script run against the real
test files and the real report; all four are reproducible.

| audit | what was checked | result |
|---|---|---|
| **Test files exist** | every `*.test.ts(x)` path named anywhere in this doc | 24/24 exist on disk |
| **Test names are real** | the 15 test titles quoted as "killed by" / "covered by" evidence, against the 681 real `it()`/`describe()` titles in the subtree | 14 exact; **1 paraphrased and corrected** (`"only the topmost Overlay closes on Escape"` → `"lets only the topmost Overlay close on Escape"`) |
| **§3 dimension claims** | 40 specific, falsifiable claims (one per behaviour, incl. the four accessibility ones) located in the tests that are supposed to carry them | **40/40 verified on re-run** — the count grew from 29 as accessibility/state claims were added, each verified as it landed. 4 initially reported MISS — **3 were bugs in the checker** (a regex expecting the literal word `failed` where the test says *"fails startup"*; a filter looking for the CJK-truncation test in a `useNewConversation`-named file when it lives in `flows.test.tsx`; `not.toHaveBeenCalled` mistyped as `not toHaveBeenCalled`) and **1 was a real error in this doc, fixed below** |
| **§2 coverage lists** | 25 files × the symbols this doc says each one covers (58 symbols) | **58/58 present in their claimed file** on re-run. Two initial MISSes were bugs in the checker — an `app/` path-prefix doubling (`src/app/app/…`), and matching the hook name `dropUp` where the test asserts the *outcome* (`bottom-7`/`top-7` classes); the doc's phrasing was already accurate |
| **Harness's own weak/disabled patterns** | `verify.py`'s `WEAK_PATTERNS` (tautological `expect(true)`/`assert!(true)`, `expect.hasAssertions`) and `DISABLED_PATTERNS` (`it.skip`/`test.skip`/`describe.skip`, `xit`/`xdescribe`/`xtest`, `#[ignore]`) applied to all 52 test files in this subtree | **0 hits for each** — the subtree would clear `verify.py weak` and `verify.py skipped` on the harness's criteria, not just mine. (Those two checks fail repo-wide only because their audit docs — `weak-test-audit.md`, `disabled-test-audit.md` — do not exist; both are repo-level files outside this task's write set.) |
| **Report completeness** | all 62 non-test source files under `src/components` + `src/app` + `src/main.tsx` vs. the 62 files the report contains | **exact match, 0 missing, 0 extra** — so no file in this subtree is silently absent from the measurement |
| **Accessibility contracts** | every `aria-*`/`role`/`tabIndex` the subtree *renders* vs. whether any test asserts it | 3 gaps found and closed — `aria-valuemax`, `aria-controls`, `tabIndex` were rendered but never asserted. Now: the AppShell separator is asserted as a **coherent** value range (`min ≤ now ≤ max`, plus the hook-derived ceiling and focusability), the ContextPanel divider's focusability is asserted, and the account trigger's `aria-controls` is asserted to resolve to the real `role="menu"` element once open. The task text names 可访问性/accessibility as a required assertion target, so these were a real hole rather than a nicety |
| **Loading / empty / error states** | every component that renders a loading/empty/error marker vs. whether its own tests assert one | 5 candidates flagged, **4 dismissed on inspection** as comments rather than states (`ActivityRail`'s skills note, `ThreadListItem`'s empty placeholder — whose observable contract *is* asserted by *"never surfaces a cancelled run as unread"* — `useAppSettings`' error prose, `useThreadStore`'s comment) and **1 closed**: the dev-environment switcher's **loading** sub-state (`OnboardingGate:317`). See §2b fault 23 |
| **Determinism / order-independence** | the whole subtree run **10 times** with `--retry=0` (so a flake cannot be masked): 3 sequential + 4 file-order shuffles + 3 shuffles of **files *and* tests** (`--sequence.shuffle.files --sequence.shuffle.tests`, the stronger form) | 3 sequential runs identical (**52 files / 643 tests** every time). Of the first 4 file shuffles, **3 passed and 1 failed** — that failure is the finding: one order-dependent test (§2). After the fix, the previously-failing seed plus 6 further shuffled orders (3 file-level, 3 file-and-test-level) all pass. This also localises the flakes seen earlier in this session: they were in **peers'** files (`features/terminal`, `features/runs`, `features/agent`), not in this subtree |
| **Hoisted-state sweep** | the same defect *class* as §2's order-dependence: for all **22** test files that use `vi.hoisted`, check whether mutable captures are written and read without being reset in `beforeEach` | **1 candidate** — `AppShell.test.tsx`, i.e. exactly the one already fixed above. The other 21 reset their captures or only ever write them. (The sweep flagged the fixed file as a leak because its reset is a `for (const key of Object.keys(children))` loop rather than a direct assignment; the shuffle result is the ground truth, and that is green.) |
| **Worker-count sensitivity** | the subtree run at `--maxWorkers=1` (fully serial), `2` (every measurement so far) and `6` (more parallel than usual), each with `--retry=0` | **52 files / 643 tests green at all three.** Serial execution rules out interleaving-dependent coupling that a parallel run can hide; 6 workers rules out ordering assumptions that only hold at low parallelism. The high-parallelism run also emitted **no `open handle`, unhandled-rejection or teardown warnings**, so my tests leak no timers or listeners — a common flake source that would otherwise show up as an unrelated failure later in the run. |
| **Per-file isolation** | each of the 52 test files run as the **only** file, in its own process | **52/52 pass, 0 failures, and the per-file counts sum to exactly 643** — the same total as the whole-subtree run. That closes the last determinism question: no file's suite passes only because a sibling file was loaded first (a global polyfill, a module-level mock registration, an import side effect). Vitest already gives each file its own worker, so the sharper reading is: this rules out *import-side-effect* dependence, and the exact sum cross-validates the 643 figure two ways. |
| **Incidental-only coverage** | for all 62 non-test source files, is there a test that **imports it directly**? A file can reach 100 % lines by accident — covered by another component's test rendering through it — while nothing asserts its own contract. This is the `overlayStack.ts` case found earlier in the task (no dedicated test until one was written), and it is invisible to a coverage percentage. | **0 orphans — every one of the 62 modules is directly imported by at least one test.** Carries a **positive control** (a scan that reports "nothing found" is only meaningful if it *can* find something): a synthetic unimported path is reported (`importers=0`) and a known-imported one is not (`importers=2`), asserted in-script. |
| **Coverage attribution** | which of the 52 test files in the group produce the measured 99.4177 %? Re-measured with three populations: all 52, only this session's 25, and 46 (excluding the 6 untracked pre-session files). | **This session's own tests account for 84.3833 % (1594/1889); 284 covered lines come from the other 27 files**, and **14 source files reach 100 % only because of tests this task did not write**. Found because last turn's disclosure (6 undocumented files) raised the obvious follow-on: *if they are not mine, whose number is this?* Written up as §1b — including the **operational consequence**: the 6 pre-session files are **untracked**, so losing them drops the group to **92.3769 % with 12 uncovered files (gate fails)**, six of which are not in §4's ledger. The checkpoint commit must include them. |
| **Reverse-direction documentation** | the converse of "every file the doc names exists": is every test file **in the group's directories** named by the doc? An undocumented test is an omission a reviewer cannot spot. | **6 undocumented files found** — all authored by an **earlier session**, not this task (attribution by mtime: they carry 01:41–01:56 versus this session id `20260926-015904` = 01:59:04 and this task's earliest file at 02:12). §2 now states explicitly that its table is *this task's* additions, not the group's full inventory, and names the six — because **their tests run in the same suite and contribute to the measured 99.4177 %**. Their existence also explains why §2's count (33 files) and the group's test-file count (52) differ. |
| **Flake sources: clock, randomness, timers** | scanned all 52 test files for the classic non-determinism inputs — `Date.now`, `Math.random`, `new Date()`, `performance.now`, and real (non-faked) timer delays — **with a positive control**, because a scan reporting zero must be shown capable of reporting non-zero (the control `expect(` matched 1695 times, `it(` 598). | **0 occurrences of any clock or randomness API.** `useFakeTimers` in **12** files; the only file touching timer APIs without them is `railMenus.test.tsx`, and its three uses are **zero-delay** (`setTimeout(…, 0)` as an async mock, and `await new Promise(r => setTimeout(r, 0))` as a macrotask flush) — deterministic drains, not elapsed-time dependence. **One genuine wall-clock dependency, disclosed:** `main.test.tsx` sets a **30 s** per-test budget (`COLD_IMPORT_MS`) because the cold import of the entry point pulls react-dom and the i18n bundle, which exceeds vitest's 5 s default on this contended box. That is a real time budget, not an assertion — and it is the only one in the subtree. **A false negative in the first pass is worth noting:** the initial literal-pattern scan missed `COLD_IMPORT_MS` because the budget is a *named constant*, not a numeric literal; the scan was extended to resolve such constants before this row was written. |
| **Waiver reasons, not just line numbers** | the gate only checks that a *file+line* appears near a category word — it cannot tell whether the **reason still describes the code**. A peer edit could drift a line and leave a waiver that passes while pointing at the wrong construct. So each waived line was checked twice: the source text at that line, **and its enclosing construct** (the guard/function the reason names). | **11/11 lines and 10/10 enclosing constructs verified**, with **0 undocumented uncovered lines** — e.g. `useAgentStatus.ts:54` really is inside `if (cancelled)` within `const commit`, `useAppSettings.ts:132` inside `if (sandboxFallbackRef.current)`, `ActivityRailMenus.tsx:256` inside `if (items.length === 0)` within `handleMenuKeyDown`, `AppShell.tsx:564` inside `if (showLeftPanel)` within `handlePreviewLeftPanel`, `OnboardingGate.tsx:211` inside `if (starting)` within `handleStart`, and `ActivityRail.tsx:703` inside `sortThreads`. The waiver ledger now reconciles at the *semantic* level, not merely structurally. |
| **Dimension sub-topics** | the contract defines each dimension by an explicit sub-topic list (28 items in total, e.g. `error-path` = *"IO failure, parse failure, network error, timeout, permission denied, malformed peer input"*) and requires each to be evidenced or an honest `N/A` with a reason — *"fabricating a row is worse than an honest N/A"*. Checked whether §3 closes each out. | §3 originally gave dense prose per dimension but **did not resolve the contract's sub-topic list**, so a reviewer could not tell a covered sub-topic from an overlooked one. Added an explicit adjudication table: **28/28 sub-topics named**, of which **8 are `N/A` with concrete reasons** (permission denied; lock behaviour; randomized; OS `#[cfg]`; path separators; case sensitivity; schema evolution; unknown fields) and the rest carry named evidence. The four candidate *gaps* this surfaced were each investigated before disposition: `timeout` turned out **covered** (a test asserts `startup_timeout`), while `permission denied`, `lock`, `path separators` and `case sensitivity` are genuine `N/A` for a jsdom-only UI subtree — and the first sweep that called them "covered" was my own false positive (`readonly: false` fixture data matching *permission*, and `role="separator"` matching *path separator*) |
| **Contract compliance** | re-read `.future/cov100/plan.md` and checked this deliverable against each requirement, rather than working from memory of it | **1 real gap found and fixed:** the contract's *"JS/TS: report BOTH line and statement coverage"* section requires both numbers, and this doc reported only lines. §1 now carries statement coverage (`99.4557 % (2010/2021)`, 11 uncovered in 6 files) plus the gate's own sanity bound (`uncovered statements - uncovered lines > max(20, uncovered lines)` → here `11 - 11 = 0`). Also confirmed the three required evidence tokens are present, and that the three aggregate docs the contract lists (`dimension-matrix.md`, `waiver-ledger.md`, `weak-test-audit.md`) are repo-level files outside this task's write set — §3 and this section are this subtree's contribution to them |
| **Statement vs line sets** | are the uncovered *statement* items and the uncovered *line* items the same set? If they differ, the line metric is hiding untested statements — the exact failure the contract warns about. | **Identical: 11 (file, line) pairs in both sets, 0 differences either way.** So §4's ledger doubles as the statement work list, and no untested guard is concealed behind a credited line here. |
| **Report internal consistency** | `verify.py` reads `coverage-summary.json`; the waiver ledger is reconciled against the raw `coverage-final.json`. If those two ever disagreed, the gate and the ledger would be validating *different measurements* and neither would notice. So per-file line and branch counts were recomputed from the raw data and compared to the summary, for all 62 in-scope files. | **0 mismatches across 62 files**, and the recomputation from raw data reproduces the headline figures *exactly* (`1878/1889` lines, `1382/1401` branches) — the same numbers the gate prints. Both shipped copies are also byte-identical (SHA-256) under their two directory names. |
| **Error boundaries** | any `ErrorBoundary` / `componentDidCatch` / `getDerivedStateFromError` in `src/components` + `src/app` | **none exist** — so this named assertion target is `N/A` for this subtree, and (as with the `#[cfg]` blind spot) that is recorded rather than silently omitted |
| **File paths exist** | every `desktop/src/…​.ts(x)` path named anywhere in this doc | 15/16 exist; **1 real typo fixed** (the toolbar row said `components/ui/ActivityRailSelectionToolbar.tsx` with a `(layout/)` note — the file is in `layout/`, full stop). The one remaining miss is *intentional*: `desktop/src/main.test.tsx` is named only as the fix that this draft no longer needs (see §4) |
| **Weak-test shapes** | all 52 test files: snapshots, `skip`/`only`/`todo`/`xit`, assertion-free `it()` bodies, constant tautologies (`expect(true).toBe(true)`, identical operands), `expect.assertions(0)` | 0 snapshots, 0 skips, **2 assertion-free tests found and fixed** (§2), 0 tautologies |

### Waivers retired rather than defended

The `main.tsx` line was waived for several revisions as
`unreachable-in-this-environment`, on the argument that the entry point can only
run in a real browser document. That argument was **wrong**, and the §3 audit is
what exposed it: a jsdom document is not obliged to be empty, so a test can give
it the one element the entry point looks up.

`desktop/src/app/main.test.tsx` now imports the entry for its side effect,
flushes React's concurrent render, and asserts the app is mounted into `#root` —
plus a negative case where `#root` is absent and the import must reject. Line 7
executes twice and the file is fully covered, so the waiver is gone rather than
rewritten. The subtlety that had blocked this was placement: the write set
enumerates `desktop/src/main.tsx` as a **file**, so a sibling test was out of
scope — but `desktop/src/app/` is in scope, and `../main` resolves to the same
module.

That is now the third claim in this document proven wrong by checking it
(`overlayStack`'s branch, `useModelSelection`'s phantom in-flight guard, and this
waiver), which is the whole argument for §5 existing.

### Error found by the §3 audit, and corrected

The concurrency row credited `useModelSelection.actions` with *"second confirm while
a write is in flight is ignored"*. That is **wrong**: `useModelSelection.ts` has no
pending/in-flight guard at all, and the test that does this work is
`flows.test.tsx` *"ignores a second confirm while the first is in flight"*, which
exercises **`useWorkspaceDialogs.confirmRename`** (asserting `renameWorkspace` is
called exactly once). The row now names `flows.test` and records the correction
inline. This is the second misattribution this doc has carried (the first was the
`overlayStack` branch), both found the same way — by locating the claimed evidence
instead of trusting the prose.

### Why this section exists

Both errors above were *prose* errors that every automated gate passed: `verify.py`
checks that uncovered files appear near a category keyword, not that a sentence is
true. The rebuild after the truncation incident made this risk acute, because there
the prose was rewritten from memory while the numbers were re-derived. The audits
are the compensating control, and they are cheap enough to re-run after any further
edit to this file.

## 6. Findings (no product bug fixed; five dead guards, one a11y note, one tooling list)

The dead guards split into **three kinds**, and the difference decides whether one
could ever be deleted safely:

1. **`OnboardingGate.tsx:211`** (`if (starting) return;`) — unreachable **and
   provably redundant**, proven by mutation (§2b round 8): removing it leaves all
   24 gate tests green, and its only caller is the button that is already
   `disabled` while `starting`. Nothing in the component depends on it, so this is
   the one guard in the group whose removal is *provably* behaviour-neutral. It is
   still waived rather than deleted, because relaxing that `disabled` clause (e.g.
   to allow a retry) would make it load-bearing again.
2. **`useContextData.ts:211`** and **`useAgentStatus.ts:54`** — unreachable
   **because of a paired clear**, not redundant. Each guard sits behind a timer
   that the same effect's cleanup (or the neighbouring `clearTimeout`) prevents
   from firing after cancellation. Delete-and-tested (§2b faults 18-19): the whole
   project suite stays green with either guard removed, but that reflects the arm
   being unreachable — **removing the paired clear would make these guards
   load-bearing again**, so they are the opposite of safe-to-delete.
3. **`ContextPanel.tsx:172`** (`if (first)`) — unreachable **but protective**, not
   redundant. Delete-and-tested (§2b fault 20): 39/39 tests and the full suite
   stay green without it, because `tabs` is one of three non-empty module
   constants. But `tabs[0]!.value` on an empty list would **throw**, so this guard
   is a genuine safety net the moment a fourth tab source could yield zero tabs —
   the opposite of `OnboardingGate:210`, whose deletion is inert.
4. **`AppShell.tsx:564`** (`if (showLeftPanel) return;`) is unreachable
   from both call sites — and unlike the guards above it carries a *failed
   counterexample* (§4 and §2b round 5): a hover delivered to an edge node
   detached by the layout flipping back to expanded still does not reach the
   handler. Tested waiver, not an argued one. Same decision: waived, not deleted.

5. **Minor accessibility observation (not fixed, not a defect that changes behaviour):**
   `ActivityRailAccountFooter` renders `aria-controls={menuId}` on the account trigger
   *unconditionally*, but the element carrying that id exists **only while the menu
   is open**. So in the closed state the reference dangles. Strictly, ARIA expects
   `aria-controls` to identify an element that exists; in practice assistive tech
   tolerates this, and the trigger still conveys state correctly through
   `aria-expanded` + `aria-haspopup`, so the user-visible behaviour is right. It is
   recorded rather than fixed because the usual fix (always rendering the menu
   hidden) changes behaviour, which is outside a test-and-document task. The new
   test pins the part that matters — the id resolves to the real `role="menu"`
   element once open — and documents the closed-state observation.

6. **Toolchain pitfalls worth knowing** (each cost real debugging time here):
   * An un-awaited async `act()` poisons the *rest of the file* — later `act()`
     calls stop flushing, which looks like unrelated test failures.
   * JSX `<C {...props} x={next} />` did not apply the override under this
     transform; `createElement(C, { ...props, x: next })` did.
   * React derives `onMouseEnter`/`onMouseLeave` from `mouseover`/`mouseout`, so
     tests must dispatch those.
   * `IS_REACT_ACT_ENVIRONMENT = true` must be set in every jsdom test file that
     mounts with `act`.
   * Error-path tests for effect-driven probes must inject the failure *inside*
     the loop (reject once, then resolve), not as a rejected-only mock promise —
     vitest 4 reports the latter as a test-scoped unhandled error even when the
     hook handles it and the state assertion passes.
   * A never-settling promise inside a mounted hook deadlocks `act` (the test
     body completes, all markers print, assertions pass, and vitest still fails
     with `Hook timed out in 10000ms`).
   * A **parse error in any test file** aborts coverage writing for the whole
     run; a **killed run** writes no report. Check the report's `LastWriteTime`
     before believing "missing coverage-summary.json".
   * **`Get-Content` drops a line on this file**, so PowerShell's line numbers
     are off by one against the report's. `Get-Content AppShell.tsx` returns 816
     elements where the file has 817 lines, `Select-String` and Python agree on
     817, and v8's line numbers match the latter. This nearly caused a *correct*
     waiver (`AppShell.tsx:564` = `return;`, the guard's true arm) to be
     "corrected" into a wrong one. **Attribute line numbers from the coverage
     data / `Select-String` / Python — never from a `Get-Content` index.**
4. `npx tsc --noEmit` in `desktop/` is clean for every file under
   `src/components`, `src/app` and `src/main.tsx` (verified after the last edit).
   The repo-wide run still reports errors in other workers' in-flight test files
   under `src/features/*`; they are outside this task's write set and need their
   owners to fix them before the PR's typecheck can go green.

## Appendix A — complete uncovered-branch inventory (19 arms, 12 files)

Machine-generated from the report behind this doc (`branchMap` + `b`; one row per
zero-count arm), then classified by reading each expression in the source.
**These are deliberately not presented as line waivers.** The gate's contract is
on *lines* — met, all 11 uncovered lines are waived in §4 — and its own output
labels branch gaps *informational*. Every arm below is now **DEAD (verified)**:
the work list this appendix opened with (20 "coverable" arms) has been worked to
zero, and the 9 arms that v8 originally reported against line 0 have all been
source-mapped. There are no UNMAPPED and no UNTESTED arms left.

**Reconciliation for a reviewer** (the table is one row per *expression*, and two
expressions each have **two** uncovered arms, so rows ≠ arms):

| | count |
|---|---|
| table rows below | **17** |
| rows covering two arms each | **2** — `ActivityRail.tsx` 703 and `OnboardingGate.tsx` 212, both marked *"(both arms)"* |
| **arms therefore covered** | **19** |
| distinct files | **12** |

Recomputed from `coverage-final.json`, the report has exactly **19 zero-count arms
across 12 files**, distributed
`ActivityRail.tsx` 6 · `OnboardingGate.tsx` 3 · then 1 each in
`ActivityRailAccountFooter`, `ActivityRailMenus`, `ActivityRailSelectionToolbar`,
`AppShell`, `ContextPanel`, `useAgentStatus`, `useAppSettings`, `useContextData`,
`DiffView`, `overlayStack` — **matching this table exactly**.

| file | line | kind | expression | category |
|---|---|---|---|---|
| `layout/ActivityRail.tsx` | 362 | cond-expr | `featureItems.length > 0 ? …` | `unreachable-by-construction` — empty module constant |
| `layout/ActivityRail.tsx` | 413 | cond-expr | pinned row's `threadSelectionMode(thread)` | `unreachable-by-construction` — `isThreadInScope` excludes pinned; guard test added |
| `layout/ActivityRail.tsx` | 702 | if | `if (a.status !== b.status)` | `unreachable-by-construction` — call site filters to `active`; guard test added |
| `layout/ActivityRail.tsx` | 703 | cond-expr | `a.status === "active" ? -1 : 1` (both arms) | `unreachable-by-construction` — enclosing `if` never runs |
| `layout/ActivityRail.tsx` | 711 | binary-expr | `lastMessageAt ?? updatedAt ?? createdAt` (3rd arm) | `unreachable-by-construction` — the producer declares `updated_at: i64` non-optional (`desktop/src-tauri/src/store/threads.rs:28`) and always selects it |
| `layout/ActivityRailAccountFooter.tsx` | 111 | binary-expr | `(prefix[0] ?? "?")` | `unreachable-by-construction` — `prefix === ""` needs a falsy email, but this variant renders only for a truthy one |
| `layout/ActivityRailMenus.tsx` | 255 | if | `if (items.length === 0)` | `unreachable-by-construction` — no menu item is ever `disabled`, and all three menus render items |
| `layout/ActivityRailSelectionToolbar.tsx` | 23 | if | `if (checkboxRef.current)` | `unreachable-by-construction` — refs attach before effects |
| `layout/AppShell.tsx` | 563 | if | `if (showLeftPanel) return;` | `unreachable-by-construction` — both call sites gated; failed counterexample (§2b round 5) |
| `layout/ContextPanel.tsx` | 172 | if | `if (first)` before `onTabChange(first.value)` | `unreachable-by-construction` — `tabs` is one of three non-empty module constants (`gitTabs` 3, `fileTabs` 2, `pendingTabs` 2). **Delete-and-tested (§2b fault 20)**: removing the guard leaves 39/39 `ContextPanel` tests and the full suite green, confirming the arm cannot be reached. Note it is *protective*, not redundant — `tabs[0]!.value` on an empty list would throw, so the guard is a safety net if a fourth tab source ever produced zero tabs |
| `layout/OnboardingGate.tsx` | 210 | if | `if (starting) return;` | `unreachable-by-construction` — only caller is the `disabled` Get-started button; **proven by mutation** (§2b round 8) |
| `layout/OnboardingGate.tsx` | 212 | binary-expr | `recommendedModels.find(…) ?? recommendedModels[0]` (both arms) | `unreachable-by-construction` — `setLoadedModels` runs only in `runInit`, which resets `selecting`, so the list is frozen at ≥2 and the selection is always one of its keys. **Invariant-broken and re-tested (§2b round 11)**: seeding a key not in the list leaves 24/24 tests green while making the arm reachable (`0 → 1`), so the arm is dead *only* because of the L240 seed. The behaviour that seed protects (the picker pre-highlights the first model and confirms it) is now locked by a test that kills that mutation |
| `layout/hooks/useAgentStatus.ts` | 53 | if | `if (cancelled)` in `commit` | `unreachable-by-construction` — cleanup clears exactly that timer |
| `layout/hooks/useAppSettings.ts` | 131 | if | `if (sandboxFallbackRef.current)` | `unreachable-by-construction` — failed counterexample `[0, 4]` |
| `layout/hooks/useContextData.ts` | 211 | if | `if (!cancelled)` in the min-spinner timer | `unreachable-by-construction` — the timer is armed only when `cancelled` is false, and cleanup clears it; **delete-and-tested (§2b fault 18)**, so it is unreachable-because-of-a-paired-clear rather than redundant |
| `ui/DiffView.tsx` | 69 | binary-expr | `oldLineNumber ?? ""` | `unreachable-by-construction` — only reached for `delete` rows, and the row builder assigns `oldLineNumber = oldLine` on that branch. **Upgraded from argued to test-guarded (§2b fault 21)**: deleting that assignment fails 3 named line-numbering tests, so the invariant is locked and the fallback can only be reached by breaking it — which the suite catches |
| `ui/overlayStack.ts` | 31 | if | `if (index !== -1)` | `unreachable-by-construction` — single mutator per id; failed counterexample |

Two further arms are branch-only and already covered in the tables above:
`useThreadDialogs.ts:66` and `useAppSettings`/`useAgentStatus` siblings. Their
`IsCurrentRefresh`-style "live" sides were also covered this segment (see §2).

### How the work lists were exhausted

| shape | arms | how it was closed |
|---|---|---|
| `activeThread?.id ?? …` / no active thread | 5 (`AppShell` 207, 521, 534, 736, 706) | a shell test that renders with **no active thread**: asserts the shell still renders, the rail gets a null id, approvals are scoped to `null` rather than a stale thread, and all four handlers refresh the whole store |
| `activeWorkspace?.path ?? null` | 3 (`AppShell` 719, `ContextPanel` 464, 473) | the same shell test plus a ContextPanel test with `activeWorkspace: null`, asserting the file tree gets `rootPath: null` and the artifacts panel gets `workspacePath: null` |
| `selectedModelId \|\| defaultAgentModelId` | 1 (`AppShell` 502) | coach-conversation test with nothing selected |
| `restoredThread.mode === "workspace" ? …` | 1 (`AppShell` 548) | restoring an archived **chat** thread from the workspace section |
| dismissed-dialog updaters | 3 (`AppShellDialogs` 65, 138; `WorkspaceDialogs` 31) | one `act` that dismisses then fires the late edit, so the updater holds `null` |
| `current?.generation === generation` stale side | 1 (`useThreadDialogs` 68) | a generation **failure** landing after dismissal |
| `description ? <span>…</span> : null` | 1 (`OnboardingGate` 363) | picker test with localized descriptions, incl. a blank preferred locale |
| **line-0 arms source-mapped** | 9 (`AppShell` 296, 624; `ContextPanel` 172, 192; `useAgentStatus` 88; `useContextData` 116, 211; `useRailSelection` 39; `useThreadStore` 265) | 7 closed by tests (non-connected agent effect; non-arrow key on the divider; inspect-run while already on runs; unmount before the first probe; the live side of the post-git generation check; non-Escape key in selection mode; bootstrap failure after unmount), 2 reclassified DEAD with a construction argument (`ContextPanel` 172, `useContextData` 211) |
| reclassified DEAD after reading the source | 4 (`ActivityRailMenus` 255; `DiffView` 69; `OnboardingGate` 212 ×2) | see the DEAD reasons above |

Net effect: **49 → 19 uncovered arms**, group branch coverage **69.09 % →
98.64 % (1382/1401)**, with **no product source changed** (every mutation was
reverted byte-exact).
