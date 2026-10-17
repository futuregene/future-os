# review-goal — independent adversarial review of the `module-goal` ledger (REV-GOAL)

**Frozen subject:** `docs/testing/module-goal.md`, §11.5 `Residual lines — category and
mechanism` — **82 uncovered lines in 13 files**, 33 rows, every line carrying one of
the four waiver categories. Scope reviewed: `packages/*`, `orchestration/tasks`,
`orchestration/goal` (round 2; §1–§10 is the round-1 record and is out of scope).

**Write scope respected:** the only file this review wrote is
`docs/testing/review-goal.md`. Neither the module record nor any source file was
edited. The one campaign-tree *read* I performed outside my report was `reconcile.py`
invocations; every probe was built and measured in a scratch tree (below).

**Scratch tree (REVIEW.md rules 3/5c):** `/tmp/rev-goal-r2-a`, unique to this segment.
Built as `git archive HEAD | tar -x` plus `rsync` of the current `packages/` and
`orchestration/` sources (verified byte-identical to the worktree), with its own
`CARGO_TARGET_DIR=/tmp/rev-goal-r2-a/tgt` and its own `HOME`. Probe *tests* live only
in that tree (`orchestration/goal/tests/rev_goal_*.rs`,
`packages/rpc/tests/rev_goal_*.rs`). No peer was writing `module-goal.md` when this
review started: the record's last edit is the committed segment §11.13/§11.14, and
`git status` shows `docs/testing/module-goal.md` clean.

---

## 0. Verdict

**Falls: 6 rows.** Three rows fall on **reachability** with measured `DA:` evidence
(F-1, F-2, F-3 — 5 lines), three more on **category** (F-6 — 3 lines), plus one
**evidence** defect covering 23 of the 54 quoted neighbour counts (F-5) and two
**mechanism corrections** inside rows that survive (F-4, F-4b).

| # | target | claim | verdict | measured evidence |
|---|---|---|---|---|
| **F-1** | `orchestration/goal/src/compat.rs:270` | `unreachable-in-this-environment` | **FELL** | fixture = one **dangling symlink**; `DA:270 0 → 1`, and `DA:235 3 → 512` (§3.1) |
| **F-2** | `packages/rpc/src/transport/unix_endpoint.rs:157` | `unreachable-in-this-environment` | **FELL** | biased race fixture (the shape AG-A used to cover `activity.rs:71`); `DA:157 0 → 2`, 4 hits in 24 000 drives (§3.2) |
| **F-3** | `orchestration/goal/src/agents/supervision.rs:315-317` | `unreachable-in-this-environment` | **FELL** | same-binary copy + `unlink`/stub swap; `DA:315/316/317 0 → 6/6/6`, 6 of 7 swap delays (§3.3) |
| **F-4** | `packages/rpc/src/transport.rs:142-145` | `unreachable-in-this-environment` | **SURVIVED, mechanism corrected** | 0/20 drives took the outer arm; the reason is *polling order*, not "the inner timer won the race" (§6.1) |
| **F-4b** | `packages/rpc/src/transport.rs:190-191` | `unreachable-by-construction` | **SURVIVED, mechanism corrected** | 47 spellings probed (the record's 7 plus 40); 0 produced `host() == None` — but the row's reason ("every authority-less URI is rejected") is false: `http://:8080`, `http://[]`, `http://.`, `http:////` are *accepted* by `from_shared` and yield `Some("")`/`Some(host)` (§6.1) |
| **F-5** | §11.5's quoted neighbour counts (**23 of 54 pairs**) | "quotes the neighbouring counts **from the same run's report**" | **FELL (evidence, not category)** | goal-crate quotes reproduce **24/27** against run 1's `goal-lcov-clean.info` but **19/29** against the pinned run-4 report; packages 15/25 vs 12/25 (§4) |
| **F-6** | 3 lines whose mechanism is "a lazy `assert!` message argument, evaluated only on failure" (`app-settings/src/lib.rs:1050`, `console.rs:10651`, `compat.rs:682`) | `attribution-artifact` | **FELL (category)** | the same shape is `unreachable-by-construction` in AG-A (its F-29) and in this record's own `console.rs:5318`/`10663` rows (§5) |
| — | §11.5's **row numbers / line set** (the F.14 hazard) | — | **PASS** | per-file, per-line **set equality**: 82 claimed = 82 residual, 0 missing, 0 extra (§1) |
| — | all 39 `attribution-artifact` lines | — | **SURVIVED** | each has a covered neighbour in the pinned report; 0 flagged by the audit (§6.2) |
| — | `platform-unmeasured` 4 rows / 11 lines | — | **SURVIVED** | all four are **shape (b)** (compiled here, another platform's runtime decides) and all four name the platform **and** the condition (§7) |
| — | `unreachable-by-construction` rows (13 rows / 18 lines) | — | **SURVIVED** | `transport.rs:190-191` corrected in §6.1; the other 12 rows re-read in the source, invalidator named per row (§8) |
| — | `unix_endpoint.rs:114-116` and `:129` | `unreachable-in-this-environment` | **SURVIVED (measured rate)** | 0 hits in **20 000** drives against an aggressive churner; the window is additionally shielded by the `nlink != 1` guard at 98-106 (§3.4) |

Every one of the 82 lines received a verdict: 39 by the neighbour audit, 11 by the
platform audit, and 32 by re-reading the invariant in the source — of which the five
lines of F-1…F-3 were attacked with instrumented probes and fell, two rows were
mechanism-corrected (§6.1), and two `unix_endpoint` rows were measured at 0 hits in
20 000 drives (§3.4).

---

## 1. The pairing, and the set-equality check the F.14 hazard demands

The supervisor's first hazard was INCIDENTS F.14: this record's ledger numbers were
once regenerated by an assertion that compared a **count** (212 rows stayed 212) while
four line numbers were wrong. So the check here compares **sets, per file and per
line**, not counts.

```
python3 /tmp/rev-goal-r2-a/setcheck.py /tmp/r2c-lcov.info
```

```
report residual lines: 82 over 13 files
ledger claimed lines : 82 over 13 rows/files
claimed that resolve to a report file: 82
IN REPORT, NOT CLAIMED (MISSING): 0
CLAIMED, NOT IN REPORT (stale/extra): 0
SET EQUALITY: PASS (claimed set == residual set)
```

Per file, the two lists are identical:

| file | claimed = report |
|---|---|
| `packages/rpc/src/connection.rs` | 427-431 |
| `packages/rpc/src/transport.rs` | 142-145, 190, 191, 197, 672, 677, 959 |
| `packages/rpc/src/transport/unix_endpoint.rs` | 114, 115, 116, 129, 157 |
| `packages/app-settings/src/lib.rs` | 317, 344, 368, 491, 499, 507, 515, 526, 540, 1050 |
| `packages/app-workspaces/src/lib.rs` | 133, 193, 225, 278, 316, 410, 411, 425, 793 |
| `orchestration/tasks/src/store.rs` | 499, 510, 571 |
| `orchestration/goal/src/console.rs` | 259, 2001, 2079, 2677, 2721, 2727, 4570, 4754, 5123-5126, 5128, 5318, 5526, 6928, 6984, 7659, 9782, 10566, 10651, 10663 |
| `orchestration/goal/src/agents/supervision.rs` | 173, 214, 259, 315, 316, 317 |
| `orchestration/goal/src/webui/server.rs` | 288, 320 |
| `orchestration/goal/src/compat.rs` | 247, 260, 270, 682 |
| `orchestration/goal/src/store.rs` | 1496-1498, 1618 |
| `orchestration/goal/src/validator.rs` | 177 |
| `orchestration/goal/src/decision/goal_frontier/replan_rules.rs` | 253 |

**Which report is the ledger's?** The campaign registry pairs this record with
`/tmp/p1b-lcov.info` (run 3, **92** lines). The ledger's own header says **82**, and
`§11.9` prints the run-4 invocation with `/tmp/r2c-lcov.info`. Measured:

```
reconcile.py --report /tmp/r2c-lcov.info … --section "Round 2"
→ 13 files, uncovered 82, mentioned 82, MISSING 0, no STALE, exit 0

reconcile.py --report /tmp/p1b-lcov.info … --section "Round 2"
→ 13 files, uncovered 92, mentioned 92, MISSING 0
  STALE: 7   COVERED 5 (console.rs:10663, unix_endpoint.rs:115/116/129/157)
             ABSENT  2 (console.rs:10651, unix_endpoint.rs:114)
```

The five "COVERED" rows are a **stale-pairing artifact, not stale rows**: run 3
predates both the peer edit that inserted 38 lines above `unix_endpoint.rs`'s rows
(76-78/91/119 → 114-116/129/157, disclosed in the record's §11.13) and §11.11's
8-line guard insertion above `console.rs:10655` (→ 10663). Against the run-3 numbering
those citations land on *other, covered* lines. So: **the ledger is clean against the
report it pins (r2c); the registry entry needs re-pointing** — the same one-line fix
the other two records needed. This is reported, not edited (the registry is outside
my write set).

---

## 2. Method

1. **Measure the claim, not the prose** (REVIEW.md §5/R2 two-step): every
   reachability attack below was run as a probe in the scratch tree, and the verdict
   rests on `DA:` counts before/after, never on the error text alone — with one
   deliberate exception, noted where it appears (F-3's string, which is unique to the
   three lines and is reported *alongside* its `DA:` counts).
2. **Name the invalidating token** for every row that survives (§8, and inline in
   §3/§6/§7). A tag that cannot be invalidated has not been reviewed.
3. **Categories audited by rule, not by trust:** `attribution-artifact` against its
   neighbours (all 39 lines), `platform-unmeasured` against the ledger's two shapes,
   and every row's mechanism against its category (the F-25/F-31 class).

**Not done, deliberately:** no whole-repo number was re-measured; the frozen-suite and
two-flavour harnesses were not run (supervisor's); the module record and all sources
were left untouched, so a *sabotage* falsifier that requires editing production code
would have been out of contract — none of my three falls needed one.

---

## 3. The three falls

### 3.1 `compat.rs:270` — the contention bail is reachable in 1.86 ms with no holder

**The row:** `unreachable-in-this-environment` — *"the `MAX_LOCK_ATTEMPTS` contention
bail: needs a holder that keeps the projection lock for 512 consecutive acquire
attempts; the harness cannot hold another process's lock across that loop
deterministically."*

**What the source actually does** (`acquisition loop`, 217-271):

```
221:  match fs::OpenOptions::new().write(true).create_new(true).open(&lock_path) {
226:      Ok(mut file) => { … return Ok(lock_path); }              // we won
231:      Err(error) if create_is_contention(&error) => {}          // AlreadyExists
232:      Err(error) => return Err(error)…                          // real error
234:  match classify_lock(&lock_path) {
235:      LockState::Released => continue,                          // file gone: NO sleep
244:      LockState::Dead => { if remove_lock_file fails … 247: continue }
252:      LockState::Empty => { sleep or take over }
269:  bail!("could not acquire {ACTIVE_STATE_LOCK} (contended)")
```

The mechanism is wrong about **which states spin**: `Released` and a
successfully-removed `Dead` both `continue` **without sleeping**, so the loop needs no
holder at all — it needs a path that `O_CREAT|O_EXCL` refuses while `O_RDONLY` reports
`ENOENT`. That is exactly a **dangling symlink** (`create_is_contention` accepts
`AlreadyExists`; `classify_lock` reads through the link and gets `NotFound` →
`Released`).

**Probe** (`rev_goal_lock_probe.rs`, embedded in §10.2; sha256 `7b4d633b…`):

```
PRECONDITION create_new -> kind=AlreadyExists raw=Some(17); read_to_string -> kind=NotFound raw=Some(2)
RESULT err in 1.864833ms: could not acquire ACTIVE_GOAL_STATE.md.lock (contended)
```

**Measured `DA:` before/after** (instrumented runs, `compat.rs`):

| run | 234 | **235** | 240 | 245 | 253 | 264 | **270** | 271 |
|---|---|---|---|---|---|---|---|---|
| BEFORE `/tmp/r2c-lcov.info` (campaign, full suite) | 821 | **3** | 2 | 2 | 58 | 54 | **0** | 1371 |
| AFTER `/tmp/rev-goal-r2-a/lock-after.info` (probe) | 583 | **512** | 1 | 0 | 0 | 0 | **1** | 2 |

`235 = 512` is the strongest possible confirmation: the loop ran **exactly
`MAX_LOCK_ATTEMPTS` = 512** times along the `Released` arm and then fell to 270 — the
number in the constant, not an approximation.

**Verdict: FELL.** The category claims this host cannot provide the fixture; the
fixture is a plain filesystem state costing 1.86 ms, no second process, no lock, no
race. Two of the row's three premises are false (`Released` spins without sleeping;
`create_is_contention` accepts a path that does not exist afterwards). Under REVIEW.md
rule 5a the mechanism is also weaker than the category it carries. The honest closure
is `coverable` with this fixture (or, if the campaign prefers, a category for
"out-of-contract but reachable filesystem states" — the ledger has no such shape).
**Named invalidator:** the set of `LockState`s that `continue` without sleeping — give
`Released` a sleep + deadline (or narrow `create_is_contention` to Windows) and the
bail goes back to needing a live adversary.

### 3.2 `unix_endpoint.rs:157` — the stale-cleanup identity guard IS hit by a racing binder

**The row:** `unreachable-in-this-environment` — *"both are TOCTOU guards … Reaching
them needs another process to replace the lease file (resp. the socket) inside a
two-syscall window; there is no injection seam there and a racing thread is not a
deterministic fixture."*

**What the source actually does** (`bind`, 153-160):

```
154:  if let Some(stale) = self.stale_socket().await? {     // reads identity at 126, connect → ECONNREFUSED
155:      let current = std::fs::symlink_metadata(&self.path)?;
156:      if identity(&current) != stale {
157:          return Err(conflict("local endpoint changed before stale cleanup"));
```

Line 154 is an **`await`**: the window spans a `tokio::time::timeout(250 ms,
UnixStream::connect)`, i.e. it has a real yield point, and the race it guards is the
*cooperating* one — two binders both finding the same stale socket and both deciding
to remove it (159/161). That is precisely the case this guard exists for.

**Probe** (`rev_goal_unix_endpoint_probe.rs`, §10.3; sha256 `92d08746…`): a churner
thread repeatedly `remove`s the socket path and re-`bind`s a listener (leaving a stale
socket file), while the probe calls the *public* `reserve_local_endpoint()` +
`bind()`, 20 000 times.

```
STALE  drives=20000 churn_rounds=308807  guard_157=3  active=461  other=19230  862µs/drive
```

**Measured `DA:` before/after** (`unix_endpoint.rs`):

| run | 153 | 154 | 155 | 156 | **157** | 158 | 159 |
|---|---|---|---|---|---|---|---|
| BEFORE `/tmp/r2c-lcov.info` | 10 | 10 | 1 | 1 | **0** | 1 | 1 |
| AFTER `/tmp/rev-goal-r2-a/rpc-after.info` | 4966 | 4966 | 1312 | 1157 | **2** | 1155 | 1155 |

**Verdict: FELL.** The line executes: `0 → 2` in the instrumented run, and 4 hits
across 24 000 drives in two runs (1/4 000 then 3/20 000). The fixture is the *biased
race* shape this campaign already accepted as coverage for AG-A's
`service/activity.rs:71` ("a thread that takes and drops a work guard continuously …
asserting 'at least one refusal in N attempts' with the tally printed"), so the row's
"I am unreachable **here**" is false *and* its stated mechanism ("a racing thread is
not a fixture") is contradicted by the campaign's own precedent and by this
measurement. **Named invalidator:** taking the identity from the same
`symlink_metadata` it compares against (or holding an fd across the window) — then the
guard is comparing a value with itself and becomes genuinely dead.

### 3.3 `supervision.rs:315-317` — the 3 s watchdog deadline is reachable with the fixture this unit already ships

**The row:** `unreachable-in-this-environment` — *"It is reached only if the spawned
watchdog neither takes its lock nor exits for 3 s of wall clock … A pre-lock stall
would need the child frozen between `spawn` and `lock_file` (**the parent holds no
pid**) or an `open` that blocks, which the O_RDWR open of a lock file never does."*

**What the source actually does** (296-320): the parent holds the `Child`
(`let mut child = cmd.spawn()?` at 299), polls `running()` and `child.try_wait()`
every 10 ms, and at the deadline does **`child.kill()` + `child.wait()`** (315-316)
before bailing. So the parent *does* hold the handle — the parenthetical is simply
false, and the kill is the evidence.

`ensure_watchdog` spawns **`std::env::current_exe()`** (274-283). A *wrapper script*
copy does not work — measured: `exec` makes the parent's `current_exe()` the real
binary, so the child was the real binary and the drive succeeded in 0.94 s (that
negative result is worth keeping: it rules out the cheapest wrong fixture). What does
work on Unix is `unlink` + write a stub at the same path: the *running* image keeps
the old inode and the *next* exec gets the stub.

**Probe** (`rev_goal_watchdog_probe2.rs`, §10.2; sha256 `f14648ea…`): copy the built
binary to `<tmp>/future`, start `future goal supervisor register --goal G`, wait T ms,
swap in `#!/bin/sh\nexec sleep 30`, and read the outcome.

```
DELAY    0 ms -> deadline_fired=true  elapsed=3.24s :: Error: supervisor watchdog did not become ready within 3 seconds
DELAY   30 ms -> deadline_fired=true  elapsed=3.19s
DELAY   80 ms -> deadline_fired=true  elapsed=3.37s
DELAY  150 ms -> deadline_fired=true  elapsed=3.45s
DELAY  250 ms -> deadline_fired=true  elapsed=3.37s
DELAY  400 ms -> deadline_fired=true  elapsed=3.52s
DELAY  700 ms -> deadline_fired=false elapsed=0.70s :: supervisor registered (session sup-stall)
SWAP-WINDOW RESULT: 6/7 delays reached the deadline
```

Every swap showed `inode … -> …` different, i.e. the running parent survived the
replacement. The window is the parent's own startup (~0.7 s), so any T ≤ 400 ms is
deterministic in practice; only a swap *after* `register` finished (700 ms) misses.

**Measured `DA:` before/after** (`supervision.rs`):

| run | 299 | 302 | 303 | 305 | 310 | 311 | 312 | 313 | 314 | **315** | **316** | **317** | 318 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| BEFORE `/tmp/r2c-lcov.info` | 4 | 8 | 2 | 6 | 1 | 1 | 1 | 4 | 4 | **0** | **0** | **0** | 4 |
| AFTER `/tmp/rev-goal-r2-a/watchdog-after.info` | 7 | 1332 | 1 | 1331 | 0 | 0 | 0 | 1331 | 1331 | **6** | **6** | **6** | 1325 |

**Verdict: FELL.** `0 → 6/6/6` — the kill/wait/bail ran six times, exactly the number
of delays that fired. The category claims this host cannot provide the fixture; the
fixture is 15 lines on top of the *same* `tests/supervision_future_stem_drive.rs`
binary-copy this unit already ships, and it costs 3 s. **Named invalidator:** the
parent spawning a fixed path instead of `current_exe()`, or `running()` counting a
lock-less child as ready — either removes the state; a longer deadline does not.

### 3.4 The two `unix_endpoint` rows that survive, with their measured rate

Same probe, same churner, 20 000 drives each:

```
LEASE  drives=20000 churn_rounds=21867  guard_114_116=0  occupied=0  other=16707
```

`DA:114/115/116 = 0` in the after-report as well. **The mechanism worth recording is
better than the row's:** an unlink drops the *open lease's* `nlink` to 0, so the
`metadata.nlink() != 1` guard at 98-106 rejects a lease that was replaced behind the
fd *before* execution reaches 113 — 16 707 of 20 000 drives died there
("untrusted local endpoint lease"). To reach 114-116 the unlink must land in the
sub-microsecond window between the `fstat` (97) and the `lstat` (113), which is why
0/20 000 is the honest number. `:129` (the non-`NotFound` arm of `stale_socket`) is
0/20 000 too: the parent directory is created and `chmod 0700`-ed by the same call,
and `ENAMETOOLONG` is consumed first by the longer lease path (95) — a longer final
component fails there, which the probe's `No such file or directory` samples show
going to the `NotFound` arm (127-128) instead.

**Verdict: SURVIVED** for both, by measurement rather than by argument. **Named
invalidator:** an injection seam (a hook between 97 and 113, or a `stale_socket`
variant that takes its identity from a caller-supplied value) turns either into a
one-line fixture.

---

## 4. The quoted neighbour counts do not come from the pinned report (F-5)

§11.5's prose says: *"Each `attribution-artifact` row quotes the neighbouring counts
**from the same run's report** as evidence."* That is checkable, so I checked it: 54
`N=M` pairs in the mechanism cells, compared against four candidate reports
(`/tmp/rev-goal-r2-a/provenance2.py`).

| report | what it is | goal-crate quotes | packages quotes |
|---|---|---|---|
| `/tmp/r2c-lcov.info` | **run 4 — the report the ledger pins** | 19/29 | 12/25 |
| `/tmp/goal-lcov-clean.info` | run 1, goal-only | **24/27** | — |
| `/tmp/pkgs-lcov-final.info` | run 1, packages | — | **15/25** |
| `/tmp/p1b-lcov.info` | run 3 | 17/27 | 12/25 |
| | **all four** | | 31/54 reproduce against *some* report; **23 do not** |

Examples from `/tmp/rev-goal-r2-a/quote_check.py` against the pinned report:

```
packages/app-settings/src/lib.rs:367  quoted 2,  report 23      (row cites 367=2 & 369=2)
packages/app-settings/src/lib.rs:316  quoted 8,  report 11
packages/app-workspaces/src/lib.rs:132 quoted 2, report 10
orchestration/goal/src/console.rs:5127 quoted 62, report 67
orchestration/goal/src/console.rs:4755 quoted 85, report 89
orchestration/goal/src/agents/supervision.rs:174 quoted 16, report 17
```

**This is an evidence defect, not a category defect** — and I verified the difference:
the independent neighbour audit in §6.2 shows every one of the 39 cited lines *does*
have a covered neighbour in the pinned report (0 flagged), and the row numbers
reconcile exactly (§1). So the labels stand; what does not stand is the claim that a
reader can check them against the report the ledger cites. The counts are the *earlier*
runs' numbers (the goal rows match run 1's goal-only report at 24/27, the packages
rows run 1's packages report at 15/25), i.e. they were carried across segments without
being re-read from the newest report. This is the same class as F.14 and as the
anchoring defects found in `module-agent-rpc.md` and `module-agent-core.md`: the fix is
to re-derive the 23 pairs from `/tmp/r2c-lcov.info` (or to state that the quoted run is
run 1).

---

## 5. Category cluster: three lazy-`assert!` arguments tagged `attribution-artifact` (F-6)

Scanning every row's mechanism against its category (`/tmp/rev-goal-r2-a/cat_scan.py`,
scanner (a)) finds three rows whose stated reason is *"evaluated only when the
assertion fails"* but whose category is `attribution-artifact`:

| line | the row's own words |
|---|---|
| `packages/app-settings/src/lib.rs:1050` | "1047-1051 is an `assert!` whose message argument is formatted only on failure" |
| `orchestration/goal/src/console.rs:10651` | "the lazy format argument of the §11.11 guard … formatted only on failure, which is why it carries 0 while the guard itself ran" |
| `orchestration/goal/src/compat.rs:682` | "the lazy format argument of a *passing* `assert!` (… the argument is formatted only on failure)" |

`attribution-artifact` means "a region llvm-cov attributes to a macro or a closing
brace" — a *reporting* artifact, i.e. nothing a fixture could ever help with. But these
three lines **do** have their own `DA:0` record (the audit in §6.2 reads it for each),
so llvm-cov has a counter for them; the reason it never fires is that the expression is
inside the failure arm of the assertion. That is the `unreachable-by-construction`
shape, and it is the shape this campaign already assigned to it twice: AG-A's F-29
moved six identical `assert!` argument lines **out of `coverable` and into
`unreachable-by-construction`**, and this very record labels `compat.rs:682`'s sibling
`console.rs:5318` (also a lazy failure arm) `unreachable-by-construction`. So the same
shape carries two different categories inside one ledger.

**Verdict: FELL (category).** Severity is low — the lines are waived either way — but
it is exactly the class the campaign's F-25/F-31 findings exist for ("a category is a
claim about *why* a line is uncovered, so a row whose mechanism contradicts its
category is a defect even when the line count does not move"), and the category chosen
misleads the next author about whether a fixture could help. **Invalidators:** nothing
a fixture can do — these stay unreachable until the assertion becomes
`assert_eq!`/eager formatting or the argument moves into the condition.

While here: the mechanism-vs-category scanners found **0** `coverable`/`OPEN` rows
(nothing unwaived), **0** `unreachable-in-this-environment` rows that name a platform
(the F-22/F-24 trap), and **4** `platform-unmeasured` rows that all name their platform
(§7).

---

## 6. `attribution-artifact` (10 rows / 39 lines) and the mechanism corrections

### 6.1 Two corrections inside surviving rows (F-4, F-4b)

* **`transport.rs:190-191`** — the record's reason is *"every authority-less URI … is
  rejected by tonic with `invalid URI`"*. Measured over **47** spellings
  (§10.3 probe; the record's 7 plus 40), that is not why the arm is dead:
  `http://:8080`, `http://[]`, `http://[]:80/`, `http://.`, `http://..`, `http://:`,
  `http:////`(as `://`) are **accepted** by `Endpoint::from_shared` and fail later as
  `transport error` — i.e. `from_shared` succeeded and the flow went *through* 186-194.
  The correct invariant is the weaker, true one: an accepted authority always yields
  `Some(host)` (empty string included), so `.ok_or_else(…)` at 189 never fires.
  `SPELLINGS_WITHOUT_HOST: []`.
* **`transport.rs:142-145`** — the record says the endpoint's own `connect_timeout`
  *"is armed on the same millisecond and won the race in every measurement"*. Measured
  `OUTER(142-145) 0/20  OTHER 20/20` (192.0.2.1:80, 200 ms budget, every sample the
  inner `transport error`). The reason is mechanical rather than a race: `timeout`
  polls the inner future *before* checking its own deadline, so when both deadlines
  have passed the inner's `Elapsed` is delivered first and the outer sees
  `Ok(Err(..))` at 138. That is a stronger reason than "won the race", and it points at
  the real invalidator: make the inner budget **longer** than the outer's (or drop the
  endpoint-level `connect_timeout`) and 142-145 becomes live.

### 6.2 The bulk: all 39 lines carry a covered neighbour

`/tmp/rev-goal-r2-a/artifact_audit.py` reads each cited line's own `DA`, its nearest
non-blank source neighbours up and down with theirs, and the source kind:

```
==== rows=10 artifact lines cited=39 without covered neighbour=0 unresolved=0
```

Every line is a `}` / `)?;` / multi-line macro or `json!` argument / lazy assertion
argument with a covered neighbour on the anchored statement. A representative row
(run 4's own numbers, which I re-read rather than trusting the cell):

| line | kind | neighbour evidence in `/tmp/r2c-lcov.info` |
|---|---|---|
| `app-workspaces/src/lib.rs:410` | `)?;` | 408 = 1, 412 = 1 |
| `app-workspaces/src/lib.rs:411` | `}` | 409 = 1, 412 = 1 |
| `orchestration/tasks/src/store.rs:571` | `)?;` | 569 = 3, 572 = 3 |
| `console.rs:2001` | `}` | 2002 = 493 |
| `console.rs:4570` | `}` | 4568 = 1 |
| `console.rs:6928` | `}` | 6930 = 7 (the `abort_worker_sessions` call) |
| `console.rs:10651` | lazy arg | 10649 = 1, 10660-10662 = 1 (see F-6) |
| `supervision.rs:173` | `}` | 171 = 17, 174 = 17 |
| `supervision.rs:214` | `}` | 212 = 5 (215 has no record) |
| `supervision.rs:259` | `}` | 257 = 1, 260 = 4 |
| `store.rs:1618` | `}` | 1616 = 6, 1619 = 276 |
| `validator.rs:177` | `});` | 175 = 1, 178 = 1 |
| `compat.rs:682` | lazy arg | 680 = 1 (see F-6) |
| `transport.rs:959` | `}` | 957 = 1 |

Those counts come from the pinned report (§4 explains why the *doc's* cells differ).
Also re-read as `unreachable-by-construction` rather than artifact by the doc, and
verified: `console.rs:10663` — its zero region is the NO-MATCH edge of
`if let Event::ProgressReported { .. } = reported[0]`, and the guard chain makes that
edge impossible: `reported` is built by a `filter_map` whose only arm is
`e @ Event::ProgressReported { .. } => Some(e)` (every other variant → `None`),
`assert_eq!(reported.len(), 1)` runs first, and §11.11's `assert!(matches!(…))` runs
before the destructuring. Run 4's own counts agree: 10648 = 1, 10649 = 1, 10660-10662 =
1, **10663 = 0**, 10665 = 1. **SURVIVED** (invalidator: the projection gaining a second
arm or the `assert_eq!` being removed).

---

## 7. `platform-unmeasured` — 4 rows, 11 lines: all shape (b), all named

`docs/testing/waiver-ledger.md` defines shape **(a)** (not even compiled here) and
shape **(b)** (compiled here, another platform's runtime decides) and says the
discriminator is *not* "is it `cfg`-gated". Applied:

| row | shape | platform + the condition that platform satisfies |
|---|---|---|
| `packages/rpc/src/connection.rs:427-431` | **(b)** | **Linux**: `connect(2)` to a non-socket occupant reports `ECONNREFUSED`, passing the gate at 416-419 into the `#[cfg(unix)]` check. **Measured here:** on Darwin a regular file, a directory and a FIFO *all* answer **ENOTSOCK (38)** (`ENOENT=2, ECONNREFUSED=61`), which fails that gate and takes the neighbouring 420-422 arm → 424-431 cannot run on this host. The lines are compiled *and* instrumented here (`DA:427-431 = 0`) |
| `orchestration/goal/src/console.rs:10566` | **(b)** | **Windows host**: `if cfg!(windows) { format!(r"C:\…") }` — `cfg!` is a runtime bool, so the line compiles and is counted here, and no fixture can flip the platform; the `else` one line down is covered |
| `orchestration/goal/src/compat.rs:247, 260` | **(b)** | **Windows**: each `continue` needs `delete_pending_retry` to return true, which needs `ERROR_ACCESS_DENIED` (5) on the unlink while the retry budget (64) is unspent; the non-Windows variant is a constant `false` (183-185), so the guard cannot pass here. The `#[cfg]` boundary is **not** the reason, as the row itself says |
| `orchestration/goal/src/store.rs:1496-1498` | **(b)** | **Windows**: `retryable_read_lock` is a `false` stub off Windows (1513-1517); the arm needs `ERROR_LOCK_VIOLATION` (33) on the ledger read |

I confirmed the `cfg` shapes mechanically (`grep -c 'cfg(target_os'` on the three files
returns 0 — these are `cfg!()`/runtime-fn boundaries, not attribute-gated modules) and
measured the Darwin half of the `connection.rs` row. **All four SURVIVED**, correctly
categorised, with the platform **and** the condition named. The one thing I cannot
measure from this host is the Linux/Windows side itself; that is what the row says, and
the row names the authority (the Linux/Windows CI or round 1's Windows machine) rather
than asserting a local result. Invalidators: the two `#[cfg(windows)]` bodies being
replaced by reachable-here code, or a Linux host re-running the `connection.rs` fixture.

---

## 8. The surviving rows and their invalidators

Each was re-read in the frozen source. All **SURVIVED**. (`transport.rs:672` carries
`unreachable-in-this-environment`, not `unreachable-by-construction`; it is listed here
because it is the same kind of re-read, and its category is right — the guard needs
this host to be root.)

| row | invariant re-read | token that would invalidate it |
|---|---|---|
| `console.rs:259` (`other => bail!("unknown command …")`) | **verified mechanically**: `build_cli_registry()` registers **43** ids (the record's number reproduced) and **every one of the 43 appears in a dispatch arm** (`registered-not-dispatched: []`), so the gate at 166-190 cannot admit an id the match lacks; `priority` fails inside that gate and `work-items` is a group name | a command added to the registry without a dispatch arm (the sweep would catch it) |
| `console.rs:2079` (`_ => Todo::advancement(…)`) | the `valid_combo` gate at 2012-2025 admits exactly `(agent, advancement\|monitor\|blocker\|coordination)`, `(_, user_gate)`, `(user, user_action)`; the match at 2065-2080 has an arm for each — enumerated pair by pair | a new admissible combo in `valid_combo` without an arm (or an arm removed) |
| `console.rs:5123-5126` ("completion contract changed during execution") | `validator` has no post-creation write site: `None` at `state.rs:352`, set once at creation (`console.rs:2154`, inside `cmd_todo_add`); the only other hits are `#[cfg(test)]` fixtures (`turn_envelope.rs:494`, `decision/stall.rs:176`), and no ledger event carries it — so two replays of the same ledger cannot disagree | a setter for `Todo::validator` after creation, or a migration that rewrites it |
| `console.rs:5318` (`_ => "verify-gate rejected the output"`) | **the stamp at 5153 is unconditional**: `record.failure_kind = Some(classify_failure(&record))` runs before the branch at 5172/5284, and `classify_failure` (executor.rs:114-127) returns `ScienceVerifyFailed` only for `Some(v)` with `!v.ok && status == Failed` — so the outer arm's `Some(v) if !v.ok` always matches first. (Note the field *is* serialized for replay, so the row's argument has to be the unconditional stamp, not "classify_failure is the only producer" — which is a mechanism refinement, not a fall) | `classify_failure` gaining a second `ScienceVerifyFailed` producer, or a code path that reaches this match without the 5153 stamp |
| `console.rs:6984` (`abort_worker_sessions` empty targets) | both call sites guard: `cmd_worker_stop` returns at **6903** ("no live worker session(s)") before the call at 6930, and the second caller returns at **7101** before the call at 7112 | a third call site without the guard, or the guards loosened |
| `console.rs:10663` | see §6.2 | projection shape change / guard removal |
| `webui/server.rs:288` and `:320` | **verified mechanically**: `api::overview` (439-528) contains **no `Err(` and no `?`** and its tail is `Ok(Overview { … })` (521-527); `api::events_page`'s only `?` is on `store.raw_ledger_lines` (772), whose body (919-930) returns `Ok(vec![])` or `Ok(fs::read_to_string(..).unwrap_or_default()…)` — it cannot fail, so `events_page` returns `Ok(Some(..))`/`Ok(None)`, never `Err` | a fallible call added to either callee (the compiler would still accept `server.rs`'s arm — this is why the row must be re-read, not trusted) |
| `transport.rs:197` (the `https → 443` default) | the `https && pinned` refusal returns at **184** and `!pinned` returns at **181**, so at 195-200 the scheme is never `https`; the `else` arm (199) is the only reachable one | the 184 refusal removed, or the port computed before the pinned check |
| `transport.rs:672` (test root guard) | `if std::fs::metadata("/").unwrap().uid() == geteuid() { return; }` — needs the suite to run as root | a root CI job running this crate's tests |
| `transport.rs:677` (`Ok(_) => panic!("foreign directory accepted")`) | a passing run takes `Err(e)` (674-676 asserts the message); the panic arm is the failure edge of the test's own subject | the ownership check at `unix_endpoint.rs:77-82` removed |
| `packages/app-workspaces/src/lib.rs:425` (`_ => unreachable!("the chat and unimported cases returned above")`) | `filing` is built at 419-422 as exactly `WorkspaceReused \| WorkspaceCreated`, and the other two `CwdFiling` variants returned at **397** and **412** | a fourth `CwdFiling` variant returned after 412, or a third arm in the 419 match |
| `packages/app-workspaces/src/lib.rs:793` (test `panic!`) | the fixture drives a fresh directory so the filing is `WorkspaceCreated` and the `let Some(...) else` succeeds (795-800 assert the workspace) | `file_session_for_cwd` returning another variant for a fresh directory |
| `replan_rules.rs:253` (`unreachable!("monitor_frontier_exhausted is the unconditional terminal rule")`) | `RULE_MONITOR_FRONTIER_EXHAUSTED` is in `default_rule_set().rule_ids` (**49**) and its `matching` arm (235-237) has **no guard**, so the second loop (248-252) always returns; the first loop's source is `effective_rule_ids()`, which falls back to the default set when the custom list names no known rule (69-75) | the terminal rule given a guard (`if`), removed from `default_rule_set`, or `effective_rule_ids` returning a list that lacks it **and** the default loop being removed |

---

## 9. What I could not check, and the flake note

* **UNVERIFIABLE here (stated, not hidden):** the Linux/Windows answers of the four
  `platform-unmeasured` rows (§7). The Darwin half of `connection.rs` I measured;
  the other half needs a Linux or Windows host, which is exactly what those rows name.
* **Flake note (not relied on for any verdict).** `compat.rs:235`
  (`LockState::Released => continue`) is a *flipping* row, and the record already
  handles it correctly (prose row, not a claim cell): **run 3 `/tmp/p1b-lcov.info`:
  `DA:0`; run 4 `/tmp/r2c-lcov.info`: `DA:3`; my probe run: `DA:512`**. With the STALE
  half fixed, a run where it reads 0 will print `STALE → COVERED` only if the row is
  ever moved back into a claim cell — the current shape is right, and this is a
  *report*, not a defect. My F-2 verdict does rely on a measured rate rather than a
  guaranteed hit (4/24 000 drives, `1/4000` then `3/20000`); the before-rate is the
  campaign's own `DA:157 = 0` in both recorded runs. `.future/cov100/flake-rate.sh`
  measures a *command's* failure rate, so it is not the right instrument for a
  *hit-count* rate — the two runs above are the equivalent numbers.
* **Not re-measured:** the whole-repo figure, the frozen suite, and the round-1
  sections §1-§10 of this record (out of scope: the supervisor's tripwire reads
  round 2).

---

## 10. Reproduction

Environment: the unit's own commands (`ulimit -n 10240`, rustc 1.97.0 pinned by
`rust-toolchain.toml`), but **in the scratch tree** —
`cd /tmp/rev-goal-r2-a/scratch` with `CARGO_TARGET_DIR=/tmp/rev-goal-r2-a/tgt`,
`HOME=/tmp/rev-goal-r2-a/home`. Every "AFTER" figure comes from one of these three
instrumented runs (each reads back with the toolchain's own tools, §10.4):

```bash
cargo llvm-cov -p future-goal --no-fail-fast --test rev_goal_lock_probe \
      --lcov --output-path /tmp/rev-goal-r2-a/lock-after.info -j 4
cargo llvm-cov -p future-goal --no-fail-fast --test rev_goal_watchdog_probe2 \
      --lcov --output-path /tmp/rev-goal-r2-a/watchdog-after.info -j 4
cargo llvm-cov -p future-rpc  --no-fail-fast --test rev_goal_unix_endpoint_probe \
      --test rev_goal_transport_probe \
      --lcov --output-path /tmp/rev-goal-r2-a/rpc-after.info -j 4
# plain (non-instrumented) runs for the rates and the spelling table:
cargo test -p future-rpc  --test rev_goal_unix_endpoint_probe -- --nocapture
cargo test -p future-rpc  --test rev_goal_transport_probe   -- --nocapture
cargo test -p future-goal --test rev_goal_watchdog_probe2   -- --nocapture
# the audits (repo root, read-only):
python3 /tmp/rev-goal-r2-a/setcheck.py /tmp/r2c-lcov.info
python3 /tmp/rev-goal-r2-a/artifact_audit.py /tmp/r2c-lcov.info
python3 /tmp/rev-goal-r2-a/quote_check.py /tmp/r2c-lcov.info
python3 /tmp/rev-goal-r2-a/provenance2.py
python3 /tmp/rev-goal-r2-a/cat_scan.py
```

`/tmp/rev-goal-r2-a/da.py <report> <path-suffix> <lo> <hi>` prints the `DA:` values
quoted in §3.

**This record is not a ledger, but a checker will read part of it as one.**
`rowscan.py docs/testing/review-goal.md` reports **22 rows / 29 claims** — the
`path:lines` cells of §1's per-file table and the verdict tables — which is the same
shape the shipped scanner gives the other review records. Every line they cite is a
line the pinned report still shows uncovered (`reconcile.py` on this document returns
82 mentioned / 0 MISSING / no STALE), so no sweep will see a disagreement; do not
"fix" it by moving the numbers out of the first cells, and do not read those cells as
a second ledger's claims.

### 10.1 The negative result worth keeping

The first `supervision.rs` fixture (a **wrapper script** copy named `future`,
`rev_goal_watchdog_probe.rs`, sha256 `a23475da7015689021263be2981871d2567f2a48cd795f8e2106a5b8852ef8dd`
for the file in the tree) **failed**:
`ensure_watchdog` spawns `current_exe()`, and after the wrapper `exec`s the real binary
the parent's `current_exe()` is the *real* path, so the spawned watchdog was the real
binary and `supervisor registered` printed in 0.94 s. The `ps` listing from that run
shows the real path, which is the evidence for the mechanism above. Reported because it
rules out the cheapest wrong fixture for anyone repeating this.

### 10.2 Probe sources (embedded — a hash alone does not survive the tree)

**`rev_goal_lock_probe.rs`** — `sha256 7b4d633bb9b14de1ca2ffec84390593a5b8fce02d1a9b5f99aadd26d0c59de58`

```rust
//! REV-GOAL scratch probe (NOT part of the campaign worktree).
//!
//! Question: is `orchestration/goal/src/compat.rs:270`
//! `bail!("could not acquire ACTIVE_GOAL_STATE.md.lock (contended)")`
//! reachable?  The record calls it `unreachable-in-this-environment` and states
//! the mechanism as "needs a holder that keeps the projection lock for 512
//! consecutive acquire attempts; the harness cannot hold another process's lock
//! across that loop deterministically".
//!
//! The record's mechanism assumes the only way to spin the loop is a LIVE holder.
//! But `classify_lock` also returns `Released` (file gone) and `Dead`, and both
//! `continue` without sleeping.  A *dangling symlink* at the lock path is a
//! deterministic filesystem state that:
//!   * makes `OpenOptions::create_new(true)` fail with EEXIST (AlreadyExists),
//!     which `create_is_contention` accepts, and
//!   * makes `fs::read_to_string` fail with ENOENT -> `LockState::Released`
//! so every one of the 512 attempts spins with no sleep and the loop falls out
//! at 270.  No other process, no lock holder, no race.
use std::os::unix::fs::symlink;

fn nanos() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
}

#[test]
fn dangling_symlink_lock_is_contended_reachable() {
    let dir = std::env::temp_dir().join(format!("rev-goal-lockdangling-{}", nanos()));
    std::fs::create_dir_all(&dir).unwrap();
    let lock = dir.join("ACTIVE_GOAL_STATE.md.lock");
    // A symlink whose target does not exist: O_CREAT|O_EXCL refuses (EEXIST)
    // while O_RDONLY (read_to_string) reports NotFound.
    symlink(dir.join("nowhere-at-all"), &lock).unwrap();

    // Precondition: the two syscalls really disagree about this path.
    let create_err = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .unwrap_err();
    let read_err = std::fs::read_to_string(&lock).unwrap_err();
    eprintln!(
        "PRECONDITION create_new -> kind={:?} raw={:?}; read_to_string -> kind={:?} raw={:?}",
        create_err.kind(),
        create_err.raw_os_error(),
        read_err.kind(),
        read_err.raw_os_error()
    );
    assert_eq!(create_err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(read_err.kind(), std::io::ErrorKind::NotFound);

    let t0 = std::time::Instant::now();
    let result = future_goal::compat::acquire_active_state_lock(&dir);
    let elapsed = t0.elapsed();
    match &result {
        Ok(p) => eprintln!("RESULT ok({}) in {:?}", p.display(), elapsed),
        Err(e) => eprintln!("RESULT err in {:?}: {e:#}", elapsed),
    }
    let msg = format!("{:#}", result.unwrap_err());
    eprintln!("MESSAGE: {msg}");
    assert!(
        msg.contains("contended"),
        "expected the MAX_LOCK_ATTEMPTS bail (compat.rs:270); got: {msg}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn live_holder_wait_path_for_contrast() {
    // The route the record *does* describe: a live holder bails at 240 instead.
    let dir = std::env::temp_dir().join(format!("rev-goal-locklive-{}", nanos()));
    std::fs::create_dir_all(&dir).unwrap();
    let lock = dir.join("ACTIVE_GOAL_STATE.md.lock");
    std::fs::write(&lock, format!("{}\n", std::process::id())).unwrap();
    let t0 = std::time::Instant::now();
    let msg = format!("{:#}", future_goal::compat::acquire_active_state_lock(&dir).unwrap_err());
    eprintln!("LIVE MESSAGE in {:?}: {msg}", t0.elapsed());
    assert!(msg.contains("held by pid"), "got: {msg}");
    std::fs::remove_file(&lock).ok();
    let _ = std::fs::remove_dir_all(&dir);
}
```

**`rev_goal_watchdog_probe2.rs`** — `sha256 f14648eadf27afbb8b3320194799e6436755ab80de9971609ca7185ca60af262`

```rust
//! REV-GOAL scratch probe #2 for `agents/supervision.rs:315-317`.
//!
//! `ensure_watchdog` (270-283) spawns `std::env::current_exe()` — so the only
//! thing needed for the 3 s deadline is for the FILE at that path to be a
//! staller at spawn time, while the parent's own image (already exec'd) stays the
//! real binary.  On Unix that is `unlink` + write a new file at the same path:
//! the running process keeps the old inode, and the next exec gets the stub.
//!
//! A wrapper *script* does not work (measured: `exec` makes the parent's
//! `current_exe()` the real binary, so the child was the real binary and the
//! drive succeeded in 0.94 s).  This probe measures the swap window instead:
//! for each delay T, start the parent, wait T, swap in the stub, and report
//! whether the 3 s deadline fired.
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use future_goal::state::{Goal, Todo};
use future_goal::store::{Event, Store};

fn seed(root: &str, gid: &str, cwd: &str) {
    let mut store = Store::open(root).unwrap();
    store.register(&Goal::new(gid, "watchdog stall probe", cwd)).unwrap();
    store.append(Event::GoalStarted { goal_id: gid.into(), ts: 1 }).unwrap();
    store
        .append(Event::AgentRegistered {
            goal_id: gid.into(),
            agent_id: "w1".into(),
            workspaces: vec![],
            ts: 2,
        })
        .unwrap();
    store
        .append(Event::TodoAdded { goal_id: gid.into(), todo: Todo::advancement("t1", "work"), ts: 3 })
        .unwrap();
}

fn swap_in_stub(path: &Path) {
    let ino = std::fs::metadata(path).map(|m| m.ino()).unwrap_or(0);
    std::fs::remove_file(path).unwrap();
    let mut f = std::fs::File::create(path).unwrap();
    writeln!(f, "#!/bin/sh\nexec sleep 30").unwrap();
    drop(f);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let ino2 = std::fs::metadata(path).map(|m| m.ino()).unwrap_or(0);
    println!("   swap: inode {ino} -> {ino2} (different inode = the running image survived)");
}

fn run_one(delay_ms: u64) -> (bool, Duration, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("loop-root");
    std::fs::create_dir_all(&root).unwrap();
    let root = root.to_string_lossy().into_owned();
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let gid = "goal_watchdog_stall_probe";
    seed(&root, gid, &cwd.to_string_lossy());

    let copy: PathBuf = dir.path().join("future");
    std::fs::copy(env!("CARGO_BIN_EXE_future-goal"), &copy).unwrap();
    std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o755)).unwrap();

    let child = Command::new(&copy)
        .args(["goal", "supervisor", "register", "--goal", gid, "--session-id", "sup-stall"])
        .env("FUTURE_GOAL_ROOT", &root)
        .env_remove("FUTURE_GOAL_NO_DETACH")
        .env("FUTURE_GOAL_AGENT_ADDR", "127.0.0.1:1")
        .env("FUTURE_AGENT_SOCKET", dir.path().join("agent.sock"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the copy");

    std::thread::sleep(Duration::from_millis(delay_ms));
    swap_in_stub(&copy);

    let t0 = Instant::now();
    let out = child.wait_with_output().unwrap();
    let elapsed = t0.elapsed();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let hit = stderr.contains("did not become ready within 3 seconds")
        || stdout.contains("did not become ready within 3 seconds");
    (hit, elapsed + Duration::from_millis(delay_ms), format!("{stdout}{stderr}"))
}

#[test]
fn measure_the_swap_window_that_reaches_the_deadline() {
    let mut hits = 0;
    for delay in [0u64, 30, 80, 150, 250, 400, 700] {
        let (hit, elapsed, text) = run_one(delay);
        if hit {
            hits += 1;
        }
        println!(
            "DELAY {delay:4} ms -> deadline_fired={hit:<5} elapsed={elapsed:?} :: {}",
            text.replace('\n', " | ").chars().take(160).collect::<String>()
        );
    }
    println!("SWAP-WINDOW RESULT: {hits}/7 delays reached the deadline");
}
```

### 10.3 rpc probes (embedded)

**`rev_goal_unix_endpoint_probe.rs`** — `sha256 92d08746ef029d3a4f8ddac86ce94618564654f32f9e53ed0726c3045ad796f9`

```rust
//! REV-GOAL scratch probe for `packages/rpc/src/transport/unix_endpoint.rs`
//! (NOT in the campaign worktree).
//!
//! Three rows claim `unreachable-in-this-environment`:
//!   114-116  "local endpoint lease changed while acquiring ownership" — a TOCTOU
//!            guard comparing the identity of an open fd with a fresh lstat;
//!            the record says "there is no injection seam there and a racing
//!            thread is not a deterministic fixture".
//!   157      "local endpoint changed before stale cleanup" — the same shape in
//!            `bind()`.
//!
//! The campaign's own precedent for this class is AG-A's `service/activity.rs:71`,
//! which was *covered* by a biased race fixture asserting "at least one hit in N
//! attempts".  So the test below is the same shape: a churner thread replaces the
//! file and the probe counts how many of N drives land on the guard.
//!
//! Drives go through the public API: `reserve_local_endpoint()` (which calls the
//! `pub(super) acquire`) and `LocalEndpointReservation::bind()`, with the path
//! pinned by `FUTURE_AGENT_SOCKET`.
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn nanos() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rev-goal-unixep-{tag}-{}", nanos()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Spin until told to stop, replacing `path` with a fresh file each round.
fn spawn_churner(path: PathBuf, make: fn(&Path)) -> (Arc<AtomicBool>, Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
    let stop = Arc::new(AtomicBool::new(false));
    let rounds = Arc::new(AtomicUsize::new(0));
    let (s, r, p) = (stop.clone(), rounds.clone(), path.clone());
    let h = std::thread::spawn(move || {
        while !s.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(&p);
            make(&p);
            r.fetch_add(1, Ordering::Relaxed);
        }
    });
    (stop, rounds, h)
}

fn make_plain_file(p: &Path) {
    let _ = std::fs::write(p, b"");
}

fn make_stale_socket(p: &Path) {
    if let Ok(l) = UnixListener::bind(p) {
        drop(l); // the socket file stays behind, refusing connections
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lease_identity_guard_rate() {
    let dir = tmp("lease");
    let sock = dir.join("agent.sock");
    let lease = dir.join("agent.sock.owner.lock");
    std::env::set_var("FUTURE_AGENT_SOCKET", &sock);

    // warm up: create the lease once so the churner has a file to fight over
    let _ = future_rpc::transport::reserve_local_endpoint().await;
    let (stop, rounds, h) = spawn_churner(lease.clone(), make_plain_file);

    let n = 20000;
    let (mut changed, mut occupied, mut other) = (0usize, 0usize, 0usize);
    for _ in 0..n {
        match future_rpc::transport::reserve_local_endpoint().await {
            Ok(r) => drop(r),
            Err(e) => {
                let t = e.to_string();
                if t.contains("changed while acquiring ownership") {
                    changed += 1;
                } else if t.contains("occupied") {
                    occupied += 1;
                } else {
                    other += 1;
                    if other == 1 {
                        println!("OTHER(lease): {t}");
                    }
                }
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    h.join().unwrap();
    println!(
        "LEASE  drives={n} churn_rounds={}  guard_114_116={changed}  occupied={occupied}  other={other}",
        rounds.load(Ordering::Relaxed)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stale_cleanup_identity_guard_rate() {
    let dir = tmp("stale");
    let sock = dir.join("agent.sock");
    std::env::set_var("FUTURE_AGENT_SOCKET", &sock);
    // Start from a stale socket so the first probe is the stale path.
    make_stale_socket(&sock);

    let (stop, rounds, h) = spawn_churner(sock.clone(), make_stale_socket);

    let n = 20000;
    let (mut changed, mut active, mut other) = (0usize, 0usize, 0usize);
    let t0 = Instant::now();
    for _ in 0..n {
        let res = match future_rpc::transport::reserve_local_endpoint().await {
            Ok(r) => r.bind().await,
            Err(e) => Err(e),
        };
        match res {
            Ok(incoming) => drop(incoming),
            Err(e) => {
                let t = e.to_string();
                if t.contains("changed before stale cleanup") {
                    changed += 1;
                } else if t.contains("active or cannot be classified as stale") {
                    active += 1;
                } else {
                    other += 1;
                    if other <= 3 {
                        println!("OTHER(stale): {t}");
                    }
                }
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    h.join().unwrap();
    println!(
        "STALE  drives={n} churn_rounds={}  guard_157={changed}  active={active}  other={other}  {:?}/drive",
        rounds.load(Ordering::Relaxed),
        t0.elapsed() / n as u32
    );
    let _ = Duration::from_millis(0);
}
```

**`rev_goal_transport_probe.rs`** — `sha256 ee6f86c81b5200db1f55a338702ada2818e4a14791e39b322d1ba9fa498ce769`

```rust
//! REV-GOAL scratch probe for `packages/rpc/src/transport.rs` (NOT in the campaign worktree).
//!
//! Two rows attacked:
//!
//!  * `190-191` (`unreachable-by-construction`): "the arm needs
//!    `endpoint.uri().host() == None` *after* `Endpoint::from_shared` succeeded.
//!    Measured over seven spellings: every authority-less URI is rejected by
//!    tonic with `invalid URI`".  The record sampled 7 spellings; but
//!    `AgentEndpoint::Tcp(String)` and `connection_plan` show the string comes
//!    from the *caller* (`agent.grpc_addr`), and `normalize_tcp_uri` passes any
//!    `http(s)://…` string through **unparsed**.  So the input space is large and
//!    the sample matters.  This probe walks a much wider list and reports, for
//!    every spelling, whether `connect_pinned_channel` produced the
//!    "TCP endpoint has no host" text (line 190) or something else.
//!
//!  * `142-145` (the outer `tokio::time::timeout` arm, `unreachable-in-this-
//!    environment`): measured rate, N attempts, against a blackhole address.
//!
//! Run: cargo llvm-cov -p future-rpc --test rev_goal_transport_probe -- --nocapture
use std::time::Duration;

use future_rpc::transport::connect_pinned_channel;

const SPELLINGS: &[&str] = &[
    // the seven the record measured
    "http://", "http:///x", "http://?q", "http://#f", "http://@", "http:///no-host", "http://:8080",
    // more authority-less / degenerate shapes, including unparsed pass-through
    "http://#", "http://?", "http:///#", "http:////", "http://:", "http://@/", "http://@?q",
    "http://[", "http://[]", "http://[]:80/", "http://[]:", "http://\\\\x", "http:// x",
    "http://\t", "http://\n", "http://%20", "http://.", "http://..", "http://./",
    "https://", "https://#f", "https://?q", "https://@",
    // strings that normalize_tcp_uri does NOT prefix (so they are prefixed whole)
    ":", "://x", "//", "///", "@", "?q", "#f", "[]", "[]:80", "://", "ftp://x",
    // a host that is empty but present, and host:port shapes
    "http://:0", "http://:/", "127.0.0.1:1", "localhost:1", "[::1]:1",
];

#[tokio::test]
async fn no_accepted_spelling_has_an_empty_host() {
    let mut accepted_without_host = Vec::new();
    for s in SPELLINGS {
        // Pinned: this is the only policy that reaches 186-191.
        let res = connect_pinned_channel(Some(s), Duration::from_millis(150), None).await;
        let text = match &res {
            Ok(_) => "CONNECTED".to_string(),
            Err(e) => format!("{e}"),
        };
        let no_host = text.contains("TCP endpoint has no host");
        if no_host {
            accepted_without_host.push(*s);
        }
        println!(
            "SPELLING {s:22} -> no_host={:<5} :: {}",
            no_host,
            text.replace('\n', " ").chars().take(150).collect::<String>()
        );
    }
    println!("SPELLINGS_WITHOUT_HOST: {accepted_without_host:?}");
    assert!(
        accepted_without_host.is_empty(),
        "transport.rs:190-191 reached by: {accepted_without_host:?}"
    );
}

#[tokio::test]
async fn outer_connect_budget_arm_rate() {
    // 192.0.2.1 is TEST-NET-1: nothing routes there, so the connect cannot
    // complete and some timer must fire.
    let mut outer = 0;
    let mut inner = 0;
    let n = 20;
    for _ in 0..n {
        let res = connect_pinned_channel(Some("192.0.2.1:80"), Duration::from_millis(200), None).await;
        let text = match res {
            Ok(_) => "CONNECTED".to_string(),
            Err(e) => format!("{e}"),
        };
        if text.contains("connection timed out") {
            outer += 1;
        } else {
            inner += 1;
            if inner == 1 {
                println!("INNER-SHAPED first sample: {text}");
            }
        }
    }
    println!("OUTER(142-145) {outer}/{n}  OTHER {inner}/{n}");
}
```

### 10.4 Audit tools (embedded)

**`setcheck.py`** — `sha256 393793439ac6ceb3096d2dd1db0d64d243746ba0c52562e71d9052228bd058f1`

```python
#!/usr/bin/env python3
"""REV-GOAL: does the §11.5 ledger's claimed line SET equal the report's residual set?

This is the check INCIDENTS F.14 says was done badly once (a COUNT assertion that
kept 212 rows while four line numbers were wrong).  So: compare sets, per file,
and print every difference in both directions.
"""
import re, sys

WT = "/Users/geilige/future-os/.worktrees/cov100-r2"
SCOPE = [".worktrees/cov100-r2/packages", ".worktrees/cov100-r2/orchestration"]

def lcov_uncovered(rep):
    per, cur = {}, None
    for raw in open(rep, errors="replace"):
        line = raw.rstrip("\n")
        if line.startswith("SF:"):
            sf = line[3:]
            cur = sf if any(s in sf for s in SCOPE) else None
            if cur:
                per.setdefault(cur, set())
        elif line.startswith("DA:") and cur:
            m, _, c = line[3:].partition(",")
            try:
                if int(c.split(",")[0]) == 0:
                    per[cur].add(int(m))
            except ValueError:
                pass
    return {k: v for k, v in per.items() if v}

ROW = re.compile(r"^\s*\|\s*`([A-Za-z0-9_./-]+):([0-9][0-9,\-\s]*)`\s*\|")

def claimed(doc, start, stop):
    txt = open(doc, errors="replace").read()
    i = txt.index(start); j = txt.index(stop, i)
    out = {}
    for line in txt[i:j].splitlines():
        m = ROW.match(line)
        if not m:
            continue
        path, nums = m.group(1), m.group(2)
        s = out.setdefault(path, set())
        for part in nums.split(","):
            part = part.strip()
            r = re.match(r"^(\d+)\s*-\s*(\d+)$", part)
            if r:
                s.update(range(int(r.group(1)), int(r.group(2)) + 1))
            elif part.isdigit():
                s.add(int(part))
    return out

def resolve(path, keys):
    parts = path.split("/")
    for depth in range(len(parts), 0, -1):
        suf = "/".join(parts[-depth:])
        hits = [k for k in keys if k.endswith("/" + suf) or k == suf]
        if hits:
            hits.sort(key=len)
            return hits[0]
    return None

rep = sys.argv[1] if len(sys.argv) > 1 else "/tmp/r2c-lcov.info"
doc = f"{WT}/docs/testing/module-goal.md"
unc = lcov_uncovered(rep)
claim = claimed(doc, "### 11.5 Residual lines", "### 11.6")

print(f"report: {rep}")
print(f"report residual lines: {sum(len(v) for v in unc.values())} over {len(unc)} files")
print(f"ledger claimed lines : {sum(len(v) for v in claim.values())} over {len(claim)} rows/files")

allc = set()
for p, s in claim.items():
    k = resolve(p, list(unc))
    tag = "" if k else "   <<< NO REPORT FILE"
    print(f"  {p}: {len(s)} lines{tag}")
    if k:
        allc |= {(k, n) for n in s}

allu = {(f, n) for f, s in unc.items() for n in s}
print(f"\nclaimed that resolve to a report file: {len(allc)}")
miss = sorted(allu - allc)
extra = sorted(allc - allu)
print(f"IN REPORT, NOT CLAIMED (MISSING): {len(miss)}")
for f, n in miss:
    print(f"   {f}:{n}")
print(f"CLAIMED, NOT IN REPORT (stale/extra): {len(extra)}")
for f, n in extra:
    print(f"   {f}:{n}")
print("\nSET EQUALITY:", "PASS (claimed set == residual set)" if not miss and not extra else "FAIL")
```

**`artifact_audit.py`** — `sha256 2ff2f8897b8e1ceebe0ee7694e2f2009cd7b80093ed12156754ea8807c3475ce`

```python
#!/usr/bin/env python3
"""REV-GOAL: neighbour-count audit of every `attribution-artifact` row in §11.5.

Row shape in this doc:  | `path:lines` | `category` | mechanism |
For every cited line: its own DA, the nearest non-blank source neighbours up/down
with their DA, and the source kind.  A cited line with no covered neighbour is
flagged for manual reading.
"""
import os, re, sys

WT = "/Users/geilige/future-os/.worktrees/cov100-r2"

def lcov(rep):
    per, cur = {}, None
    for raw in open(rep, errors="replace"):
        l = raw.rstrip("\n")
        if l.startswith("SF:"):
            cur = l[3:]
            per.setdefault(cur, {})
        elif l.startswith("DA:") and cur:
            m, _, c = l[3:].partition(",")
            try:
                per[cur][int(m)] = int(c.split(",")[0])
            except ValueError:
                pass
    return per

def pick(path, keys):
    """Longest-suffix match of a ledger path against the report's keys."""
    parts = path.split("/")
    for depth in range(len(parts), 0, -1):
        suf = "/".join(parts[-depth:])
        hits = [k for k in keys if k.endswith("/" + suf)]
        if hits:
            hits.sort(key=len)
            return hits[0]
    return None

def src(path):
    for cand in (f"{WT}/{path}",):
        if os.path.exists(cand):
            return open(cand, errors="replace").read().splitlines()
    return None

def kind(s):
    t = s.strip()
    if t.startswith("}"):
        return "brace"
    if t.endswith("?;") or t.endswith(")?;") or t.endswith(")?"):
        return "qtail"
    if t.endswith(","):
        return "macro-arg"
    if t.startswith(")"):
        return "brace"
    return "other"

ROW = re.compile(r"^\|\s*`([A-Za-z0-9_./-]+):([0-9][0-9,\-\s]*)`\s*\|\s*`?([a-z-]+)`?\s*\|")

rep = sys.argv[1]
cat = sys.argv[2] if len(sys.argv) > 2 else "attribution-artifact"
counts = lcov(rep)
keys = list(counts)
txt = open(f"{WT}/docs/testing/module-goal.md", errors="replace").read()
i = txt.index("### 11.5 Residual lines"); j = txt.index("### 11.6", i)
sec = txt[i:j]

tot, flagged, unresolved, matched = 0, [], 0, 0
for line in sec.splitlines():
    m = ROW.match(line)
    if not m or cat not in m.group(3):
        continue
    matched += 1
    path, nums = m.group(1), m.group(2)
    k = pick(path, keys)
    if k is None:
        unresolved += 1
        print(f"  ?? unresolved {path}"); continue
    d, s = counts[k], src(path)
    if s is None:
        print(f"  ?? no source {path}"); continue
    lines = []
    for part in nums.split(","):
        part = part.strip()
        r = re.match(r"^(\d+)\s*-\s*(\d+)$", part)
        lines += list(range(int(r.group(1)), int(r.group(2)) + 1)) if r else (
            [int(part)] if part.isdigit() else [])
    for ln in lines:
        tot += 1
        up = dn = None
        for q in range(ln - 2, max(ln - 7, 0), -1):
            if s[q - 1].strip():
                up = (q, d.get(q), s[q - 1].strip()[:44]); break
        for q in range(ln + 1, ln + 6):
            if q <= len(s) and s[q - 1].strip():
                dn = (q, d.get(q), s[q - 1].strip()[:44]); break
        kk = kind(s[ln - 1]) if ln - 1 < len(s) else "?"
        cov = any(x and (x[1] or 0) > 0 for x in (up, dn))
        if not cov:
            flagged.append((path, ln, kk, up, dn))
        print(f"  {path}:{ln} DA={d.get(ln)} {kk:9s} up={up} dn={dn}"
              f"{'' if cov else '   <<< NO COVERED NEIGHBOUR'}")
print(f"==== rows={matched} artifact lines cited={tot} without covered neighbour={len(flagged)} unresolved={unresolved}")
for f in flagged:
    print("   FLAG:", f)
```

**`quote_check.py`** — `sha256 7eddf9c717a3edb7b5d4119ebb375671e641c41c579c76ebef58c6930b3f3df1`

```python
#!/usr/bin/env python3
"""REV-GOAL: do the neighbour counts QUOTED in §11.5 reproduce against the report?

Every attribution row carries evidence of the form `316=8 & 318=8`.  That is a
checkable claim about the pinned report; INCIDENTS F.14 is precisely the class
where a ledger's numbers were regenerated without being re-read.  For each
`N=M` pair in a row's mechanism cell, compare M with the report's DA for line N
in that row's file.
"""
import os, re, sys

WT = "/Users/geilige/future-os/.worktrees/cov100-r2"

def lcov(rep):
    per, cur = {}, None
    for raw in open(rep, errors="replace"):
        l = raw.rstrip("\n")
        if l.startswith("SF:"):
            cur = l[3:]
            per.setdefault(cur, {})
        elif l.startswith("DA:") and cur:
            m, _, c = l[3:].partition(",")
            try:
                per[cur][int(m)] = int(c.split(",")[0])
            except ValueError:
                pass
    return per

def pick(path, keys):
    parts = path.split("/")
    for depth in range(len(parts), 0, -1):
        suf = "/".join(parts[-depth:])
        hits = [k for k in keys if k.endswith("/" + suf)]
        if hits:
            hits.sort(key=len)
            return hits[0]
    return None

rep = sys.argv[1]
counts = lcov(rep)
keys = list(counts)
txt = open(f"{WT}/docs/testing/module-goal.md", errors="replace").read()
i = txt.index("### 11.5 Residual lines"); j = txt.index("### 11.6", i)
sec = txt[i:j]

ROW = re.compile(r"^\|\s*`([A-Za-z0-9_./-]+):([0-9][0-9,\-\s]*)`\s*\|\s*`?([a-z-]+)`?\s*\|\s*(.*)\|")
ok = bad = norec = 0
for line in sec.splitlines():
    m = ROW.match(line)
    if not m:
        continue
    path, nums, cat, mech = m.group(1), m.group(2), m.group(3), m.group(4)
    pairs = re.findall(r"(?<![\d])(\d{1,6})\s*=\s*(\d{1,7})(?![\d])", mech)
    if not pairs:
        continue
    k = pick(path, keys)
    if k is None:
        continue
    d = counts[k]
    diffs = []
    for n, v in pairs:
        n, v = int(n), int(v)
        actual = d.get(n)
        if actual is None:
            norec += 1
            diffs.append(f"{n}=? (quoted {v})")
        elif actual != v:
            bad += 1
            diffs.append(f"{n}: quoted {v}, report {actual}")
        else:
            ok += 1
    if diffs:
        print(f"  {path}:{nums} [{cat}]")
        for x in diffs:
            print(f"      {x}")
print(f"==== quoted neighbour counts: reproduce={ok} mismatch={bad} no-record={norec}")
```

**`provenance2.py`** — `sha256 094bd07699da2db9679a05374bae0c691ee7b33a3a7dd32271f7b7c36828e05b`

```python
#!/usr/bin/env python3
"""REV-GOAL: which report do §11.5's quoted neighbour counts actually come from?

Split by crate (the unit measured `packages/*` and `orchestration/goal` in
different runs: /tmp/pkgs-lcov-final.info and /tmp/goal-lcov-clean.info are run 1's
per-crate reports; /tmp/r2c-lcov.info is the run-4 report the ledger's 82-line
header and line set are pinned to).
"""
import re, sys

WT = "/Users/geilige/future-os/.worktrees/cov100-r2"

def lcov(rep):
    per, cur = {}, None
    for raw in open(rep, errors="replace"):
        l = raw.rstrip("\n")
        if l.startswith("SF:"):
            cur = l[3:]
            per.setdefault(cur, {})
        elif l.startswith("DA:") and cur:
            m, _, c = l[3:].partition(",")
            try:
                per[cur][int(m)] = int(c.split(",")[0])
            except ValueError:
                pass
    return per

def pick(path, keys):
    parts = path.split("/")
    for depth in range(len(parts), 0, -1):
        suf = "/".join(parts[-depth:])
        hits = [k for k in keys if k.endswith("/" + suf)]
        if hits:
            hits.sort(key=len)
            return hits[0]
    return None

txt = open(f"{WT}/docs/testing/module-goal.md", errors="replace").read()
i = txt.index("### 11.5 Residual lines"); j = txt.index("### 11.6", i)
sec = txt[i:j]
ROW = re.compile(r"^\|\s*`([A-Za-z0-9_./-]+):([0-9][0-9,\-\s]*)`\s*\|\s*`?([a-z-]+)`?\s*\|\s*(.*)\|")

quotes = []   # (crate, path, line, value)
for line in sec.splitlines():
    m = ROW.match(line)
    if not m:
        continue
    path, cat, mech = m.group(1), m.group(3), m.group(4)
    if "=" not in mech:
        continue
    crate = "goal" if "orchestration/goal" in path else "packages"
    for n, v in re.findall(r"(?<![\d])(\d{1,6})\s*=\s*(\d{1,7})(?![\d])", mech):
        quotes.append((crate, path, int(n), int(v)))

print(f"quoted pairs extracted from §11.5 mechanisms: {len(quotes)}")
print("  (note: `43 = 43` in the console.rs:259 row is the registry count, not a line count)")

REPS = {"/tmp/r2c-lcov.info": "run 4 (the pinned report)",
        "/tmp/pkgs-lcov-final.info": "run 1 packages",
        "/tmp/goal-lcov-clean.info": "run 1 goal-only",
        "/tmp/p1b-lcov.info": "run 3"}
cache = {r: lcov(r) for r in REPS}

for r, label in REPS.items():
    counts = cache[r]; keys = list(counts)
    tot = ok = 0
    per_crate = {}
    for crate, path, n, v in quotes:
        if path.endswith(("console.rs", "compat.rs", "store.rs", "validator.rs",
                          "supervision.rs", "replan_rules.rs")) and crate == "goal":
            pass
        k = pick(path, keys)
        if not k or counts[k].get(n) is None:
            continue
        tot += 1
        hit = counts[k].get(n) == v
        ok += hit
        d = per_crate.setdefault(crate, [0, 0])
        d[0] += 1; d[1] += hit
    print(f"{r:32} {label:24} {ok}/{tot}  " +
          " ".join(f"{c}={h}/{t}" for c, (t, h) in sorted(per_crate.items())))
```

**`cat_scan.py`** — `sha256 7b5d10f2116a1b5e5c929f21ecb9c5506d8eafc49d750189bbaabacd9f2c17ba`

```python
#!/usr/bin/env python3
"""REV-GOAL: rows whose MECHANISM contradicts the CATEGORY they carry.

Two scanners:
 (a) a row whose mechanism says the line is a lazy `assert!`/format message
     argument ("only on failure", "lazy", "formatted only") but whose category is
     `attribution-artifact` — the shape AG-A's F-29 reclassified to
     `unreachable-by-construction`;
 (b) a `platform-unmeasured` row that does not name a platform, and an
     `unreachable-in-this-environment` row that names one (the category the
     ledger says must NOT be used for a platform boundary).
"""
import re, sys

WT = "/Users/geilige/future-os/.worktrees/cov100-r2"
txt = open(f"{WT}/docs/testing/module-goal.md", errors="replace").read()
i = txt.index("### 11.5 Residual lines"); j = txt.index("### 11.6", i)
sec = txt[i:j]
ROW = re.compile(r"^\|\s*`([A-Za-z0-9_./-]+):([0-9][0-9,\-\s]*)`\s*\|\s*`?([a-z-]+)`?\s*\|\s*(.*?)\s*\|?\s*$")

LAZY = re.compile(r"only on failure|formatted only|lazy (format|message)|not evaluated|argument of a .*assert", re.I)
PLATFORM = re.compile(r"\bwindows\b|\blinux\b|macos|darwin|platform:", re.I)

print("== (a) lazy-assert-argument mechanism under `attribution-artifact` ==")
n = 0
for line in sec.splitlines():
    m = ROW.match(line)
    if not m:
        continue
    path, nums, cat, mech = m.groups()
    if cat != "attribution-artifact" or not LAZY.search(mech):
        continue
    n += 1
    print(f"   {path}:{nums}\n      mech: {mech[:150]}")
print(f"   -> {n} row(s)")

print("\n== (b) platform-unmeasured rows and whether a platform is named ==")
for line in sec.splitlines():
    m = ROW.match(line)
    if not m:
        continue
    path, nums, cat, mech = m.groups()
    if cat == "platform-unmeasured":
        print(f"   {path}:{nums}  names_platform={bool(PLATFORM.search(mech))}"
              f"  mech: {mech[:110]}")
print("\n== (c) unreachable-in-this-environment rows that name a platform ==")
n = 0
for line in sec.splitlines():
    m = ROW.match(line)
    if not m:
        continue
    path, nums, cat, mech = m.groups()
    if cat == "unreachable-in-this-environment" and PLATFORM.search(mech):
        n += 1
        print(f"   {path}:{nums}  mech: {mech[:110]}")
print(f"   -> {n} row(s)")

print("\n== (d) category census over §11.5 rows ==")
from collections import Counter
c = Counter()
nlines = Counter()
for line in sec.splitlines():
    m = ROW.match(line)
    if not m:
        continue
    path, nums, cat, mech = m.groups()
    lines = []
    for part in nums.split(","):
        part = part.strip()
        r = re.match(r"^(\d+)\s*-\s*(\d+)$", part)
        lines += list(range(int(r.group(1)), int(r.group(2)) + 1)) if r else ([int(part)] if part.isdigit() else [])
    c[cat] += 1
    nlines[cat] += len(lines)
    if cat in ("coverable", "OPEN"):
        print("   NOT WAIVED:", path, nums, cat)
for k in sorted(c):
    print(f"   {k:34} rows={c[k]:3} lines={nlines[k]:3}")
print(f"   {'TOTAL':34} rows={sum(c.values()):3} lines={sum(nlines.values()):3}")
```

**`da.py`** — `sha256 8878e80f103a72b7e6866c533fcfdea24dc396b47c8b6289e35de6819fb51744`

```python
#!/usr/bin/env python3
"""Print DA counts for a file/line range from an lcov report."""
import sys

def show(rep, suffix, lo, hi, label):
    cur, printed = None, False
    print(f"=== {label}  {suffix} {lo}-{hi}")
    for raw in open(rep, errors="replace"):
        l = raw.rstrip("\n")
        if l.startswith("SF:"):
            cur = l[3:].endswith(suffix)
        elif l.startswith("DA:") and cur:
            m, _, c = l[3:].partition(",")
            n = int(m)
            if lo <= n <= hi:
                print(f"   {n}: {c.split(',')[0]}")
                printed = True
    if not printed:
        print("   (no DA records in range)")

if __name__ == "__main__":
    rep, suf, lo, hi = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
    label = sys.argv[5] if len(sys.argv) > 5 else rep
    show(rep, suf, lo, hi, label)
```

### 10.5 Hash verification

Every block above is byte-identical to the file in `/tmp/rev-goal-r2-a` (verified after
the last edit: `sha256(embedded block) == sha256(file)`, 10/10). The three instrumented
`lcov` reports and the raw `profdata` are **not** embedded (binary, hundreds of KB);
what reproduces without the scratch tree is the fixture (embedded), the command, and
the numbers in §3. One honest caveat: the probe files write no assertions about the
*rates* (they print them), so re-running them reproduces the shape and the hit counts,
not a fixed number of hits.

---

## 11. Next useful checks (for the owning unit / the supervisor)

1. **Close the three falls with the embedded fixtures** — `compat.rs:270` (a dangling
   symlink, 1.86 ms, deterministic), `unix_endpoint.rs:157` (the churner; accept the
   biased-race fixture the campaign already accepted for `activity.rs:71`), and
   `supervision.rs:315-317` (the copy + unlink/stub swap, 3 s, 6/7 delays). Together
   they move 5 lines from `unreachable-in-this-environment` to covered and would leave
   that category with 3 rows (142-145, 672, 114-116/129).
2. **Re-derive the 23 quoted neighbour counts** from `/tmp/r2c-lcov.info` (F-5), or
   state in §11.5 that the quoted run is run 1 — §11.5 currently promises the reader
   the pinned report.
3. **Re-categorise the three lazy-`assert!` lines** to `unreachable-by-construction`
   (F-6), which is what this campaign already decided for that shape (AG-A F-29).
4. **Re-point the campaign registry** entry for this record from `/tmp/p1b-lcov.info`
   (92 lines) to `/tmp/r2c-lcov.info` (82 lines), the report §11.5's header and line set
   belong to (§1) — otherwise the gate's STALE half reports five false rows.
5. **Correct two mechanisms that survived** (`transport.rs:142-145`: polling order, not
   a lost timer race; `transport.rs:190-191`: an accepted authority always reports
   `Some(host)`, rather than "authority-less URIs are rejected") — the corrections tell
   the next author which refactor would revive each line.
6. **Only then** does this record's ledger claim "82 residual, all 82 waived, nothing
   flagged" hold *as evidence*: after items 1-3 it would read 77 residual, 5 of them
   newly covered, 3 recategorised, and every quoted count re-derived from one report.
