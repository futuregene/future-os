# review-goal-2 — adversarial verification of the REV-GOAL rework (`module-goal.md` §11.15)

**Frozen subject:** `docs/testing/module-goal.md` §11.5 + §11.15 — the 77-line residual
ledger over three scopes (`packages/*`, `orchestration/tasks`, `orchestration/goal`) and
the rework that answered `docs/testing/review-goal.md`'s six findings (F-1…F-6).

**Ticket:** `todo_cd99a76b7a69` (REV-GOAL-2), re-scoped by the supervisor on 2026-10-17
from P1 to **P2**: the three fixtures' *coverage counts* were already reproduced by the
supervisor, so the budget here went to (1) **falsifiability** of the three fixtures,
(2) a **ruling on the two test-only fixes**, (3) the F-6 / F-4 / F-4b confirmations,
(4) a **re-derivation audit** of §11.5's quoted counts, and (5) the **untouched list**.

**Write scope respected:** the only file this review wrote in the repository is
`docs/testing/review-goal-2.md`. All probes ran in a scratch tree
(`/tmp/r2d/scratch`, built with `git archive HEAD | tar -x`, its own
`CARGO_TARGET_DIR=/tmp/r2d/target-fal` and `HOME`), the same arrangement
`review-goal.md` used; no worktree source file, test target or build cache was touched
by me, and `git status --short` shows `docs/testing/review-goal-2.md` as the only file
this review added. (Two `desktop/src-tauri/src/remote_peer/*.rs` files became modified
in this worktree *during* the review, by another campaign unit working beside this one;
they are outside this review's scope and I did not touch them.) Scratch scripts and logs
live in `/tmp/r2d/`.

**Method.** Measure, don't argue: every verdict below rests on a `DA:` count from the
pinned report, on a probe run in the scratch tree, or on a mechanical parse of the
record. Where a claim could only be read (not measured), it is marked so in §8.

---

## 0. Verdict

| # | subject | claim under test | verdict | evidence |
|---|---|---|---|---|
| **M1** | pairing + tripwire | 77 uncovered / 77 mentioned / MISSING 0 / no STALE against `/tmp/r2d-lcov.info` | **PASS** | `reconcile.py` prints `files=13 uncovered=77`, `mentioned 77`, `MISSING 0`, no STALE block (§1) |
| **M2** | set equality (the F.14 hazard) | claimed set == residual set, per file and per line | **PASS** | 77 = 77, 0 missing, 0 stale, per-file equality for all 13 files (§1) |
| **M3** | category census | 11 + 9 + 21 + 36 = 77 (4 + 3 + 16 + 8 rows) | **PASS** | mechanical re-count of §11.5's table = 11/9/21/36 and 4/3/16/8 (§1) |
| **F-1** | `compat.rs:270`, `:235` | fixture reaches the contention bail; `DA:270 ≥ 1`, `DA:235 = 512` (= the constant) | **PASS + falsifiable** | run 5: `270=1`, `235=512`, `234=1378`, `271=1372`; sabotaged bail text → FAIL; sabotaged sleepless arm → FAIL (§2.1) |
| **F-2** | `unix_endpoint.rs:157` | stressed-race fixture reaches the identity guard; `DA:157 ≥ 1`; landed shape is the re-shaped one | **PASS + falsifiable** | run 5: `157=1` (`155/156=22`, `158/159=21`); control hits at attempt 14; guard disabled → FAIL (§2.2) |
| **F-3** | `supervision.rs:315-317` | swap fixture reaches the 3 s deadline; `DA:315/316/317 ≥ 1` | **PASS, falsifiability PARTIAL** | run 5: `315=1,316=1,317=1`; deadline-message sabotage → FAIL; **removing `kill`+`wait` leaves it green** (§2.3) |
| **F-4** | `transport.rs:142-145` | mechanism is **polling order**, not a lost race | **CONFIRMED (source + counts)** | `142-145 = 0`; `137=215`, `138=45`, `141=45`; `196=224`, `199=224`; the 0/20 driving is the reviewer's measurement, not re-run (§4) |
| **F-4b** | `transport.rs:190-191` | "an accepted authority always yields `Some(host)`", 0 of 47 spellings | **CONFIRMED (source + counts)** | `190=0`, `191=0`, `189=224`; the 47-spelling probe is the reviewer's, not re-run (§4) |
| **F-5** | §11.5's quoted neighbour counts | every `N=M` re-derived from run 5 | **PASS** | 108 anchored pairs, **0 mismatches**; the pre-rework document against run 4 gives **21 mismatches / 48** — F-5 independently reproduced (§5) |
| **F-6** | 3 lazy-`assert!` lines | `1050`, `10651`, `747` → `unreachable-by-construction` | **PASS** | exactly those three lines changed category (39→36 artifact lines); neighbour counts reproduce (`1048=8/1052=8`, `10649=1/10654=1`, `745=1/749=1`) (§4) |
| **Fix A** | SSE heartbeat test | bounded re-drive of the heartbeat; no assertion weakened; cannot pass for the wrong reason | **ACCEPTED, with a scope note** | assertions verbatim; loop 0..64 bounded; probe (5/5 turns) ends the task on **iteration 1** via `heartbeat-write-failed`; flake re-rate 30/30 green under 8 burners (§3.1) |
| **Fix B** | connect-poll loop | "no early `break`, so the retry path executes every run" | **STATEMENT REJECTED; RULED AN ATTRIBUTION/SHAPE CHANGE** | retry count unchanged in runs 3, 4 and 5 (2 connects, 1 failure each); the `yield_now` statement moved onto the unconditional tail (§3.2) |
| **U** | untouched list | `compat.rs:235` still a prose row; `platform-unmeasured` readings unchanged; nothing else re-labelled | **PASS** | `235` is not a table row (77 table lines = 77 residual) and reads `DA:512`; the four platform rows keep platform + condition verbatim (§6) |
| **D** | new defects | — | **3 minor + 1 bookkeeping** | the poll-loop evidence sentence, the `235` "plus one incidental hit", F-3's un-pinned kill/wait; the seven-attempt arithmetic (§7) |

Nothing here reverses a §11.5 category or a residual line. The residual number **77**,
the set equality, the census, the three fixtures' `DA:` counts and the F-5/F-6 fixes all
reproduce. What does not reproduce is §11.15's *statement of mechanism* for the
connect-poll fix (Fix B), and three small numeric/scope claims listed in §7.

---

## 1. Pairing and machine checks

`/tmp/r2d-lcov.info` (run 5, 2026-10-17 04:46:21, the log `/tmp/r2d-cov7.log`:
107 `test result:` lines, 0 `FAILED`). The registry pairing was re-pointed by the
supervisor before this review, so the tripwire reads a clean pairing:

```
python3 .future/cov100/reconcile.py --report /tmp/r2d-lcov.info --scope packages \
  --scope orchestration/tasks --scope orchestration/goal --doc docs/testing/module-goal.md
→ [unit] files with uncovered lines=13 uncovered lines=77
    mentioned in module-goal.md: 77
    MISSING (uncovered, not mentioned): 0            # exit 0, no STALE block
```

Because a count is not a set (INCIDENTS F.14), the per-line check was repeated with the
same rule `review-goal.md` used:
```
report residual lines (three scopes): 77 over 13 files
ledger claimed lines:                  77 over 13 files
claimed that resolve to a report file: 77
IN REPORT, NOT CLAIMED (MISSING): 0 {}
CLAIMED, NOT IN REPORT (STALE):  0 {}
SET EQUALITY: PASS
```

The gate's own pairing line agrees (no DRIFT for `goal`):
`goal  authoritative  module-goal.md  77  77  0  0  0 section  PASS`.

The per-file sets in §11.5's table are exactly the report's — including
`webui/server.rs`'s **only two** residual lines (288, 320); the connect-poll lines do
**not** appear in either run 4 or run 5, which matters for §3.2.

**Census**, re-counted mechanically from the table (not from the prose):
`attribution-artifact` 36 lines / 8 rows · `unreachable-by-construction` 21 / 16 ·
`unreachable-in-this-environment` 9 / 3 · `platform-unmeasured` 11 / 4 → **77**, and
`compat.rs:235` is not among them because it is a bullet, not a table row (as §11.5
says).

---

## 2. The three fixtures, falsified

The supervisor measured the *counts*; this section tests the other half: **would the
fixture fail if the behaviour of the line it covers broke?** Each answer below is a
measured sabotage in the scratch tree, with a control in the same tree and the same
build profile (`cargo test`, un-instrumented; coverage is already established, and the
question here is about assertions, not counters). Every libtest invocation puts
`--test-threads=1` **immediately after the single `--`** (PLAN §2.0), and every edit was
reverted and the tree verified byte-identical afterwards.

| experiment | edit (scratch tree only) | result |
|---|---|---|
| **A0 control** | none | `compat::tests::a_dangling_symlink…` **PASS**, 0.01 s |
| **S1a** | `compat.rs:270` bail text `could not acquire … (contended)` → `gave up on …` | **FAIL** at `compat.rs:679`: *"the bail, not an earlier failure: gave up on ACTIVE_GOAL_STATE.md.lock"* |
| **S1b** | `compat.rs:235` `LockState::Released => continue` → sleep 20 ms then continue | **FAIL** at `compat.rs:688`: *"the sleepless `Released` arm took 13.543252833s"* |
| **B0 control** | none | `a_socket_replaced…` **PASS**, 0.02 s, `drives=14 attempts=14 churn_rounds=61 guard_157=1 first_hit_at=14` |
| **S2** | `unix_endpoint.rs:156` guard → `if false && …` (budget cut 100 000 → 300) | **FAIL**: `drives=300 attempts=300 churn_rounds=1289 guard_157=0 first_hit_at=0` |
| **S2c control** | budget cut only, guard intact | **PASS**, 0.11 s |
| **C0 control** | none | `a_watchdog_that_stalls…` **PASS**, 3.43 s — `DELAY 0 ms → deadline_fired=true`, inode changed, `elapsed=3.423 s`, exit 1, message present |
| **S3a** | `supervision.rs:317` message `3 seconds` → `4 seconds` | **FAIL** after 17.06 s: *"deadline fired on 0 of 5 attempted swap delays (5 swaps landed)"* |
| **S3b** | `supervision.rs:315-316` `kill`+`wait` removed, bail kept | **PASS**, 3.17 s |

### 2.1 `compat.rs:270` — a dangling symlink: falsifiable, and it pins the mechanism

Both halves of the claim are pinned by assertions, not by the message alone:
**S1a** shows the fixture distinguishes the `MAX_LOCK_ATTEMPTS` bail from every other
error path in `acquire_active_state_lock_with` (no other message contains
`could not acquire`), and **S1b** shows the `< 5 s` bound really is a test of the
`sleepless` `Released` arm — the same arm whose counter is the `512`. With
`MAX_LOCK_ATTEMPTS = 512` (source: `compat.rs:126`) the run-5 value `235 = 512` is
exactly the fixture's own drive count, so the loop spun the full budget on the
`Released` arm: the mechanism is confirmed end to end. **No fixture defect.**

### 2.2 `unix_endpoint.rs:157` — the re-shaped race holds, and it is falsifiable

Shape confirmed against the source and against the run that broke the first version:
`attempts_budget = 100_000`, a **fresh endpoint + stale socket per attempt**, **four**
churner threads whose first `rename` races that attempt's single drive, and a tally that
prints `drives/attempts/churn_rounds/guard_157/first_hit_at`. The landed shape is
evidenced by its own log line (`attempts=`/`first_hit_at=` fields do not exist in the
first version), and the record's disclosure that the first version failed a real suite
run is confirmed in `/tmp/r2d/suite-r2d.log`:

```
STALE-CLEANUP GUARD: drives=20000 churn_rounds=19940 guard_157=0
panicked at unix_endpoint.rs:541: the stale-cleanup identity guard must be reachable
```

(the same log family also holds the old shape's `drives=14115 … guard_157=1` and
`drives=1277/1331/3549/7180 … =1` runs — i.e. the 1/14 000-ish rate the record quotes).
**S2** proves the assertion fails when the guard's behaviour is broken; **S2c** proves
the shortened budget used in S2 is not the cause (the guard hits at attempt 14 in the
control). One scope caveat carried into §7: the control run shows the fixture is fast in
this environment (14 attempts), but the assertion remains "≥1 hit in ≤100 000 attempts" —
a long-tail scheduling failure is possible and is *documented*, not hidden.

### 2.3 `supervision.rs:315-317` — falsifiable for the deadline, not for the kill

**S3a** shows the fixture fails if the deadline bail (317) disappears or changes text
(it then exhausts all five swap delays and panics with the tally `0 of 5`). **S3b** is
the finding: removing `child.kill()` and `child.wait()` (315-316) while keeping the bail
leaves the test **green**. So the fixture pins the *observation that the 3 s readiness
deadline fired* — the real elapsed budget, the absence of a registration, the swapped
inode — but it does not discriminate the kill/wait. That is not a ledger defect (the
lines are executed, `315/316/317 = 1`), but the record's sentence that "the parent kills
it and bails" describes the code rather than something the fixture falsifies. Suggested
one-clause addition: *"the fixture pins the deadline bail (317) and the 3 s budget;
315-316 are covered by the same drive but not discriminated by an assertion."*

---

## 3. Ruling on the two test-only fixes (`orchestration/goal/src/webui/server.rs`)

Both are test-only, both keep their assertions verbatim (verified by lifting the two
hunks out of `5f8440220` and diffing: the SSE assertions are unchanged word for word,
the poll loop's `client.expect("server binds and accepts")` is unchanged), and both loops
stay bounded (`0..64`, `attempt < 500`). **No assertion was weakened.**

### 3.1 Fix A — `serve_sse_ends_cleanly_when_the_client_vanishes_before_a_ping`

**Can it pass for the wrong reason? No — measured, not argued.** The hazard the
supervisor named is "the task ends because the 64 iterations ran out". That outcome
cannot pass: the loop only exits on `server.is_finished()`, and the next statement is
`assert!(server.is_finished(), "the heartbeat write to a vanished client must end the
task")`, so exhausting the budget is a *failing* outcome by construction.

The stronger question — does the task end for the *intended* reason, or via one of the
other `Ok(())` exits of `serve_sse`? — was settled with probes in the scratch tree (an
`eprintln!` at each of the four `Ok(())` exits, plus an iteration counter in the test;
5 runs; the fifth exit, the head write at 346, propagates an error and is caught by the
`is_ok()` assertion instead):

```
PROBE loop iteration 1
PROBE exit: heartbeat-write-failed
PROBE loop iterations used: 1, finished=true
test result: ok. 1 passed …
```

5 of 5 runs: the task ends on the **first** iteration through the heartbeat-write
failure, `finished=true`, and none of the other three `Ok`-exit probes (initial snapshot,
change snapshot, broadcaster gone) fires. The pinned
report agrees with the mechanism: `362 = 2` (the heartbeat write attempted twice),
`363 = 1` (that arm's `return Ok(())` taken), while each alternative exit is driven by
its own dedicated test — `349 = 1` (`serve_sse_returns_when_initial_snapshot_fails`),
`357 = 1` (`serve_sse_returns_when_change_snapshot_fails`), `360 = 4`
(`serve_sse_writes_head_snapshot_and_exits_when_gone`).

**Flake re-rate** (the supervisor asked for it before acceptance). The seven logs of the
six-crate command are 3 red + 4 green; all three reds failed exactly this test at the
pre-fix assertion line (`server.rs:1015`, which is the old file's `assert!(server.is_finished(), …)`)
and aborted before `llvm-cov export` — 43 % of that run set. The fix landed between the
last red (`r2d-cov5.log`, 04:25) and the eight whole-goal-lib verification runs
(`gsw.*.log`, 04:28–04:29); after it, the six-crate command is green in the two later
attempts (`r2d-cov6.log` 04:39, `r2d-cov7.log` 04:46) and in those eight runs
(447 tests, 0 failed, 5.07 s each).
My own A/B (30 runs per arm, 8 busy-loop burners of my own, host 1-min load ≈5–7 →
≈13–15) is **30/30 green for the shipped body and, importantly, also 30/30 for the old
body** — at that load the flake does not reproduce, so my A/B cannot *prove* necessity;
it can only say the shipped body is green where the old body was also green. The
necessity evidence is the three red instrumented runs, which the fix addresses directly.

**Ruling: ACCEPTED.** One scope note: the fixture's two assertions cannot distinguish a
heartbeat-write exit from the other `Ok` exits at *runtime*; what makes the fixture
sound is its construction plus the probe/DA evidence above. A future edit that moves the
`drop(client)` earlier would leave the test green while changing which arm ends it — the
DA counters (or a probe) are the instrument that would catch it, not the assertions.

### 3.2 Fix B — `run_server_serves_and_broadcaster_ticks`, the connect-poll loop

**The stated mechanism is false; the observation it was attached to is true.** Measured
from the pinned reports (old numbering from the pre-fix file, new from the shipped one):

| run | `for/while` header | connect | body tail (`yield_now`) | read as |
|---|---|---|---|---|
| run 1 `/tmp/goal-lcov-clean.info` (baseline) | 1010 = 1 | 1011 = **2** | 1015 = 1 | 2 connects, 1 failed |
| run 3 `/tmp/p1b-lcov.info` | 1047 = 1 | 1048 = **2** | 1052 = 1 | 2 connects, 1 failed |
| run 4 `/tmp/r2c-lcov.info` | 1047 = 1 | 1048 = **2** | 1052 = 1 | 2 connects, 1 failed |
| run 5 `/tmp/r2d-lcov.info` | 1069 = **3** | 1071-1073 = **2** | 1074 = **2** | 2 connects, 1 failed |

* **The retry rate did not change.** Every run that can be read, before and after the
  fix, made exactly **two** connect calls — one refused (the server task had not bound
  yet), one accepted. Removing `break` cannot make a retry happen more often, exactly as
  the supervisor read it: both shapes retry iff a connect fails, and *whether the first
  connect fails is still the same bind race*.
* **What did change is where the statement lives.** In the old shape the `yield_now()`
  sat in the failure arm (`if let Ok(..) { …break }` then `yield`), so its counter was
  `1` and could have been `0` had the first connect won the race. In the shipped shape it
  sits on the unconditional loop tail, so it runs once per attempt — `2` in run 5,
  including the successful attempt. The loop bookkeeping the `for` header held
  implicitly is now an explicit line with its own counter (`1070 attempt += 1 = 2`).
* **No ledger row moved.** `webui/server.rs`'s residual set is `{288, 320}` in both run 4
  and run 5; the poll-loop lines appear in neither. The change's value is *measurement
  stability* — the residual count could have been one higher had the flipper landed
  uncovered (83 against run 4's 82-line ledger, 78 against run 5's 77) — not a newly
  covered production line.
* **The record's supporting numbers do not reproduce.** §11.15 says "the two lines read
  `1` and `1` … the loop's counters show the real attempt count (3 attempts in that run),
  where the same report's predecessor had them at `DA:0`". In
  `/tmp/r2d-goallib.info` the loop reads `1067=1, 1068=1, 1069=3, 1070=2, 1071=2,
  1072=2, 1073=2, 1074=2`: the only `1`s are the two *declarations* (`let mut client =
  None;`, `let mut attempt = 0;`), the loop body reads `2`, and "3" is the number of
  **condition evaluations**, not attempts (the attempt counter says **2**). No surviving
  report in which the goal crate ran against this tree has any `DA:0` in `server.rs`
  1005–1085: across the 11 campaign reports that carry the file (`goal-lcov-clean`,
  `pkgs-lcov{,-after,-final}`, `p1b{,-full}`, `r2b`, `r2c`, `svc-baseline-norm`,
  `r2d-goallib`, `r2d`) the zeros are only ever `{88, 102–104, 288, 320, 363}` or
  `{288, 320}`, and the pre-fix goal-lib report was overwritten at the same path.

**Ruling: the change is a code-shape re-attribution in test-only code, not a behavioural
change and not "the retry path now runs".** Product behaviour, test behaviour, the
budget, the assertions and the attempt pattern are all identical; what changed is that a
statement previously reachable only through the failure arm now sits on the common path.
That is the `journal.rs` 200/201→202/203 shape (F-55: a re-compile/restructure
re-attributes the region), not the policy's `attribution-artifact` definition ("a region
llvm-cov attributes to a macro or a closing brace") — so I would not file a row under
that category either. The campaign's own instrument for a run-dependent line is the
**flipping-row rule** (`module-agent-rpc.md`: number in the mechanism cell, text says it
flips), and this fix *removes* the flip, which is better than documenting it.
Recommended replacement sentence for §11.15: *"the `yield_now` statement moved out of the
connect-failure arm onto the unconditional loop tail, so its counter is run-independent;
the retry rate itself is unchanged (two connects, one refused, in runs 3, 4 and 5). No
ledger row moved."*

---

## 4. F-6 recategorisations, and the F-4/F-4b mechanism corrections

**F-6 — confirmed, and only the claimed three lines moved.** A mechanical diff of
§11.5's table before the rework (`b251ae28d^`) and after, per (path, line):

```
lines present in BOTH with a DIFFERENT category:
  orchestration/goal/src/console.rs:10651   attribution-artifact -> unreachable-by-construction
  packages/app-settings/src/lib.rs:1050     attribution-artifact -> unreachable-by-construction
in OLD, not in NEW:  supervision.rs 315, 316, 317; compat.rs 270; unix_endpoint.rs 157;
                     compat.rs 682 (attribution-artifact)
in NEW, not in OLD:  compat.rs 747 (unreachable-by-construction)
```

i.e. exactly the three F-6 lines changed category (the third via the renumbering
682 → 747), exactly the five covered lines left the table, and **nothing else moved** —
`attribution-artifact` goes 39 → 36 lines for exactly that reason. The neighbour evidence
the rows keep as proof that the guard ran reproduces in run 5:
`app-settings 1048=8 & 1052=8` (1050 = 0), `console 10649=1 & 10654=1` (10651 = 0),
`compat 745=1 & 749=1` (747 = 0).

**F-4 / F-4b — confirmed in the source and in the report's neighbours.** Run 5 gives
`transport.rs 142-145 = 0` with `137=215, 138=45, 141=45` (the inner `transport error`
path executed while the outer arm did not) and `190=191=0` with `189=224` (the
`.ok_or_else` line executed 224 times without a single `host() == None`). Both corrected
mechanisms are consistent with the code: `tokio::time::timeout` polls the inner future
before checking its own deadline, so with both budgets elapsed the inner `Elapsed` is
delivered first and the caller's arm at 138 sees `Ok(Err(..))`; and the `190-191` arm
requires `uri().host() == None` *after* an accepted `from_shared`, which the "authority
is always present, possibly empty" invariant excludes. **Not re-run by me:** the 0/20
outer-arm driving and the 47-spelling table are the reviewer's measurements (its probe
source is embedded in `review-goal.md` §10.3); they are recorded as the reviewer's, not
adopted as mine.

---

## 5. §11.5's re-derived counts, against the report it pins

The rule the record says it now follows (nearest `DA:`-bearing line up and down, number
must match) was applied mechanically to every quoted `N=M` in §11.5:

```
run 5 /tmp/r2d-lcov.info :  anchored quoted pairs checked: 108   mismatches: 0
pre-rework doc vs run 4  :  anchored quoted pairs checked:  48   mismatches: 21
```

The second line is the independent reproduction of F-5: against run 4 — the report that
version pinned — **21 of 48** anchored pairs were wrong (e.g. `app-settings:317` quoted
`316=8`, run 4 says `11`; `367=2` vs `23`; `console:4754` quoted `4755=85`, run 4 says
`89`; `supervision:173` quoted `174=16`, run 4 says `17`) — the record's claim that the
old cells carried *earlier runs'* numbers is therefore correct, and its fix (one report
backs every cell) is verifiable: 108/108 reproduce against run 5, including the six
examples the reviewer named (`368→367=23 & 369=23`; `316→316=11 & 318=11`;
`132→132=10 & 134=10`; `5127→5127=67 & 5129=67`; `4755→4753=21 & 4755=89`;
`174→172=17 & 174=17`).

---

## 6. The untouched list

* **`compat.rs:235`** is still a **bullet, not a table row** — confirmed mechanically: the
  table cites exactly 77 lines, `235` is not among them, so a table row was not quietly
  reintroduced — and it reads **`DA:512`** in run 5, matching the documented flipper
  (`run 3 = 0`, `run 4 = 3`, the reviewer's probe `= 512`, run 5 `= 512`). The category
  (`unreachable-in-this-environment`) is unchanged.
* **The four `platform-unmeasured` rows keep their readings.** Row text diffed
  before/after: `connection.rs:427-431` byte-identical; `console.rs:10566`,
  `compat.rs:247,260`, `store.rs:1496-1498` keep the platform, the condition, the errno
  codes (`ENOTSOCK` 38 on Darwin; `ERROR_ACCESS_DENIED` 5 / `ERROR_LOCK_VIOLATION` 33 on
  Windows) and the authority sentence; the only additions are run-5 neighbour counts
  (`10565=2, 10568=2`; `246=1/248=1, 259=1/261=1`; `1495=1/1499=1` — all four reproduce).
* **`attribution-artifact` is 36 lines, not 39**, and the only three that left it are
  F-6's — §4's per-line diff is the evidence. No other row was re-labelled, and the
  "other 12 `unreachable-by-construction` rows" and `unix_endpoint.rs:114-116/129`
  (still `DA:0` in run 5, matching the 0-in-20 000 measurement) are untouched.

---

## 7. Defects found (all minor; none moves a line or a category)

**7.1 The poll-loop evidence sentence does not reproduce** (§3.2). Fix: replace the
numbers with the measured ones and drop or mark the unreproducible predecessor clause
(the pre-fix goal-lib report no longer exists).

**7.2 `compat.rs:235`'s "plus one incidental hit" is arithmetically wrong.** §11.5 says
the fixture "drives the `Released` arm exactly `MAX_LOCK_ATTEMPTS` times (512) plus one
incidental hit", but the report reads **exactly 512** and `MAX_LOCK_ATTEMPTS = 512`
(`compat.rs:126`), so the incidental count is **0**, not 1. The §11.15 draft recorded
`235 … → 513`; the landed table was corrected to `512` but this bullet was not. Fix:
delete "plus one incidental hit" (or say "and no incidental hit: the total is 512").

**7.3 F-3's falsifiability is narrower than its prose.** Covered in §2.3: the fixture
fails when the deadline bail breaks and stays green when `kill`/`wait` are removed. One
clause in §11.15 should say which of the three lines the *assertions* pin.

**7.4 Bookkeeping: the seven attempts account for six.** `/tmp/r2d-cov*.log` is 3 red
(03:27, 03:49, 04:25) + 4 green (03:32, 03:55, 04:39, 04:46), but §11.15 accounts for
three red, "two earlier green runs [that] were superseded" and the reported run = six.
`r2d-cov6.log` (04:39, green) is described nowhere. It is not a number defect — a green
run's lcov would have been overwritten by the reported one — but a successor comparing
the log set with the prose will stop on it. Fix: name it, or write "four green attempts,
two of which were superseded".

---

## 8. What I could not check, and uncertainties

* **Not re-run:** the reviewer's 0/20 outer-timeout driving and its 47-spelling table
  (F-4/F-4b). They need a tonic endpoint probe; the reviewer's probe source is embedded
  in `review-goal.md` §10.3 and the corrections are consistent with the source and with
  run 5's neighbours. Recorded as the reviewer's measurement.
* **Not instrumented:** my falsification runs are plain `cargo test` in a scratch tree —
  they answer "do the assertions fail when the behaviour breaks?", not "does the line
  still execute"; the coverage half is the supervisor's measurement and run 5.
* **The SSE flake does not reproduce at my load.** 30/30 green for both the shipped and
  the old body under 8 burners (host 1-min load ≈5–7, ≈13–15 after the burners). The
  flake needs the 16–55 load the red runs saw; my A/B is a *non-reproduction*, and the
  necessity argument rests on those three red runs. Stated rather than glossed.
* **S2's shortened budget** (300 instead of 100 000) was used so the sabotage fails in
  0.31 s instead of ~2–5 minutes. It cannot affect the *verdict*: with the guard
  disabled `hits` is identically 0 for every budget, and the S2c control (same budget,
  guard intact) passes at attempt 14.
* **Uncertain:** whether §11.15's "the two lines read 1 and 1" referred to some
  intermediate goal-lib report that no longer exists. I could not find any report that
  supports it; the surviving report contradicts it, so it is filed as 7.1 either way.

---

## 9. Reproduction

```bash
# machine check (repo root, read-only)
python3 .future/cov100/reconcile.py --report /tmp/r2d-lcov.info --scope packages \
  --scope orchestration/tasks --scope orchestration/goal --doc docs/testing/module-goal.md
python3 /tmp/r2d/seteq.py  /tmp/r2d-lcov.info docs/testing/module-goal.md
python3 /tmp/r2d/listset.py docs/testing/module-goal.md "### 11.5 Residual lines"
python3 /tmp/r2d/pairs.py  docs/testing/module-goal.md "### 11.5 Residual lines" /tmp/r2d-lcov.info
python3 /tmp/r2d/pairs.py  /tmp/r2d/prerework-module-goal.md "### 11.5 Residual lines" /tmp/r2c-lcov.info
python3 /tmp/r2d/catdiff.py /tmp/r2d/prerework-module-goal.md docs/testing/module-goal.md \
  "### 11.5 Residual lines"

# the falsification sandbox (scratch tree, own target dir + HOME)
bash /tmp/r2d/falsify-controls.sh     # A0/B0/C0 controls      -> /tmp/r2d/falsify-controls.log
bash /tmp/r2d/falsify-sabotage.sh     # S1a/S1b/S2/S2c/S3a/S3b -> /tmp/r2d/falsify-sabotage.log
bash /tmp/r2d/sse-ab.sh               # 30+30 under 8 burners  -> /tmp/r2d/sse-ab.log
bash /tmp/r2d/sse-probe.sh            # which exit ends the task -> 5/5 heartbeat-write-failed
```

Evidence reused rather than regenerated: `/tmp/r2d-goallib.info` (the goal-lib-only
instrumented run), `/tmp/r2d-lcov.info` (run 5), `/tmp/r2d-cov{,2..7}.log` (the seven
six-crate attempts), `/tmp/r2d/suite-r2d.log` (the first `unix_endpoint` fixture's real
suite failure), `/tmp/r2d/gsw.*.log` (8 whole-goal-lib runs), `/tmp/r2d/new1115.md` (the
§11.15 draft, which still carries `513`).
