# Flake elimination workstream — brief (every flake unit must read this first)

Goal: **every flaky test in this repository is either fixed with measured
before/after rates, or shown not to be flaky at all. A fix without a rate is not
a result, and a green run after a change is not proof of anything.

This is a separate workstream from the coverage units, with its own evidence
standard, but it shares the same worktree, the same environment rules
(`.future/cov100/PLAN.md` §2, §3) and the same prohibition on state-changing git
commands.

## 1. What counts as a flake

A test whose **outcome depends on something other than the code under test**:
host timing, load, wall-clock date, intra-process lock/singleton contention,
shared temp paths, Mock-server accept races, an unordered iteration. Distinguish
two shapes, they need different evidence:

* **Probabilistic** — fails on some runs, passes on others with the same code.
  Evidence = a rate over N runs (before and after).
* **Order/topology-dependent** — deterministic given a particular *set* of tests
  or a particular run mode (e.g. two test groups in one process deadlock; a suite
  in parallel deadlocks but serially passes). Evidence = the exact failing
  invocation, which must fail before and pass after (a negative control).

## 2. The harness

```bash
../.future/cov100/flake-rate.sh --label <name> --runs 30 --timeout 900 -- <command...>
# prints: runs=N failures=F timeouts=T, then each distinct failing test with how
# many runs it appeared in, then the first panic line per failing run.
# logs land in $TMPDIR/flake-<name>/ (one file per run).
```

Always run the *same* command before and after your change, from the same
directory, with the same `-j`. Rates measured on a machine running nine other
workers are noisy — say so in the record, and prefer a **rate of 0 in N runs**
over a comparison of two small rates.

The suite's own iteration loop for Rust is `timeout 300 cargo test -p <crate> …`
(the timeout is what turns a deadlock into a datum instead of a hang).

## 3. The protocol (in order, no shortcuts)

1. **Reproduce first, on an unmodified tree.** Record the invocation and the
   rate. If you cannot reproduce it in N ≥ 10 runs, say so — "could not
   reproduce in 10 runs, mechanism X is still visible in the code at
   path:line" is an honest and useful outcome, not a failure.
2. **Find the mechanism**, not the symptom: name *which* shared resource,
   ordering or clock the outcome depends on. A test that fails randomly and is
   "fixed" by adding a sleep is not fixed.
3. **Fix the cause**, preferring a construction that cannot be order- or
   timing-dependent (a deterministic seam, an injected clock, a per-test lock
   order, a bounded wait for a *condition*). Never fix a flake by:
   * weakening or deleting an assertion, retrying on failure, `#[ignore]`ing or
     `--skip`ping the test, or adding a `sleep` that papers over the window;
   * serializing the whole suite as the fix. `--test-threads=1` as a *workaround*
     in a documented command is acceptable; making it the only way the suite can
     pass is not a fix, because CI runs in parallel.
4. **Verify**: the same command at the same `-j`, N runs, 0 failures (and the
   negative control now passing, where the flake is order-dependent). Also run
   the flake's neighbourhood once to show you did not fix it by breaking
   something else.
5. **Record** in your unit's file (`docs/testing/flaky-<unit>.md`): test name,
   file:line, mechanism, why it was timing-dependent, the exact fix, the
   before/after rates with their commands and run counts, and anything you could
   not reproduce.

## 4. Two rules that keep this from being a "make it green" exercise

* **A flake is not a bug in the test by default.** Several documented flakes here
   are *real production* races that the test surfaced (the local-endpoint lease
   release race is the clearest case). If the mechanism turns out to be in
   production code, fix production code and say so loudly in the record — do not
   quiet the test.
* **Do not fix a flake you cannot reproduce** beyond recording it with the
   mechanism and the reproduction attempts. A claimed fix with no reproduction
   is worse than a documented open flake.

### 4b. A FIX can introduce a flake — rate the whole suite, not just the target

Added 2026-10-16 after the concrete case: **F-CLI-2's own new test
`doctor_captures_partial_version_from_hanging_binary` now fails 8/80 under 32 burners**
(FLAKES.md item 25). The fix it belonged to (a `#[cfg(test)]` probe-budget seam) was
correct for the flake it targeted; the *new test it added* then became a flake of the
"marker orders the child's write, not the probe's capture" family, one layer out. Two
consequences worth stating as rules:

* A flake fix's evidence must include a **rate for the suite it lives in**, not only for
  the test it targeted — a targeted rig cannot see a flake the fix introduced elsewhere.
* A fix whose follow-up test is itself flaky leaves the ORIGINAL item open. Do not
  close an item because the targeted rate moved; close it when the suite rate supports it
  (this is why the F-CLI-2 delivery is recorded `rework` while its change is accepted).


### 4c. A flake can be DESTRUCTIVE: leaked process-global state fails other tests

Added 2026-10-16 (FLAKES.md item 27). The class: a test mutates a process-global (`PATH`,
`HOME`, `FUTURE_*`, `WT_SESSION`, the cwd, a signal disposition) and restores it **on the
success path only**. When an earlier assertion fails or panics, the mutated value survives
and later tests fail for reasons that have nothing to do with them. Measured impact: one
such test produced **27 cascading failures** and voided an instrumented run.

What this changes about the protocol:

* **Before blaming the suite for a cluster of failures, look for one leaked global.** A red
  run whose failures are all `NotFound`/wrong-env is one bug, not thirty.
* A `Mutex` that serialises global-state tests (`crate::test_env::lock`) is **not** a
  restore: it stops the race, not the leak. The restore must be panic-safe (RAII guard or
  `catch_unwind`), or the test must use a helper that owns both.
* `cargo test` runs tests in threads of ONE process, so a leaked global is visible to every
  later test in that binary - including tests in other modules.
* The scan is a triage tool, not a finding list: `.future/cov100/global-leak-scan.py`
  reports "mutates a global and can panic before the last restore", which is a shape, not a
  verdict (a helper in another module can make it safe). Confirm before fixing.


### 4d. A red run must carry its assertion text (harness rule, 2026-10-17)

The tui suite lost the panic text of every red for weeks because of a leaked panic hook (item 24's root cause). The rule that came out of it: **a flake report that says only `FAILED` is not evidence.** If a lib red is textless AGAIN, the first two checks are (a) is `run(...)` installing a hook in a test build (`install_scoped_for_entry_point` exists so it must not), and (b) is the failing test one of the hook tests, whose own text is swallowed by construction. Scripts behind a flake record should print the assertion rather than a bare FAIL.


### 4e. `test result: ok` is not enough - require a SUMMARY per target (2026-10-17)

The quotability rule required "every target printed `test result: ok`". Item 29 shows the failure that rule alone misses: **a target can print nothing at all** (the process exits early) while the run still exits 0. The check is now two-sided: (a) no `FAILED`/`panicked at`, AND (b) the number of `test result:` lines is at least the number of `Running (unittests|tests)[/ ]` lines. Fewer summaries than targets means TRUNCATED, not green. `measure-final.sh` enforces both; any unit re-running a suite by hand should grep for a `test result:` line before believing a green run.

## 5. Known inventory (starting point — the discovery task extends it)

Rates and repositories in `.future/memory/testing-gotchas.md` are the source for
the mechanism; **re-measure on the current revision** before trusting previous
numbers. The supervisor has confirmed item 1 on this host today by negative
control.

**Where the mechanisms actually are (a defect this campaign caused):** the task
texts cited `.future/memory/testing-gotchas.md`, which does **not exist in a
worktree** — `.future/` is gitignored, so a fresh worktree has only what the
supervisor puts there, and one worker reported taking its guidance from round-1
material instead. That file is now copied to `.future/cov100/GOTCHAS.md` and
referenced from here; every "see .future/memory/…" instruction in an earlier task
text is dangling. Prefer in-worktree paths (`FLAKES.md`, `GOTCHAS.md`, `PLAN.md`).

| # | flake | where | documented mechanism | status |
|---|---|---|---|---|
| 1 | `desktop/src-tauri` lib suite deadlocks in parallel | `remote_host/business/settings.rs` ↔ `remote/commands.rs` | two process-global test locks taken in **opposite order** (`HomeGuard`→`mock_agent_lock` vs `mock_agent_lock`→`HomeGuard`) | **FIXED by the F-TAURI unit (2026-10-17), supervisor-reproduced**: the command that used to hang (>60 s, all threads at 0 % CPU, 300 s timeout in my hands) now completes in **7.44 s / 71 passed**, and the unit's enforcement test (which scans `src/` for lock-order sites and asserts >100 files inspected) passes. See `docs/testing/flaky-tauri.md` | fixed (F-TAURI) |
| 2 | `future-rpc` local-endpoint lease tests | `packages/rpc/src/transport/unix_endpoint.rs`, `transport.rs` | `flock` lease FD released/held racily → `AddrInUse: endpoint ownership is occupied`. Measured 9/30 on a branch, **16/30 on clean main** | not fixed |
| 3 | `agent/tests/cli_smoke.rs` profile-timer races | `cli_smoke.rs` (`agent_default_ipc_shuts_down_…`, `agent_starts_serves_and_shuts_down_…`) | `--profile-seconds 0` makes the timer win the race against the bind log line the test asserts | not fixed |
| 4 | `llm::tests::a_transport_error_is_classified_by_the_kind_that_caused_it` | `agent/src/llm/mod.rs` | test builds a loopback socket to produce an error outside every named kind; a reset handshake turns it into `is_request()` | not fixed |
| 5 | `cli/tests/bin.rs` agent singleton-lock contention | `cli/tests/bin.rs` | 11 tests spawn agents; one holds `~/.future/agent/agent-instance.lock`, so a sibling returns early with an empty stdout | not fixed |
| 6 | `safari_manager` first-launch readiness window | `cli/src/browser/safari/safari_manager.rs` | 10 s ready-window for a python fake driver is not enough under full-suite load | **fixed by the F-CLI unit (2026-10-16)**: the launch test no longer depends on a separate interpreter starting inside the window (deterministic negative control: 12 s-slow `python3` shim fails before, passes after; 10/10 clean). Its sibling `launch_then_become_reachable_reports_started` keeps the `free_port()`/`resolve_port` exposure. See `docs/testing/flaky-cli.md` §2 | fixed (F-CLI) |
| 7 | desktop backend `database is locked` | `agent_bridge/test_support.rs` | shared HOME/SQLite migration contended by parallel tests | not fixed |
| 8 | desktop backend transfer/download family | `remote_peer/runtime.rs`, `commands/remote_peer.rs` | **corrected 2026-10-17 by measurement (F-TAURI-2/-3)** — this row was stale in three ways: (a) the **500 ms** test budget it named was **removed** (one value for app and suite: `request_budget()` = `PHONE_REQUEST_TIMEOUT`), verified in the code; (b) the production figure is **10 s, not 15 s**, and it is now **named and pinned** (`NATS_CONNECT_TIMEOUT` 5 s, `PHONE_REQUEST_TIMEOUT` 10 s) instead of inherited silently from async-nats; (c) the attributed mechanism is **wrong**: the budget-class signature (`remote_download_chunk_missing`) appears **0 times in 26 runs**, so the chunk budget is not what load breaks. What load does break is the **request/connection half** on the production 10 s budget: `Remote request failed: request timed out` and `Failed to connect to NATS: timed out`. Nothing is lost on the wire (0 of 1093 broker deliveries reached no subscriber; a 12331 ms handshake reply was delivered *after* the deadline). **The mechanism to fix is `runtime::on_transport_failure` DROPPING THE CONNECTION on a timeout**, which is how a slow-but-alive peer becomes `peer_not_connected` — the CI #1094 signature | characterized by F-TAURI-3; **successor `todo_75ff0be76e75` (F-TAURI-4)** owns the fix. Do NOT raise a budget: a cfg(test) 60 s tolerance was implemented, measured and **withdrawn** (it turned the rig's reds into 900 s timeouts of the whole filter, vs 370–415 s pre-fix) |
| 9 | mobile `useSkillRecommendation` "local midnight" | `mobile/src/features/skills/…` | test writes a UTC date key, hook reads a local-date key; diverges 00:00–08:00 CST | not fixed |
| 10 | mobile jest never exits (leaked lane retry timer) | `mobile/src/remote/syncEngine*` | a real `setTimeout` retry keeps the loop alive; jest waits forever, and CI's job hangs for 30+ min with a green suite | partially fixed (the known test has `clear()`); "process exits" is the judgement |
| 11 | channel `imessage` poll tests are date-rotten | `channels/src/providers/imessage_tests.rs` | fixture stamps rows at a fixed Apple epoch (2026-09-30) while the poll watermark starts at `now()` | assigned to the channels coverage unit |
| 12 | **the coverage report itself is sometimes zeroed** | `desktop/coverage/coverage-final.json`, any `--lcov` output | observed 2026-10-16: a complete-looking 3.8 MB report with **331 of 333 files at zero hits** (same file/line count as a healthy run) written right after the volume hit 98 % full; a per-line tool then reports "everything uncovered". A run that dies mid-flight can still write coverage. **Always sanity-check the report before recording a number**; the campaign tools now exit 3 on a zeroed report or a scope mismatch | mitigation in place (tool guard); root cause open — treat as part of the sweep |

## 5b. Mined from the workers' own transcripts (discovery channel, supervisor)

The journal text of every running unit was scanned for flake language
(`.future/cov100/mine-flakes.py`); each item below is a **candidate with the
unit's own words as evidence**, not a confirmed flake. Verify by reproduction
before fixing; a candidate that does not reproduce still gets an entry.

| # | candidate | unit's own evidence (quoted) | where | status |
|---|---|---|---|---|
| 13 | `session::persistence::tests::timeout_drain_handoff_executes_queued_command` fails intermittently in the full agent lib suite | "Two flakes, both diagnosed and neither a regression here: the pre-existing `cli_smoke` timer race, and `session::persistence::tests::timeout_drain_handoff_executes_queued_co…`" | `agent/src/session/persistence.rs:1587` | not reproduced yet |
| 14 | `doctor_detects_agent_binary_not_running_as_issue` fails in some full runs, passes alone | `cli/src/commands/doctor.rs:815`, sibling `:892` | **mechanism named, fixed by F-CLI-2 (2026-10-16)**: the probe's 5 s wall-clock deadline races the host scheduling a `#!` fixture child (forced: a fixture that writes after the deadline loses its line — bare path, no version, `which` succeeded). The other arm (`Command::spawn` failed) rendered identically and is now labelled `(version probe could not be started: …)`. Fix: `VersionProbe` + a `#[cfg(test)]` probe-budget seam (capture tests run with no deadline; `binary_version_kills_a_probe_that_never_exits` keeps the production 5 s). The `which()` route to the same failure is fixed too. See `docs/testing/flaky-cli.md` §7.1/§7.2 | fixed (F-CLI-2) |
| 21 | `a_socket_that_answers_only_the_slower_probe_points_at_a_retry` (campaign-new) and `a_socket_answering_garbage_reports_a_failed_probe` (campaign-new, same defect) | `cli/src/commands/browser_tools.rs` (mock) | **fixed by F-CLI-2 (2026-10-16)**: the mock recorded each connection's bytes in read-completion order while the assertions required accept order; the bare reachability probe's read returns 0 only when the peer closes, so the pair can invert (recorded panic: `["GET /json/version HTTP/1.1…", ""]`). Fix: take the recording slot at `accept`, fill it from the connection task, join every connection before reading | fixed (F-CLI-2) |
| 22 | `start_reports_started_when_the_endpoint_answers_in_the_readiness_window` fails in the loaded gate, passes alone (pre-existing, campaign-rewritten) | `cli/src/commands/browser_tools.rs:3554` | **fixed by F-CLI-2 (2026-10-16)**: the marker only ordered the launch — the endpoint cannot exist before `browser_start` (a live endpoint takes the reuse / `already_running` arms), so the *launcher's own start* (an external `#!` process) and the test's own scheduling both sat inside the product's 10 s window; measured needing more than the whole window (marker + 40-probe budget still failed loaded). Fix: mock comes up when the launch commits (profile directory, before the spawn) + a `#[cfg(test)]` probe-count budget; the 10 s window keeps `start_launch_never_reachable_reports_starting` | fixed (F-CLI-2) |
| 23 | `a_server_that_dies_reports_its_stderr` | `cli/src/commands/mcp_servers.rs:1982` | **observed 1/5 in F-CLI-2's after gate** (0/4 before, lower load): the assertion wants the dying child's stderr in the handshake error, but the run reported `the MCP server closed its output before answering 'initialize'` — the closed-pipe arm won the race against attaching stderr. Outside F-CLI-2's write set; handed to the supervisor | not fixed (handed off) | **FIXED 2026-10-17 (F-CLI-4).** The §8.5 patch landed in `cli/src/commands/mcp_client.rs` (`STDERR_DRAIN_GRACE` = 2 s, the drain task joined before the tail is read - the missing happens-before). Re-measured with the fix: **37/320 -> 0/320**, both arms inside one 48-burner window back to back; the earlier F-CLI-3 figure was 58/320 on another binary in another load window, same shape. Neighbour check: `cargo test -p future-cli --lib -- commands::mcp_` = 81 passed / 0 failed. **STATUS: fixed.**
| 15 | `feishu/feishu_ws.rs:254` oscillates between covered and uncovered across runs | "the documented pong-send race line (covered 4 of 5 runs) — not a regression"; independently "wobbles ±1 line between green runs" | `channels/src/feishu/feishu_ws.rs` (the `let Err(e) = ws_stream.send(..)` arm) | known race, needs a deterministic trigger |
| 16 | `orchestration/goal/src/compat.rs:235` oscillates between covered and uncovered across runs | "confirmed as a scheduling race, not a stable residual: covered in some runs of this suite, uncovered in o…"; and "the first is a scheduling race (covered in two runs, uncovered in the clean one)" | `orchestration/goal/src/compat.rs:235` (the `LockState::Released => continue` arm inside the lock-classification loop) | candidate; also a **measurement** hazard — a flaky line changes every before/after number. **Owner is the goal/packages unit, NOT `flake-rpc`** |
| 17 | **negative data point, worth as much as the positives**: the desktop frontend suite showed no flake under load | "**No flake.** Both full runs completed green at load average 22 with 13 workers activ…" | `desktop` vitest | recorded as a rate with its load |

| 18 | **the fd-1 trap in the `tui` suite is only partly fixed** - it still kills runs (**corrected 2026-10-16** after the F-TUI unit measured the rate) | `tui/src/terminal_posix.rs` (`write_stdout_retries_on_eintr`) | fd 1 is process-global and this test points it at a **full** pipe for ~0.5 s, so libtest's own progress line (and the unlocked `write_stdout_writes_all_bytes`) queues inside that pipe; on macOS a write *asleep* in a full pipe is broken by the `dup2` that hands fd 1 back (`EPIPE`/`SIGPIPE` with the read end open - two standalone probes in `flaky-tui.md` 1.3), and libtest then aborts the binary (`io error when listing tests: ... BrokenPipe`: **no lcov**); an orphaned sleeping write instead **hangs** the suite, which `timeout` cannot end because the suite catches `SIGTERM` (only SIGKILL works - the harness now uses `timeout -k`). **Two fixes, two different outcomes (both rates recorded, 2026-10-16).** Fix A (drain to empty before the restore, reader parked at fd>=100, drainer to EOF): measured **before 1/8, after 1/8** - the route was removed but **the rate did not move**, and the unit said so instead of claiming success. Fix B (F-TUI-2, `todo_0023e49af074`): a write-trace interposer named the writer that actually earns the EPIPE - **libtest's own console thread**, whose progress-line write resolves fd 1 into the pipe (12/12 green runs, always that tid, dt 0.500-0.510 s, released 107 us before the restore) - and the fix is that the fd-1 tests hold the **process-global stdout lock** (`io::stdout().lock()`, the lock every Rust-stdout writer already takes per write, so the harness blocks in userspace instead of queueing in the pipe) with production `write_stdout` taking the same lock. Rates for fix B: unit **0/8 + 0/8 = 0/16** (same command, `timeout -k 5`, `-j 4`) and **supervisor-independent 0/8** (4+4 runs of the identical command; 0 `BrokenPipe`/`io error when listing tests` signatures in any log). One documented boundary: the kitty-query test **cannot** hold that lock (its reader thread writes fd 1 itself, so locking would hang `t.stop`) and is made safe by the drainer-absorbs ordering instead. Status: **fixed by fix B**, with the item's sibling sub-items (the `app.rs` wall-clock budgets) still open - see item 24 for one the supervisor then found | measurement-level flake (item 12 family), **fixed by F-TUI-2 (fix B); the earlier fix A did not move the rate** | **INSTRUMENTED RATE MEASURED 2026-10-17 (F-TUI-8, closing item 9.3 - the last unmeasured number for this class):** 8 runs of the module-record command under `cargo llvm-cov` -> **runs=8 failures=0**, no 'io error when listing tests' in any log, and a profraw written on every run (the 'lost lcov' face never appeared); per-run 218 s incl. the instrumented build, then 79/73/84/71/69/126/65 s. Before fix B the same command produced **5 of 8** instrumented attempts (this item's own history), so the instrumented rate is **0/8 now vs 5/8 before**. SCOPE OF THE CLAIM, stated by the unit and kept: load was 4-10 for all eight runs (a quiet window) and this host is load-sensitive (it reached 14 later; that collapse happened at a load never reproduced that day), so the claim is **'0/8 at load 4-10'**, NOT '0/8 period'. A high-load instrumented batch is handed to a successor. 
| 19 | *(merged into item 16 — it recorded the same flake twice)* | the same "scheduling race" report from the `packages/*` unit | the file is `orchestration/goal/src/compat.rs:235`; **`packages/rpc/src/compat.rs` does not exist** (checked with `ls`) | attribution corrected by the supervisor after the `flake-rpc` unit reported the wrong path I had put here; it is not an rpc item, so it was never handed to `flake-rpc` after all |
| 24 | **`app::tests::a_real_repository_creates_a_worktree_and_switches_to_it` fails under load** (found by the SUPERVISOR, 2026-10-16, while independently rate-checking item 18) | not the unit's words but my own measurement: **1 failure in 4 runs** of `timeout -k 5 280 cargo test -p future-tui --lib -j 4` at **load average 34** (`rc=101`), while the same test **passed 3/3 standalone** at load 6 (5.30 s, 5.23 s, then 0.26 s — the spread is itself the tell) and the four green runs of that same batch finished at 114 s | `tui/src/app.rs:23124` (its harness is wall-clock-bounded: `pump_until_msg`/`pump_until_workdir` spend `PUMP_BUDGET_ITERS = 4_800` × `PUMP_INTERVAL_MS = 25` = **120 s** waiting for live-gRPC round trips, `app.rs:11942`) | **new, not fixed.** Same family as item 14 (the doctor probe's 5 s wall-clock deadline) and as the sibling `wait_for_agent_retries_until_agent_appears` budget that F-TUI-2 reported. **Evidence gap, stated plainly:** the failing run's log ends at the summary and did **not** capture the panic text, so I cannot name which assertion lost (the 120 s pump timeout, `path.is_dir()`, or `seen.len()==1`). Next check: reproduce under load and `--nocapture`, then either remove the wall-clock dependency (paused clock — `tokio` `test-util` is already a dev-dependency, see item 18's sibling finding) or re-register with the host condition named. Measurement-level: a red suite run means **no lcov** | **ROOT CAUSE OF THE MISSING PANIC TEXT FOUND (2026-10-17, F-TUI-5).** The gap I recorded above ('the failing run's log ends at the summary and did NOT capture the panic text') was not bad luck: `tui/src/index.rs` called `crate::crash::install()`, a BARE process-global `std::panic::set_hook` with no restore, and ~20 `index.rs` tests call `run(...)` - so in the lib test target the crash hook replaced **libtest's capturing hook permanently**, and because `hook_body` returns early under `cfg(test)` unless the reporting is armed, every LATER panic lost its message. That is why a panic two minutes into a run was invisible while an early one was not (the hook is installed as soon as any `run(...)` test executes). FIXED in F-TUI-5: the install is now reference-counted (`crash::HOOK_STATE`, capturing the previous hook once and restoring it when the outermost guard drops, poison-recovering), `index::run` uses the scoped `InstalledHook::install()`, and two tests pin both faces (a panic after the entry point returns reaches the previous hook; overlapping scoped installs do not leak). **Any tui flake whose log lacked a panic message before 2026-10-17 should now be re-measured**, because the missing text was a tooling defect, not a flake property.  **ROOT CAUSE AND BOTH SITES NAMED, 2026-10-17 (F-TUI-6) - this is a PATH-swap flake, i.e. item 27's family, not a wall-clock one.** The two failure sites, readable for the first time: `app.rs:12019` 'timed out waiting for system message containing "Created worktree demo on branch feat/demo from main."' (suite 137.25 s / 147.53 s / 147.00 s) and `app.rs:23200` 'called Result::unwrap() on an Err value: Os { code: 2, kind: NotFound }' (suite 70.00 s). Mechanism: ANOTHER test in the same binary swaps the PROCESS-GLOBAL `PATH` - `components/autocomplete.rs::with_stubbed_path` (:1709) points `PATH` at a temp dir holding only its own stubs, so while it runs `git` cannot be resolved anywhere in the process; it holds `test_env::lock()`, but this long test does NOT, so any interleaving during its 120 s window lands inside the swap. FIX: the test pins an ABSOLUTE `git` (`app.rs`'s `absolute_git()` helper, so it no longer depends on `PATH` at all). RATES: **3 of 12 concurrent-suite reds before -> 0 of 12 after** (4-way at -j 8, three rounds), and suite times fell back from 137-147 s to 25-30 s. RESIDUAL the unit states rather than hides: other tests in this binary still swap `PATH`, so a future test that spawns `git` without pinning it has the same exposure - recorded as a convention question for the successor. **STATUS: fixed, with a positive control** (the two regression tests still fail on pre-fix code).  **SUITE-WIDE CONVENTION, raised twice and still open (F-TUI-6 doc 12, F-TUI-7):** item 24's fix pinned ONE test to an absolute `git`; every OTHER test in this binary that spawns a program without pinning one keeps the same exposure, because any test may swap the process-global PATH. The suggested shapes are a shared `absolute_tool(name)` helper or a lint. This is not a flake someone can rate-close - it is a convention, and it is carried by a successor todo. 
| 20 | `agent/src/llm/mod.rs:422` flips covered/uncovered between runs with the function untouched | found by the AG-B unit while re-measuring: tokio 1.53.1's `select!` "randomly picks a branch to check first", and that line needs two ready arms, so whether it executes depends on the branch order the runtime happens to pick (covered in the 20:39 run, uncovered at 20:54 and 21:00) | `agent/src/llm/mod.rs:422` | measurement-level nondeterminism, the 4th of its kind (after cli `cdp_connection.rs:229`, channels `feishu_ws.rs:254`, tui `publisher.rs`). CONSEQUENCE FOR THE FINAL REPORT: an AG-B total may legitimately read 212 or 213 depending on branch order, so the number must be published with the run named |

| 25 | `doctor_captures_partial_version_from_hanging_binary` fails **8/80 under 32 burners** (+1/5 in a gate window) | found by the F-CLI-3 unit in its own gate, in **F-CLI-2's own new test** (`cli/src/commands/doctor.rs:1076`, assertion "expected partial --version output to survive the timeout") | `cli/src/commands/doctor.rs` (`get_binary_version`, 307-376) | **new, mechanism named, split not in hand.** The fixture `echo`es its line and only THEN touches the marker the test waits for, so the child's write provably precedes the armed kill; the failing report still shows a bare path (`VersionProbe::NoVersion`). So the kill is ordered and **the capture is not**: `get_binary_version` spawns its two pipe reads on tasks of its own and classifies whatever they have accumulated - the same missing-happens-before family as item 23, one layer out. Two candidate paths remain unsplit: (a) the buffer at classification, (b) `is_version_candidate` rejecting the fixture line. The split check is written and named (`PROBE diag bin=… out_len=… out_err=… out=…` at the classification point, installed in a scratch copy then reverted) but two instrumented windows caught no failure (**64 runs** at 32 burners, 16 at 56) - so the **rate** exists and the split does not. NOTE for the record: this is a flake **introduced by F-CLI-2's fix**, which is why F-CLI-2's own entry stays open. | **FIXED 2026-10-17 (F-CLI-4).** Root cause: the test armed its kill off the ELAPSED BUDGET rather than off the marker, so a loaded host killed the probe's child before it wrote and the test read a bare path while the product behaved correctly. Control table: pre-fix arming + a 25 s fixture -> **4/4 red** (bare path, 25.4-26.4 s, the exact assertion); fixed arming + the same fixture -> **0/4** (55.4-55.8 s); landed at **0/48** with 48 burners. Honest gap: the NATURAL rate did not reproduce (0/80 at 56 burners, load -> 77) - the margin is the product of that window (the marker appeared 435-6334 ms after each start against a 20 s fallback). **STATUS: fixed** - and note this flake was introduced by a FIX (rule 4b), so the fix's own suite rate is what closed it.
| 26 | `create_page_times_out_when_target_never_discovered` fails **1/5 in a gate window, 0/80 loaded** | found by the same unit; the message assertion above it PASSED ("Target … not discovered within timeout", `chromium_page.rs:340`) while `assertion failed: started.elapsed().as_secs() >= 5` | `cli/src/browser/chromium/chromium_page.rs:780`; the budget at `:328` is `crate::utils::time::now_millis() + 5000` - a **wall clock** (`SystemTime`, `cli/src/utils/time.rs:9`) - while the assertion measures `Instant` | **candidate, not reproduced** (0/80 in a 16-stream x 5-round rig at 32 burners, 5.02 s per run when green). A backwards wall-clock step (NTP) inside the loop is the only path the unit could see, and the assertion has **no slack** (4.999… s renders as `as_secs() == 4`). Named check to hand forward: print the loop's deadline origin beside its exit time, or measure the loop with an `Instant` and compare the two clocks over a run. Worth keeping even unreproduced: a production timeout budget taken from `SystemTime` is a real (if rare) defect class. | **CLOSED 2026-10-17 (F-CLI-4), by removing the defect rather than by reproducing the flake.** The clock-origin check ran (`CLOCK diag origin wall_ms=… exit wall_ms=… exit_instant_delta_ms=…`), 48 runs per arm at 32 burners: before 0/48 (wall 5002-5051 ms, Instant 5001-5051 ms), after 0/48 (5009-5041 ms) - the two clocks agreed **within 1 ms in all 96 runs**, so the candidate (a >=5 s forward wall step inside a 5 s window) did not reproduce. The budget at `chromium_page.rs:333` is now an `Instant` - the same clock the assertion measures - so the two cannot diverge; the alternative (re-register with the reason and keep two clocks) was explicitly rejected. Remaining work moved to F-CLI-5 for the same wall-clock shape at `backend.rs:16-28` and `chromium_navigation.rs:167,214`. **STATUS: defect removed, flake unreproduced.**

| 27 | **one test's leaked `PATH` fails 27 others** (`cli_tool::tests::a_runnable_cli_on_path_is_discovered_and_probed`) | found by the TAU-B unit at segment 6 in its OWN instrumented gui run: the run came back RED with 31 failures, the first being that test, and 27 of the rest dying with `Os { code: 2, kind: NotFound }` while spawning `git` ("the same code that produced 31 failures under load produced 0" at load 3.12) | `desktop/src-tauri/src/cli_tool.rs:441` (and its three siblings at 469/499/531): the test does `std::env::set_var("PATH", <fixture dir>)` at the top and restores `PATH` as its **last statement**, with no panic-safe guard | **new, mechanism confirmed by reading the source and measured by the unit.** This is a **destructive** flake: it does not merely fail itself, it fails others, and one occurrence turned a green suite red and voided an instrumented run's lcov (which is why segment 6's merged number needed a coverage caveat for two files inside its scope). CLASS SIZE, measured this turn by a scanner I wrote (`.future/cov100/global-leak-scan.py`): **218 sites** across desktop 70, orchestration 38, tui 27, packages 6, cli 4, agent 2, channels 1. NOTE the scanner answers a narrow question ("mutate a global and can panic before the last restore") and its precision was spot-checked: it flags the known-true site, and three tui sites it flags DO hold `crate::test_env::lock()` - but **that lock is a mutex, not a restore** (`tui/src/lib.rs:82`: it is poison-safe so a panicking test does not cascade-fail on the lock, while the leaked ENV VALUE survives). So the tui hits are real too, just serialised. | **SECOND AND THIRD CONSUMERS, measured 2026-10-17 (TAU-C shell segment 4)**: the same family reddened that segment's gui leg - `skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin` failed with `CLI discovery timed out ... no fallback CLI was executed; skipping built-in skill bootstrap` and its assertion `the spawned CLI must have run its init` (src/skills_bootstrap.rs:225), i.e. a test that spawns a CLI lost the `PATH` a peer test had replaced. So the blast radius is not one file: a leak in `cli_tool.rs` reaches `skills_bootstrap.rs` too, and both are outside the shell unit's twelve measured scopes - which is why its 242-line figure still stands (a red run executes less code, and every failure was outside scope). The fix todo (todo_16d1326ad286) names cli_tool.rs; the fixing unit should check skills_bootstrap.rs for the same `set_var`-without-guard shape while it is in there.  **THIRD CONSUMER, and the one with the largest measured blast radius (2026-10-17).** `tui/src/components/autocomplete.rs::with_stubbed_path` (:1709) is the PATH swapper behind FLAKES item 24 - it holds `test_env::lock()`, which serialises the OTHER global-state tests but does NOT stop this leak from breaking a test that (a) runs concurrently and (b) spawns a process. That is the same lesson as the tui `ENV_LOCK` note below: **a mutex is not a restore, and a lock only protects tests that take it.** Measured impact: the long `app.rs` worktree test failed 3 of 12 concurrent-suite runs, each failure costing the whole suite's time budget (137-147 s vs 25-30 s green). NOTE FOR THE SCANNER: my `global-leak-scan.py` DID flag this site (`autocomplete.rs:1708 fn with_stubbed_path mutates ['PATH']`), i.e. the tool found it before the unit named it - but the scan output was a triage list of 218 sites and this one was not prioritised, which is the honest lesson: a scanner that flags 218 sites is a census, not a worklist, and the sites that MATTER are the ones whose leak can reach a test that does not take the lock. 


**A note that belongs with items 18/24/27 (added 2026-10-17 by the supervisor, on F-TUI-6's evidence).** The tui lib target's signature 'a failing test with NO `panicked at` line and no `---- <test> stdout ----` body' was CAUSED by a leaked panic hook (item 24's root cause; fixed by F-TUI-5 and hardened by F-TUI-6, which made a test build install nothing at all via `install_scoped_for_entry_point()`). So any tui flake recorded before 2026-10-17 whose log lacked a panic message should be RE-MEASURED rather than trusted as unexplainable - which is exactly what F-TUI-6 did, and it named both item-24 sites as a result. The item 18/25/26 entries keep their earlier 'no text captured' phrasing as history; the text is available now.

| 28 | `agent_sidecar_launch.rs::a_sidecar_candidate_that_exits_is_reported_and_the_next_one_is_tried` red **1 of 34 whole-suite batches** (0 of 20 alone) | found by F-TUI-6 (doc 11.3) and ROOT-CAUSED by F-TUI-7 | `tui/tests/agent_sidecar_launch.rs` (the fixture's window vs the loop's product budget) | **FIXED 2026-10-17.** Mechanism, from a temporary diagnostic inside `ensure_agent_running_within` (reverted): a green run shows the `/bin/sh` stub exiting ~375 ms after the spawn against a 5 s window (margin ~13x), so the fixture was racing the loop's PRODUCT budget - the same finding doc 7.1 named in the sibling file - and when the host is slower than the window the loop exhausts its attempts and kills the child SILENTLY, so the report lists only the candidates that failed to spawn and omits the one that spawned, which is exactly what 11.3 recorded. FIX (TEST FILE ONLY): the fixture passes its own window, `ensure_agent_running_within("auto", LAUNCH_FIXTURE_WINDOW)` with 60 s (160x the measured latency); the production default (`LAUNCH_PROBE_WINDOW` = 5 s) is UNCHANGED, so a sidecar that cannot answer in 5 s is still killed at 5 s. CONTROLS, both reproducible on demand: a 300 ms window (public API, no perturbation) reproduces the recorded message CHARACTER FOR CHARACTER; and an A/B on one tree - product default + an 8 s stub -> FAIL with that message (5.13 s), fixture window + the SAME 8 s stub -> PASS (8.18 s) - shows the failure gone for a child **21x slower** than the one that triggered it. RATES: 1 of 34 before; 0 of 8 concurrent whole-suite runs after (4-way at -j 8). HONEST LIMIT stated: **a 1-in-34 flake cannot be excluded by 8 (or 12) runs** - the load-independent evidence is the A/B pair, and the doc says so rather than claiming the rate. NOT TAKEN, with its reason: `ensure_agent_running_within` kills a window-expired candidate silently, so its report omits a candidate it tried; one failure line there would have made this self-explanatory, but it changes a user-visible message and no assertion needs it. |

| 29 | **A run can be TRUNCATED WHILE EXITING 0, so `green` is not sufficient** (NEW-A; found 2026-10-17 by the tauri-headless unit, mechanism verified by the supervisor) | the unit's report; my verification: the vendored plugin source + a live `/tmp/_si.sock` on this host | `tauri-plugin-single-instance` 2.4.3, `platform_impl/macos.rs:25-29` (+ `socket_path` at `:61-72`), reached from `desktop/src-tauri/src/lib.rs` via the mock app in tests | **new; the most dangerous item in this inventory.** The plugin calls `std::process::exit(0)` when the singleton socket already answers, and the socket path is the FIXED `/tmp/{identifier}_si.sock` with the identifier normalized from the app config - **the mock app's identifier is EMPTY, so the path is `/tmp/_si.sock`, shared host-wide across worktrees**. Consequence: a colliding run ends mid-target, prints NO `test result:` line, and still exits 0 - a truncated run that LOOKS GREEN, so a coverage measurement taken in that window silently measures less than it claims. Reproduced by the unit (the gui filter alone 17/17; the same command twice concurrently -> both die, 2/2; five runs ended this way). **Campaign exposure checked and NEGATIVE:** a scan of 2586 cargo-format logs found exactly one truncated log (a synthetic control of mine), i.e. no published figure was affected. **Harness hardened the same day:** `measure-final.sh` now flags `summaries < targets` as `PARTIAL(TRUNCATED)` for the rust and tauri tracks, with controls that caught two wrong regexes of mine before they shipped. Fix todo: NEW-A (P0). |

| 30 | **A WARM target dir can silently drop ONE FLAVOUR from a two-flavour merge** (found 2026-10-17 by the app-data unit at segment 7) | the unit's own measurement: its first re-run produced a report reading **626** uncovered with `scheduler` at 175 instead of 23 | `desktop/src-tauri` x `cargo llvm-cov`: rendering ONE lcov from both flavours' profraw after a rebuild, when the target dir was warm from a previous run | **new; the dangerous direction.** The first flavour's profraw no longer mapped to the binary the second flavour rebuilt, so its data was dropped from the merge and the number came out WORSE than the truth - a silently understated measurement, which no green/red check can catch (nothing failed). CHECKABLE SYMPTOM, which is what makes it usable: a `cfg(feature = "gui")`-only file (e.g. `scheduler/mod.rs` 205-231) reading uncovered, or a covered count collapsing. FIX, and the campaign pattern: render each flavour IMMEDIATELY after its own leg (`--lcov --output-path`) and UNION the two - equivalent for the `DA:0` metric (a line is uncovered iff NEITHER flavour covered it) and it cannot lose a flavour. `lcov-union.py` is now permanent at `.future/cov100/lcov-union.py`; I confirmed it reproduces a known union exactly (the shell unit's per-flavour pair -> 242, its published number). **Harness hardened the same day:** `measure-final.sh`'s tauri track now renders per flavour, unions, prints each flavour's file count, and FAILS the track when either flavour produced no report - my own harness had the fragile one-shot `report` form, i.e. the exact exposure this item names. |

## 6. Handoff requirements

The acceptance contract on these todos is now **empty**, on purpose: the
machine check only verified that a literal token appeared as a substring of the
last 4000 bytes of the reply, which is a spelling test, not a verification (see
the memory entry `loop-async-run-lifecycle`). What still holds is what the
handoff must CONTAIN, because that is what the reviewer reads:

* the exact commands you ran and their results,
* an lcov-based before/after, **with the measurement tool's exit status** (a
  nonzero exit means the number is invalid — see the zeroed-report trap),
* every residual uncovered line either covered by a named test or waiver-tagged
  with a category **and** the mechanism,
* plainly, what you could not do or could not reproduce.

Partial + honest is a correct handoff. An unclassified or hand-waved residual is
not.

## 7. Dispatch rule (supervisor)

The `--blocks` gate names *specific todos*, and this campaign delivers in
segments: when a coverage unit completes a segment it creates a **successor** todo
and keeps editing the same files. So a gate that names the finished segment
silently opens while its owner is still writing. Therefore, before dispatching
any flake unit the supervisor must confirm, on the live board, that **every**
todo whose `required_write_scope` overlaps the flake unit's scope is done (not
merely that the named blocker is done) — or re-point the gate at the current
segment todos. Then, and only then, dispatch. Flake units are never dispatched
automatically.

**AND one more, added 2026-10-17 (see section 8b): before dispatching a fix for a
flake, confirm the MECHANISM, not just the message.** The same assertion text
(`CLI discovery timed out … no fallback CLI was executed`) now has two measured
candidate mechanisms in this campaign — a too-short test-side budget under load
(NEW-B) and a peer test's leaked process-global `PATH` (item 27) — and the two
require OPPOSITE fixes: relaxing the budget would MASK the leak, and eliminating
the leak would leave the budget untouched. The one-command discriminant is to pin
an absolute path and re-run: the message disappearing means the leak. Do not
dispatch "make this flaky test pass" against a message two mechanisms can produce.

## 7b. Two hazards the campaign hit (rules, not advice)

* **Never format across a crate.** `cargo fmt -p <crate>` rewrote a peer's
  in-flight files while that peer was mid-run (measured: no content lost, but the
  anchor-invalidation failure mode is real). Format per file, and only when
  `cargo fmt --check -- <file>` says so.
* **Only a unit deletes its own `target/cov-*`.** The supervisor reclaimed dirs
  that no live process matched for an instant while their owners were still
  active, costing a unit a warm dir and a ~13 min rebuild. Under 20 GB free, ask
  rather than reclaim.

## 8. Discovery (the "全面" part)

**The supervisor's own channel works too** and costs no worker turns: their journals
already contain the observations, so `mine-flakes.py` scans every unit's text for
flake language and attaches the quoted sentence. It produced items 13–17 above in
one pass after six units had reported nothing. Run it again near the end, and
prefer its output over asking a worker to re-summarise.

Two channels, and both must produce findings:

1. **Report channel** — each unit records what it hits in a `## Flakes observed`
   section of **its own module record** (`docs/testing/module-<x>.md`), which is
   already inside its declared write scope: the exact invocation, run count,
   failing test, panic line. (Not a shared file, and **not**
   `.future/cov100/flakes/<agent>.md` — that directory is not in any unit's write
   scope, so writing there would be a cross-scope write. The copy of this brief
   that named it was corrected after the fact.) One line is enough: *did it flake,
   under what command, how many runs, which test*. "No non-deterministic failures
   in N runs at load L" is a finding too, and the ones that matter most are the
   ones that do not fail but *move the metric* (a line that flips covered /
   uncovered changes every before/after number by one).
2. **Repeat-run sweep** — at the end, on the frozen final revision, run every
   suite of the repo N times with `flake-rate.sh` and record the table in
   `docs/testing/flaky-sweep.md`. A suite is only "flake-free" for the rate and
   the load it was measured at; state both.

## 8b. 一处**归因歧义**必须先解决，否则会修错东西（2026-10-17，supervisor 记，来自 TAU-D3 的交付）

同一个断言消息 **`CLI discovery timed out … no fallback CLI was executed; skipping built-in skill bootstrap`** 现在有两个**不同的候选机制**在盘面上，而两个都已被实测过：

| 候选 | 谁报的 | 机制 | 它会要求什么修法 |
|---|---|---|---|
| **A. 3 s 探针预算对负载敏感**（本文件记作 NEW-B） | TAU-D3（本轮）、F-CLI-2 的变体 | `probe` 的墙钟 3 s 窗口在实际负载下不够，探针没走完 | 加 `#[cfg(test)]` 预算缝（F-CLI-2 用在 `doctor` 上的形状），**不动生产 3 s** |
| **B. 同业测试换了进程级 `PATH`**（本文件第 27 项） | TAU-C shell 段 4 | 一个不持锁的测试在跑时把 `PATH` 指向只有自己桩的目录，于是被测进程**找不到** CLI | RAII 还原 / 钉绝对路径（第 27 项的两种修法） |

**为什么必须先分清**：A 的修法是**放宽测试侧预算**，B 的修法是**消除泄漏**。对 B 施加 A 会**掩盖泄漏**（下一次它换个受害者继续发作）；对 A 施加 B 则**什么都没修**（预算仍然是 3 s）。

**现有的判别手段**（照做即可，不必新发明）：
1. **看它是否只在该测试单独跑时也失败**——A 是（窗口不够），B 不是（没有别人换 PATH）。
2. **看同时失败的是谁**——B 的特征是**同一个二进制里、不持锁的其它 spawn 型测试一起红**（第 27 项实测 27 个级联）；A 只红它自己。
3. **钉住绝对路径后重跑**——若消息消失，是 B；仍在，是 A。
4. **数负载**：A 应该随负载单调变坏；B 应该随**并发测试的构成**变坏（谁在换 PATH）。

**当前状态：未裁定。** TAU-D3 把它归为 A（并说明它在本单元 scope 内、**代价 0 行**——那次失败文件的未覆盖集合与测试通过时**逐字节相同**）；TAU-C 把它归为 B。**两者都没有做上面的第 3 步**，所以这是一条**未裁决的归因**，不是"两个都记一下就算了"。派发**任何**关于这条消息的修法之前，先跑第 3 步——这条纪律与本仓已记的"同一个字符串可能来自两个地方"（F-44/F.46）是同一件事，只是这次字符串在**错误消息**里而不在代码里。

## 8c. `--test-threads=1` 在**两个 `--`** 的形状下会被 libtest 当成**测试名过滤器**（2026-10-17，NEW-B 单元发现；supervisor 用一条更便宜的对照独立证明，并做了全库普查）

**机制**（两步，缺一不可）：
1. `cargo` 在**第一个** `--` 处切分自己的参数与测试二进制的参数；
2. **libtest 在它看到的第一个 `--` 处停止解析选项**。

所以这个形状——**过滤器在第二个 `--` 之前**——会把标志变成过滤词：

```bash
cargo test --lib -j 8 -- cli_tool::tests -- --test-threads=1     # ← 错：flag 被当成 filter
cargo test --lib -j 8 -- --test-threads=1 cli_tool::tests        # ← 对：flag 被解析
```

后果：harness 以**默认并发**运行（本机 32 线程）。于是"`--test-threads=1` 是强制的（lib 套件并行会死锁）"这句指令，**在过滤器出现在那个位置时根本没有生效**。

**我的独立证明（比计时更便宜、更硬）**：用**非法值**做对照——把 `--test-threads=1` 换成 `--test-threads=ABC`。

| 形状 | 结果 | 含义 |
|---|---|---|
| `-- --test-threads=ABC <filter>`（flag 在前） | **报错**：`argument for --test-threads must be a number > 0` | flag **被解析** |
| `-- <filter> -- --test-threads=ABC`（文档形状） | **跑完 81 个测试、无报错** | flag **被当成过滤词** |

**判据**：非法值只有**被解析**时才会报错；因此"不报错"就是"没被解析"的直接证据。这比测两个慢测试的墙钟（1.51 s vs 3.01 s）快得多、也更难被负载干扰——**凡要证明"某个选项有没有生效"，先问它有没有一个"只有生效才会失败"的取值。**

**汇报口径的更正（我做了全库普查，结果是**范围比我预期的小**）**：
我写了 `tt-classify.py` 按上面的规则分类**每一个**含 `--test-threads` 的调用：

| 语料 | 调用数 | SWALLOWED |
|---|---|---|
| **权威 harness**（`measure-final.sh` + 各 tauri 测量脚本） | 全部 | **0** |
| 仓库自己的 `scripts/`、`Makefile` | 全部 | **0** |
| `docs/`（全部文档） | 124 | **2** |
| 目标账本（todo 文本 + 证据里记录的调用） | 54 | **7** |

而那 **2 + 7 处**逐一查过，**全部是"引用这个错误形状作为反例"的引文**（NEW-B 的工单、它的完成证据、以及它的后继），**不是任何人真正执行过的测量命令**。⇒ **本 campaign 已记录的速率并没有因此被追溯作废**；被此坑到的真实执行只有一处，见下。原始发现者的措辞（"到处都在用的形式""每一个用该形式取的 flake 速率都是 32 路并行的"）**高估了范围**——这正是本仓反复要做的"单元对整棵树的断言必须由普查来核"（F.9 家族）。

**而那一处真实执行是**我自己的**——这是必须记的**：我给 NEW-B 写工单时，把确认测量记成 `cargo test --lib -- cli_tool::tests -- --test-threads=1`（**两个 `--`**），并据此写下"load 12-13 时 11 passed / 2 FAILED"。**所以我在那条 flake 上的"我自己也确认过"是在 32 路并行下取的**，而那 2 个失败**很可能就是并行本身造成的**（NEW-D：进程级 `PATH` 交换对**并发**运行的测试可见）。**supervisor 自己的测量方法成了缺陷**，这是本 campaign 第二次（第一次是 F.67 的快照）——而它恰好**强化**了那个单元的发现：它解释了为什么我的"确认"看到 2 个失败，而修正形式下是 0。

**规则（写进 PLAN §1/§6）**：**`--test-threads` 必须紧跟在唯一的那个 `--` 之后，前面不许有任何过滤器**；要过滤就把过滤器放在标志**之后**。任何"这句话只有在某处才效"的断言，都要问它**在被记录的命令里是否真的成立**。
