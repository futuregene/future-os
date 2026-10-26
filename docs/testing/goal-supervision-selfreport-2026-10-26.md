# Supervision self-report: R1–R4 (2026-10-26)

Handoff record for the four `orchestration/goal` supervision defects found while
one TUI-optimization goal (`goal_0ddb0dbc08fd`) ran for twelve hours with a
silent supervisor. Branch: `feat/goal-supervision-selfreport`. **Nothing has been
merged or pushed** — this session was told not to, so the work is on the task
branch only (see "What is not done" at the end).

## R1 — a dispatched child stays in the caller's job

**Change.** `orchestration/goal/src/detach.rs` (new) owns how a child that must
outlive its starter is created; both spawn sites use it (`console.rs` detached
`run`, `agents/supervision.rs` `ensure_watchdog`). On Windows the flag set is an
ordered attempt list — `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS |
CREATE_BREAKAWAY_FROM_JOB`, then the same minus the breakaway bit — because a job
that does not grant `JOB_OBJECT_LIMIT_BREAKAWAY_OK` answers a breakaway attempt
with `ERROR_ACCESS_DENIED` (5). Only that error earns the retry; any other spawn
failure is reported. A degraded spawn says so once on stderr.

Unix keeps `process_group(0)`, which is what the Windows breakaway flag buys:
the child is no longer in the group a `kill -- -<pgid>` targets. `setsid` is not
used — it would need `pre_exec` and fails outright when the child is already a
group leader.

Tests: `detach::tests` (flag shape; a refused breakaway retries exactly once with
the plain flags; any other failure is not retried), plus two process-level ones —
on Unix, the parent's process group is SIGKILLed and the detached child must be
alive and in a group of its own; on Windows, a **real** Job Object with default
limits (which is the state every shell leaves behind) is joined in a helper
process, the refusal is asserted to be exactly `ERROR_ACCESS_DENIED`, and the
seam must still start the child.

Measured on this machine (worktree build, `FUTURE_GOAL_ROOT` a temp dir):
`future-goal run --goal … --agent-id w9 --model future/deepseek-flash` returns in
**0.14–0.18 s** and prints `detached run pid=…`; the worker process is a separate
process that keeps running (`worker tail` shows its tools and usage) and an
`ensure_watchdog` watcher spawned the same way has been observed alive across
several minutes and several caller timeouts.

## R2 — the watcher is re-armed on cheap paths

`supervision::rearm` probes the watch lock **first**: `Held` returns immediately
(no replay, no child, no latency), which is the normal case. Otherwise it replays
the goal, runs the observation sweep nobody else is running, spawns a watcher
(`ensure_watchdog`), and — when no watcher could be started or the lock cannot be
taken — flushes a pending note inline, the same pattern `notify_supervisor` uses.
Notes the ledger refuses are returned to the caller; a watcher that could not be
spawned is printed and dropped, so a command never fails over its sidecar.

Call sites: `goal status` (named goal, or every registered goal — after the
report is printed), `goalie worker tail`, `todo complete`, `delivery record`, and
`scheduler tick` (`notify_dead_holders` now delegates to `rearm`).

Tests (`tests/supervision_self_report_drive.rs`): a dead holder is reported and
delivered inline through a plain `status` poll with no watcher; a watcher tick
reports it by itself (`watch --once`, no supervisor command); while the lock is
held the same poll adds no note and no delivery; and, driven through a copy of
the real binary named `future`, a `status` poll **starts a watcher that had
died**. Live: 20 `status` polls against a goal with a live watcher took 0.26 s in
total (13 ms each) and the `future-goal.exe` process count stayed at 1.

## R3 — a clean completion produces a note

`supervision::record_completions` queues a `todo_done` note for every advancement
todo that reached `done` and has not been reported, deduped on
`todo_id + completed_at`. It runs from `todo complete`, from every watcher tick,
and from the R2 paths. The note says explicitly that it reports the completion
and is not a verdict: it never writes a delivery outcome, and a later
`delivery record --outcome verified` neither repeats it nor retracts it.

Two guards, both deliberate:

* **One completion, one report.** The run loop already reports the turn that
  completed a todo (`completed`, carrying the evidence pointer); a completion a
  `completed` note already covers gets no second note. This was caught by
  `agent_run_drive::run_notifies_supervisor_on_completion`, which asserts exactly
  one report per completion.
* **A 900 s catch-up window.** The sweep is a restart net for a completion whose
  process died before queueing, not a history replay: without the bound, the
  first tick after an upgrade would wake the supervisor about every advancement
  todo ever finished.

`queue` was split into `queue_many`, so a sweep with several observations reads
the ledger once instead of once per note.

Tests: `agents::supervision::tests` (batch dedup; window + idempotence; the
run-loop report suppresses a second note) and the drive (complete → exactly one
`todo_done` → record `verified` → still exactly one, note intact).

## R4 — regressions kept

`tests/supervision_self_report_drive.rs` (5 tests) covers the dead-holder chain
(queued note → inline delivery with no watcher, and the same via a watcher tick)
and the completion note. The parent-kill case lives in `detach.rs` (Unix): the
parent's process group is SIGKILLed and the detached child must survive and be
in its own group. That case is Unix-only because a Windows Job Object cannot be
terminated in the shared test harness without affecting every other test in the
process; the Windows half is covered instead by the isolated helper that joins a
real job and proves the refusal code, which is the flag's actual failure mode.

## Verification run

* `cargo test -p future-goal` — 99 test binaries, 0 failures (includes every
  pre-existing supervision drive).
* `cargo clippy -p future-goal --all-targets -- -D warnings` — clean.
* `cargo fmt -p future-goal --check` — clean.
* `make check-docs` — 337 docs OK.
* `cargo check -p future-cli` — the embedded `future goal` builds.

## What is not done / not reproducible here

* **The `+2 min` half of acceptance 1 was not reproduced.** Each dispatched
  worker finished its turn in about a minute: the todo asked for a 160–220 s
  `powershell` loop, and the worker's own shell wrapper could not run that nested
  invocation literally (the workers' receipts are what showed it), so they
  substituted shorter work and completed. The dispatch itself returned in
  0.15 s and the worker demonstrably outlived it — but "still alive at exactly
  two minutes" is unproven here. Also, a debug-build worker on this host hit the
  known Windows main-thread stack overflow (`windows-main-thread-stack-1mb`),
  unrelated to detachment; the release build ran the turns fine.
* **A Windows caller that keeps a second inheritable copy of its own caller's
  pipe still waits.** Windows children created with `bInheritHandles = TRUE`
  inherit every inheritable handle, so a worker can hold a caller's pipe open
  even after the command itself exited. `detach::stop_inheriting_our_stdio`
  clears the inherit bit on this process's own three std handles (the handle the
  leak normally runs through), but an intermediate shell that keeps its own copy
  defeats it. Closing that for good needs `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (a
  `CreateProcessW` with an allow-list of exactly the three stdio handles), which
  this change does not do. The live checks above therefore report the CLI
  process's own exit time rather than a wrapper's.
* **`grep -n`-level line references from the ticket are stale** after the edit;
  `supervision.rs` line numbers moved as code was added.
* The branch is **not merged into `dev-huichen` and not pushed**, per the task's
  redline. Finishing it means the usual flow: rebase onto `origin/dev-huichen`,
  merge `origin/main`, re-run the scoped gates, then merge into the integration
  branch from the main worktree.
