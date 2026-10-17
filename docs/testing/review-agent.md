# review-agent — independent adversarial review of the two agent ledgers (REV-AGENT)

**Scope (frozen sources, re-read at review time):**

| unit | record | ledger | measured state |
|---|---|---|---|
| **AG-A** | `docs/testing/module-agent-rpc.md` | `§0.5 Residual ledger` | **203 uncovered** lines / 43 files |
| **AG-B** | `docs/testing/module-agent-core.md` | `§6 Waiver ledger` | **213 uncovered** lines / 21 files |

**Write scope respected:** the only file this review wrote is
`docs/testing/review-agent.md`. Neither module record nor any source file was
edited (REVIEW.md rules 2 and 6: a fallen waiver is *rework for the owning unit*,
reported here as a finding; the supervisor creates the follow-up). No peer was
writing either of the two records when this review started — the worktree's only
unrelated modification was `docs/testing/module-goal.md`, from another session.

**Scratch tree (REVIEW.md rule 3/5c):** `/tmp/rev-agent-r2-a`, unique to this
segment, built as an *output* directory only: the probes there run the agents
themselves (isolated `HOME`, one free TCP port each) and never touch the campaign
worktree or `/tmp/cov-*`. The instrumented binary used for every measurement is
the campaign's own build
(`target/cov-agent-a/llvm-cov-target/debug/future-agent`, unchanged).

**This record is deliberately not a ledger.** `rowscan.py docs/testing/review-agent.md`
reports **0 shipped claims / 0 claims**, because every line number it cites sits
in prose or after a dash in a table's first cell — the campaign's convention is
that `` `path` 12, 15 `` in a first cell *asserts* those lines are uncovered, and
a review's per-line verdicts are not that kind of claim. The tool's
"STILL blind after the fix" line is therefore the expected shape here; do not
"fix" it by moving the numbers back into first cells.

---

## 0. Verdict

| # | target (record row) | claim | verdict | measured evidence |
|---|---|---|---|---|
| F-1 | `module-agent-rpc.md` `cli.rs` **590** | `unreachable-in-this-environment` (by cost) | **FELL** | `DA:590` **0 → 1** with a 5-line fixture (§3); the first `select!` arm is taken |
| F-2 | `module-agent-core.md` §6.3 (+ its F-33): *"`platform-unmeasured` is empty for this unit"* | 2 rows moved to `unreachable-in-this-environment` | **FELL** (category defect, 2 rows / 29 lines) | both are the ledger's **shape (b)** — they compile here and need another platform's runtime (§7.2) |
| F-3 | `module-agent-rpc.md` `skills/manager.rs` **1776, 1777, 1778** | `attribution-artifact` | **FELL** (category defect, 3 lines) | a never-called `flush` **body** (`1776 = 0`, `1777 = 0`); the row's own clause "the sink is flushed by the subscriber" is falsified by its own counts (§6.1) |
| F-4 | `module-agent-core.md` §6 pin | "taken from `/tmp/agb-p6.info`" | **documentation anchoring defect** | §6's own command + rows give 3 MISSING + 3 ABSENT against `agb-p6`; the same record's §9 pins `agb-p7`, which reconciles 213/213 MISSING 0 (§8) |
| F-5 | `cli.rs` 590 mechanism; `rpc/broadcaster/journal.rs` 202/203 mechanism | stated reasons | **mechanism corrected, reachability unchanged** | 590's refuted route (b) *"every unary command returns promptly"* is false (`shell` blocks up to 120 s) — it is the **drain's cancellation** that defuses it (§3.4); 202/203's reason names `to_string`, the code calls `serde_json::to_value` (§5) |
| — | `cli.rs` **591** | `unreachable-by-construction` | **SURVIVED** (with a stronger mechanism than the record's) | `DA:591 = 0` in the very run that reaches 590, and the causality is exact: the report cannot be published before `!server_running`, whose only writer runs in the same poll that completes the serve future (§3.4) |
| — | `rpc/session.rs` **3497, 3713, 4974, 5011, 5147, 5426** | `unreachable-by-construction` (`assert!` message args) | **SURVIVED** | source read at both ends of the fixture: `assert!(cond, "fmt", EXPR)` — the cited lines are the evaluated **only on failure** arguments (§4) |
| — | `rpc/broadcaster/journal.rs` **202, 203** | `unreachable-by-construction` ("the unit of the claim is the arm") | **SURVIVED as an arm**; the pair is a region-attribution artifact | the zero-count region moved 200/201 → 202/203 between `s32` and `s33` with the neighbours' counts unchanged; 4 consecutive reports now agree at 202/203 (§5) |
| — | AG-A `attribution-artifact`: 46 rows / **87 lines** | — | **84/87 SURVIVED**; the other 3 are F-3 | per-line neighbour audit with the report's own `DA:` counts (§6.1) |
| — | AG-A `platform-unmeasured`: 7 rows / **29 lines** | — | **SURVIVED** | all seven are shape **(b)**; the platform condition is named per row (§7.1) |
| — | AG-B `attribution-artifact`: 17 rows / **55 lines** | — | **SURVIVED** | anchors re-measured; the record's quoted neighbouring counts reproduce (§6.2) |
| — | AG-B sampled `unreachable-*` rows (`agent/mod.rs` 1290-1292, `llm/mod.rs` 32/36, `run_loop.rs` 478/703, `llm/mod.rs` 422/D6, `skill_reco/mod.rs` 872/1493/1626, `evidence.rs` group) | `unreachable-by-construction` / `-in-this-environment` | **SURVIVED** on the rows whose counts I could re-measure; **UNVERIFIABLE by me** on the rows whose verdict rests on a *sabotage experiment* or a Linux-only run (§9) | counts reproduce; the falsifier token is named per row |

**Falls: 5 rows / 33 lines** (F-1 1 line, F-2 29 lines, F-3 3 lines), **2
documentation defects** (F-4, plus the unattributable example in
`waiver-ledger.md` §7.2) and **2 mechanism corrections** (F-5). Everything else
attacked here — `cli.rs` 591, the six `assert!` rows, `journal.rs` 202/203, all
142 artifact lines in both ledgers, all 7 `platform-unmeasured` rows, and the
sampled AG-B `unreachable-*` rows — came back SURVIVED or SURVIVED-with-a-
correction.

---

## 1. Pairing: is each record checked against the report it pins?

Both records were re-measured against their **current** report before any tag was
attacked, because a stale pairing is the one failure that makes every other
verdict meaningless.

```
# AG-A, the report §0.8 declares current, and the section the gate reads
python3 .future/cov100/reconcile.py --report /tmp/agent-a-s36-norm.info \
  --scope $PWD/agent/src/rpc --scope $PWD/agent/src/grpc --scope $PWD/agent/src/service \
  --scope $PWD/agent/src/session --scope $PWD/agent/src/skills \
  --scope $PWD/agent/src/approval_review --scope $PWD/agent/src/cli.rs \
  --doc docs/testing/module-agent-rpc.md --section "0.5 Residual ledger" --label AG-A
```

```
[AG-A] files with uncovered lines=43 uncovered lines=203
  mentioned in module-agent-rpc.md: 203
  MISSING (uncovered, not mentioned): 0
EXIT=0            (no STALE block printed)
```

That is a clean pairing: 203 uncovered, 203 mentioned, **0 MISSING, 0 STALE**.
The §0.5 "How this ledger is checked" block still prints the *superseded*
`/tmp/agent-a-s25-norm.info` invocation — already routed as
`todo_f38668bffc51` and **not** re-reported here (tripwire on it prints 7 false
MISSING). With `s36` the ledger is clean.

AG-B is **not** clean, and it is a documentation defect rather than a row defect:

```
# §6's own pinned report (the header: "taken from /tmp/agb-p6.info")
python3 .future/cov100/reconcile.py --report /tmp/agb-p6.info \
  --scope …/agent/src/{tools,llm,agent,compaction,sandbox,config,models,runtime,utils,auth,prompt,types,skill_reco} \
  --doc docs/testing/module-agent-core.md --section "6. Waiver ledger" --label AG-B
→ 21 files, uncovered 213, mentioned 210, MISSING 3, STALE 3 (COVERED 0, ABSENT 3)
   MISSING  sandbox/mod.rs:992, sandbox/mod.rs:1010, sandbox/mod.rs:1043
   ABSENT   sandbox/mod.rs:1013, :1031, :1064
# the same record's §9 ("pass 6") and §6's table rows cite /tmp/agb-p7.info
→ 21 files, uncovered 213, mentioned 213, MISSING 0, STALE 0          (exit 0)
```

The three MISSING lines and the three ABSENT lines are the *same* three source
lines under two different report revisions (`sandbox/mod.rs` `platform_*` arms
shifted by +21 in pass 7 — the record's §6.2 says so explicitly: "These three
shifted **+21** in pass 7"), so §6's rows were written from `agb-p7` while its
header and command name `agb-p6`. Verbatim, §6's own command is red. That is the
same class as AG-A's already-filed anchoring defect and needs the same one-line
fix (point §6 at `agb-p7`, or at least say which of the two the rows came from).

---

## 2. Method

The two-step from `review-desktop.md` §5 / `review-rust.md` §R2:

1. **MEASURE the tag's own claim in a scratch tree**, before reading the
   mechanism, so the mechanism cannot prime the measurement. Every probe below
   was run in `/tmp/rev-agent-r2-a`; the numbers in this record are the runs'
   `DA:` counts, taken from the campaign's own instrumented binary with the
   toolchain's own tools (see §10).
2. **NAME the token whose change would invalidate the tag.** A tag that cannot be
   invalidated has been restated, not reviewed. Each verdict below carries that
   token.

**What this review did not do** (declared, not hidden): no whole-repo number was
re-measured; the two-flavour and frozen-suite harnesses were not run; and no
source file was edited, so the *sabotage* experiments AG-B used as falsifiers
(`run_loop.rs:703`'s policy-JSON edit, the shell-tool registration edit) were not
re-run — §9 lists exactly which verdicts rest on that and are UNVERIFIABLE here.

**A note on where the campaign's own authority lies.** `reconcile.py` is a
*mention* checker: it proves a record names every uncovered line, not that a tag
is true. Both records are green on it (AG-A on its current report, AG-B on
`agb-p7`), so nothing in §3–§7 could have been found by re-running it — they are
all tag-vs-code questions.

---

## 3. `cli.rs` 590/591 — the two adjacent statements of one `select!` arm

### 3.1 What the source fixes (all line numbers re-read from the frozen tree)

`agent/src/cli.rs` 586–599 — `async_main`'s **final `select!`, with no `biased;`**:

```
586:     let report = handle.shutdown(std::time::Duration::from_secs(4));
587:     tokio::pin!(report);
588:     tokio::select! {
589:         report = &mut report => {
590:             anyhow::ensure!(report.status == crate::service::ShutdownStatus::Stopped, "Agent failed to stop: {}", report.errors.join("; "));
591:             server.await
592:         }
593:         result = &mut server => {
594:             result?;
595:             let report = report.await;
596:             anyhow::ensure!(report.status == crate::service::ShutdownStatus::Stopped, "Agent failed to stop: {}", report.errors.join("; "));
597:             Ok(())
598:         }
599:     }
```

The fact that decides both rows is **who can publish the report, and when**:

* `handle.shutdown(d)` = `request_shutdown(d)` + `wait_for_shutdown()`
  (`service/shutdown.rs` 117–120 and 124–137). `request_shutdown` (70–115)
  spawns a **native thread** ("agent-shutdown", 99–111) that runs `drain()`,
  plus a **deadline watcher** (shutdown.rs 82–90). The `report` future is only a
  `watch` receiver: the drain runs concurrently with the select and is *not*
  polled by it.
* The drain's last waits are `wait("session writers")`, then
  `close_streams(); stop_listener();` (shutdown.rs 298–299), then
  **`wait("RPC listener and streams", || !self.shutdown.server_running.load(..))`**
  (shutdown.rs 300–303), and after that only synchronous calls remain
  (`session_manager.shutdown()` and the storage close, shutdown.rs 305–314,
  ending in the bare `errors` at 314).
* `server_running` is stored **`false` only in `ServerGuard::drop`**
  (`service/mod.rs` 316–319), and that guard is a local of `RunningAgent::serve`:
  it drops **in the same poll that returns the serve future's `Ready`**
  (`service/mod.rs` 353 — `guard.completed = server_completed;` — and the tail
  `… result` that follows it).
  `grep -rn server_running agent/src` finds exactly two writers: `store(true)`
  at `service/mod.rs:326` and that drop.
* `finish()` is **idempotent and never rewrites an existing report**
  (shutdown.rs 40–57), so every early `return errors` in `drain`
  (shutdown.rs 187, 232, 273, 303) can only fire *after* a report already exists —
  i.e. after the watcher published `FailedStopping`. `Stopped` therefore means
  "the drain reached its end", which means it observed `!server_running`.

So: **the report can be `Ready` before the serve future only via the watcher, and
the watcher only ever publishes `FailedStopping`.** That is the whole difference
between 590 (which runs whenever the first arm is taken, whatever the status) and
591 (which needs `Stopped` on the first arm).

### 3.2 Three probes, each aimed at a different route to the first arm

| probe | fixture (all: isolated `HOME`, fresh free port, subprocess agent) | result |
|---|---|---|
| **P1** `probe2.py` | one **admitted `shell` command** (`ExecuteCommand type="shell"`, `command="sleep 20"`, `shell_timeout_ms` default = `SHELL_EXECUTION_TIMEOUT` = 120 s, `packages/rpc/command_policy.rs:18`) | shell handler **is** blocked (the grpcurl call is still running after 2.5 s) — but the drain **cancels** it: the reply is `"shell command cancelled"` and the process exits **0 in 0.1 s**. No stall. |
| **P2** `probe3.py` | the route the supervisor asked about: **`models.json` + `settings.json` written for a subprocess** (exactly `agent/tests/cli_smoke.rs:583-620`'s shape) pointing at a local listener that accepts and never answers, plus `FUTURE_LLM_TIMEOUT_SECS=60` (`agent/src/llm/mod.rs:71-77`), then one `prompt` | the run **does** reach the endpoint (1 held connection, `held connections: 1`) and cannot finish — but the drain's `session.abort()` + `stop_cancellation_timer()` **cancel the run**: exit **0 in 0.0 s**. No stall. |
| **P3** `probe4.py` | `generate_session_title` (direct model call in the handler) then **`sync_future_models`** (a platform fetch, `auth.json`'s `future.base_url` pointed at the stalling listener) | `generate_session_title` returned quickly; **`sync_future_models` is still blocked after 3 s** — it is not session runtime state, so the drain cannot cancel it. Exit **130 after 5.1 s**. Drain stalled. |

The 130 is not an incident: `agent/src/cli/shutdown.rs` arms a 5 s
`SHUTDOWN_GRACE` on the first Ctrl-C and then `std::process::exit(130)` — the
force-exit watchdog fired because the shutdown had not finished at 5 s. It is
also why the *log* does not name the failure: the force exit skipped the unwind
that would have logged it. **This is exactly why the verdict rests on `DA:`
counts, not on the log** (rule 4: the covered-line evidence).

### 3.3 Measured before/after (the numbers)

Each run's own `profraw`, merged and exported with the toolchain's
`llvm-profdata`/`llvm-cov` (§10), restricted to `agent/src/cli.rs`:

| run | fixture | `590` | `591` | `593` | `594` | `595` | `596` | `597` | wall |
|---|---|---|---|---|---|---|---|---|---|
| P1 | admitted `shell sleep 20` | **0** | 0 | 1 | 1 | 1 | 1 | 1 | 0.1 s |
| P2 | `prompt` vs stalling endpoint | **0** | 0 | 1 | 1 | 1 | 1 | 1 | 0.0 s |
| **P3** | admitted `sync_future_models`, blocked | **1** | **0** | 1 | **0** | **0** | **0** | **0** | 5.1 s |

`590 = 0, 594-597 = 1` is the first arm skipped and the **second** arm taken
(`result?; report.await; ensure!; Ok(())`). `590 = 1, 591 = 0, 594-597 = 0` is the
**first** arm taken, its `ensure!` **evaluated and failing** (so `server.await`
and the whole second arm never ran). The three rows are the same binary, the same
instrumentation, and the same shutdown trigger; only the admitted command differs.

### 3.4 Verdicts

**`cli.rs:590` — FELL.** The line executes. The waiver's category
(`unreachable-in-this-environment`) claims "a host/fixture **this** machine
cannot provide"; this machine provides one in ~5 s of wall time, out of pieces the
repo already has: an isolated `HOME`, `auth.json`'s documented `future.base_url`
(`packages/rpc/proto/future.proto`'s resolution block), one `sync_future_models`
call, one SIGINT. Under REVIEW.md rule 5a the row also falls on its own terms —
its mechanism admits reachability ("*can* be reached in principle … a
cost/feasibility judgement, not a proof") while its category asserts
unreachability — and the unit says so itself, asking the supervisor to add a
"waived for fixture cost" shape to `docs/testing/waiver-ledger.md`. With this
evidence the cheapest correct closure is a **test**, not a new category: the
line is `coverable`, and the fixture is smaller than several tests already in
`agent/tests/cli_smoke.rs`.

**`cli.rs:591` — SURVIVED, and the record's own reasoning should be upgraded.**
The record's argument ("the drain has more work after the listener stage, so the
report lands strictly later") is sound but stops one step short: it leaves a
non-`biased` `select!` as the decider, and a random tie-break would then make 591
a *flake* rather than dead code. The stronger, measured reason is causal and
enumerates the failure modes:

* a `Stopped` report can only be published by the drain (the watcher publishes
  `FailedStopping`; `finish()` is idempotent);
* the drain publishes only after the `"RPC listener and streams"` wait, which
  requires `!server_running`;
* `server_running` has exactly two writers, and the `false` one is
  `ServerGuard::drop`, which runs in the same poll that makes the serve future
  `Ready` — so at the instant the report *can* become ready, the second arm is
  already ready and the select takes it (branch-order randomness cannot bite:
  the report branch is *always pending* at the start of that poll, and the select
  returns within it).

Named invalidator (the token to watch): **a `Stopped` report that does not
observe `!server_running`** — i.e. a `drain` whose listener wait is moved after
`finish`, a watcher that publishes `finish(vec![])`, or a third writer of
`server_running = false` that is not the serve future's own poll. Any of those
makes 591 live; the row should then be re-derived, not patched.

---

## 4. The six `assert!` failure-message rows in `rpc/session.rs` — SURVIVED

The record's F-29 moved `3497, 3713, 4974, 5011, 5147, 5426` out of `coverable`
into `unreachable-by-construction` on the claim that all six are expressions
**after the first comma of an `assert!`**. That is a claim about the *shape of the
statement*, and it is checkable by reading two lines instead of one:

```
3490:         assert!(
3491:             result["checkpointId"].as_str().is_some(),
3492:             "the manual compaction must commit: {result}"
3493:         );
3494:         assert!(
3495:             session.broadcaster.persistence_error().is_none(),
3496:             "a manual compaction's own lifecycle signals must not take the session's journal down: {:?}",
3497:             session.broadcaster.persistence_error()          ← the cited line
3498:         );
```

and, at the other end of the same file:

```
3707:         assert!(
3708:             result["summaryOutcome"]["fallback_reason"]
3709:                 .as_str()
3710:                 .unwrap()
3711:                 .contains("compaction provider failed"),
3712:             "{}",
3713:             result["summaryOutcome"]                            ← the cited line
3714:         );
```

Measured source shape of all six (read at the freeze):

| line | enclosing condition (read at the freeze) | cited expression | why a green suite cannot evaluate it |
|---|---|---|---|
| 3497 | `session.broadcaster.persistence_error().is_none()` (3495) | `session.broadcaster.persistence_error()` | reached only if 3495 fails |
| 3713 | `result["summaryOutcome"]["fallback_reason"]…contains("compaction provider failed")` (3708-3711) | `result["summaryOutcome"]` | reached only if the containment fails |
| 4974 | `(session.cumulative_cost_split.lock().input - 1.0).abs() < 1e-9` (4972) | `*session.cumulative_cost_split.lock()` | reached only on failure — and it **re-locks the same mutex** the condition just locked |
| 5011 | `(… .input - 101.0).abs() < 1e-9` (5009) | `*session.cumulative_cost_split.lock()` | same, "a failed cache write must not fail the session open" |
| 5147 | `(… .input - 101.0).abs() < 1e-9` (5145) | `*session.cumulative_cost_split.lock()` | same, the journal-covers-the-whole-session assertion |
| 5426 | `(*session.cumulative_cost.lock() - expected).abs() < 1e-12` (5424) | `*session.cumulative_cost.lock()` | same, the model-rates pricing assertion |

**Verdict: SURVIVED (6/6).** Rust expands `assert!(cond, fmt, args…)` to
`if !cond { panic!(fmt, args…) }`, so the cited expressions are inside the
failure arm; a green suite cannot take them, and "reaching" one would mean the
test failed. Category `unreachable-by-construction` is exact, and the mechanism
("a green suite cannot reach them") matches it — this is *stronger* than the
neighbour-count evidence most artifact rows carry. Named invalidator: an
`assert!` becoming `assert_eq!`/a plain `if`, or the argument moving *before* the
condition — either makes the line coverable by construction.

---

## 5. `rpc/broadcaster/journal.rs` 202/203 — the arm is real, the wording is not

The record's F-55 says these two numbers "move between runs with no source
change" and that "the unit of the claim is the arm, not the pair". Both halves
were checked against the campaign's reports rather than trusted:

| report | region at zero | neighbours |
|---|---|---|
| `s25`, `s30`, `s31` | (different source shape: `199 = 9, 200 = 8, 201 = 9, 203/204 = 194`) | — |
| `s32` | **200, 201** | `199 = 11919`, `202 = 11919`, `203 = 44771` |
| `s33`, `s34`, `s35`, `s36` | **202, 203** | `200/201 = 11919…11921`, `204 = 11919…11921` |

So the zero-count pair did move `200/201 → 202/203` on a segment whose only
in-scope deltas were `session_reset.rs` and `session/database/mod.rs` (AG-A's own
segment-33 note), while the neighbours' magnitudes are unchanged. The frozen
source is unambiguous about what the region *is*:

```
200:         let event_type = event.event_type.clone();
201:         let event = serde_json::to_value(event).map_err(|error| {
202:             AppendFailure::Unavailable(format!("event serialization failed: {error}"))
203:         })?;
204:         writer.append(event, !is_buffered_delta(&event_type))
```

`201` is evaluated on every append (its call is covered, `200/201 = 11921` in
`s36`), and the closure body `202/203` never is — a `.map_err` closure's
unentered region is attributed to lines inside it, which is precisely why the
pair slides. **Verdict: the arm survives; reading the pair as "these lines are
uncovered" is a documented, measured convention, not a defect.**

Two corrections (rule 7), neither changing reachability:

1. **The stated reason names the wrong function.** The row says "the event is a
   plain JSON value pair and `to_string` cannot fail". The code calls
   `serde_json::to_value(event)` on `&SseEvent` (`rpc/protocol.rs:254-268`: five
   `String`s, two `i64`s — no non-string map keys, no custom serializers), so
   `to_value` cannot fail either, but the reason should name `to_value`.
2. **The pair is currently stable, so "expect the numbers to drift" overstates
   it.** Four consecutive reports (`s33`–`s36`) agree at 202/203; the drift
   happened once (`s32` → `s33`). A reviewer should read the arm (as F-55 says)
   *and* re-read the pair from the same report the rest of the ledger cites.

Named invalidator: a second `SseEvent` shape whose serialization can fail (a map
with non-string keys, a fallible custom `Serialize`) — then the closure becomes
enterable and the arm is coverable.

---

## 6. `attribution-artifact` bulk — the neighbour audit

Both ledgers' artifact rows were audited **against the report's own `DA:` counts**
by script (embedded in §10.3), one row at a time: for each cited line, its own
count, its nearest non-blank neighbours, and the source kind (brace / `?`-tail /
macro argument). The rule applied is the ledger's own: an artifact row is only
admissible **with the enclosing statement's counts**.

### 6.1 AG-A — 46 rows / 87 lines

**84 of 87 lines SURVIVED** with the neighbour (or statement-anchor) evidence the
rows state; the remaining three are F-3 below. My script first flagged 8 lines as
"no covered neighbour": seven are artifacts of *where* a macro's anchor sits and
all seven are confirmed by the numbers the row itself quotes; the eighth
(`skills/manager.rs:1778`) is the fall.

| flagged line(s) | what the anchor is | measured (report `s36`) |
|---|---|---|
| `session.rs` — flagged 885, 886 | the `tracing::info!` **statement** at 881 | `881 = 6` (row says "runs 6 times" ✓) |
| `session/manager/mod.rs` — flagged 150 | the macro statement at 149 | `149 = 826` (row says 782; same shape, count drifted) |
| `cli.rs` 506 (the same row also cites 505) | the macro statement at 503 | `503 = 25` ✓ |
| `rpc/commands/providers.rs` — flagged 593-595 | the payload build at 591/592 | `591 = 2, 592 = 2` ✓ |
| `grpc/mod.rs` — flagged 100, 101 | the macro statement at 98 | `98 = 51` (row says 31 ✓ shape) |
| `rpc/mod.rs` — flagged 388, 390 | the retry loop's covered body | `385-387 = 8` each, `392 = 18` ✓ |

**F-3 — the one that falls: `skills/manager.rs` 1776, 1777, 1778.**

```
1770:                 return Ok(buf.len());          ← DA 1
…
1775:                 "client went away",            ← DA 2
1776:         fn flush(&mut self) -> std::io::Result<()> {   ← DA 0
1777:             Ok(())                                    ← DA 0
1778:         }                                             ← DA 0
```

The row reads: *"`flush` of a test-local `Write` double: the `write` half on the
neighbouring lines is covered and **the sink is flushed by the subscriber**; a
`flush` that returns `Ok(())` has no other input."* Two things are wrong:

* **The second clause is falsified by the row's own measurement.** Had the sink
  been flushed, `1777` (`Ok(())`, the whole body) would be `> 0`. It is `0`.
* **The category does not fit.** `attribution-artifact` means "a region llvm-cov
  attributes to a macro or a closing brace … (needs the neighbouring covered
  lines as evidence)". Here the neighbours are covered and the cited region is
  **a function that nothing calls**: llvm-cov is attributing the region exactly
  where the unexecuted code is. A never-called trait method is either
  **coverable** (the fixture can flush its double — `BufWriter::new(sink)` plus
  `flush()`, or one explicit `w.flush().unwrap()` in the test) or needs a named
  "no caller in this build" category. Either way the artifact label suppresses
  work that one line of fixture code would do.

Named invalidator: any code path in the test that flushes that sink — then the
three lines become covered and the category is moot.

### 6.2 AG-B — 17 rows / 55 lines

Same audit against `agb-p7`; **all lines SURVIVED**. The 28 lines my script first
flagged as neighbour-less are all the documented multi-line-macro shape
(`Field = expr` argument lines), and the anchors the record quotes re-measure:

| row | record's quoted anchor | measured in `agb-p7` |
|---|---|---|
| `run_loop.rs` 232/233 | 228 = 195, 229 = 8 | 228 = **199**, 229 = **8** |
| `run_loop.rs` 1053/1054 | 1052 = 219, 1055 = 219 | 1052 = **222**, 1055 = **222** |
| `run_loop.rs` 1095/1096 | 1092 = 44 | 1092 = **44** |
| `run_loop.rs` 1412/1413 | 1410 = 1, 1415 = 23 | 1410 = **1**, 1415 = **23** |
| `run_loop.rs` — flagged 1479-1484 | 1476 = 3, 1482 = 3, 1486 = 81 | 1476 = **3**, 1482 = **3**, 1486 = **82** |
| `skill_reco/mod.rs` — flagged 228 | 225 = 10, 230 = 10 | 225 = **10**, 230 = **10** |

One of these deserves the supervisor's attention because the *record itself*
says it flips: `skill_reco/mod.rs:228` is a multi-variant match-arm line that
"carried a non-zero count in the 18:01 run of the identical test set". It is the
same class as `llm/mod.rs:422` (D6, §9) and AG-A's `journal_compaction.rs:171` —
**see §9.1: those rows can turn the gate red on a different load**, now that the
STALE half sees fully-covered files.

---

## 7. `platform-unmeasured` — shape and platform, row by row

`docs/testing/waiver-ledger.md` defines two shapes: **(a)** the line is not even
*compiled* here (`#[cfg(target_os)]`), **(b)** the line compiles here but only
another platform's runtime executes it. The ledger's discriminator is explicit:
"**The discriminator is NOT 'is it `cfg`-gated'**… The question that separates a
real defect is 'is the line reachable here with a better fixture?'".

### 7.1 AG-A — 7 rows / 29 lines: all shape (b), all correctly categorised

The `cfg` sites were read (`grep -c 'cfg(target_os'` on the three files that
carry runtime branches returns **0** — these are `cfg!()`-style *runtime*
branches, not `#[cfg]` attributes):

| row (file:lines) | shape | platform condition that would execute it |
|---|---|---|
| `settings.rs` 21-24, 43-46 (8) | (b) | **Windows** bench: the only `Err` producer is `sandbox/mod.rs:1073-1075` (`#[cfg(target_os = "windows")]`); the macOS/Linux probe bodies build `Ok` unconditionally |
| `settings.rs` 33 (1) | (b) | **Windows**: needs a probe result carrying a diagnostic; off Windows the helper is `unavailable_without_error` (`sandbox/mod.rs:1202-1207`) |
| `settings.rs` 53-56 (4) | (b) | **Windows**: `reset_windows_sandbox_capabilities()` returns `Err(Unsupported)` off Windows, so the `?` propagates before the `Ok` arm |
| `settings.rs` 513-527 (13) | (b) | **Windows**: same dependency as 21-24 |
| `session.rs` 2567 (1) | (b) | **Windows** PowerShell branch: `if cfg!(windows) { "Start-Sleep -Seconds 60" } else { "sleep 60" }` (`session.rs:2566-2570`) |
| `service/instance.rs` 264 (1) | (b) | **Windows**: `if cfg!(windows) { "%COMPUTERNAME%" } else { "$(hostname)" }` (`instance.rs:262-267`) |
| `prompt_helpers.rs` 733 (1) | (b) | **Windows**: `if cfg!(windows) { PathBuf::from(r"C:\workspace") }` (`prompt_helpers.rs:731-737`) |

Nothing here is a defect. Note the two test-helper rows (`instance.rs:264`,
`prompt_helpers.rs:733`): they are in helper functions whose non-Windows arm is
covered, so the platform condition is exactly "a Windows test run", which is the
shape (b) contract.

### 7.2 AG-B — the "empty" claim is a defect (F-2)

`module-agent-core.md` §6.3 states **"`platform-unmeasured` is empty for this
unit as of pass 4"** and moves two rows into
`unreachable-in-this-environment` on the argument that "this category's
definition is 'compiles only on another OS'… and neither row is in code that
fails to compile here". That is the *old, single-shape* definition, and it is
exactly the test the current ledger says does not decide the category:

* `sandbox/file_request.rs:117` — the row itself says the needed thing is "a
  filesystem that admits byte names (**Linux**)", and quotes a measured macOS
  `EILSEQ`/`raw_os_error 92`. The line compiles here; the platform condition
  named is another OS's **capability** → **shape (b)**, `platform-unmeasured`.
* `tools/shell/execution.rs`'s helper-report block (28 lines) — the record's own
  reasoning is that the only producer of a Linux boundary is
  `#[cfg(target_os = "linux")]` (`sandbox/mod.rs:289-308`); the *lines* compile
  here and need the Linux bwrap runtime → **shape (b)**, `platform-unmeasured`.

Both rows name their missing platform and condition, so the substance is right
and only the category is wrong — but the category is the thing the gate and the
final sweep read, and the record's §6.3 headline ("empty") is a claim a reader
will quote. The one-line fix is a category move; a re-argument is not needed.

While here: `waiver-ledger.md`'s shape-(a) example ("`module-agent-core`'s
Chromium rows") is **unattributable** — `grep -i chromium
docs/testing/module-agent-core.md` returns nothing, nor do the module's files
(`agent/src/{tools,llm,agent,compaction,sandbox,config,models,runtime,utils,auth,prompt,types,skill_reco}`).
The example appears to predate the record's scope change. That file is outside
this review's write set, so it is reported, not edited.

---

## 8. AG-B: the §6 pin (F-4)

Covered in §1; the numbers are the evidence. Two details worth carrying into the
fix:

* The three MISSING/ABSENT lines are `sandbox/mod.rs` `992, 1010, 1043` (p6) vs
  `1013, 1031, 1064` (p7) — the record's own §6.2 says those rows "shifted +21 in
  pass 7", so the *rows* are from p7 and only the header/command are stale. This
  is a two-line edit, not a re-measurement.
* The record's §6 already anticipates a *second* kind of gate failure on this
  section: `llm/mod.rs:422` (D6) "flips to covered on a run where tokio's `select!`
  picks the other arm, and is then reported as STALE by design". That instruction
  is correct, but see §9.1 for what it means for the gate.

---

## 9. What I could not check, and one gate-noise finding

### 9.1 Rows documented as *flipping* will now trip the gate (report, not fall)

The STALE half of `reconcile.py` was fixed (per the supervisor's note) so a file
whose uncovered set is empty is no longer skipped. The consequence for these two
ledgers is concrete: any row that registers a **line whose coverage depends on
load or on tokio's branch randomisation** will print `STALE → COVERED (DA>0)` on
the runs where it flips, which reads like a stale row but is not one:

| record | row | what the record already says |
|---|---|---|
| AG-A | `session/journal_compaction.rs` **171** | "covered in the supervisor baseline and in this unit's first run, uncovered in the last two — a load-sensitive flip" (number is in the **claim cell**) |
| AG-B | `llm/mod.rs` **422** (D6) | "uncovered in this run, but on a run where tokio's randomized `select!` picks the other arm it reads covered… re-measure rather than 'fixing' the entry" |
| AG-B | `skill_reco/mod.rs` **228** | "the *same line carried a non-zero count in the 18:01 run of the identical test set*" (§6.4; number in the claim cell) |

AG-A already has the convention for this (`shutdown.rs` 273/303 and `session.rs`
1217 keep their numbers in the *mechanism* cell precisely so the claim cell
cannot assert them), and applying it to these three rows would make the three
known flippers invisible to the gate instead of indistinguishable from real
staleness. That is a record edit for the owning units, not a measurement
question — reported here so the supervisor can decide whether to route it.

### 9.2 UNVERIFIABLE by this review (stated, with what I would need)

* **AG-B's sabotage falsifiers.** `run_loop.rs:703` (removing `"trigger":trigger`
  from the policy JSON makes the line execute — the record quotes the measured
  error), and the shell-tool registration falsifier (§7.5/§7.7 of that record).
  Verifying them means editing the scratch source and re-running the suite
  (rule 2 forbids editing the reviewed tree; I did not build a second instrumented
  tree). I read the mechanism and the cited source; I did **not** reproduce the
  experiment. The token that would invalidate the tag is named in the record
  already (a second writer of a `checkpoint: null` receipt under this policy, or
  a changed policy JSON).
* **AG-B rows that need another platform.** `sandbox/file_request.rs`'s byte-name
  test and `linux_sandbox_smoke` are Linux-gated; the record declares them and
  the ledger's `platform-unmeasured` reasoning covers them (§7.2).
* **AG-A's `unreachable-by-construction` group (24 rows / 50 lines) and
  `unreachable-in-this-environment` group (12 rows / 32 lines)** beyond the rows
  named in this task: I verified the *mechanism text vs category* lexically
  (0 candidates of the F-24/F-25 self-contradiction class) and read the source for
  the six rows whose mechanisms name a pinning test (`rpc/mod.rs` 1514-1569,
  `session.rs` 1338, `reclamation.rs` 46/80, `entry_records.rs` 15,
  `commands/mod.rs` 404, `journal.rs` 43/45) — all six cite a guard that exists,
  which is the *opposite* of the TAU-B trap (a test named for an arm whose
  fixture is weaker than the app's contract). I did not re-run those tests.
* **Whole-suite re-measurement** of either ledger: out of scope by instruction
  (the supervisor's tripwire numbers were used for the ceilings).

---

## 10. Reproduction

### 10.1 Environment

The campaign's own instrumented binary
(`target/cov-agent-a/llvm-cov-target/debug/future-agent`, built by
`cargo llvm-cov -p future-agent`, unchanged by this review) was run directly with
`LLVM_PROFILE_FILE` pointing into the scratch tree; each probe's `profraw` was
merged and exported with the **matching** toolchain tools:

```bash
TB=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/lib/rustlib/aarch64-apple-darwin/bin
$TB/llvm-profdata merge -o p4.profdata prof/p4_52856.profraw
$TB/llvm-cov export -instr-profile=p4.profdata \
  -object=$WT/target/cov-agent-a/llvm-cov-target/debug/future-agent -format=lcov > p4.info
# then read DA: for agent/src/cli.rs 586-600   (see da.py, §10.3)
```

`llvm-cov --version` → `LLVM 22.1.6-rust-1.97.0-stable`; the same toolchain's
`llvm-cov`/`llvm-profdata` were used, so no cross-version merge was involved (the
system `xcrun` LLVM is 21.0.0 and was **not** used).

### 10.2 Probe sources (embedded — a hash alone does not survive the tree)

**P1 — `probe2.py`** (admitted `shell` command; `sha256
68a5ec0cd0e08602e51325e13aefd6926943e7db3516c6828c1728f9abfd43e4`)

```python
#!/usr/bin/env python3
"""REV-AGENT probe P1 (cli.rs:590/591).  Scratch tree /tmp/rev-agent-r2-a (rule 5c).

Fixture: an isolated-HOME future-agent whose shutdown drain is stalled by ONE
admitted `shell` command (proto.FutureAgent/ExecuteCommand type="shell",
command="sleep 20").  No model / no provider is involved.  Then SIGINT asks for
a 4 s shutdown; the deadline watcher must publish FailedStopping while the serve
future is still pending.

Question: does cli.rs:590 (`anyhow::ensure!(report.status == Stopped, ...)` in the
FIRST arm of async_main's final select) execute?  Any "Agent failed to stop:" log
line then comes from 590 or from 596 (the second arm) -- the DA counts separate
them.
"""
import json, os, shutil, signal, socket, subprocess, sys, time

D = "/tmp/rev-agent-r2-a"
WT = "/Users/geilige/future-os/.worktrees/cov100-r2"
BIN = f"{WT}/target/cov-agent-a/llvm-cov-target/debug/future-agent"
HOME_DIR = f"{D}/home-{os.getpid()}"
WORK = f"{D}/work-{os.getpid()}"

for p in (HOME_DIR, WORK, f"{D}/prof"):
    os.makedirs(p, exist_ok=True)
for f in os.listdir(f"{D}/prof"):
    os.unlink(f"{D}/prof/{f}")


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


PORT = free_port()
LOG = open(f"{D}/agent.out", "wb")
env = dict(os.environ)
env.update({
    "HOME": HOME_DIR,
    "USERPROFILE": HOME_DIR,
    "LLVM_PROFILE_FILE": f"{D}/prof/agent_%p.profraw",
})
proc = subprocess.Popen(
    [BIN, "--grpc-addr", f"127.0.0.1:{PORT}", "--verbose"],
    cwd=WORK, env=env, stdout=LOG, stderr=subprocess.STDOUT,
)
print(f"PORT={PORT} PID={proc.pid}")

# wait for the port
deadline = time.time() + 40
while time.time() < deadline:
    s = socket.socket()
    s.settimeout(0.3)
    ok = s.connect_ex(("127.0.0.1", PORT)) == 0
    s.close()
    if ok:
        break
    time.sleep(0.2)
else:
    print("PORT NEVER OPEN"); sys.exit(3)

ADDR = f"127.0.0.1:{PORT}"
# grpcurl's Go flag parser stops at the first positional argument, so every flag
# must precede the address; the address must precede the method.
PROTO = [ "grpcurl", "-plaintext",
          "-import-path", f"{WT}/packages/rpc/proto",
          "-proto", "future.proto" ]

def call(payload, headers=None, timeout=30):
    cmd = PROTO + ["-d", json.dumps(payload)]
    for h in headers or []:
        cmd += ["-H", h]
    cmd += [ADDR, "proto.FutureAgent/ExecuteCommand"]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return r.returncode, r.stdout, r.stderr

rc, out, err = call({"id": "r1", "type": "get_agent_readiness"})
print("readiness rc=", rc, out[:400], err[:200])
ready = json.loads(out)
data = ready.get("payload", {}).get("getAgentReadiness")
if data is None:
    data = json.loads(ready["data"])
INST, ROOT, VER = data["agentInstanceId"], data["dataRootId"], data["version"]
H = [f"x-future-agent-instance: {INST}",
     f"x-future-agent-data-root: {ROOT}",
     f"x-future-client-version: {VER}",
     "x-future-client-protocol: 2",
     "x-future-client-storage-schema: 6"]
print("INST=", INST, "VER=", VER)

rc, out, err = call({"id": "r2", "type": "new_session"}, H)
print("new_session rc=", rc, out[:300], err[:200])
sess = json.loads(out)
sdata = sess.get("payload", {}).get("newSession") or {}
if not sdata:
    sdata = json.loads(sess.get("data") or "{}")
SID = sdata.get("sessionId") or sdata.get("session_id") or ""
print("SID=", SID)

rc, out, err = call({"id": "r2b", "type": "set_cwd", "session_id": SID, "cwd": WORK}, H)
print("set_cwd rc=", rc, out[:200], err[:200])

# launch the blocking shell command (do not wait)
shell_cmd = PROTO + ["-d", json.dumps({"id": "r3", "type": "shell",
                                       "session_id": SID, "command": "sleep 20"})]
for h in H:
    shell_cmd += ["-H", h]
shell_cmd += [ADDR, "proto.FutureAgent/ExecuteCommand"]
shellproc = subprocess.Popen(shell_cmd, stdout=open(f"{D}/shell.json", "wb"),
                             stderr=subprocess.STDOUT)
time.sleep(2.5)
print("shell handler started:", shellproc.poll() is None)

t0 = time.time()
proc.send_signal(signal.SIGINT)
try:
    rc = proc.wait(timeout=40)
except subprocess.TimeoutExpired:
    proc.kill(); rc = "TIMEOUT"
print(f"AGENT_EXIT={rc} after {time.time()-t0:.1f}s")
shellproc.poll()
LOG.close()

tail = open(f"{D}/agent.out", errors="replace").read()
for line in tail.splitlines():
    if any(k in line for k in ("failed to stop", "Agent failed", "deadline exceeded",
                               "instance ownership", "Ctrl-C", "failed")):
        print("LOG:", line)
print("--- profraw:", os.listdir(f"{D}/prof"))
```

**P2 — `probe3.py`** (stalling endpoint + `prompt`; `sha256
d033f37817972929ae8503fcbb3bda4620a56996d6df657b5f7de09ce07e4080`)

```python
#!/usr/bin/env python3
"""REV-AGENT probe P2 (cli.rs:590).  Scratch tree /tmp/rev-agent-r2-a (rule 5c).

The cheap-fixture route the record says does not exist: an isolated HOME whose
model config (`models.json` + `settings.json`, written exactly the way
agent/tests/cli_smoke.rs:583-620 already writes them for a SUBPROCESS agent)
points at a local TCP endpoint that accepts and never answers.  One `prompt`
starts a run that cannot finish; SIGINT then asks for a 4 s shutdown.

Measure: does the process die with "Agent failed to stop" (cli.rs:590 or :596),
and did the drain stall (wall time ~4 s) rather than complete?
"""
import json, os, signal, socket, subprocess, sys, threading, time

D = "/tmp/rev-agent-r2-a"
WT = "/Users/geilige/future-os/.worktrees/cov100-r2"
BIN = f"{WT}/target/cov-agent-a/llvm-cov-target/debug/future-agent"
pid = os.getpid()
HOME_DIR = f"{D}/p2home-{pid}"
WORK = f"{D}/p2work-{pid}"
for p in (HOME_DIR, WORK):
    os.makedirs(p, exist_ok=True)
agent_dir = f"{HOME_DIR}/.future/agent"
os.makedirs(agent_dir, exist_ok=True)

# ── the stalling endpoint: accept, read nothing, never answer ───────────────
srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", 0))
srv.listen(8)
STALL_PORT = srv.getsockname()[1]
HOLD = []
def staller():
    while True:
        try:
            c, _ = srv.accept()
        except OSError:
            return
        HOLD.append(c)          # keep the connection open, never reply
threading.Thread(target=staller, daemon=True).start()
print("stall endpoint 127.0.0.1:%d" % STALL_PORT)

models_cfg = {
    "providers": {
        "custom": {
            "api": "openai-completions",
            "baseUrl": f"http://127.0.0.1:{STALL_PORT}/v1",
            "apiKey": "sk-custom",
            "models": [
                {"id": "stall-model", "limit": {"context": 64000, "output": 4096}}
            ],
        }
    }
}
open(f"{agent_dir}/models.json", "w").write(json.dumps(models_cfg))
open(f"{agent_dir}/settings.json", "w").write(
    json.dumps({"defaultModel": "custom/stall-model", "maxTurns": 9}))

def free_port():
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p

PORT = free_port()
LOG = open(f"{D}/agent-p2.out", "wb")
env = dict(os.environ)
env.update({"HOME": HOME_DIR, "USERPROFILE": HOME_DIR,
            "LLVM_PROFILE_FILE": f"{D}/prof/p2_%p.profraw",
            "FUTURE_LLM_TIMEOUT_SECS": "60"})
proc = subprocess.Popen([BIN, "--grpc-addr", f"127.0.0.1:{PORT}", "--verbose"],
                        cwd=WORK, env=env, stdout=LOG, stderr=subprocess.STDOUT)
print("PORT=%d PID=%d" % (PORT, proc.pid))

deadline = time.time() + 40
while time.time() < deadline:
    s = socket.socket(); s.settimeout(0.3)
    ok = s.connect_ex(("127.0.0.1", PORT)) == 0; s.close()
    if ok: break
    time.sleep(0.2)
else:
    print("PORT NEVER OPEN"); sys.exit(3)

ADDR = f"127.0.0.1:{PORT}"
BASE = ["grpcurl", "-plaintext", "-import-path", f"{WT}/packages/rpc/proto",
        "-proto", "future.proto"]

def call(payload, headers=None, timeout=60, outfile=None):
    cmd = BASE + ["-d", json.dumps(payload)]
    for h in headers or []: cmd += ["-H", h]
    cmd += [ADDR, "proto.FutureAgent/ExecuteCommand"]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if outfile: open(outfile, "w").write(r.stdout + r.stderr)
    return r.returncode, r.stdout, r.stderr

rc, out, err = call({"id": "r1", "type": "get_agent_readiness"})
data = json.loads(out)["payload"]["getAgentReadiness"]
H = [f"x-future-agent-instance: {data['agentInstanceId']}",
     f"x-future-agent-data-root: {data['dataRootId']}",
     f"x-future-client-version: {data['version']}",
     "x-future-client-protocol: 2", "x-future-client-storage-schema: 6"]

rc, out, err = call({"id": "r2", "type": "new_session"}, H)
SID = json.loads(json.loads(out)["data"])["sessionId"]
call({"id": "r2b", "type": "set_cwd", "session_id": SID, "cwd": WORK}, H)
print("SID=", SID)

rc, out, err = call({"id": "r3", "type": "prompt", "session_id": SID,
                     "message": "hello"}, H)
print("prompt rc=", rc, out[:200])
time.sleep(3)

t0 = time.time()
proc.send_signal(signal.SIGINT)
try:
    rc = proc.wait(timeout=60); elapsed = time.time() - t0
except subprocess.TimeoutExpired:
    proc.kill(); rc = "TIMEOUT"; elapsed = time.time() - t0
print("AGENT_EXIT=%s after %.1fs" % (rc, elapsed))
LOG.close()
txt = open(f"{D}/agent-p2.out", errors="replace").read()
for line in txt.splitlines():
    if any(k in line for k in ("failed to stop", "Agent failed", "deadline exceeded",
                               "instance ownership", "Ctrl-C", "profile", "stop")):
        print("LOG:", line)
print("held connections:", len(HOLD))
```

**P3 — `probe4.py`** (the decisive one; `sha256
92ae6006efa62e5dfd57187fe0a5dec6445d7ac588ac1829b81e0eed46c737ec`)

```python
#!/usr/bin/env python3
"""REV-AGENT probe P3 (cli.rs:590).  Scratch tree /tmp/rev-agent-r2-a (rule 5c).

P2 showed the drain CANCELS both an admitted `shell` command and an unresolved
prompt run, so neither stalls it.  P3 tries admitted commands the drain does NOT
cancel (they are not session runtime state): `generate_session_title` (a direct
model call in the handler) and `sync_future_models` (a platform fetch), both
pointed at a local endpoint that accepts and never answers.

Everything printed (agent log + DA counts) is measured, not asserted.
"""
import json, os, signal, socket, subprocess, sys, threading, time

D = "/tmp/rev-agent-r2-a"
WT = "/Users/geilige/future-os/.worktrees/cov100-r2"
BIN = f"{WT}/target/cov-agent-a/llvm-cov-target/debug/future-agent"
pid = os.getpid()
HOME_DIR = f"{D}/p3home-{pid}"
WORK = f"{D}/p3work-{pid}"
for p in (HOME_DIR, WORK):
    os.makedirs(p, exist_ok=True)
agent_dir = f"{HOME_DIR}/.future/agent"
os.makedirs(agent_dir, exist_ok=True)

srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", 0)); srv.listen(8)
STALL = srv.getsockname()[1]
HOLD = []
def staller():
    while True:
        try: c, _ = srv.accept()
        except OSError: return
        HOLD.append(c)
threading.Thread(target=staller, daemon=True).start()
print("stall endpoint 127.0.0.1:%d" % STALL)

models_cfg = {"providers": {"custom": {
    "api": "openai-completions",
    "baseUrl": f"http://127.0.0.1:{STALL}/v1",
    "apiKey": "sk-custom",
    "models": [{"id": "stall-model", "limit": {"context": 64000, "output": 4096}}]}}}
open(f"{agent_dir}/models.json", "w").write(json.dumps(models_cfg))
open(f"{agent_dir}/settings.json", "w").write(
    json.dumps({"defaultModel": "custom/stall-model", "maxTurns": 9}))
# platform root for the Future catalogue fetch (the model API base is {root}/api)
open(f"{agent_dir}/auth.json", "w").write(json.dumps(
    {"future": {"type": "api_key", "key": "k",
                "base_url": f"http://127.0.0.1:{STALL}"}}))

s = socket.socket(); s.bind(("127.0.0.1", 0)); PORT = s.getsockname()[1]; s.close()
LOG = open(f"{D}/agent-p4.out", "wb")
env = dict(os.environ)
env.update({"HOME": HOME_DIR, "USERPROFILE": HOME_DIR,
            "LLVM_PROFILE_FILE": f"{D}/prof/p4_%p.profraw",
            "FUTURE_LLM_TIMEOUT_SECS": "60"})
proc = subprocess.Popen([BIN, "--grpc-addr", f"127.0.0.1:{PORT}", "--verbose"],
                        cwd=WORK, env=env, stdout=LOG, stderr=subprocess.STDOUT)
print("PORT=%d PID=%d" % (PORT, proc.pid))
deadline = time.time() + 40
while time.time() < deadline:
    s = socket.socket(); s.settimeout(0.3)
    ok = s.connect_ex(("127.0.0.1", PORT)) == 0; s.close()
    if ok: break
    time.sleep(0.2)
else:
    print("PORT NEVER OPEN"); sys.exit(3)

ADDR = f"127.0.0.1:{PORT}"
BASE = ["grpcurl", "-plaintext", "-import-path", f"{WT}/packages/rpc/proto",
        "-proto", "future.proto"]

def cmdline(payload, headers):
    c = BASE + ["-d", json.dumps(payload)]
    for h in headers: c += ["-H", h]
    return c + [ADDR, "proto.FutureAgent/ExecuteCommand"]

def call(payload, headers, timeout=90):
    r = subprocess.run(cmdline(payload, headers), capture_output=True, text=True,
                       timeout=timeout)
    return r.returncode, (r.stdout + r.stderr)

rc, out = call({"id": "r1", "type": "get_agent_readiness"}, [])
data = json.loads(out)["payload"]["getAgentReadiness"]
H = [f"x-future-agent-instance: {data['agentInstanceId']}",
     f"x-future-agent-data-root: {data['dataRootId']}",
     f"x-future-client-version: {data['version']}",
     "x-future-client-protocol: 2", "x-future-client-storage-schema: 6"]
rc, out = call({"id": "r2", "type": "new_session"}, H)
SID = json.loads(json.loads(out)["data"])["sessionId"]
call({"id": "r2b", "type": "set_cwd", "session_id": SID, "cwd": WORK}, H)
print("SID=", SID)

blocker = None
for name, payload in [
    ("generate_session_title", {"id": "t1", "type": "generate_session_title",
                                "session_id": SID}),
    ("sync_future_models", {"id": "t2", "type": "sync_future_models"}),
    ("install_skill", {"id": "t3", "type": "install_skill",
                       "skill_id": "x", "session_id": SID}),
]:
    p = subprocess.Popen(cmdline(payload, H), stdout=open(f"{D}/{name}.out", "wb"),
                         stderr=subprocess.STDOUT)
    time.sleep(3.0)
    alive = p.poll() is None
    print(f"{name}: still blocked after 3s = {alive}")
    if alive:
        blocker = (name, p)
        break
    p.wait()

if blocker is None:
    print("NO BLOCKING COMMAND FOUND")
    t0 = time.time(); proc.send_signal(signal.SIGINT); proc.wait(timeout=30)
    print("AGENT_EXIT (no stall) after %.1fs" % (time.time() - t0))
    LOG.close(); sys.exit(0)

name, p = blocker
print("using blocker:", name)
t0 = time.time()
proc.send_signal(signal.SIGINT)
try:
    rc = proc.wait(timeout=60); el = time.time() - t0
except subprocess.TimeoutExpired:
    proc.kill(); rc = "TIMEOUT"; el = time.time() - t0
print("AGENT_EXIT=%s after %.1fs  (drain stall?)" % (rc, el))
LOG.close()
for line in open(f"{D}/agent-p4.out", errors="replace").read().splitlines():
    if any(k in line for k in ("failed to stop", "Agent failed", "deadline exceeded",
                               "instance ownership", "Ctrl-C", "exited with error")):
        print("LOG:", line)
print("held connections:", len(HOLD))
print("blocker exit:", p.poll() if p.poll() is not None else "still running")
```

### 10.3 Audit tools (embedded)

`da.py` (`sha256 70ad06d84da04286e4ed4101d2c8cd59347131b6b32fc70e013d5e9fd592af96`)
— print `DA:` counts for cited lines from `agent-a-s36-norm.info` (scope `A`) or
`agb-p7.info` (scope `B`); paths are normalised to `agent/src/…` on both sides,
the same normalisation `reconcile.py` applies.

```python
#!/usr/bin/env python3
"""Print DA counts for cited lines from the two authoritative reports."""
import sys
WT = "/Users/geilige/future-os/.worktrees/cov100-r2"

def lcov(path, prefixes):
    per, cur = {}, None
    for raw in open(path, errors="replace"):
        line = raw.rstrip("\n")
        if line.startswith("SF:"):
            sf = line[3:]
            if "agent/src/" in sf:
                sf = "agent/src/" + sf.split("agent/src/", 1)[1]
            cur = sf if any(sf.startswith(p) for p in prefixes) else None
            if cur and cur not in per:
                per[cur] = {}
        elif line.startswith("DA:") and cur:
            m, _, c = line[3:].partition(",")
            try:
                per[cur][int(m)] = int(c.split(",")[0])
            except ValueError:
                pass
    return per

R = {"A": ("/tmp/agent-a-s36-norm.info", ["agent/src/rpc", "agent/src/grpc",
        "agent/src/service", "agent/src/session", "agent/src/skills",
        "agent/src/approval_review", "agent/src/cli.rs"]),
     "B": ("/tmp/agb-p7.info", ["agent/src/tools", "agent/src/llm", "agent/src/agent",
        "agent/src/compaction", "agent/src/sandbox", "agent/src/config",
        "agent/src/models", "agent/src/runtime", "agent/src/utils", "agent/src/auth",
        "agent/src/prompt", "agent/src/types", "agent/src/skill_reco"])}

def show(which, sub, lines):
    rep, pref = R[which]
    per = lcov(rep, pref)
    key = None
    for k in per:
        if k.endswith(sub):
            key = k
    if key is None:
        print("no file for", sub); return
    print(f"== {key}")
    for ln in lines:
        print(f"   {ln}: {per[key].get(ln, 'no record')}")

if __name__ == "__main__":
    which = sys.argv[1]
    sub = sys.argv[2]
    lines = [int(x) for x in sys.argv[3:]]
    show(which, sub, lines)
```

`artifact_audit.py` (`sha256
c9fc6e4009a78ca15bcfb67f95b7498829acf4f187aba8858554e82724123162`) — the
`attribution-artifact` neighbour audit of §6; it strips the report path prefix
(observed: the normalised reports can carry `/Users/geilige/future-os/`), resolves
each ledger row's relative path by longest-suffix match *exactly as
`reconcile.py`'s `claim_targets` does*, and prints for every cited line
`DA=…`, its source kind, and its nearest non-blank neighbours up/down.

```python
#!/usr/bin/env python3
"""REV-AGENT: neighbour audit of `attribution-artifact` rows (v2).

Row paths in a ledger are written relative (`rpc/mod.rs`, `broadcast/journal.rs`),
so each is resolved by longest-suffix match against the report's keys, exactly as
`.future/cov100/reconcile.py` does.  Only rows in the named ledger section count.
"""
import re, sys, os

WT = "/Users/geilige/future-os/.worktrees/cov100-r2"

def lcov(path, prefixes):
    per, cur = {}, None
    for raw in open(path, errors="replace"):
        line = raw.rstrip("\n")
        if line.startswith("SF:"):
            sf = line[3:].replace(WT + "/", "")
            if "agent/src/" in sf:
                sf = "agent/src/" + sf.split("agent/src/", 1)[1]
            cur = sf if any(sf.startswith(p) for p in prefixes) else None
            if cur and cur not in per:
                per[cur] = {}
        elif line.startswith("DA:") and cur:
            m, _, c = line[3:].partition(",")
            try:
                per[cur][int(m)] = int(c.split(",")[0])
            except ValueError:
                pass
    return per

ROW = re.compile(r"^\s*\|\s*`?([A-Za-z0-9_./-]+\.rs)`?\s*:?\s*`?([0-9][0-9,\-\s]*?)`?\s*\|\s*([0-9—]+)\s*\|\s*\**\s*([a-z-]+)")

def section(doc, needle, stop):
    txt = open(doc, errors="replace").read()
    i = txt.index(needle)
    j = txt.index(stop, i)
    return txt[i:j]

def rows(sec, cat):
    out = []
    for line in sec.splitlines():
        m = ROW.match(line)
        if not m:
            continue
        if cat not in m.group(4):
            continue
        nums = []
        for part in m.group(2).split(","):
            part = part.strip()
            r = re.match(r"^(\d+)\s*-\s*(\d+)$", part)
            if r:
                nums += list(range(int(r.group(1)), int(r.group(2)) + 1))
            elif part.isdigit():
                nums.append(int(part))
        out.append((m.group(1), nums))
    return out

def resolve(path, keys):
    parts = path.split("/")
    for depth in range(len(parts), 0, -1):
        suf = "/".join(parts[-depth:])
        hits = [k for k in keys if k == suf or k.endswith("/" + suf)]
        if hits:
            hits.sort(key=len)
            return hits[0]
    return None

def src_of(path):
    for cand in (os.path.join(WT, "agent/src", path), os.path.join(WT, path)):
        if os.path.exists(cand):
            return open(cand, errors="replace").read().splitlines()
    return None

def kind(s):
    t = s.strip()
    if t in ("}", "};", ");", ")", "};") or t.startswith("}"):
        return "brace"
    if t.endswith("?;") or t.endswith(")?;") or t.endswith(")?"):
        return "qtail"
    if t.endswith(","):
        return "macro-arg"
    if t.startswith(")"):
        return "brace"
    return "other"

def audit(doc, report, prefixes, cat, needle, stop, label):
    counts = lcov(report, prefixes)
    keys = list(counts)
    sec = section(doc, needle, stop)
    tot = 0
    bad = []
    for path, lines in rows(sec, cat):
        key = resolve(path, keys)
        if key is None:
            print(f"  ?? unresolved {path}")
            continue
        d = counts[key]
        src = src_of(path)
        for ln in lines:
            tot += 1
            nb = na = None
            if src:
                for j in range(ln - 2, max(ln - 6, 0), -1):
                    if src[j - 1].strip():
                        nb = (j, d.get(j), src[j - 1].strip()[:52]); break
                for j in range(ln + 1, ln + 5):
                    if j <= len(src) and src[j - 1].strip():
                        na = (j, d.get(j), src[j - 1].strip()[:52]); break
            k = kind(src[ln - 1]) if src and ln - 1 < len(src) else "?"
            covered_nbr = any(x and (x[1] or 0) > 0 for x in (nb, na))
            flag = "" if covered_nbr else "   <<< NO COVERED NEIGHBOUR"
            if not covered_nbr:
                bad.append((path, ln, k, nb, na))
            print(f"  {path}:{ln} DA={d.get(ln)} {k:9s} up={nb} dn={na}{flag}")
    print(f"  ==== {label}: {tot} artifact lines cited, {len(bad)} with no covered neighbour")
    for b in bad:
        print("     UNSUPPORTED:", b)

if __name__ == "__main__":
    A_PREFIX = ["agent/src/rpc", "agent/src/grpc", "agent/src/service", "agent/src/session",
                "agent/src/skills", "agent/src/approval_review", "agent/src/cli.rs"]
    B_PREFIX = ["agent/src/tools", "agent/src/llm", "agent/src/agent", "agent/src/compaction",
                "agent/src/sandbox", "agent/src/config", "agent/src/models", "agent/src/runtime",
                "agent/src/utils", "agent/src/auth", "agent/src/prompt", "agent/src/types",
                "agent/src/skill_reco"]
    w = sys.argv[1] if len(sys.argv) > 1 else "A"
    if w == "A":
        audit(f"{WT}/docs/testing/module-agent-rpc.md", "/tmp/agent-a-s36-norm.info", A_PREFIX,
              "attribution-artifact", "### 0.5 Residual ledger", "### 0.6", "AG-A")
    else:
        audit(f"{WT}/docs/testing/module-agent-core.md", "/tmp/agb-p7.info", B_PREFIX,
              "attribution-artifact", "## 6. Waiver ledger", "### 6.1", "AG-B(table)")
        audit(f"{WT}/docs/testing/module-agent-core.md", "/tmp/agb-p7.info", B_PREFIX,
              "attribution-artifact", "### 6.4", "### 6.5", "AG-B(6.4)")
```

### 10.4 Hash verification

All five fenced blocks above are **byte-identical** to the files that produced the
measurements (the supervisor can hash the block as pasted and get the number
below; verified after the last edit to this record, `sha256` of the embedded
block == `sha256` of the file, 5/5 MATCH):

| probe / tool | `sha256` |
|---|---|
| `probe2.py` (P1) | `68a5ec0cd0e08602e51325e13aefd6926943e7db3516c6828c1728f9abfd43e4` |
| `probe3.py` (P2) | `d033f37817972929ae8503fcbb3bda4620a56996d6df657b5f7de09ce07e4080` |
| `probe4.py` (P3) | `92ae6006efa62e5dfd57187fe0a5dec6445d7ac588ac1829b81e0eed46c737ec` |
| `da.py` | `70ad06d84da04286e4ed4101d2c8cd59347131b6b32fc70e013d5e9fd592af96` |
| `artifact_audit.py` | `c9fc6e4009a78ca15bcfb67f95b7498829acf4f187aba8858554e82724123162` |

Two honest limits: (1) the `profraw`s the verdicts rest on live in
`/tmp/rev-agent-r2-a/prof/` and are **not** embedded — each is a multi-hundred-KB
binary, so what is reproducible without the scratch tree is the *fixture*
(embedded) and the *readback command* (§10.1), not the binary profile; (2) the
values in §3.3 are the `DA:` counts of those profiles, and re-running the probes
will reproduce the *patterns* (`590 = 1` only in P3) but the neighbouring counts
in the same table are per-run and will differ in the last digit.

---

## 11. Next useful checks (for the owning units / the supervisor)

1. **Turn F-1 into the test it deserves** (AG-A or a follow-up unit): an
   integration test in `agent/tests/` that points `auth.json`'s `future.base_url`
   at a local stalling listener, issues `sync_future_models`, sends SIGINT, and
   asserts the process reports the drain's stage — five seconds of wall time,
   no model, no provider. That closes `cli.rs:590` and turns the pair into
   "covered + unreachable-by-construction", which is what the ledger already
   claims for 591.
2. **Fix the three category items** (F-2, F-3) and the two anchoring items (F-4
   and the `waiver-ledger.md` Chromium example). Each is a one-to-three-line
   edit; none needs a re-measurement.
3. **Decide the flipping-row convention** (§9.1): three rows across the two
   records register lines whose coverage depends on load or on tokio's branch
   randomisation, and with the STALE half fixed they will print `COVERED` on the
   runs where they flip. AG-A already has the mechanism-cell convention for
   exactly this; applying it (or a `flip` marker) would keep the gate's COVERED
   list meaningful.
4. **If a peer wants the remaining depth:** the two things this review did not
   reproduce are AG-B's sabotage falsifiers (`run_loop.rs:703` and the shell-tool
   registration) and AG-A's remaining **~33** non-artifact `unreachable-*` rows
   (43 non-artifact rows, minus the 10 attacked here: `cli.rs` 590/591, the six
   `assert!` lines, `journal.rs` 202/203 and the 7 `platform-unmeasured` rows).
   Both need an *edited* scratch build, which REVIEW.md rule 2 reserves for a
   unit that is not also the writer of the record under review.
