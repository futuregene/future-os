# Module: `desktop-tauri-headless` (`desktop/src-tauri`, headless/agent-supervision/terminal slice)

Status: **see §2.3** — the current figure is **283 uncovered** (D8 union at
`36c568972`, §2.3/§4.3); §2 is the earlier after-2/segment-3 history (359 → 300),
kept as measured history. All figures come from the **merged gui + headless**
lcov run, which is the only honest number for this crate (§1). This record
covers the slice that one unit owns:

| scope | files |
|---|---|
| `desktop/src-tauri/src/headless/` | `mod.rs`, `agent.rs` |
| `desktop/src-tauri/src/terminal/` | `pty.rs`, `session.rs`, `server.rs`, `manager.rs`, `shell.rs`, `cwd.rs` (+ `test_support.rs` fixtures) |
| entry/discovery adapters | `cli_tool.rs`, `agent_host.rs`, `agent_supervisor.rs`, `skills_bootstrap.rs` |

Owner: `todo_a36bdbba13fb` (TAU-D), goal `goal_160bc247aa63`. Write scope:
exactly the paths above plus this file. `docs/testing/module-tauri.md` (the
peer unit) owns the rest of `desktop/src-tauri`.

## 1. Run identity and the exact measurement

Baseline revision `8449acf6` (campaign start). Host: macOS 15 (arm64), rustup
toolchain `1.97.0-aarch64-apple-darwin`. Metric: **per-line LCOV `DA:<n>,0`**
(`.future/cov100/covlines.py`), never the `summary` block.

`desktop/src-tauri` is *not* a workspace member and is built in **two feature
flavors**; CI runs both (`Rust tests (Linux) (desktop gui|headless)`) and they
compile different code — so a single-flavor number is not a coverage number for
this crate (`.future/cov100/TAURI.md`). One atomic invocation, both flavors:

```bash
cd desktop/src-tauri
for v in $(env | sed -n 's/^\(CARGO[A-Za-z_]*\)=.*/\1/p'); do unset "$v"; done
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home
export CARGO_TARGET_DIR=/Users/geilige/future-os/.worktrees/cov100-r2/target/cov-tauri-tau
ulimit -n 10240
make -C /Users/geilige/future-os/.worktrees/cov100-r2 desktop-sidecar-placeholder
# --test-threads=1 is MANDATORY: the lib suite deadlocks in parallel on this host
# (HOME<->MOCK lock-order inversion, .future/memory/testing-gotchas.md).
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
cargo llvm-cov --no-report --no-fail-fast -j 4 --no-default-features --features headless -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/tauri-tau-lcov.info   # merges BOTH profraw sets

python3 .future/cov100/covlines.py total  /tmp/tauri-tau-lcov.info --under desktop/src-tauri/src/headless \
                                                                    --under desktop/src-tauri/src/terminal \
                                                                    --under desktop/src-tauri/src/cli_tool.rs \
                                                                    --under desktop/src-tauri/src/agent_host.rs \
                                                                    --under desktop/src-tauri/src/agent_supervisor.rs \
                                                                    --under desktop/src-tauri/src/skills_bootstrap.rs
python3 .future/cov100/covlines.py detail /tmp/tauri-tau-lcov.info --under desktop/src-tauri/src/terminal
```

The script that runs exactly this is `.future/cov100/measure-tau-tau.sh` (first
segment) and `.future/cov100/measure-tau-tau2.sh` (second segment, which deletes
its own private target dir); their logs are `/tmp/tau-measure.log` and
`/tmp/tau-measure2.log`. `cargo test --lib <filter> -- --test-threads=1` (no
instrumentation) is the inner loop.

Known-host failures that are **not** this unit's regression and must not be
"fixed" here (`.future/cov100/TAURI.md`, `.future/memory/testing-gotchas.md`):
the rotating `remote_peer::transfer*` / `commands::remote_peer::*download*` /
`sandbox_availability` failures under one serialized run.

## 2. Before / after (gate metric: per-line `DA:0`)

_Before_ = the supervisor's merged baseline `.future/cov100/baseline/tauri-merged-lcov.info`
(revision `8449acf6`). Two measured *afters*:

* **after-1** = `/tmp/tauri-tau-lcov.info` (first segment's tests; §3.2): gui lib
  suite **1965 passed / 4 failed / 12 ignored**, headless **1574 passed / 5 failed / 12 ignored**.
* **after-2** = `/tmp/tauri-tau2-lcov.info` (second segment's tests added; §3.1,
  run by `.future/cov100/measure-tau-tau2.sh`, log `/tmp/tau-measure2.log`):
  **gui lib 1980 passed / 0 failed / 12 ignored**, headless **1592 passed / 2 failed / 12 ignored**.
  The two headless failures are
  `remote::tests::runtime_tests::start_runs_the_full_bridge_and_stop_winds_it_down`
  and `remote_peer::transfer_tests::a_chunk_write_on_a_dead_socket_is_recorded` —
  the documented, host-wide, rotating `remote*` family
  (`.future/cov100/GOTCHAS.md`); neither is in this unit's scope. A copy of the
  lcov is kept at `.future/cov100/tauri-tau-after2-lcov.info`; the private target
  dir was deleted by the same script.

| file / scope | before | after-1 | after-2 | after-2 / measured |
|---|---|---|---|---|
| `src/terminal/server.rs` | 75 | 70 | **59** | 59 / 1836 |
| `src/terminal/pty.rs` | 42 | 21 | **18** | 18 / 458 |
| `src/terminal/session.rs` | 26 | 20 | **14** | 14 / 727 |
| `src/terminal/manager.rs` | 20 | 8 | 8 | 8 / 568 |
| `src/terminal/shell.rs` | 11 | 11 | 11 | 11 / 153 |
| `src/terminal/cwd.rs` | 4 | 4 | 4 | 4 / 280 |
| `src/headless/mod.rs` | 160 | 105 | **104** | 104 / 806 |
| `src/headless/agent.rs` | 4 | 4 | 4 | 4 / 25 |
| `src/cli_tool.rs` | 50 | 15 | 15 | 15 / 382 |
| `src/agent_host.rs` | 26 | 24 | 24 | 24 / 272 |
| `src/agent_supervisor.rs` | 108 | 81 | 81 | 81 / 233 |
| `src/skills_bootstrap.rs` | 26 | 17 | 17 | 17 / 171 |
| **scope total** | **552** | **380** | **359** | 359 / 5911 |

**193 lines covered** by the two segments (552 → 359). The `measured` column
grows with each added test (the new test code is itself instrumented):
`terminal` 3895 → 4186 → 4279, `headless` 604 → 818 → 831, `cli_tool` 265 → 382,
`agent_supervisor` 183 → 233. The *after* numbers are therefore not comparable to
the baseline by percentage alone; the uncovered-line counts above are.

Reproduce the split:

```bash
python3 .future/cov100/covlines.py files  /tmp/tauri-tau2-lcov.info \
    --under .worktrees/cov100-r2/desktop/src-tauri/src/terminal \
    --under .worktrees/cov100-r2/desktop/src-tauri/src/headless \
    --under .worktrees/cov100-r2/desktop/src-tauri/src/cli_tool.rs \
    --under .worktrees/cov100-r2/desktop/src-tauri/src/agent_host.rs \
    --under .worktrees/cov100-r2/desktop/src-tauri/src/agent_supervisor.rs \
    --under .worktrees/cov100-r2/desktop/src-tauri/src/skills_bootstrap.rs
```

(`--under` is matched against the path with the `/Users/geilige/future-os/`
prefix stripped, so a run from the worktree needs the `.worktrees/…` form.)

**Scope honesty:** this checkout is shared with three peer units (their edits and
tests are in the same tree), so the *whole-crate* totals of this lcov include
their in-flight code and are not comparable to the baseline revision. The table
above is per-file for this unit's scope, which is comparable: every `before`
value is the same file at `8449acf6`.

### 2.0 Measurement hazard inside this scope (report it, do not hide it)

`terminal/server.rs` **533** and **539** — the `break` of the pump's "the shell is
gone, so writing this input frame failed" arm, one for a `Binary` frame and one
for `Text` — are **not stable**: covered in the after-1 report, uncovered in
after-2, with no code change in that region. The mechanism is a genuine race, not
a stale report: the pump ends the tunnel when it receives `SessionEvent::Exited`,
and those two lines are reachable only if a client input frame is processed
*before* that event in the same `select!`. The real-socket e2e
(`terminal::server::end_to_end::the_pump_answers_pings_carries_input_and_ends_with_the_shell`)
reaches that ordering only when the shell happens to have died before its input
frame is dequeued, so the pair flips between runs — the same ±N hazard the
supervisor flagged for `channels/feishu_ws.rs:254`. Consequence: **this unit's
residual is 359 ± 2**, and a reviewer comparing two runs must expect those two
lines to move. They are classified `unreachable-in-this-environment` in §4.1 with
that mechanism.

### 2.1 The second segment, in the same metric

The `after-2` column of §2 **is** the second segment's measurement; it was taken
by `.future/cov100/measure-tau-tau2.sh` (log `/tmp/tau-measure2.log`, lcov
`/tmp/tauri-tau2-lcov.info` plus a copy at
`.future/cov100/tauri-tau-after2-lcov.info`), which deleted its own private target
dir in the same invocation. It reports **359** uncovered lines for this scope,
i.e. **21 fewer** than after-1.

Uninstrumented evidence (the compiles-and-passes half of the claim), from
`cargo test --lib -- --test-threads=1 terminal:: cli_tool:: agent_supervisor:: skills_bootstrap:: agent_host:: headless::`:

```
test result: ok. 189 passed; 0 failed; 0 ignored; 0 measured; 1803 filtered out
```

The lines that segment was written for (all of them were OPEN rows of §4.2 before
it) and the test that covers each:

| line(s) | file | test |
|---|---|---|
| 334 | `terminal/pty.rs` | `the_ps_based_session_query_parses_and_survives_a_broken_ps` (its fixture `ps` now also prints a single-column line) |
| 448, 450 | `terminal/pty.rs` | `wait_for_exit_answers_a_cursor_query_and_then_reaps_the_child` |
| 656-662 | `terminal/session.rs` | `draining_a_quiet_viewer_waits_out_idle_moments` |
| 534, 547 | `terminal/server.rs` | `the_pump_carries_binary_input_and_ends_on_close` (unix) |
| 2397-2404 | `terminal/server.rs` | `client_frames_carry_every_length_form` |
| 2500-2502 | `terminal/server.rs` | `the_fake_socket_shuts_its_writer_half_down_cleanly` |
| 343 | `headless/mod.rs` | `an_accepted_platform_key_needs_no_terminal` |

Those are assertions about behaviour, not line touches: the frame test decodes
every masked payload back and checks both extended length prefixes, the pump test
asserts the markers reached a `cat` child's stdin (so the bytes cannot be the
shell's own echo alone) and that the shell stays listed after the viewer closes,
and the drain test asserts the wait really lasted to its deadline.

The whole-crate suite is **not** green on this host and never has been: the
rotating `remote*` family (`remote_peer::transfer_tests`,
`commands::remote_peer::*download*`, `sandbox_availability`,
`remote::tests::runtime_tests`) fails on a clean `origin/main` too
(`.future/cov100/GOTCHAS.md`, "desktop(src-tauri) 整套 lib 测试在本机 macOS 会死锁 + 2 个
host 相关的确定失败"). after-1 recorded 4 + 5 such failures and after-2 0 + 2; none
is in this unit's scope, and **this unit's scope is green in both flavors** (the
scope-filtered run above, and 1980/1980 in the after-2 gui library suite).

**Post-measurement edits (unmeasured, test code only).** Two regions of
`server.rs` were open after after-2 and needed no fixture: **548**
(`Some(Ok(_)) => {}`, "any other client frame is ignored") is covered by adding an
unsolicited `Pong` frame to the unix pump test, and **533/539** are the
oscillating pair of §2.0 — left alone deliberately, since forcing them would mean
asserting a race. Only 548 was taken, so the next measurement should read
**358 ± 2**.

### 2.2 The third segment, in the same metric

The segment-3 baseline was **re-measured** (the supervisor's instruction: after-2
predates the unit's own gate fixes *and* the `cli_tool` guard of
`docs/testing/flaky-tauri.md`, so its line numbers and its `measured` totals are
not the revision this segment ships on). Command, both flavors, one report — the
§1 recipe with `CARGO_TARGET_DIR=target/cov-tauri-tau3`, driver
`/tmp/tau3-measure.sh`, log `/tmp/tau3-measure-before.log`:

```bash
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
cargo llvm-cov --no-report --no-fail-fast -j 4 --no-default-features --features headless -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/tauri-tau3-before-lcov.info   # exit 0
```

| file / scope | before (seg-3) | after-2 (old) | after (seg-3) | after / measured |
|---|---|---|---|---|
| `src/terminal/server.rs` | 58 | 59 | **58** | 58 / 1839 |
| `src/terminal/pty.rs` | 18 | 18 | 18 | 18 / 460 |
| `src/terminal/session.rs` | 14 | 14 | 14 | 14 / 727 |
| `src/terminal/manager.rs` | 8 | 8 | 8 | 8 / 569 |
| `src/terminal/shell.rs` | 11 | 11 | 11 | 11 / 153 |
| `src/terminal/cwd.rs` | 4 | 4 | 4 | 4 / 280 |
| `src/headless/mod.rs` | 104 | 104 | **51** | 51 / 826 |
| `src/headless/agent.rs` | 4 | 4 | **1** | 1 / 25 |
| `src/cli_tool.rs` | 11 | 15 | 11 | 11 / 384 |
| `src/agent_host.rs` | 24 | 24 | **26** | 26 / 272 |
| `src/agent_supervisor.rs` | 81 | 81 | 81 | 81 / 233 |
| `src/skills_bootstrap.rs` | 28 * | 17 | 17 | 17 / 173 |
| **scope total** | **365** | 359 | **300** | 300 / 6198 |

**65 lines fewer** on the same scope, 300 = 365 − 53 (`headless/mod.rs`) − 3
(`headless/agent.rs`) − 11 (`skills_bootstrap.rs`) + 2 (`agent_host.rs`).

Two entries in that table are *measurement* effects rather than code, and both are
named so no reviewer has to guess:

* `skills_bootstrap.rs` **28\*** — the pre-segment-3 run lost that test
  (`skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin`)
  to the load-induced 3 s `probe` deadline (`CLI discovery timed out`), which left
  its drain-loop lines uncovered. It passed in the segment-3 run, so the honest
  reading is **17 → 17** for that file and **54** lines closed by the segment's work.
  The same deadline flake fired once in the segment-3 *gui* leg, on
  `cli_tool::tests::a_runnable_cli_on_path_is_discovered_and_probed`; it cost
  nothing measurable (that file's uncovered set is identical in both runs) because
  the headless leg runs the same tests. Both are `FLAKES.md` item 27's surviving
  half — the deadline, not the leak — and the successor todo for it is named in the
  delivery report.
* `agent_host.rs` **24 → 26** — segment 3 closed the attach-only/mode refusals
  (160, 208) and *lost* four external-attach lines (111, 265, 319, 320) that the
  pre-segment-3 run happened to cover: those four are the branch taken when the
  host fixtures' mock Agent answers `select_agent` inside its 2 s budget, so they
  oscillate with load. §4.2 carries them with that mechanism; the residual is
  therefore **300 ± 4** in the same sense §2.0 gives `terminal/server.rs` ± 2.

**The after figure is a union of three executions** — the segment-3 gui leg, the
segment-3 headless leg (both in `/tmp/tau3-measure-after.log`, both
`--no-fail-fast`), and one re-run of the `headless_cli` integration target
(`/tmp/tau3-union-leg2.log`) taken because the *first* integration run of the gui
leg lost its own revoked arm to a fixture bug that is written up in §8.6. All
three share the profile store in `target/cov-tauri-tau3` (no `clean` between
them), and the report is `/tmp/tauri-tau3-union-lcov.info` (`cargo llvm-cov
report` exit **0**). A union of executions can only *lower* the residual — never
report a line as covered that no run executed — which is the §5.3 direction
argument.

### 2.3 `cli_tool.rs` at HEAD — re-measured, and the statement set changed

The segment-3 union (`aff0501b3`) measured a **626-line** `cli_tool.rs`. Two
commits moved it afterwards: `224dcea1e` ("NEW-B deadline seam", 626 → 758) and
`1876ffe78` (the Linux port of the probe fixtures: 11 insertions / 2 deletions,
758 → **767**, the revision this record now describes).

The re-measurement at `36c568972` is the §1/§6a-ter recipe: one render per
flavour, `cargo llvm-cov clean` between the legs, then a union.

```bash
cargo llvm-cov clean
cargo llvm-cov --lcov --no-fail-fast -j 4 --output-path /tmp/d8-gui.info -- --test-threads=1
cargo llvm-cov clean
cargo llvm-cov --lcov --no-fail-fast -j 4 --no-default-features --features headless \
    --output-path /tmp/d8-headless.info -- --test-threads=1
python3 .future/cov100/lcov-union.py --out /tmp/d8-union.info /tmp/d8-gui.info /tmp/d8-headless.info
python3 .future/cov100/leg-flavour-check.py /tmp/d8-gui.info /tmp/d8-headless.info   # LEG IDENTITY: ok
python3 .future/cov100/flavor-alive.py /tmp/d8-gui.info /tmp/d8-headless.info        # both carry nonzero hits
```

Driver `/tmp/d8-measure.sh`, log `/tmp/d8-measure.log`, union retained as
`.future/cov100/tauri-tau-d8-union-lcov.info`. **Both legs came back green**:
gui lib **2142 passed / 0 failed / 14 ignored**, headless lib **1717 / 0 / 14**,
and every integration target passed in both (this removes the §8.5 §5.3 caveat
for this revision: there is no red run to reason around).

The file's residual is **still exactly 11 lines, but the set is not the same** —
the previous note's claim that "the 11 uncovered statements are unchanged" was
wrong in two directions. Measured `DA:0` set at HEAD: **83, 92, 285-288, 293,
305, 356, 672, 759**.

| union line (`aff0501b3`) | HEAD line (`36c568972`) | what changed |
|---|---|---|
| 82 | 83 | pure shift (`"future.exe"` under `cfg!(windows)`) |
| 91 | 92 | pure shift (`"bundled"`) |
| 223-226 | 285-288 | shift (the `try_wait` `Err` arm) |
| 231 | 293 | shift (post-exit stderr limit) |
| 243 | 305 | shift (post-exit stdout limit) |
| 294 | **356** | shift; the earlier note wrote 357, which is the following `set_var` statement |
| 338 | **none — now covered** | the `EnvGuard::drop` `None` arm (HEAD 400) is measured `DA>0`: `remote/verify_e2e.rs` arms the same guard with five absent `VERIFY_E2E_*` names |
| 598 | **672** | the fail-only message argument; the earlier note wrote 666, which is `let started` (covered). `1876ffe78` added six lines above it |
| — | **759** | **new statement**, not a shift: the `panic!` body of the `unwrap_or_else` closure the NEW-B seam introduced |

So 356 and 672 correct two numbers the previous note got wrong, 400 is no longer
a residual line at all, and 759 is a statement that did not exist at the union
revision. The mechanisms are in §4.1; the pairing's `expect` is re-recorded from
this report in §4.3.

## 3. What changed

### 3.1 Second segment (2026-10-16, after the first measurement)

Seven more tests, all for the cheapest OPEN rows of §4.2, plus one fixture:

| file | test | what it asserts |
|---|---|---|
| `terminal/pty.rs` | `wait_for_exit_answers_a_cursor_query_and_then_reaps_the_child` | a child that asks for the cursor and then finishes on its own is reaped instead of wedging the wait to its deadline (the reply carries no terminator, so the *echo* is the only trace — and the helper has drained that channel by then; delivery of the reply to the child is asserted one layer up, in `session.rs`) |
| `terminal/pty.rs` | (`the_ps_based_session_query_…`) fixture fix | the fixture `ps` now also prints a single-column line, so the “not a pair” arm is exercised |
| `terminal/session.rs` | `draining_a_quiet_viewer_waits_out_idle_moments` | a live, silent session is not mistaken for a finished one: the drain waits out idle moments to its deadline |
| `terminal/server.rs` | `client_frames_carry_every_length_form` | the frame builder’s three length forms (7-bit / 16-bit / 64-bit) and the masked payloads they carry |
| `terminal/server.rs` | `the_fake_socket_shuts_its_writer_half_down_cleanly` | the test transport’s `poll_shutdown` reports ready and writes nothing |
| `terminal/server.rs` | `the_pump_carries_binary_input_and_ends_on_close` (unix) | both client-input arms (Binary + Text) reach the child’s stdin, an unsolicited `Pong` is ignored rather than typed, and a `Close` frame ends the tunnel while the shell stays running — the Windows-only test could not run here |
| `headless/mod.rs` | `an_accepted_platform_key_needs_no_terminal` | a key the platform accepts short-circuits the login flow without ever asking for a terminal |

Three test-code edits the additions required (none weakens anything):
`server.rs`'s test module imported `std::time::Instant` only under
`#[cfg(windows)]`, which the unix pump test also needs (now unconditional); the
`OP_CLOSE` and `OP_PONG` opcode constants were `#[cfg(windows)]` (now used by both
flavors — `OP_PONG` by the ignored-frame assertion, taken *after* the after-2
measurement, which is the 548 line of §2.1).

### 3.2 First segment

Test-only changes; **no production line was deleted, weakened or `#[cfg]`-ed
away**, and no assertion was relaxed (the one existing-test edit *adds* an input
the test already documents — see `session.rs` below).

| file | change |
|---|---|
| `src/terminal/test_support.rs` | two unix fixtures: `cursor_query_command` (asks for the cursor and *reads* the six-byte reply, printing its byte count) and `query_then_marker_command` |
| `src/terminal/pty.rs` | 6 tests: DSR handshake, pid sentinels + direct signalling, teardown without a process group, SIGKILL escalation, the `ps`-based session query, and the session sweep it feeds |
| `src/terminal/session.rs` | 3 tests (lagged drain, disconnected drain, DSR-answering wait helper) + one added input to `an_exiting_child_still_notifies_a_late_viewer` (queued output ahead of the held exit — the case its own comment describes) |
| `src/terminal/manager.rs` | 4 tests: the whole `ManagerError` contract (code/status/message), the `CwdError` conversion, `Manager::default`, and a DSR-asking tab through `wait_for_child_exit` |
| `src/cli_tool.rs` | 8 tests over a *runnable* CLI fixture: discovery + probe, the capability refusal, the changed-identity refusal, `prepare_path`, the empty display version, and the `probe` limits (timeout / output limit / non-zero exit) |
| `src/agent_supervisor.rs` | 4 tests: the GUI start path under a mock app, `authorize_exit` ⇒ no dialog, recovery-poll == plain poll, and the `AgentStatus` wire shape |
| `src/skills_bootstrap.rs` | 3 tests: the channel-closed drain, the real plugin spawn (marker file = the CLI really ran), and an unusable CLI |
| `src/agent_host.rs` | inline `mod coverage_tests`: a fresh host is idle/unowned and reports no activity; `request_stop` on it is a no-op |
| `src/headless/mod.rs` | 10 tests: the `future init` bootstrap (success, failure, missing CLI), the platform-login branches (rejected key, network failure), the malformed-reply wait, the unencodable-invitation fallback, `pairing_link` refusal, and `print_invitation` |

Test names, by file:

* `terminal/pty.rs`: `a_cursor_query_is_answered_on_the_childs_stdin`,
  `pid_running_separates_live_children_from_the_sentinel_pids`,
  `teardown_without_a_process_group_signals_the_pid`,
  `teardown_escalates_to_sigkill_for_a_child_that_ignores_sigterm`,
  `the_ps_based_session_query_parses_and_survives_a_broken_ps`,
  `the_session_sweep_signals_reported_members_and_escalates`.
* `terminal/session.rs`: `draining_a_lagged_viewer_stops_immediately`,
  `draining_a_detached_viewer_ends_immediately`,
  `a_cursor_querying_child_is_answered_by_the_wait_helper`.
* `terminal/manager.rs`:
  `every_manager_error_reports_its_code_status_and_message`,
  `a_read_only_conversation_keeps_its_own_error_code`,
  `the_default_manager_starts_empty_and_usable`,
  `a_cursor_querying_child_is_reaped_and_recorded_as_exited`.
* `cli_tool.rs`: `a_runnable_cli_on_path_is_discovered_and_probed`,
  `a_missing_capability_is_reported_with_the_cli_identity`,
  `a_cli_that_changes_during_discovery_is_rejected`,
  `prepare_path_prepends_the_discovered_cli`,
  `a_blank_display_version_is_rejected`,
  `a_silent_cli_is_killed_and_reported_as_a_timeout`,
  `a_flooding_cli_is_killed_at_the_output_limit`,
  `a_failing_cli_is_reported_with_its_exit_status`.
* `agent_supervisor.rs`:
  `supervision_through_the_gui_entry_point_is_also_safe_for_a_mock_app`,
  `an_authorized_exit_proceeds_without_a_dialog`,
  `the_recovery_poll_reports_the_same_phase_as_the_plain_poll`,
  `the_status_serialization_matches_the_webview_contract`.
* `skills_bootstrap.rs`:
  `drain_skill_events_ends_when_the_channel_closes_without_termination`,
  `the_bootstrap_spawns_the_cli_through_the_shell_plugin`,
  `a_cli_outside_the_runnable_contract_is_reported_and_ignored`.
* `agent_host.rs`: `a_fresh_host_is_idle_unowned_and_reports_no_activity`.
* `headless/mod.rs`: `the_builtin_skill_bootstrap_runs_the_cli`,
  `a_failing_builtin_skill_bootstrap_is_ignored`,
  `a_missing_cli_skips_the_builtin_skill_bootstrap`,
  `a_rejected_platform_key_is_reported_as_expired_authorization`,
  `a_network_failure_while_checking_the_platform_login_is_not_a_logout`,
  `login_wait_gives_up_after_persistent_malformed_replies`,
  `login_wait_backs_off_after_a_malformed_reply`,
  `an_unencodable_invitation_falls_back_to_the_link`,
  `pairing_link_rejects_codes_that_are_not_v2_invitations`,
  `print_invitation_writes_the_link_and_the_qr_only_when_asked`.

Three of these tests exist because the *fixture* had to be discovered first and
is worth knowing (they are the reason some lines below are covered at all):

* **The shell plugin works under `tauri::test::MockRuntime`.** `mock_builder()
  .plugin(tauri_plugin_shell::init())` + `app.shell().command(path).spawn()`
  really spawns a child on this host, which is what makes
  `run_builtin_skills`'s spawn path (and the drain loop's exit-code handling)
  testable without a real sidecar.
* **A unix shell can be made to ask for the cursor** (`test_support::cursor_query_command`):
  Windows/ConPTY shells query the cursor on their own and block until answered;
  a unix shell does not, so the fixtures ask (`ESC [ 6 n`) themselves and the
  test replies. `cursor_query_command` also *reads* the reply and prints its byte
  count, which is what turns "the helper wrote six bytes" into "the child
  received them".
* **The `ps` session column is unusable on macOS** (finding F1 in §5): a fixture
  `ps` supplies the pid/session pairs instead, which is how the parser and the
  sweep that consumes it are covered.

## 4. Residual uncovered lines (per line, with the mechanism)

Category vocabulary and the policy: `docs/testing/waiver-ledger.md`
(`unreachable-by-construction` needs the invariant, `unreachable-in-this-environment`
needs the host/fixture that cannot be provided here, `attribution-artifact` needs
the neighbouring covered lines as evidence, `platform-unmeasured` needs the OS
that measures it). The already-checked waivers of `docs/testing/module-tauri.md`
§5.1 and `.future/memory/coverage-llvm-cov.md` ("tauri / src-tauri 专坑") are
reused where they apply.

### 4.1 Waived (checked against the code and against the measurement)

Line numbers below are those of the **D8 union** report
(`.future/cov100/tauri-tau-d8-union-lcov.info`, measured at `36c568972`; §2.3),
the report the pairing is re-pointed at. The **after-2** table in §2 is retained
as history: the third segment's gate fixes added lines above several test
residues, so its numbers are not these. §4.4 is the explicit crosswalk between
the two revisions; it exists because an audit pointed at the retained after-2
artifact reports 27 lines as "unmentioned" and 13 as "stale" — both are revision
effects, not gaps in this ledger.

| lines | category | mechanism |
|---|---|---|
| `pty.rs` 161-165, 621-622 | `platform-unmeasured` | The `if cfg!(windows)` UTF-8 code-page block and its mirror assertion in `the_child_environment_is_scrubbed_and_tagged`: `cfg!` compiles both arms, so on macOS the five `command.env(…)` lines exist but never run. A Windows measurement executes them; the *unix* half of the same test is covered. |
| `pty.rs` 434, 456 | `unreachable-by-construction` | The two timeout returns of the test helpers `wait_for` / `wait_for_exit`. Every caller asserts a needle or an exit code that its fixture produces within 20 s; reaching either line means the assertion after it fails, i.e. only a failing run executes it. |
| `pty.rs` 745-747, 787-789, 792 | `unreachable-in-this-environment` | The deadline arm (message argument + 20 ms sleep) of the two teardown tests' reap loops: on this host the signalled child is usually already reaped by the first `try_wait`, so the loop does not spin. It *did* spin in the segment-3 union run, which closed 744, 785 and 786; the lines that stay uncovered are the same arms on the other polls. Needs a host where reaping the killed child is reliably slower than one poll (§8.3). |
| `pty.rs` 843, 886 | `unreachable-in-this-environment` | Fixture restores written as `match saved { Some(v) => set, None => remove }`. The `None` arm needs the variable to have been **absent** from the process environment when the fixture started; `PATH` is always set by the campaign harness (PLAN §2). |
| `skills_bootstrap.rs` 223, 253, 271 | `unreachable-in-this-environment` | The same restore shape (`match saved { Some => set, None => remove }`); the variables these fixtures pin are always present when the guard is created. |
| `headless/mod.rs` 952, 956 | `unreachable-in-this-environment` | The same restore shape in `with_path_and_broken_agent`: `PATH` is set by the harness, and `FUTURE_AGENT_GRPC_ADDR` is installed process-wide by the agent-bridge mock (`agent_bridge/test_support.rs:323`), which is why 952 and 956 read `DA:0` while their `Some` arms (951, 955) are each hit 4 times. |
| `cwd.rs` 451, 454 | `unreachable-in-this-environment` | The same restore shape; the pinned variable is present for the fixture's lifetime. |
| `session.rs` 526 | `unreachable-by-construction` | `on_eof` collects subscriber keys and then re-looks-up each token **under the same `MutexGuard`** (`inner` is locked at 522 and held through 533). A key taken from `keys()` therefore cannot be missing; the `else { continue }` arm is the compiler's requirement, not a reachable state. Evidence: 522-525 and 527-533 are covered in the same run. |
| `session.rs` 611, 796, 853, 1022, 1046, 1223, 1264, 1284, 1289 | `unreachable-by-construction` | Failure-only code: the helper's timeout `panic!` (611), the message arguments of passing assertions (`inner.buffer.len()`, `String::from_utf8_lossy(…)` ×5, `listed.len()`, `started.elapsed()`), the untaken arm of a passing `matches!` (1221), and the `Err(_) => break` of the bounded scan that follows a missing exit event (1022 — reaching it means the assertion two lines later fails). A passing run cannot evaluate them. |
| `manager.rs` 533 | `unreachable-by-construction` | The message argument of a passing assertion in the `shutdown_all` fixture; a passing run cannot evaluate it. |
| `manager.rs` 307-310 | `unreachable-by-construction` | `shutdown_checked`'s "Terminal children did not exit" error needs a session whose child is still un-reaped after `shutdown_all`, i.e. a process that survives `SIGKILL`. No fixture can provide one. |
| `manager.rs` 355, 361, 365 | `unreachable-by-construction` | `prune_exited`: `pop_front` cannot return `None` while `exit_order.len() > EXITED_LIMIT ≥ 0`; an id enters `exit_order` only while its session is exited, ids are issued monotonically and never reused, so a still-running session can never be in that queue; 365 is the closing brace of that never-taken branch. Evidence: 350-354, 356-360, 362-364, 366-368 are covered in the same run. |
| `pty.rs` (sweep) | — | not a waiver: the `ps`-based session sweep is empty on macOS, so it is *covered* through a fixture `ps` rather than through the platform query — see §5.1. |
| `cli_tool.rs` 83 | `platform-unmeasured` | The `name = if cfg!(windows) { "future.exe" } else { "future" }` arm. `cfg!` compiles both arms, so on macOS the Windows spelling exists but never runs; a Windows measurement executes it. |
| `cli_tool.rs` 92 | `unreachable-in-this-environment` | The `"bundled"` source label needs a usable `future` file **in the directory of the running executable**, i.e. beside the libtest binary in `target/<flavor>/deps/`. Writing one there would also make `resolve()` succeed for `host_environment::tests::a_host_without_a_resolvable_cli_still_starts_logging`, which asserts `resolve().is_err()` with an empty `PATH` and does **not** take `TEST_HOME_LOCK`; a scoped guard therefore cannot protect it under CI's parallel run. Evidence: 93-99 (the `"path"` label) are covered in the same run. |
| `cli_tool.rs` 285-288 | `unreachable-in-this-environment` | The `try_wait`-failed arm of `probe`. Needs `Child::try_wait` to return `Err` for a live child this process spawned; no fixture can make `waitpid` fail there (the process has no `SIGCHLD` reaper). Evidence: 283 (`Ok(Some)`) and 284 (`Ok(None)`) are covered. |
| `cli_tool.rs` 293 | `unreachable-in-this-environment` | The post-exit stderr re-check of the output limit. The poll loop (267-281) already kills a child as soon as *either* stream passes 64 KiB, so reaching this line requires stderr to grow past the limit **between** the loop's last `metadata()` call and the `try_wait` that returns `Some` — a microsecond window with no deterministic fixture. Evidence: 267-280 (the kill + "exceeded its output limit") are covered by `a_flooding_cli_is_killed_at_the_output_limit`. |
| `cli_tool.rs` 305 | `unreachable-in-this-environment` | The post-exit stdout re-check, same window as 293: `stdout.take(LIMIT + 1)` can only read past the limit if the file crossed it after the loop's check. Evidence: 298-304 are covered. |
| `cli_tool.rs` 356 | `attribution-artifact` | The closing brace of `if let Some(path) = std::env::var_os("PATH")` in `prepare_path`; its `None` region. The `Some` path is what the harness produces. Evidence: 354-355 (the `directories.extend`) and 357, 359-361 (`set_var` / `join_paths` / `Ok`) are covered in the same run (358 is a string line with no `DA` record). |
| `cli_tool.rs` 672 | `unreachable-by-construction` | The message argument (`started.elapsed()`) of `assert!(started.elapsed() >= …)` in `a_silent_cli_is_killed_and_reported_as_a_timeout` — evaluated only when the assertion fails. (It was 598 at the union revision; `1876ffe78` moved it to 672, §2.3.) |
| `cli_tool.rs` 759 | `unreachable-by-construction` | The `panic!("without a deadline the fixture's reply must be captured: …")` body of the `unwrap_or_else` closure in `a_reply_that_arrives_after_the_deadline_is_captured_only_without_one`. The closure runs only when the no-deadline probe returns `Err`, which is the failure the surrounding test exists to report. This statement is new since the union revision (the NEW-B seam, §2.3). |
| `skills_bootstrap.rs` 104 | `unreachable-by-construction` | `CommandEvent`'s `#[non_exhaustive]` wildcard `_ => {}` in `handle_skill_event`. The compiler requires the arm and the released enum has no further variants (`.future/memory/coverage-llvm-cov.md`, "`CommandEvent` 是 `#[non_exhaustive]`"). |
| `agent_supervisor.rs` 33 | `unreachable-by-construction` | `ensure_agent_running`'s real body is inside `#[cfg(not(test))]`. The lib is compiled twice (`cfg(test)` for the unit tests, without it for the bins/integration tests); only the former is executed by any test binary, which is the documented cfg-split residue (`.future/memory/coverage-llvm-cov.md`, "lib 双编译合并"). |
| `agent_supervisor.rs` 41, 45 | `unreachable-by-construction` | The `return false` of `ensure_agent_ready_for_login`: in `cfg(test)` `ensure_agent_running` is a constant `Ok(())` (lines 35-38), so `!matches!(…)` cannot be false in a test binary and the guard needs a real embedded-Agent start that returns `Err`. |
| `agent_supervisor.rs` 65-66 | `unreachable-by-construction` | The `is_stopping() → "stopping"` arm of `agent_status` requires `snapshot() == None` **and** `is_stopping() == true`. `coordinator::begin` sets `ADMISSION.closed = true` and registers the operation in one critical section, and neither is ever cleared, so the first branch (57) always wins. Evidence: 57-62 are covered in the same run. |
| `agent_supervisor.rs` 157-160, 162, 164-182 | `unreachable-in-this-environment` | `confirm_quit` builds and shows a dialog: it selects one of two messages, then chains `.dialog().message(…).title(…).kind(…).buttons(…)`, optionally `.parent(&window)`, and calls `.show(…)`. `tauri-plugin-dialog` drives **`rfd`'s native modal dialog**, i.e. a real window on this host that no automated run can dismiss. |
| `agent_supervisor.rs` 200-211, 216-218 | `unreachable-in-this-environment` | What is left of `request_quit`'s waiter thread: (200-211) the "Cleanup failed" native dialog when the stop operation returns `Err` and `force` is false, and (216-218) the dialog shown when the waiter thread itself fails to spawn. Both are `rfd` dialogs, not callable from a test binary without a dialog seam. The rest of the waiter body is **covered**: the macOS-only mock-loop test `gui::startup_paths_tests::an_exit_requested_from_the_mock_loop_takes_the_guarded_quit_arm` takes the Proceed arm and publishes the stop, and the D8 union measures `DA>0` at 188, 189, 191, 192, 196-199, 212-215, 219 and 220 (they were the `196-220` row before this measurement). |
| `server.rs` 115-117 | `unreachable-by-construction` | `spawn_server_task`'s `#[cfg(not(test))]` half: `crate::runtime::spawn` exists only in the non-test lib copy, which is linked but never driven by a test binary. Same cfg-split class as `agent_supervisor.rs` 33. |
| `server.rs` 149, 157, 525, 704, 168, 1184, 1900 | `unreachable-by-construction` / `attribution-artifact` | Four invariants: (149) the connection `Semaphore` is owned by `SERVER` and never closed, so `acquire_owned` cannot fail; (157) `SERVER` is a `OnceLock` set once in `bind` and never cleared, so the `if let Some(server)` guard is always true inside the accept loop; (525) the pump holds `Attachment` (which owns `Arc<Session>`), so its subscriber — and therefore the event sender — outlives the loop and `recv()` cannot return `None`; (704) the header loop runs over `buffer[..head_end]`, a slice that ends before the terminating `CRLFCRLF`, so no line it yields is empty. 168, 1184 and 1900 are closing-brace / `})` regions of test helpers and of the `spawn_server_task` call. Evidence for each: the neighbouring lines (148-156, 500-524, 700-703) are covered in the same run. |
| `server.rs` 535 | `unreachable-in-this-environment` | The closing-brace region of the pump's "the shell is gone, so this input frame could not be written" arm. Reaching the arm requires a client input frame to be processed **before** the `SessionEvent::Exited` that ends the tunnel in the same `select!` — an ordering, not a state, which is why this family oscillates between runs (§2.0). In the D8 union 533 and 535 are uncovered while **539** (the `Text`-frame sibling) is covered — it was uncovered in the segment-3 union — so only 535 is claimed here. |
| `server.rs` 140-142, 163-164, 496-497, 519-523 | `unreachable-in-this-environment` | (140-142) `TcpListener::from_std` must fail for a listener `bind` just created; (163-164) a transient `accept` error (EMFILE/ECONNABORTED) must occur; (496-497) the subscriber must be removed between `attach` and `activate`, a microsecond window the pump itself keeps empty; (519-523) a viewer must accumulate > `SUBSCRIBER_LIMIT` (1 MiB) of *unread* pending output before its `activate` call — the pump activates immediately after attach, so only a scheduler stall could produce it. |
| `server.rs` 1349-1353, 1962-1966, 1992-1996 | `platform-unmeasured` | The e2e's own DSR answering blocks (`answered_dsr = true; socket.send(Text("\u{1b}[1;1R"))`). A Windows ConPTY shell emits the query and blocks; the unix shell these tests spawn does not, so the arm never runs here. A Windows measurement executes it — the same reason `docs/testing/module-tauri.md` §6.1 had to teach the e2e to answer it on Windows. |
| `server.rs` 1342-1346, 1135, 1953, 1956, 1988, 2025, 2061 | `unreachable-in-this-environment` | The e2e loops' "some other frame arrived" / "nothing more" arms: the fixture shell emits no Text frame and never errors between the awaited frames, so no other variant is queued in those windows. (1135) `shared_server`'s `if let Err(error) = serve()` body — the *first* caller binds and serves, so a later caller is the only one that can see `serve` fail; the segment-3 runs did not reach it. |
| `server.rs` 1756, 1957-1959, 1989, 2026, 2062, 2617 | `unreachable-by-construction` | Fail-only assertions and loop-exit arms of the e2e: `panic!("socket error: …")`, `Err(_) => break`, and `panic!("the pump never wrote the control frame")`. Their loop guard is the awaited frame itself, so reaching one means the assertion immediately after it fails — only a failing run executes them. |
| `cwd.rs` 253 | `attribution-artifact` | The message argument of `assert!(!resolved.to_string_lossy().starts_with(r"\\?\"))` — evaluated only on failure. |
| `cwd.rs` 206 | `unreachable-by-construction` | `resolve_for_thread`'s `ThreadNotWritable` arm needs a thread row with `readonly = 1` or `deleted_at` set. No store API writes either (`delete_thread` hard-deletes), which the file's own test `a_thread_resolves_to_its_own_workspace_and_is_refused_once_deleted` already documents. |
| `shell.rs` 108, 113 | `unreachable-in-this-environment` | `account_login_shell`'s two `None`s need `getpwuid_r` to fail or to return an empty login shell. This host always resolves one for the current uid. |
| `shell.rs` 303, 315 | `unreachable-by-construction` | (303) both producers of a candidate (the account shell, `SHELL`) already reject an empty value, so `path.as_os_str().is_empty()` is never true; (315) the `Some(path)` arm for a *relative* candidate requires the candidate loop to reach a non-absolute path, which needs the account login shell (always absolute and executable) to be rejected first — see 325. |
| `shell.rs` 325, 329-332 | `unreachable-in-this-environment` | `tried.push` and the final `SHELL_UNAVAILABLE` error need **every** candidate unusable, including `/bin/bash` and `/bin/sh`. Needs a host without either. |
| `shell.rs` 296, 352 | `attribution-artifact` | Closing-brace regions of the `SHELL`-candidate push (296: the `if !shell.is_empty()` block) and of the `/etc/shells` read (352). Evidence: 291-295 and 344-351 are covered in the same run. |
| `headless/mod.rs` 281, 352, 649-651, 981, 1131 | `unreachable-by-construction` | (281) `require_terminal`'s `Ok(())` needs both `stdin` and `stdout` to be terminals, which a libtest process never has; (352) the closing brace of `ensure_login`'s `match fetch_profile()` — the match is exhaustive over `AppError`, so the fall-through region is impossible; (649-651) the body of the future passed to `until_shutdown` in `interrupt_drops_pending_startup_before_it_can_complete`, which the test asserts *never completes* (`completed` is false at 654); (981) the `Err(error) => panic!("the fixture CLI must be accepted by discovery")` arm of a `match` the fixture's CLI must satisfy — reaching it is the failure; (1131) the `Err(_) => return` of the mock HTTP server's bounded accept. |
| `headless/mod.rs` 836 | `attribution-artifact` | The closing brace of the `async move` block whose last expression is `std::future::pending().await` (835) in `interrupt_cancels_an_already_waiting_authorization_request`. The future is dropped while pending, so it never completes and llvm-cov attributes no hit to the block's end. Evidence: 830-835 and 837 are covered in the same run. |
| `headless/mod.rs` 313-315 | `unreachable-in-this-environment` | The spawn-failure arm of `bootstrap_builtin_skills`. To reach it the CLI must pass `usable()` **and** the identity re-read (which pins length, mtime and inode) and then fail to spawn; any change to the file between those two points is exactly what the re-check rejects. Same TOCTOU class as `skills_bootstrap.rs` 28-30. |
| `headless/mod.rs` 354-360, 364, 368-372, 374 | `unreachable-in-this-environment` | The device-code login flow after `require_terminal`: it needs an interactive terminal and a live platform authorization endpoint (`start_with_browser` + `wait_for_login` polling). `wait_for_login` itself is covered directly (§3). |

### 4.2 OPEN — reachable, not waived, next segment's work

Each row names the concrete next step; all of them are reachable from a test on
this host. The five single-test items this table named before the second segment
(pty 334/448/450, session 656-662, server 534-535/547-548, server 2397-2404,
server 2500-2502, headless 343) are now covered — see §2.1.

Not in the rows below, because they are *environments* rather than sections and
are classified with their mechanism in §4.1: `terminal/server.rs` 140-142 (a
transient `accept` error: EMFILE/ECONNABORTED), 163-164 (the same error's
back-off sleep), 496-497 (the viewer removed between `attach` and `activate`),
519-523 (a viewer past `SUBSCRIBER_LIMIT` that is still inactive when the pump
activates it — needs a scheduler stall, not a fixture), 533/535/539 (the
oscillating pair of §2.0), 1342-1346 and 1953-2062 (the e2e's "some other frame
arrived" arms), and the `#[cfg(not(test))]` halves (`server.rs` 115-117,
`agent_supervisor.rs` 33).
| lines | why it is not covered yet | next step |
|---|---|---|
| `agent_supervisor.rs` 58-62, 70-71, 73-75, 92-93, 116, 122, 125-130, 190, 193-194 | `agent_status` / `active_work_count` need a *controlled* Agent-host phase (`Starting`, `Failed`, `Stopped`, owned activity); `190` is the already-pending early return of `request_quit`, and `193-194` the `begin_quit() == None` arm. `agent_host::host()` is a process-wide singleton and the shutdown flags are sticky, so doing it in the shared libtest process is order-dependent. (The rest of `request_quit` is no longer OPEN: the macOS mock-loop test covers it, §4.1.) | a subprocess re-exec fixture in this file (`run_case`, the pattern `src/agent_host/tests.rs` already uses) with an isolated `FUTURE_HOME`/`FUTURE_AGENT_SOCKET`: cases `starting` (unanswering H2 endpoint), `failed` (held instance lock), `stopped` (real start→stop), `double-quit` (covers 190) and `no-quit-operation` (covers 193-194). |
| `agent_host.rs` 83, 88, 116, 124-126, 147-150, 200, 216, 223, 233-234, 250, 265-266, 281, 289, 334-336 | Each needs a host phase (`Ready`, `Stopped`, `Starting`) or a race this host cannot be made to lose. The **external-attach** arms 111 and 319-320 (and its stop path 265-266) are the oscillating family: the host fixtures take the attach arm only when their mock Agent answers inside `select_agent`'s 2 s connect+handshake budget. In the D8 union 111, 319 and 320 are measured `DA>0` (as in the segment-3 `before` run) and are therefore **not claimed** here; 265-266 are uncovered in this run. A row that claimed all four would be STALE in whichever direction the next run lands. The rest need a real startup timeout/cancel, a worker-thread spawn failure, or an un-reaped child at the stop deadline. | the same subprocess fixture (this file may host it): extend `CASE` handling in an inline `#[cfg(test)] mod` here rather than in `src/agent_host/tests.rs`, which is outside this unit's write set. **Segment 3 closed 160 and 208** with the agent-mode arm of `tests/headless_cli.rs` (§8.2), which shows the shape those two wanted: the *real binary* is a cheaper way to a controlled refusal than a phase injector. |
| `headless/agent.rs` 15 | `check_running` must observe `Phase::Failed`. | the subprocess fixture below, with a case that forces a failed phase (the segment-3 sessions cover the successful `ensure_running`, so only the failed branch is left). |
| `headless/mod.rs` 136-138, 147-149, 182, 188, 207-216, 220-221, 224-227 | The rest of `session()`: (136-138) the *failure* branch of the model-catalog refresh; (147-149) the `--re-pair` revocation, which needs an interactive terminal; (182) the "platform login changed" exit, which needs the account key to change mid-run; (188) the `available` arm of the phase line — with an unreachable pairing the supervisor reports `agent_available: false`, so the segment-3 arms take the other arm; (207-227) the pairing invitation, the `Ready` hand-over and the expired-invitation refusal, all of which need a `RemoteStatus` carrying a **pairing code** or `phase == Ready`, i.e. a *successful* `remote::start`. | (136-138) a platform whose **model catalog** endpoint fails while the profile one succeeds; (147-149) a PTY fixture (`portable-pty` is a dependency) so `require_terminal` passes; (188) an arm where the Agent is Ready but the remote start is not; (207-227) a **live NATS broker** — `connect_nats` sets `require_tls(true)` outside `cfg(test)`, and the crate's in-process fake broker (`remote/test_support.rs`) is `#[cfg(test)]`, so it is not linked into the binary. That one is `unreachable-in-this-environment` until this host has a broker with JWT auth and TLS. |
| `skills_bootstrap.rs` 28-30, 49-54, 69-70, 72, 80 | (28-30) the spawn failure after a successful discovery — a TOCTOU the identity re-check closes (see §4.1 `headless` 313-315 for the same mechanism); (49-54, 69-70, 72, 80) the stop path needs `desktop_shutdown::is_stopping() == true`, a process-global irreversible flag, plus a real `CommandChild` to kill and wait for. | (28-30) needs a fault-injection seam (see §6); (49-54/69-70/72/80) a subprocess fixture in this file that calls `desktop_shutdown::begin("fixture")` and drains a real plugin-spawned child (the plugin does work under `MockRuntime`, see §3). |
| `session.rs` 220, 222, 223, 231 | The reader thread's `Weak::upgrade() == None`, `EINTR` and read-error arms, and the thread-spawn failure. | needs a fault-injection seam for the reader (or a fixture child that makes the master read fail after its last byte); the manager kills the tree before dropping the session, so the `None` arm is a race. |

One class is listed last because it is a *decision*, not a task: the
`#[cfg(not(test))]` halves (`server.rs` 115-117, `agent_supervisor.rs` 33) can
ever only be executed by a real application run, and the campaign treats that as
a blind spot (`docs/testing/module-tauri.md` §5.3) rather than as waivable work;
they are nevertheless registered in §4.1 with that mechanism.

`server.rs` 1349-1353, 1962-1966 and 1992-1996 (the e2e's DSR answering blocks)
and `pty.rs` 161-165 stay `platform-unmeasured` (§4.1): a Windows measurement
executes them, this host cannot by construction.

### 4.3 Reconciliation

Per file: the measured residual (after-2, §2), the OPEN count (the rows of §4.2),
and therefore the withdrawn-as-waived count (§4.1). The table is exact by
construction — every line is either named in a §4.2 row or classified in §4.1 —
and a reviewer can regenerate both line lists with
`covlines.py detail /tmp/tauri-tau2-lcov.info --under <file>`.

| file | residual | §4.2 OPEN | §4.1 waived |
|---|---|---|---|
| `terminal/server.rs` | 58 | 0 | 58 |
| `terminal/pty.rs` | 18 | 0 | 18 |
| `terminal/session.rs` | 14 | 4 | 10 |
| `terminal/manager.rs` | 8 | 0 | 8 |
| `terminal/shell.rs` | 11 | 0 | 11 |
| `terminal/cwd.rs` | 4 | 0 | 4 |
| `headless/mod.rs` | 51 | 24 | 27 |
| `headless/agent.rs` | 1 | 1 | 0 |
| `cli_tool.rs` | 11 | 0 | 11 |
| `agent_host.rs` | 23 | 23 | 0 |
| `agent_supervisor.rs` | 67 | 23 | 44 |
| `skills_bootstrap.rs` | 17 | 13 | 4 |
| **total** | **283** | **88** | **195** |

(Counted from the **D8 union** report, §2.3. The arithmetic is exact: every
uncovered line of the scope is named in a §4.2 row or classified in §4.1.)

The D8 re-measurement moved the figure **300 → 283** without any change in this
unit's own test code, and the 17 lines are attributable line by line rather than
being a whole-file revision effect:

* `agent_supervisor.rs` **81 → 67** (−14): the macOS-only mock-loop test
  `gui::startup_paths_tests::an_exit_requested_from_the_mock_loop_takes_the_guarded_quit_arm`
  (landed by the shell unit after the segment-3 union) takes the Proceed arm and
  publishes the stop, covering 188, 189, 191, 192, 196-199, 212-215, 219-220.
  On a non-macOS runner that test does not exist, so these 14 lines are
  platform-dependent coverage, not a claim that they run everywhere.
* `agent_host.rs` **26 → 23** (−3): 111, 319 and 320, the oscillating
  external-attach arms, were covered in this run (they were uncovered in the
  segment-3 union and covered in its `before` run).
* `cli_tool.rs` 11 → 11: same count, different set (§2.3 — one statement covered
  by `verify_e2e`, one new fail-only statement from the NEW-B seam).

So across the three segments: **552 → 359 → 300 → 283**, i.e. **269 lines
covered** for this scope, **195 waived with a mechanism** and **88 OPEN** — of
which the fixture-shaped arms inside this unit's own test code are named
individually in §4.1. The residual is **283 ± 2** for the oscillating family
named above (`agent_host.rs` 111, 265-266, 319-320).

Remaining work, in the order a successor should attempt it:

1. `headless/mod.rs` 207-227 (12 of the 24 OPEN lines there) — the pairing
   invitation, the `Ready` hand-over and the expired-invitation refusal. All three
   need a *successful* `remote::start`, i.e. a NATS broker this host does not have
   (§4.2 has the mechanism). Reachable only with either a real broker or a
   production seam that lets the test binary use the `#[cfg(test)]` fake one.
2. `agent_host.rs` 23 + `agent_supervisor.rs` 23 — one subprocess re-exec fixture
   each, the pattern `src/agent_host/tests.rs` already uses. Segment 3 showed the
   cheaper half of this: the *real binary* closes the refusals (160/208) without any
   fixture, so a successor should try a `FUTURE_DESKTOP_AGENT_MODE`/held-lock case
   through `tests/headless_cli.rs` before writing a phase injector.
3. `skills_bootstrap.rs` 13 — a subprocess case that calls
   `desktop_shutdown::begin("fixture")` and drains a real plugin-spawned child.
4. `headless/mod.rs` 136-138, 147-149, 182, 188 — a model-catalog failure, a PTY
   fixture for `--re-pair`, a mid-run platform change, and a Ready Agent with a
   non-ready remote start. Four independent arms, each with a named next step.
5. `terminal/session.rs` 4 and `headless/agent.rs` 1 — the reader thread's
   `EINTR`/error/`Weak::upgrade` arms, and a forced `Phase::Failed`.

**What the total does and does not claim.** 283 is the measured residual at
`36c568972`, not zero, and this record does not call the module "done":
195 lines are classified in §4.1 and **88 remain OPEN** in §4.2. The headline
alone is not the 口径 A claim; the claim is that every one of the 283 is named.

### 4.4 Revision crosswalk: the retained after-2 artifact (359)

`reconcile.py` against `.future/cov100/tauri-tau-after2-lcov.info` (359 lines,
twelve scopes) reports `MISSING 27`, `STALE 13` (8 covered + 5 absent). Those
counts are **revision effects**, not 27 unclassified lines: §2.2 and §4 were
re-measured at segment 3, and §8.7 already records `MISSING 0 / STALE 0` for the
union (300 lines). The same audit against the union report is clean. This table
gives every line the after-2 audit names a disposition, so the older artifact is
not left unexplained.

**A. The 27 "MISSING" after-2 lines (uncovered then; what happened to each).**

| after-2 line | file | disposition at segment 3 |
|---|---|---|
| 131, 135, 139, 145, 146, 183, 184, 187, 195, 228, 229, 230, 258 | `headless/mod.rs` | **covered** by the segment-3 `session()`/`ensure_login` tests in `desktop/src-tauri/tests/headless_cli.rs` (`a_deferred_remote_start_keeps_serving_and_stops_on_a_signal`, `a_revoked_pairing_stops_the_run_with_the_reason_and_drops_the_credential`; §8.2, §8.3 rows 128-139, 142, 145-146, 154-170, 171-198, 229-230, 258). No waiver is owed. |
| 810, 912, 916, 941, 1086 | `headless/mod.rs` | **renumbered**, not closed: 810 → **811** and 941 → **948**, 1086 → **1098** (`unreachable-by-construction`, test-only panic/`Err` tails); 912 → **919**, 916 → **923** (`unreachable-in-this-environment`, fixture-restore `None` arms) — §8.3's closing paragraph; §4.1 carries each category. |
| 790, 841, 884 | `terminal/pty.rs` | **renumbered** to **792, 843, 886** (+1/+2 from segment-3 test fixtures), all `unreachable-in-this-environment` (reap-loop deadline arm / `None` restore); §4.1 cites the new numbers. |
| 1262, 1282, 1287 | `terminal/session.rs` | **renumbered** to **1264, 1284, 1289** (+2), `unreachable-by-construction` (message arguments of passing assertions); §4.1 cites the new numbers. |
| 251, 269 | `skills_bootstrap.rs` | **renumbered** to **253, 271** (+2), `unreachable-in-this-environment` (fixture-restore `None`); §4.1 cites the new numbers. |
| 103 | `cli_tool.rs` | **covered** by the segment-3 `desktop/src-tauri/tests/headless_cli.rs` run (the `"development"` source label, §8.3). |

None of the 27 is an unclassified line: 14 are covered at segment 3 and 13 are
the same statements as rows already in §4.1 at their new numbers.

**B. The 8 "COVERED" after-2 candidates (the other 5 STALE rows are the ABSENT group below).**

| after-2 line | file | why the after-2 report shows it covered |
|---|---|---|
| 811, 948, 1098 | `headless/mod.rs` | **line drift**: the statements are after-2's **810, 941, 1086**, which the after-2 report itself shows *uncovered*. Segment-3 test fixes inserted 1/7/12 lines above them; §8.3 records exactly this renumbering. |
| 111, 216, 319, 320 | `agent_host.rs` | **oscillation**: the external-attach `Ready`/`Stopped` returns, taken only when the mock Agent answers `select_agent` inside its 2 s budget. The after-2 run happened to take them; the union run did not. §2.2/§4.2 name the family. |
| 1135 | `terminal/server.rs` | **run difference**: the `shared_server` `if let Err(error) = serve()` body, reachable only by a *later* caller in the same process. The after-2 run reached it; the segment-3 runs did not. §4.1 records it as `unreachable-in-this-environment`. |

The first group is renumbering (the ledger is already correct at the union
numbers); the second and third are the documented ±N families. **No row is
retired**, because every one of these lines is genuinely uncovered in the union
report and carries a mechanism in §4.1/§4.2.

**C. The 5 "ABSENT" after-2 rows (no `DA:` record at that number).**

| after-2 line | file | real line |
|---|---|---|
| 598 | `cli_tool.rs` | after-2 **567**, the `started.elapsed()` message argument of the timeout assertion; segment 3's guard moved it to union **598**, `224dcea1e` to 666, and `1876ffe78` to **672** (§2.3 — 666 is `let started`, which is measured covered). |
| 747, 788, 789, 792 | `terminal/pty.rs` | after-2 **746, 787, 790** (the reap-loop `sleep`/`assert!` lines); segment-3 fixtures shifted them by +1/+2. §4.1 cites the union numbers (745-747, 787-789, 792). |

These are not evidence of a missing line: they are the union numbers read against
the older report. Reading the source at the union numbers confirms each is a real
statement, and §4.1 already carries the mechanism.

**D. The reconciliation rule.** Against the D8 union: `uncovered 283 / mentioned
283 / MISSING 0 / STALE 0 / COVERED 0 / ABSENT 0` (§2.3, and
`staletable.py --gate` below). Against after-2: `MISSING 0` after this
crosswalk (every line is named), `STALE 8` (the first two groups above) and
`ABSENT 5` (group C; the rows cite the union/D8 numbers). The 13 cannot be
driven to zero **while the same rows remain in the union ledger**, because those
numbers are uncovered in one report and covered — or absent — in the other; the
pairings registry therefore has to point at the report the ledger describes.
That registry change is now made: the `tauri-headless` pairing points at
`.future/cov100/tauri-tau-d8-union-lcov.info` with `expect.uncovered = 283`
(`.future/cov100/pairings.json`), and the earlier recommendation to use the
segment-3 union with `300` is superseded. Numbers in this section are after-2 /
segment-3 history; current line numbers are §4.1's.

## 5. Findings

### 5.1 F1 — `session_members` silently returns nothing on macOS (real defect)

`pty::session_members` is the guarantee behind `kill_tree`'s
"background jobs escaped the process group" sweep (the T02 FINDING-2 contract
quoted in the source). On Linux it reads `/proc`; on every **other** unix it runs
`ps -axo pid=,sess=` and keeps rows whose `sess` column equals the session id.
Measured on this host (macOS 15, arm64):

```
$ ps -axo pid=,sess=,pgid= | grep -E '57405|57408'
57405      0 57405 /bin/sh -c sh -c 'sleep 30' & sleep 30   # the PTY child: its own session AND group leader
57408      0 57405 sleep 30                                  # a background job, same group
$ ps -o pid,sess,pgid,command -p $$      # even a plain shell on a tty
56758      0 56758 /bin/zsh -c ...
```

`pgid` is populated, **`sess` is 0 for every process** — so
`session_members(sid)` matches nothing, `for member in &members` never runs, and
the grace loop's `all()` is vacuously true (it returns immediately). In other
words the FINDING-2 teardown holds on **Linux only**, silently: the existing
`teardown_reaches_background_jobs` test is `#[cfg(target_os = "linux")]`, which
is exactly why this was never seen.

*Consequence for this unit's numbers:* `pty.rs`'s sweep lines are unreachable on
macOS **because the platform query is empty, not because the code is dead** — so
they are covered here through a fixture `ps` that reports the pairs the function
is documented to consume
(`the_session_sweep_signals_reported_members_and_escalates`), which also pins
the escalation order (members SIGTERM → grace → fresh enumeration → SIGKILL →
shell last).

*Smallest seam (proposed, not applied — it is a production behaviour change):*
enumerate candidate pids with `ps -axo pid=` (which works on macOS) and ask
`libc::getsid(pid)` per pid — exactly the Linux branch's logic with a portable
pid list. The `sess` column can be dropped entirely. Applying it would make the
macOS sweep real; it needs its own cross-platform test (the Linux branch already
has one) and a decision from the terminal owner, so this record proposes it
rather than inventing it.

### 5.2 F2 — an `#[ignore]`-free test-suite hazard found while writing fixtures

A fixture CLI that appends to its own script while running
(`printf x >> "$0"`) makes the *shell reading that script* execute the appended
word: the child exits `127` (`x: command not found`) and the identity check it
was meant to exercise is never reached. The self-modifying fixture must append
something inert (`printf '\n' >> "$0"`). Not a product bug, but it cost a
debugging cycle and will bite the next person who writes a "the CLI changed under
us" test.

## 6. Seams worth deciding on (nothing here was invented silently)
| where | why it cannot be tested as written | smallest seam |
|---|---|---|
| `agent_supervisor::agent_status` / `active_work_count` (`Phase::Starting`/`Failed`/`Stopped` arms, `desktop_shutdown` branches) | the Agent host is a **process-wide singleton** (`agent_host::host()`); its phase only changes by starting/stopping a real embedded Agent, which the unit's own test process cannot do without breaking every other test in the same binary (the host fixture that can is `src/agent_host/tests.rs`'s subprocess re-exec, owned by another slice) | pass the phase/`Phase` in (or add a `#[cfg(test)]` phase injector beside `request_stop`) |
| `agent_supervisor::confirm_quit` / `request_quit`'s waiter | both call `app.dialog()…show()`, which reaches **`rfd`'s native modal dialog** on this host (a test would open a real window nobody can dismiss) and then `app.exit(0)` | inject the dialog/window effect (`*_with(dialog, exit)`), as the repo already does for relaunch elsewhere |
| `headless::session` (`mod.rs` lines ~128-230) | after `ensure_login` the entry point needs a live platform API, a real NATS/remote session and an interactive terminal; the integration test that drives the *binary* is `desktop/src-tauri/tests/headless_cli.rs`, which is **outside this unit's declared write set** | extend the write set to `tests/headless_cli.rs` and serve the platform endpoints from a fake loopback HTTP server (the shape `future_login`'s own tests use) |
| `skills_bootstrap::drain_skill_events` stop path | needs `desktop_shutdown::is_stopping() == true`, which is a process-global, irreversible flag (`coordinator::begin` sets `ADMISSION.closed` and nothing clears it); flipping it in the shared libtest process makes every later test see a stopping Desktop | a subprocess fixture (same re-exec pattern) or a test-only `reset` that restores admission |

## 7. Quality gates (PLAN §5.1/§5.2) — run, and their exit codes

CI runs typecheck/lint as **separate required steps**, so a green suite is not a
green PR. For a Rust unit the gates are `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings`; for `desktop/src-tauri` CI runs
exactly the three commands below (`.github/workflows/ci.yml`, job
`Rust fmt + clippy (Linux)`, step `rustfmt (desktop backend)` + step
`clippy (desktop backend)`).

| gate (exact command) | exit | scope of what is red |
|---|---|---|
| `cargo fmt --check --manifest-path desktop/src-tauri/Cargo.toml` | **1** | 4 files, **none of them in this unit's write set**: `src/commands/skills.rs`, `src/commands/tasks.rs`, `src/commands/threads.rs`, `src/commands/update.rs` |
| `cargo clippy --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings` | **101** | 5 errors, **none in this unit's write set**: `src/remote/verify_e2e.rs:1349,1366,1395,1406` and `src/remote_host/lean.rs:1437` (`clippy::needless_borrow`) |
| `cargo clippy --no-default-features --features headless --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings` | **101** | the same 5 errors (the headless flavor does not compile `terminal/`, `skills_bootstrap.rs`, `agent_supervisor.rs`) |
| `rustfmt --check` over this unit's 12 files | **0** | — |
| `cargo test --lib -- --test-threads=1 terminal:: cli_tool:: agent_supervisor:: skills_bootstrap:: agent_host:: headless::` | **0** | 189 passed / 0 failed / 0 ignored |
| PLAN §5.2 `reconcile.py` (per-line ledger vs the segment-3 union, §8.7) | **0** | 300 mentioned, **0 MISSING**, 0 stale (the §2 after-2 table's 359 is historical; §4.4 is its crosswalk) |

Script for the clippy pair (private `CARGO_TARGET_DIR`, deleted afterwards):
`.future/cov100/gate-tauri-clippy.sh`; logs `/tmp/tau-clippy.log`,
`/tmp/tau-clippy2.log`, `/tmp/tau-clippy3.log`.

**Both gates were red when first run, and this unit's share was real.** The first
run reported 12 clippy errors and 26 rustfmt diffs *inside this unit's files* —
all of them introduced by this unit's own test code:

* `cargo fmt --check`: three places where an edit of mine joined a statement onto
  the function's opening line
  (`headless/mod.rs` `login_denial_expiry_and_http_failure_do_not_authorize`,
  `terminal/pty.rs` `teardown_escalates_to_sigkill_…`,
  `terminal/server.rs` `client_frame`), plus four over-long assertion lines.
  Valid Rust, but exactly what CI's fmt step rejects.
* `cargo clippy -D warnings` (6 real lints, all in test code):
  `clippy::field_reassign_with_default` ×3 (`cli_tool.rs`, `headless/mod.rs`,
  `skills_bootstrap.rs` — `Default::default()` then assigning `commands`);
  `clippy::manual_is_multiple_of` (`terminal/pty.rs`);
  `clippy::needless_borrow` ×2 (`terminal/session.rs`);
  `clippy::unused_mut` (`terminal/session.rs`).

All six were fixed as **semantics-preserving test-code edits** (struct-update
syntax, `is_multiple_of`, dropping a redundant reborrow and the `mut` it made
unnecessary) — no assertion was changed, and the scoped suite is green again
(189/0) after the fixes. The remaining red in both gates is in peer units' files
and is outside this unit's declared write set, so it is **reported here rather
than touched**: a fix in `src/remote/verify_e2e.rs` or `src/commands/*.rs` would
write outside the coordination contract and could collide with the unit that owns
those files.

**Staleness note.** The `after-2` measurement (§2) was taken *before* these gate
fixes. The fixes are test-code-only and change no assertion, so the per-file
uncovered-line counts of §2 are expected to be unchanged; the *line numbers* of
test residues in `cli_tool.rs`, `headless/mod.rs`, `terminal/pty.rs` and
`terminal/session.rs` shifted by a few lines (the struct-update rewrites add
lines). The successor's measurement (`.future/cov100/measure-tau-tau2.sh`)
renumbers them, which is why §4.1 gives each row's mechanism rather than only its
line number.

### 7.1 Segment 3 — the gates re-run on the revision this segment ships

The staleness note above is now discharged: §2.2 is the re-measurement of the
current revision, and these are the gates on the same content. Every command was
run from `desktop/src-tauri` with the environment of §1.

| gate (exact command) | exit | scope of what is red |
|---|---|---|
| `cargo fmt --check --manifest-path Cargo.toml` | **1** | 4 diffs, **none in this unit's write set**: `src/agent_bridge/persist.rs:1422`, `src/agent_bridge/reconciliation.rs:830,900,942`. All 12 of this unit's files are clean under `rustfmt --edition 2021 --check` (checked file by file, not crate-wide — `PLAN` §7b). |
| `cargo clippy --all-targets --manifest-path Cargo.toml -- -D warnings` | **0** | — (the peer `remote/verify_e2e.rs` + `remote_host/lean.rs` errors recorded above are fixed: the owners were notified and the gui flavor is clean now) |
| `cargo clippy --no-default-features --features headless --all-targets --manifest-path Cargo.toml -- -D warnings` | **0** | — |
| `cargo test --lib -- --test-threads=1 terminal:: cli_tool:: agent_supervisor:: skills_bootstrap:: agent_host:: headless::` | **101** | 182 passed / **8 failed**, all eight in the same family: `cli_tool::tests::{a_cli_that_changes_during_discovery_is_rejected, a_failing_cli_is_reported_with_its_exit_status, a_flooding_cli_is_killed_at_the_output_limit, a_missing_capability_is_reported_with_the_cli_identity, a_runnable_cli_on_path_is_discovered_and_probed}`, `headless::tests::{a_failing_builtin_skill_bootstrap_is_ignored, the_builtin_skill_bootstrap_runs_the_cli}`, `skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin`. Every one is the 3 s `probe` deadline (`CLI discovery timed out`) under load 40 — the same flake the pre-segment-3 measurement lost `skills_bootstrap` to (§2.2), now visible as a *set*. **A green run of this same command is the segment-3 evidence for `agent_host`, `agent_supervisor`, `terminal` and `headless`** — the eight failures are all fixture-CLI discovery, so no assertion in this unit's own tests is implicated; but the command as run is red and is reported as red (log `/tmp/tau3-scoped-suite.log`). |
| `cargo test --no-default-features --features headless --test headless_cli -- --test-threads=1` | **0** | 12 passed / 0 failed — the 9 pre-existing subprocess tests plus the 3 segment-3 arms (§8.2); 3 consecutive runs, then once more inside the instrumented union run **at load 65** |
| PLAN §5.2 `reconcile.py` per scope (6 runs, §8.7) | **0** | MISSING 0, STALE 0 (0 COVERED, 0 ABSENT) for all six scopes |

Logs: `/tmp/tau3-fmt.log`, `/tmp/tau3-clippy-gui.log`,
`/tmp/tau3-clippy-headless.log`, `/tmp/tau3-scoped-suite.log`,
`/tmp/tau3-union-leg2.log`.

## 8. Segment 3 (2026-10-17) — the foreground session of the real binary

This segment closes the largest single block in the unit: `headless/mod.rs`
`session` (118-232), the entry point a server actually runs. Its rows were OPEN in
§4.2 with the note "needs the write-set extension to `tests/headless_cli.rs` plus
a fake loopback platform API"; both are in place, and the residue is renumbered
from the segment-3 union report (§2.2).

### 8.1 Why this block needs a subprocess, and why that is the honest way in

`src/headless/mod.rs` has a `#[cfg(test)] mod tests` that drives the parts of the
module which take their inputs as parameters (`ensure_login`, `wait_for_login`,
`print_invitation`, `starting_with_notice`, …). `session` cannot be driven that
way: it *is* the entry point. It reads the real account file, starts the real
embedded Agent, spawns the background observers and the scheduler, and then
supervises the remote bridge for the lifetime of the process. The pieces it drives
are process-wide singletons — `agent_host::host()` (one host per process, with a
one-way `Phase` state machine), the remote supervisor's `SUPERVISOR`, and
`desktop_shutdown`'s irreversible admission flag — so running it inside the
libtest process would leak that state into every other test in the lib target,
which is the failure class `docs/testing/flaky-tauri.md` documents.

Spawning the built `futureos-headless` binary is therefore the only way in, and it
buys what a unit test cannot: the `#[cfg(not(test))]` halves of the crate are
compiled into that binary and executed by it. Measured this segment: the
attach-only refusal and the agent-mode guard in `agent_host` (160, 208), the real
embedded Agent service scope in `agent_host::run`, and — the clearest case —
`cli_tool`'s `"development"` source label, whose candidate list the old ledger had
registered as `unreachable-by-construction` *because a libtest binary never
compiles it*. Segment 3's own report shows that row's line covered (§8.3).

What it did **not** buy, and this is worth stating because it is the boundary of
the method: `agent_supervisor.rs` stays at 81 uncovered. Its `#[cfg(not(test))]`
body (`ensure_agent_running`) is reached only from `commands/login.rs`, i.e. from a
Tauri **command** in the graphical build, and the headless binary never calls it.
A subprocess fixture for the headless path does not help there; that block still
needs the phase fixture §4.2 names.

The fixture is offline and deterministic — no production endpoint is contacted,
and no test can see another test's state:

| piece | what it is |
|---|---|
| account | `{home}/.future/agent/auth.json` with `future.key` + `future.base_url` pointing at the test's loopback listener (the shape the module's own `an_accepted_platform_key_needs_no_terminal` uses) |
| pairing | `{home}/.future/remote_pairing.json`, `handshakeVersion: 2` **with** `secure` (so `load_creds_checked` accepts it and `establish` takes the refresh path), a *real* user NKey seed generated in the test (`nkeys::KeyPair::new_user()`), and `natsUrl` on a closed port |
| platform | a loopback HTTP server owned by the test: `GET /client/v1/account/profile` → 200; every other path answers per arm (5xx, or 401 `invalid_remote_credential`) |
| Agent | `FUTURE_AGENT_GRPC_ADDR` is *removed*, so `select_agent("auto")` sees no external endpoint and takes `LocalStart`: the run starts the real embedded Agent in its isolated `FUTURE_HOME` |

### 8.2 The arms, and what each one asserts

Three tests in `desktop/src-tauri/tests/headless_cli.rs`, gated
`#[cfg(all(unix, feature = "headless"))]` like the file's existing subprocess
tests. The assertions are about behaviour, not line touches.

| test | state | asserted behaviour |
|---|---|---|
| `a_deferred_remote_start_keeps_serving_and_stops_on_a_signal` | platform up, pairing service **down** (5xx) | the run passes login (`Platform login authorized`), reports a **deferred** phase (`Remote: Reconnecting`) instead of exiting, is still serving three seconds later, leaves the saved pairing **byte-identical**, prints **no** invitation without a terminal, and stops cleanly on `SIGINT`; the platform's own request log must show the pairing endpoint was called, so the deferred phase cannot be a dead listener |
| `a_revoked_pairing_stops_the_run_with_the_reason_and_drops_the_credential` | platform up, pairing **revoked** (401 `invalid_remote_credential`) | the run passes login, then **fails** (exit 1) naming the reason (`Remote connection stopped (… CredentialRevoked …)`), prints no invitation, and the unusable credential is **dropped** rather than served forever |
| `a_non_auto_agent_mode_never_starts_an_embedded_agent` | `FUTURE_DESKTOP_AGENT_MODE` = `external`, then `bogus` | both are refused (exit 1) with their own reason, before login: attach-only mode must never start an embedded service, and an unrecognised mode is a configuration error, not a guess |

Measured wall time alone: 5.2 s, 3.0 s and 2.6 s; the whole integration target
12/12 in 8.5-11.8 s over four runs, including one *instrumented* run at load 65.

### 8.3 The lines this segment closed

| line(s) | file | what runs there |
|---|---|---|
| 128-139 | `headless/mod.rs` | `ensure_login` (the profile check against the fake platform), the account key and platform URL captured for the loop, and the tolerated model-catalog refresh |
| 142, 145-146 | `headless/mod.rs` | the headless scheduler start, the built-in-skill bootstrap spawn, and the `--re-pair` guard taken as *false* |
| 154-170 | `headless/mod.rs` | the six background projections, the headless task executor, the interrupted-run/approval reconciliation, and `remote::start` |
| 171-198, 205-206, 218, 223, 229-230 | `headless/mod.rs` | the supervision loop: `check_running`, the logout/platform-change guard, the phase line, the invitation/expiry bookkeeping and the re-read of the pairing from disk |
| 203-204 | `headless/mod.rs` | the terminal-phase exit (`Failed`/`Revoked`/`Stopped` → stop with the reason), from the revoked arm |
| (3 lines) | `headless/agent.rs` | `Agent::ensure_running`'s body against the real embedded Agent, which no test had ever started |
| 160, 208 | `agent_host.rs` | the agent-mode guard and the attach-only refusal, both `#[cfg(not(test))]` |
| 103 | `cli_tool.rs` | the `"development"` source label — the `#[cfg(not(test))]` candidate list exists in the binary the integration test spawns (§8.1) |
| 456, 479, 511, 535 | `cli_tool.rs` | the four hand-written fixture restores, **deleted** by the guard of `docs/testing/flaky-tauri.md` (the new `EnvGuard::drop` `None` arm is the one residue left, listed in §4.1) |
| 744, 785, 786 | `terminal/pty.rs` | two of the reap-loop deadline arms, taken this time (§4.1 keeps the other polls) |
| 533, 548 | `terminal/server.rs` | the oscillating pump arm (§2.0) and the "any other client frame is ignored" arm |
| 258 | `headless/mod.rs` | a normal completion of `starting_with_notice`'s awaited future (the slow-start notice never wins, so the call returns at that line) |

Not closures, for the avoidance of doubt: `810`, `941` and `1086` read as *covered* in an early reconcile pass only because the segment-3 report renumbers them (the unit's own gate fixes inserted lines). Under the segment-3 numbering the same three lines are `811`, `948` and `1098`, and the report shows all three **uncovered** — they are cited in §4.1, not closed.

### 8.4 What stays OPEN in that block, with the next step

§4.2's `headless/mod.rs` row carries the 24 lines and the four next steps; the
short version, because the reasons are mechanical rather than stylistic:

* **207-227 (12 lines)** — the pairing invitation, the `Ready` hand-over and the
  expired-invitation refusal need a `RemoteStatus` that carries a pairing code or
  `phase == Ready`, which only a *successful* `remote::start` produces. That needs
  a live NATS broker: `connect_nats` sets `require_tls(true)` outside `cfg(test)`,
  and the crate's in-process fake broker lives in `src/remote/test_support.rs`
  behind `#[cfg(test)]`, so it is not linked into the binary the test spawns. So
  these are `unreachable-in-this-environment` *for this host* — a broker with JWT
  auth and TLS would close them, and the harness is otherwise ready for it.
* **147-149** — `--re-pair` calls `require_terminal()` first. `portable-pty` is a
  dependency, so a PTY fixture is the next step, not a seam.
* **188** — the `available` arm of the phase line: with an unreachable pairing the
  supervisor reports `agent_available: false`, so the arms above take the other
  arm. An arm where the Agent is Ready and the remote start is not would close it.
* **136-138, 182** — the *failure* branch of the model-catalog refresh (the arms
  above had a reachable catalog) and the "platform login changed" exit (needs the
  account key to change mid-run).

### 8.5 The segment-3 measurement

Baseline and after are both in §2.2 (the re-measurement of this revision is what
the supervisor asked for, because after-2 predates the unit's gate fixes and the
`cli_tool` guard). Summary: **365 → 300** uncovered lines on the scope,
`headless/mod.rs` **104 → 51**, `headless/agent.rs` **4 → 1**, `skills_bootstrap`
28 → 17 *of which 11 are a measurement artefact* (§2.2), `agent_host` 24 → 26 with
four oscillating external-attach lines named. The after figure is the union of the
gui leg, the headless leg and one re-run of the integration target; `cargo llvm-cov
report` exit 0; lcov `/tmp/tauri-tau3-union-lcov.info` (beside
`/tmp/tauri-tau3-before-lcov.info`).

**§5.3 statement.** The three executions that make up the after figure each came
back red, and every failure is named with its cause: `remote_peer::runtime_tests`
×6 and `remote_peer::transfer_tests` ×1 (the documented host-wide `8022`
bind-contention family, outside this scope), `cli_tool::tests::a_runnable_cli_on_path_is_discovered_and_probed`
(the 3 s probe deadline under load, in scope, **cost 0 lines** — that file's
uncovered set is identical to the pre-segment-3 run where the same test passed),
and the segment-3 revoked arm itself, whose first instrumented run failed on the
fixture bug of §8.6 and cost `headless/mod.rs` line 203 — recovered by the re-run
that is part of the union. So: no failure of the union leaves a line that the
segment's tests would have covered unaccounted for, and the direction argument
holds (a union of executions cannot report a line as covered that no run reached).

### 8.6 F3 — a fixture that changed which arm it was measuring (macOS `O_NONBLOCK`)

The first instrumented run of the revoked arm did not fail; it **measured
something else**. The child's log showed
`Failed to refresh remote credential: error sending request for url (…auth/token)`
and `Remote: Reconnecting`, i.e. the *deferred* arm of the previous test — while
the assertion waited 300 s for an exit that this arm never produces.

Mechanism, and it is a platform trap worth knowing: on macOS a socket accepted
from a **non-blocking** listener inherits `O_NONBLOCK`, so a handler's `read` that
arrives before the client's bytes returns `EAGAIN`
(`Resource temporarily unavailable (os error 35)`) instead of waiting. The
fixture's per-connection handler treated any read error as fatal, closed the
connection, and the client reported a transport failure — which the entry point
correctly turns into a *deferred* start. The whole arm had quietly become a
duplicate of the wrong test, and the failure looked like a timeout.

Fix: `set_nonblocking(false)` (and an explicit read timeout) before reading, one
thread per connection so a stalled connection cannot hold up the next request, and
an accept loop that records transient errors and keeps listening instead of
returning. The arm now also asserts *which* endpoints the platform served, so this
class of silent swap fails loudly next time. Verified: 12/12 on four runs,
including an instrumented one at load 65 — the load that had produced the wrong
arm. This is the same family as §5.2 F2 (a fixture whose behaviour, not the code
under test, decided what was measured).

### 8.7 PLAN §5.2 tripwire, per scope (segment-3 report)

`reconcile.py` needs one `--scope` per file prefix, so it was run six times
against `/tmp/tauri-tau3-union-lcov.info`:

```bash
for s in headless terminal cli_tool.rs agent_host.rs agent_supervisor.rs skills_bootstrap.rs; do
  python3 .future/cov100/reconcile.py --report /tmp/tauri-tau3-union-lcov.info \
      --scope "desktop/src-tauri/src/$s" --doc docs/testing/module-tauri-headless.md --label TAU-D3
done
```

Result for every scope: **MISSING 0, STALE 0** (0 COVERED, 0 ABSENT) — 300
mentioned, 300 uncovered. Two findings came out of running it as the segment's
first act, both fixed in the rows rather than in the check:

* the previous ledger had a **MISSING** line — `headless/agent.rs` 12 was never
  named (the row said "10-11, 15") — which the renumbered row now carries;
* 16 rows cited lines the *new* report shows covered (the closed lines of §8.3),
  and one cited a line that no longer exists (`cli_tool.rs` 567 → 598 after the
  guard). The steering's warning applied exactly here: a row claiming an uncovered
  line that the report shows covered is a ledger **overstating** its residual, so
  each was moved out of the ledger (into §8.3's prose) rather than re-pointed.



