# Flaky-test record — Mobile (jest)

Task record for the two documented Mobile flakes of the cov100 round-2 campaign
(`.future/cov100/FLAKES.md`, inventory items 9 and 10). One was already fixed
before the campaign baseline and is *verified* here with a forced-clock control;
the other does not reproduce on a green run at all, and its residual hazard — a
red assertion turning the suite into a CI hang — was found, proved by negative
control and fixed.

## 1. Run identity

| | |
| --- | --- |
| host | macOS 26 (darwin, arm64); the box is shared with the other campaign workers (`PLAN.md` §2: eight workers) |
| worktree | `/Users/geilige/future-os/.worktrees/cov100-r2`, branch `test/cov100-r2` |
| campaign baseline | `8449acf6` (owner of this unit: worker `F-MOBILE`) |
| measured content | the tree as snapshotted at `93c1d30f` … `58fdd7b0`; the final content of the touched file equals `58fdd7b0` |
| toolchain | node `v26.3.0`, jest `29.7.0`, preset `jest-expo` 57 |
| suite state | 163 suites, 3246 tests |
| judgement command (what CI runs) | `mobile/package.json` → `"test": "jest --runInBand"` |

Every command below was run from `mobile/` after

```bash
export HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home
ulimit -n 10240
```

(`PLAN.md` §2). The judgement is the **process exit code**, not the log:
`timeout 240 npx jest --runInBand; echo "exit=$?"` — `124` means still hanging.

## 2. Summary

| item | shape | outcome |
| --- | --- | --- |
| 9 — `useSkillRecommendation` local-midnight | calendar-dependent, fails 00:00–08:00 at UTC+8 | **fixed before the baseline** (`d90a6f46`, ancestor of `8449acf6`); verified by a forced-clock control that fails on the pre-fix construction and passes on the current one |
| 10 — jest never exits, green run | leaked real timer | **does not reproduce**: 12 whole-suite invocations on the unmodified tree, 0 timeouts, `--detectOpenHandles` finds nothing |
| 10 — jest never exits, *red* run | same mechanism, hidden by the hang | **reproduced and fixed**: one failing assertion in `syncEngine.test.ts:610` gave `exit=124` (240 s hang, `Jest did not exit…`); after the fix the same failing assertion gives `exit=1` in 50 s with the failure reported |

## 3. Item 9 — `useSkillRecommendation` "local midnight" flake

### 3.1 What it is

`mobile/src/features/chat/__tests__/useSkillRecommendation.test.ts` —
*"skips a skill already shown today instead of offering another"*
(`:232` in the campaign baseline). It seeds the stored daily budget, then
asserts the hook skips the already-shown skill:

```ts
expect(await evaluate(api, DRAFT)).toBe(false);
```

### 3.2 Mechanism

`skillRecoBudget.ts:31-35` defines the day as the **local** calendar day
(`today()` reads `getFullYear/getMonth/getDate`, deliberately not
`toISOString`, which is UTC), and `parseDay` discards any record whose `day`
differs from today's local day (`skillRecoBudget.ts:97`). The pre-fix test
seeded the record with `new Date().toISOString().slice(0, 10)` — a **UTC** date.
For a host east of UTC the two disagree between local midnight and
`00:00 + |offset|`: at UTC+8 (this host, CST) for the first eight hours of every
local day the seed said "yesterday", `parseDay` threw the record away as stale,
the budget looked unspent, the hook offered the skill again — and the assertion
failed. Nothing about the product was wrong; CI (a UTC host) never saw it.

### 3.3 The fix — already in the baseline, not made by this unit

* `d90a6f46` *"fix(test): stop two flaky tests from lying about the code"*
  (PR #845, 2026-09-26) changed the seed to the production `today()` and added a
  keeper regression test that pins the invariant itself.
* Verified as an ancestor of the campaign baseline:
  `git merge-base --is-ancestor d90a6f46 8449acf6` → exit 0.
* Current seed: `useSkillRecommendation.test.ts:240` (`day: today(),`); the
  invariant test is `skillRecoBudget.test.ts:213-241` (*"the day is the local
  calendar day"*, `it` bodies at `:214` and `:232`), which forces the
  disagreement with spies and therefore does not depend on the host clock or the
  hour of the run.
* Structural argument: seed and read now call the *same* function, so a
  UTC/local drift between them is not expressible any more.

### 3.4 Reproduction (the protocol's negative control)

At the current host hour (19:16 CST = 11:16 UTC) the window is closed, so the
divergence was **forced** rather than waited for — with the same spy technique
the keeper test uses, local date parts saying `2026-10-17` while
`toISOString()` says `2026-10-16T17:00Z` (= `2026-10-17 01:00` at UTC+8):

```ts
const iso = jest.spyOn(Date.prototype, "toISOString").mockReturnValue("2026-10-16T17:00:00.000Z");
const year = jest.spyOn(Date.prototype, "getFullYear").mockReturnValue(2026);
const month = jest.spyOn(Date.prototype, "getMonth").mockReturnValue(9);
const date = jest.spyOn(Date.prototype, "getDate").mockReturnValue(17);
```

A temporary scratch suite (deleted after the measurement — it asserts a pre-fix
failure and must never be part of the suite) ran the *identical* production body
twice, differing only in the seed:

| case | seed | `expect(await evaluate(api, DRAFT)).toBe(false)` |
| --- | --- | --- |
| pre-fix construction | `new Date().toISOString().slice(0, 10)` | **FAILS** — `Expected: false, Received: true` |
| current construction | `today()` | passes |

Run: `timeout 120 npx jest --runInBand src/features/chat/__tests__/zzMidnightControl.test.ts; echo exit=$?`
→ `exit=1`, `Tests: 1 failed, 2 passed`, with the first test asserting the
divergence itself (`"2026-10-16" !== "2026-10-17"`) so the control cannot pass
for the wrong reason. The failing line in the log is the production assertion,
not the scratch harness.

### 3.5 Sweep for siblings of this class

The same class would show up anywhere the suite still keys a fixture on the UTC
date while production reads local dates (or the reverse). Probed by putting the
**whole suite** inside the disputed window:

```bash
TZ=Pacific/Kiritimati bash ../.future/cov100/flake-rate.sh --label mobile-jest-utcplus14 \
    --runs 2 --timeout 240 -- npx jest --runInBand     # UTC now 2026-10-16 11:16 → local 2026-10-17 01:16
```

`runs=2 failures=0 timeouts=0` — no other date-fragile test in `mobile/src`.
(`TZ=Pacific/Kiritimati` is UTC+14, so the runner's local date was one day ahead
of UTC, exactly the condition that broke item 9.)

## 4. Item 10 — "jest never exits": a leaked timer turns CI into a hang

`mobile/package.json` runs `jest --runInBand`; a **single process**, and jest
does not force-exit, so any handle left armed at the end of the run keeps the
process (and therefore CI's Mobile job) alive forever while the tests themselves
are green.

### 4.1 The green-run half does not reproduce on this revision

| measurement | runs | failures | timeouts |
| --- | --- | --- | --- |
| `npx jest --runInBand`, tree as found | 1 (warm-up, 58 s) | 0 | 0 |
| same, `--detectOpenHandles` (86 s) | 1 | 0 | 0 |
| `flake-rate.sh --label mobile-jest-before --runs 5 --timeout 240` | 5 | 0 | 0 |
| `flake-rate.sh --label mobile-jest-before-b --runs 5 --timeout 240` | 5 | 0 | 0 |

12 whole-suite invocations on the unmodified tree: every one exits by itself,
none prints `Jest did not exit`, `--detectOpenHandles` reports no handle. So the
*green-run* leak of inventory item 10 is closed at this revision; what remained
is the hazard below.

### 4.2 The residual: a **red** assertion becomes a hang (reproduced)

`mobile/src/remote/__tests__/syncEngine.test.ts:610-651` —
*"reports the failed stage and scheduled retry for diagnostics"*.

Mechanism: the test builds a lane whose replay **always throws**
(`fetchReplay: async () => { throw new Error("replay unavailable"); }`). A failed
reconcile calls `scheduleRetry` (`syncEngine.ts:551-560`), which arms a real
`setTimeout` (`:555`) for `Math.min(30_000, 500 * 2 ** (attempt-1))` ms — 500 ms,
1 s, 2 s, … capped at 30 s (`RECONCILE_RETRY_MAX_MS = 30_000`,
`syncEngine.ts:181`).
When that timer fires, the step fails again and re-arms: an **unbounded chain of
real timers**. The only thing that ends it is `SyncEngine.clear()`
(`syncEngine.ts:417-426`), which bumps the generation so `isCurrent(lane)`
becomes false. The test cleared at the *end of the body*, so any failing
assertion above it skipped the clear and left the chain armed — jest then waited
forever, and the failure was visible only in a log nobody reads.

Negative control — the same tree, one assertion temporarily made wrong
(`retryInMs: 500` → `501`, reverted afterwards):

```bash
timeout 240 npx jest --runInBand > /tmp/m-neg-before.log 2>&1; echo "exit=$?"
```

| | before the fix |
| --- | --- |
| exit code | **124** (hung; the 240 s timeout is what ended it) |
| elapsed | 240 s (green runs finish in ~50-60 s) |
| log | `Test Suites: 1 failed, 162 passed, 163 total` / `Tests: 1 failed, 3245 passed, 3246 total` / `Jest did not exit one second after the test run has completed.` |

That is exactly the CI symptom: a job parked at `in_progress` with a
green-looking test summary, the real failure buried in the log.

### 4.3 The fix

`syncEngine.test.ts:610-651`:

1. **fake timers instead of a real sleep** — `jest.useFakeTimers()` and
   `await jest.advanceTimersByTimeAsync(20)` (`syncEngine.test.ts:619,632`) in
   place of `await new Promise(resolve => setTimeout(resolve, 20))`. This is the
   file's own idiom for retry tests (`syncEngine.test.ts:418`, `481`, `939`,
   `1290`, …).
2. **cleanup in a `finally`** — `engine.clear(); jest.useRealTimers();`.

Why this construction cannot hang: fake timers hold no real handle, so even if
every assertion in the body fails there is nothing left for the event loop to
wait on; the `finally` still owns the chain, and the deterministic clock also
removes the old real-time assumption (20 ms of wall clock had to be enough for
the reconcile chain to discover the failure — under a loaded runner that is
itself a way to go red, which is what made the pre-fix shape dangerous rather
than merely untidy).

After the fix, the same deliberately-wrong assertion
(`timeout 240 npx jest --runInBand`):

| | after the fix |
| --- | --- |
| exit code | **1** (a normal failure) |
| elapsed | 50 s |
| log | `Test Suites: 1 failed, 162 passed, 163 total` / `Tests: 1 failed, 3245 passed, 3246 total` — and **no** `Jest did not exit` line |

### 4.4 Audit of the rest of the class

Every other test in `mobile/src` that can arm a lane retry timer either runs on
fake timers or clears the engine in a `finally`. The two sites whose `finally`
is formatted unusually (`} finally {` on its own line, so a naive grep for
`} finally { engine.clear(); }` misses them) already carry an explicit comment
about this trap: *"the same jump on a lane that trims nothing is still a gap"*
and *"a projection without a usable cursor is refused, and the prefix is not
built from it"*. The production owner of the timers is also sound:
`useTimelineController.ts:624-629` clears its engine on unmount, and
`SyncEngine.clear()` is the single documented owner of the retry timer.

Grep that finds the remaining class in a file you are editing:

```bash
grep -n "engine.clear()" src/remote/__tests__/*.ts   # check each has `finally` above it or fake timers in the test
```

### 4.5 Rates for item 10

| measurement | runs | failures | timeouts |
| --- | --- | --- | --- |
| green, tree as found | 12 | 0 | 0 |
| green, after the fix (`--label mobile-jest-after`, `-after-b`, on the `finally`-only version) | 10 | 0 | 0 |
| green, after the final fix (`--label mobile-jest-final`) | 5 | 0 | 0 |
| `syncEngine.test.ts` alone, final code (the converted test's neighbourhood) | 11 | 0 | 0 (62/62 tests each) |
| induced-failure control, before the fix | 1 | 1 | **1 (`exit=124`)** |
| induced-failure control, final code | 1 | 1 | 0 (`exit=1`, 50 s) |

`pass` in `flake-rate.sh` means the process exited **by itself** within the
per-run timeout, so a rate of 0 timeouts is the property under test. Rates were
measured while the host was shared with the other campaign workers; the suite
takes ~50-90 s per run here.

## 5. Discovery notes (for `docs/testing/flaky-sweep.md`)

* No new flake appeared in ~30 whole-suite runs of `mobile` during this unit,
  under the host timezone, under UTC+14, and with `--detectOpenHandles`.
* The only non-green runs observed were the ones this unit induced on purpose.
* The class is real and cheap to trip while *editing* tests: any new test that
  drives a permanently-failing lane with real timers and does not clear the
  engine in a `finally` reintroduces "the suite hangs instead of failing". The
  rule to follow when touching `src/remote/__tests__/`: fake timers, and
  `finally { engine.clear(); }`.

## 6. What was not done / could not be reproduced

* **Item 9 could not be reproduced by waiting** — its window (00:00-08:00 local
  at UTC+8) was closed at the time of this work; it was reproduced by forcing
  the clock with spies (strictly stronger: it works at any hour and on a UTC
  host). No change to item 9's fix was needed or made, since it precedes the
  baseline.
* **The green-run leak of item 10 did not reproduce** on this revision, so there
  is no before/after rate for a green hang — only "0 timeouts in 12 runs" plus a
  clean `--detectOpenHandles`. What *was* reproduced is the red-run half (§4.2).
* **The negative-offset direction of the calendar class** (a host west of UTC,
  where the local date can lag the UTC date) cannot be probed at the current UTC
  hour without a fake clock; the keeper test in `skillRecoBudget.test.ts` forces
  the disagreement with spies and is independent of the hour, so the invariant is
  pinned either way.
* The `syncEngine.test.ts` fix was written while the supervisor was taking
  safety snapshots, so commit `93c1d30f` contains the *negative control*
  (`retryInMs: 501`, marked with a `NEGATIVE CONTROL (temporary)` comment) and
  the next snapshot `58fdd7b0` contains the final, correct content. No
  intermediate state is a keeper.

## 7. How to re-run

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/mobile
export HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home
ulimit -n 10240

# the judgement CI makes: does the process exit on its own?
timeout 240 npx jest --runInBand; echo "exit=$?"

# the rate, with the campaign harness (repeat for `after`, same command/-j)
bash ../.future/cov100/flake-rate.sh --label mobile-jest-check --runs 5 --timeout 240 -- npx jest --runInBand

# the calendar control (item 9): the whole suite in the disputed window
TZ=Pacific/Kiritimati npx jest --runInBand

# the red-run control (item 10): break one assertion in
#   src/remote/__tests__/syncEngine.test.ts:641 (retryInMs: 500 -> 501),
#   run the whole suite, then restore it:
#   before the fix = exit 124 + "Jest did not exit"; after = exit 1 in ~50 s.
```
