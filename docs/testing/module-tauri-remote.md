# Module: `tau-remote` — the `desktop/src-tauri` remote lane

Scope (unit TAU-A, goal `goal_160bc247aa63`, todo `todo_dd42c37ab9b9`):

```
desktop/src-tauri/src/remote/          the bridge: NATS lane, secure channel, supervisor, e2e harness
desktop/src-tauri/src/remote_host/     the host-side command surface the phone calls
desktop/src-tauri/src/remote_peer/     the client role: pairing, link, session, transfers
docs/testing/module-tauri-remote.md    this file
```

Baseline: revision `8449acf6` (`origin/main` at campaign start), whose per-line
counts are in `.future/cov100/baseline/tauri-merged-lcov.info` (both feature
flavors merged). **1125 uncovered lines in 31 files** across the three subtrees at
that revision: `remote` 681 (of which `remote/verify_e2e.rs` alone is 399),
`remote_host` 428, `remote_peer` 16.

## 1. Run identity and the exact measurement

Host: macOS 26.6.2, arm64. Toolchain: rustc 1.97.0 (`rust-toolchain.toml`).
Worktree `/Users/geilige/future-os/.worktrees/cov100-r2`, branch `test/cov100-r2`,
whose `desktop/src-tauri/src/{remote,remote_host,remote_peer}` were
byte-identical to the main worktree at `8449acf6` when this unit started — the
baseline's `SF:` paths (`/Users/geilige/future-os/desktop/...`) and its line
numbers therefore apply to this tree at the start of the unit.

`desktop/src-tauri` is not a workspace member and builds in **two feature
flavors**; one merged LCOV over both is the metric (`.future/cov100/TAURI.md`).
One instrumented measurement, private target dir, `--test-threads=1` (this
crate's lib suite deadlocks in parallel on this host):

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home
ulimit -n 10240
export CARGO_TARGET_DIR=/Users/geilige/future-os/.worktrees/cov100-r2/target/cov-tauri-remote

make -C /Users/geilige/future-os/.worktrees/cov100-r2 desktop-sidecar-placeholder
cargo llvm-cov clean
cargo llvm-cov --no-report -j 4 --no-fail-fast -- --test-threads=1
cargo llvm-cov --no-report -j 4 --no-default-features --features headless \
    --no-fail-fast -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/tauri-tau-remote-lcov.info
```

The `report` step merges the profraw of **both** runs. Per-line counts, the
campaign's metric (`DA:<line>,0`; never the summary block):

```bash
python3 .future/cov100/covlines.py total  /tmp/tauri-tau-remote-lcov.info --under desktop/src-tauri/src/remote
python3 .future/cov100/covlines.py detail /tmp/tauri-tau-remote-lcov.info --under desktop/src-tauri/src/remote_host
python3 .future/cov100/covlines.py files  /tmp/tauri-tau-remote-lcov.info
```

Run identity of the numbers below: report written **2026-10-16 20:54 CST**
(`/tmp/tauri-tau-remote-lcov4.info`), `gui` flavor **exited 0** (every test
target green) and `headless` **exited 101** with
`test result: FAILED. 1595 passed; 5 failed; 12 ignored; 0 measured; finished in
434.50s` — all five failures are the host flake of §5.2, and `--no-fail-fast`
keeps the profraw usable either way. `-j 4`, `--test-threads=1`. The gui flavor's
own summary line was interleaved with the next target's header in the captured
log, so its count is quoted from the uninstrumented run of the same 2017-test
set (§ gates below): 2004 passed / 1 failed / 12 ignored, the one failure being
the load flake that passes alone and in this instrumented run.

Iteration used the uninstrumented suite
(`cargo test --manifest-path desktop/src-tauri/Cargo.toml -j 4 -- --test-threads=1`),
~4x cheaper; the instrumented run was taken after the tests were believed
complete, and repeated when it was not (see §5.1).

### The gates CI runs (PLAN §5.1), as of this handoff

| command | exit | note |
|---|---|---|
| `rustfmt --edition 2021 --check <every .rs in the three subtrees>` | **0** | the scoped form of `cargo fmt --check`; `cargo fmt` was never run across the crate (other units own other files) |
| `cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets -- -D warnings` (default = `gui`) | **0** | after fixing the 5 errors it found in this unit's new code (§5.1); one further error it reported was in `src/terminal/session.rs:1212` (another unit's file, fixed by its owner during this run) |
| `cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --no-default-features --features headless -- -D warnings` | **0** | the second flavor |
| `cargo test --lib -j 4 -- --test-threads=1` (uninstrumented, whole lib, gui) | **2004 passed / 1 failed** | that one failure was `skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin` (a CLI-discovery timeout under the campaign's machine load); it passes alone in 0.44 s and was green in the instrumented run |

## 2. Before and after

`covlines.py total` over each subtree (uncovered = `DA:0` records):

| subtree | measured | before (8449acf6) | after | Δ |
|---|---|---|---|---|
| `desktop/src-tauri/src/remote` | 11030 | 681 | **331** | −350 |
| `desktop/src-tauri/src/remote_host` | 8674 | 428 | **106** | −322 |
| `desktop/src-tauri/src/remote_peer` | 3426 | 16 | **13** | −3 |
| **total** | 23130 | **1125** | **450** | **−675** |

The `after` column is this unit's **first landing**. Later landings moved it
further down — §4's current pin is **1195** counted lines (**343** without the
real-network harness), i.e. 1078 / 106 / 11 per subtree. Read §2 as the shape of
the drop, not as the residual.

Per file, every file this unit moved (before → after):

| file | before | after |
|---|---|---|
| `desktop/src-tauri/src/remote/verify_e2e.rs` | 399 | **70** |
| `desktop/src-tauri/src/remote_host/lean.rs` | 151 | **19** |
| `desktop/src-tauri/src/remote_host/business/tasks.rs` | 106 | **0** |
| `desktop/src-tauri/src/remote_host/business/wire_limits.rs` | 66 | **6** |
| `desktop/src-tauri/src/remote_host/sync_measurement.rs` | 53 | **40** |
| `desktop/src-tauri/src/remote/test_support.rs` | 22 | **2** |
| `desktop/src-tauri/src/remote_host/business/catalog.rs` | 17 | **11** |
| `desktop/src-tauri/src/remote_host/session_files.rs` | 3 | **1** |
| `desktop/src-tauri/src/remote_host/catalog.rs` | 1 | **0** |
| `desktop/src-tauri/src/remote/publisher/coalesce.rs` | 1 | **0** |
| `desktop/src-tauri/src/remote_peer/transfer.rs` | 3 | **0** |
| `desktop/src-tauri/src/remote_host/business/mod.rs` | 2 | **0** |

Two files moved the other way for a reason worth knowing: `remote/publisher.rs`
10 → **12** (its tick loop is timing-driven — two lines that a previous run
happened to execute are `DA:0` here; see §5.3) and `remote_host/business/*`
totals shift by a line or two where a helper moved. Untouched by this unit and
unchanged in kind: `remote/supervisor/start.rs` 173, `remote/transport.rs` 31,
`remote/supervisor/shutdown.rs` 24, `remote/commands.rs` 10, `remote/transfer.rs`
6, `remote_host/business/{settings,prompt,providers}.rs` 8/7/7,
`remote_host/files.rs` 3, `remote_host/{pairing,read_pages}.rs` 1/1,
`remote_host/business/{history,transfers}.rs` 1/1, `remote/secure.rs` 1,
`remote/web_server.rs` 1, `remote/lifecycle.rs` 1, `remote_peer/runtime.rs` 12,
`remote_peer/session.rs` 1.

## 3. What changed

### 3.1 Tests added (grouped by file; all assert behaviour, none is `#[ignore]`d)

| file | tests |
|---|---|
| `remote/verify_e2e.rs` | `a_journal_dump_parses_line_by_line_with_its_defaults`, `an_oversized_event_is_measured_as_the_truncated_body`, `a_reply_summary_separates_the_wire_size_from_the_decrypted_one`, `a_real_broker_reads_its_url_from_the_environment`, `the_subscription_probe_round_trips_its_own_echo`, `the_phone_accounting_finds_the_connection_by_name`, `a_long_replay_drains_the_bounded_queue_while_it_publishes`, `the_real_traffic_lane_holds_its_invariants_on_fixture_inputs`, `the_catalog_measurement_publishes_activity_only_to_a_declared_client` |
| `remote_host/business/tasks.rs` | `a_summary_longer_than_the_budget_is_cut_on_a_character_boundary`, `the_phone_vocabulary_maps_to_the_kernel_enums`, `creating_a_task_keeps_the_conversation_and_retention_the_phone_chose`, `a_long_run_summary_is_cut_on_the_wire`, `pausing_running_and_deleting_answer_the_stored_record`, `a_home_that_cannot_hold_a_store_is_reported_by_every_task_command`, `an_unknown_task_is_named_by_every_command_that_takes_one`, `a_row_that_cannot_be_decoded_is_a_read_failure_not_an_empty_answer`, `applying_a_revision_that_does_not_exist_is_refused`, `a_task_name_already_in_use_is_refused_by_both_writes` |
| `remote_host/business/catalog.rs` | `a_created_or_renamed_workspace_is_not_confirmed_from_a_catalogue_we_cannot_read`, `a_fork_whose_source_lookup_fails_is_not_reported_as_a_missing_source`, `a_refused_fork_is_reported_with_the_agents_reason` |
| `remote_host/lean.rs` | `the_render_fixture_derivation_trims_through_the_shipping_code`, `the_trim_measurement_reports_where_the_bytes_went`, `guard_arms_tests::unparsable_or_missing_bodies_are_forwarded_untouched` |
| `remote_host/sync_measurement.rs` | `a_body_split_across_two_writes_is_reassembled`, `an_rpc_request_without_the_probe_header_is_rejected`, `the_gzip_declaration_is_honoured_on_the_measurement_lane` |
| `remote/commands.rs` | `bridge_tests::the_mock_answers_an_unknown_tool_call_identity_with_nothing` |
| `remote_peer/transfer_tests.rs` | `the_prune_keeps_a_partial_write_and_tolerates_a_cache_it_cannot_read`, `the_prune_is_a_no_op_when_no_home_can_be_resolved` |
| `remote_host/business/wire_limits.rs` | `the_phone_page_measurement_holds_its_invariants_on_a_fixture_page` |
| `remote_host/session_files.rs` | `entries_that_are_neither_files_nor_directories_are_skipped` (`#[cfg(unix)]`) |
| `remote_host/catalog.rs` | `a_snapshot_that_is_not_a_list_has_one_signature` |
| `remote/test_support.rs` | `paginate_backward_takes_the_newest_user_exchanges_up_to_the_limit` |
| `remote/publisher/coalesce.rs` | `the_measurement_environment_is_restored_the_way_it_was` |

34 tests. The fixtures are the store's own error paths rather than
re-implementations: `$HOME/.future` as a plain file (the task store cannot be
opened), a task id that no longer exists, a duplicate task *name* (what actually
makes the store's insert/update fail), and out-of-band SQLite rows whose columns
hold values the record decoder refuses — written through a direct connection
because the store's write path cannot produce one.

### 3.2 Seams introduced (four; each is the smallest change that made an
env-gated harness reachable from the ordinary suite)

The three `#[ignore]`d measurement/fixture drivers in this scope are gated on env
vars only their `scripts/measure/*.py` driver sets (a live agent's journal dump,
a real `nats-server`, a browser fixture directory). Their *bodies* are ordinary
code and were the largest single block of uncovered lines, so each was split so
the body takes its inputs as arguments:

| seam | was | now |
|---|---|---|
| `remote/verify_e2e.rs`: `run_real_traffic_with(broker, journal, session, run, entries, monitor)` | `run_real_traffic(broker)` read four env vars and did the work | the reader is 12 lines; `the_real_traffic_lane_holds_its_invariants_on_fixture_inputs` drives the body on the in-process broker |
| `remote_host/lean.rs`: `derive_render_fixtures(dir: &Path)` | the whole derivation lived inside the `#[ignore]`d fixture generator | the ignored test is the env gate + one call; `the_render_fixture_derivation_trims_through_the_shipping_code` drives it on a fixture it writes to a temp dir |
| `remote_host/lean.rs`: `measure_entries(&mut Value) -> Value` | the per-kind byte report was printed inside the ignored measurement | the ignored test prints the returned report; `the_trim_measurement_reports_where_the_bytes_went` asserts it |
| `remote_host/business/wire_limits.rs`: `measure_page(&Value) -> Value` | the phone-page measurement was one `#[ignore]`d function | the ignored test prints `measure_page`'s report; `the_phone_page_measurement_holds_its_invariants_on_a_fixture_page` asserts the invariants |

No production behaviour changed: the seams only move the env read out of the
body, every `#[ignore]` and its reason string is intact, and no assertion was
weakened (the one assertion changed in this unit was *strengthened*: a refused
fork now has to name the Agent's own reason). **The follow-up seam proposed here
for the biggest remaining block (`remote/supervisor/start.rs`, §4.1) — a
`start_once` variant taking an injected "connect result" — is WITHDRAWN: the
supervisor ruled against it on 2026-10-17 (§6.2), because the injection point
does not exist yet and creating one would be a production redesign whose only
purpose is to make lines measurable. Those 132 lines are registered instead.**

### 3.3 Assertion expressions made eager (a real reduction, not a waiver trick)

An `assert!(cond, "…{}…", expr)` evaluates `expr` **only when the assertion
fails**, so llvm-cov reports that line as uncovered however green the suite is.
Fifteen such sites in this unit's own harness files were rewritten to compute the
value into a local (or to call a helper such as
`sync_measurement::tests::RoundTrip::answered()`), which the assertion then
reports. The assertions are unchanged in strength and the failure messages carry
the same values; 28 `DA:0` lines disappeared this way rather than being
registered (`verify_e2e.rs`, `sync_measurement.rs`,
`remote_host/business/mod.rs`, `remote_host/business/tasks.rs`).

### 3.4 The cross-module fixture (added for TAU-C4, the shell unit)

The shell unit's registered zeros need a *running* remote bridge in a test that
lives outside `src/remote/**`: `lib.rs`'s auto-connect pass and
`desktop_shutdown.rs`'s remote-deadline arm. `SUPERVISOR` and `RemoteState` are
`pub(in crate::remote)`, so a test in `lib.rs` can neither build nor install one;
the only installer used to be `tests::runtime_tests::{fake_state, install_state}`,
private to the remote test module.

Added (all `#[cfg(test)]`, no production surface):

| item | what it does |
|---|---|
| `remote::test_support::install_fake_state(tag) -> InstalledFakeState` | starts its own in-process NATS broker, builds a connected `RemoteState` (pending tasks, generation 1) and installs it into `SUPERVISOR.state`; the pair id is `unique(tag)`. Call from inside a Tokio runtime context. |
| `remote::test_support::InstalledFakeState` | RAII guard; `Drop` takes the state out, aborts the generation (`abort_generation`), resets `start_requested`/`suspended`, clears `bridge_shared` and kills the broker — poison-tolerant locks only, so it also runs while a panicking test unwinds (`FLAKES.md` item 27 / §4c: a mutex is not a restore). |
| `remote::test_support::fake_state` | the existing builder, **moved** here from `tests::runtime_tests` (which now imports it) so the shell fixture is the same implementation as the in-module one. |

Three tests in `remote/test_support.rs` (all green under
`--test-threads=1`; `cargo test --lib -- --test-threads=1 install_fake_state`):

| test | what it settles |
|---|---|
| `install_fake_state_installs_a_live_bridge_and_drives_the_deferred_arm` | the install is a live bridge (`status()` = `Ready`, unique pair id) and `remote::start` then returns `Ok(Failed/account_authorization)` — the auto-connect pass's **deferred** arm |
| `install_fake_state_drives_the_failed_arm` | a persisted pairing with an unreadable NKey makes `remote::start` return `Err` — the pass's **failed** arm — with no HTTP, NATS or port involved |
| `install_fake_state_restores_when_the_test_panics` | the guard really is panic-safe: after `catch_unwind` over a panicking body, `SUPERVISOR.state` is `None` |

**What this does and does not unblock** (measured/read, not assumed):

- The three-condition gate in front of the pass (`future_api_key`,
  `auto_connect_remote`, `load_creds().is_some()`) is already covered by the
  shell's own fixture, and the two arm bodies above are drivable from a shell
  test — **but `remote::start` is `pub` (re-exported by `crate::remote`), so
  those two arms do not need this fixture at all**; they need the home state
  (no sign-in / a bad stored NKey). The fixture is what a test needs when the
  row sits behind an *installed* bridge rather than behind `start`'s result.
- The third arm (`Ok(status)` with `phase == Ready`) is **not** drivable from
  this fixture: it needs a fully successful start, i.e. the mock platform + a
  broker + `bind_web_listener()` on port 8022, which is the route the shell unit
  explicitly rejected (`module-tauri-shell.md` §S2.7, the port this suite fails
  to release). Those two lines (`lib.rs` 1374/1375 at the measured revision)
  stay registered unless the shell unit re-decides that trade.
- `desktop_shutdown.rs`'s deadline arm: the fixture supplies the `state.is_some()`
  half (without it `mobile_disconnect_notice` returns `None` and
  `stop_gracefully` cannot await anything). The other half is arithmetic the
  shell unit has to choose: `stop_gracefully`'s notice is capped by
  `DISCONNECT_NOTICE_TIMEOUT` (**750 ms**, `remote/diagnostics.rs:161`) while the
  deadline it must exceed is `operation.remaining(2 s)` — and `remaining` is
  `min(limit, deadline − now)` (`desktop_shutdown/coordinator.rs:31`), so the
  2 s window can also be spent *before* the remote stage by the earlier
  `callers` wait. Starting the operation with a nearly-spent budget is a
  shell-side knob this unit cannot reach from `src/remote/**`; it was **not**
  measured here.
- Rejected while building this: a `kill_broker()` helper meant to stall the
  installed client. Measured and falsified — `FakeNats::kill()` drops the
  server's write half but leaves its read half alive, so a `publish`+`flush` on
  the client still completed in <300 ms. The helper was removed rather than
  shipped with a claim it does not meet.

Row numbers above are the ones in the shell record's report
(rev `e7231000`, report written 20:54 CST); the working tree has since drifted
~28 lines in that region, so `lib.rs`'s auto-connect block now sits at
**1400-1411** (spawn block 1400, `match` 1401, `Ready` arm 1402-1404, deferred
arm 1405-1408, failed arm 1409) and its `enabled` gate at 1391-1398. A shell test
should re-derive its own row numbers before writing them down.

### 3.5 TAU-D7: the nine OPEN lines, closed by tests

Seven test functions (in four files) and no product change. Each one reaches a
line that §4.2 of the previous revision had listed as OPEN; §4.2 now carries the
crosswalk, and this section states what each test actually drives.

| test | file | drives |
|---|---|---|
| `a_transport_failure_costs_one_retry_and_the_transfer_completes` / `a_chunk_that_never_lands_reports_its_first_failure` | `remote_peer/transfer.rs` (`mod tests`) | `retrying`'s two unexercised paths, through its `FnMut` step: one `RemoteTransport` then success, and every attempt failing (asserting the **first** message survives). A `mod tests` inside the file is required — `retrying` is private to it, which is exactly why the sibling `transfer_tests` module could never reach it. |
| `reset_for_test_aborts_a_resident_supervisor` | `remote_peer/runtime.rs` (`mod tests`) | the reset's drain, with a loop left resident by `install_supervisor_for_test`. The fixture's ordinary setup never sees one: `teardown()` has already emptied the map. |
| `an_activity_packet_arrives_tagged_with_its_catalogue_domain` | `remote_peer/runtime_tests.rs` | the `state.activity` lane: subscribe, create a row (structural packet the peer is guaranteed to see), then move only its recency with `record_thread_message_activity`; the peer must emit a `catalog` event tagged `sessions_activity`. |
| `a_run_scoped_page_repeats_its_run_on_every_chunk` | `remote_peer/runtime_tests.rs` | a ~640 KiB page (past the host's 512 KiB threshold) requested **with** a `runId`; the chunk fetch must repeat it or the host refuses the chunk with `remote_read_owner_mismatch`. |
| `a_fn_without_a_body_is_not_a_call_site` / `a_nested_block_comment_is_blanked_whole` | `remote/test_support.rs` (`mod tests`) | the two scanner branches no shipped source reaches: a `fn` token with no `{`, and a nested block comment (`/* /* */ */`). |

**Measurement identity for the pinned report.** `/tmp/tauri-tau-d7b-lcov.info`,
written 2026-10-17 16:02 CST, both flavors merged with the §1 procedure. `gui`:
**2114 passed / 1 failed / 14 ignored** in 551.58 s, the failure being
`gui::startup_paths_tests::an_exit_requested_from_the_mock_loop_takes_the_guarded_quit_arm`
(`src/lib.rs:2771`) — the shell unit's own load flake, unchanged and outside this
scope (§5.2). `headless`: **exit 0**, 1691 passed / 0 failed. `-j 4`,
`--test-threads=1`, `--no-fail-fast`.

**One test was flaky in its first version and was fixed before this run.** The
activity test originally created the thread *before* subscribing; whether the
structural snapshot had already gone out (NATS Core does not replay) then decided
whether the later recency move produced an activity packet at all. It passed
alone and failed in both full suites of the first measurement (gui 565 s,
headless 434 s) — a genuine race in the test's ordering, not in the product. The
committed order (subscribe → create → wait for the structural packet → move
recency) is deterministic; it ran 10/10 green across both flavors before this
measurement, and both flavors of this run are green for it.

## 4. Uncovered-line ledger (residual: 1195 lines; 343 of them predate the real-network harness)

**1195 waived · 0 OPEN · 0 unexplained.** Every residual line is registered in
§4.1 under one of the four categories of `docs/testing/waiver-ledger.md`, with
the mechanism verified in the source. The nine lines this record carried as
OPEN on 2026-10-17 were closed by tests rather than waived; §4.2 records that
closure line by line, and why each is now measured.

Line numbers are from `/tmp/tauri-tau-d7b-lcov.info`, written **2026-10-17
16:02 CST**, both flavors merged with the §1 procedure (`gui` exit 101 with one
failure, `headless` exit 0 — the run's identity is in §3.5). The delta from the
previous pin (`/tmp/tauri-tau-a3-lcov2.info`, 14:48) is **exactly ten lines**:
`remote_peer/transfer.rs` 458-459 and 466, `remote_peer/runtime.rs` 202-203,
1159 and 1699, and `remote/test_support.rs` 1838 and 1896-1897 — all now `DA>0`
because seven tests reach them (§3.5). Two registered rows moved *because those
tests were inserted above them* in their own files: `remote_peer/runtime.rs`
2694 → **2721** and `remote/test_support.rs` 2433 → **2477**. No line became
uncovered, and no new uncovered line appeared anywhere in the three subtrees
(the 60 production lines of `remote/publisher/coalesce.rs` added at 15:00 are
fully covered).

Three row groups were written against a still earlier report,
`/tmp/tauri-tau-remote-lcov4.info` (2026-10-16 20:54), and were re-pointed on
2026-10-17: `remote_peer/runtime.rs` (1103/1603/1643/1667/1680/1705-1707/2544
read `DA:0` there, and `DA:1327`/`2`/absent/`16`/`16`/absent/`6` in lcov2 — the
old numbers now land on *different, covered* code), `remote_peer/session.rs`
(668 → 767) and `remote/test_support.rs` (668/673 → 678/683). The source moved
under them in `ccb51ffa3` (2026-10-17 09:32, +43/−4 in `runtime.rs`) and
`5569d8c22` (10:52, +30/−2); those three groups were *record* drift, not
coverage regressions, and d7b reproduces every re-pointed number unchanged. The
`remote/supervisor/start.rs`, `remote/supervisor/shutdown.rs` and
`remote/publisher.rs` rows are the *other* case: their numbers were current and
stayed correct, but the mechanism claim they carried ("the real bodies stay
`DA:0` because only the `cfg(test)` copy runs") is refuted — the integration
tests spawn the instrumented binaries, whose non-test lib runs the
`start()`/shutdown entry path — so those rows are corrected in place instead of
re-pointed. The `real_network.rs` row's claim cell also carried a trailing
"(788 measured lines)", which made it parse as ZERO claims (the F.94 shape) —
the count is now only in the mechanism cell.

**One file moved after the measurement — and is now measured.**
`remote/publisher/coalesce.rs` was last written at 15:00 (after lcov2's 14:48),
adding 60 production lines at `:313` plus tests — the previous turn's
uncommitted §9 work, outside this ledger's write set. d7b (16:02) contains it and
reports **0 uncovered** for the file, so no row here depends on that revision
gap any more.

The count moved from 450 for one structural reason and one coverage reason, and
they must not be confused: the real-network ticket added **852 lines of
`#[cfg(test)]` harness code** whose tests are `#[ignore]`d for lack of an external
broker/Agent (`real_network.rs` 788, `verify_e2e.rs`'s cross-process
`InstalledHost` fixture 64) — those are *new* lines, not newly-missed product
code. Excluding them the unit is at **343**, down from 450. Of that drop, **67
lines are this ticket's** (the `verify_e2e.rs` driver/window row, 68 → 1); the
rest moved in other units' landings since the 450 was written (`start.rs` alone
reads 138 here against the 173 it carried then).


### 4.1 Waived, checked against the code

| file:lines | category | mechanism |
|---|---|---|
| `remote/supervisor/start.rs` 251, 274, 283, 286-289, 293-298, 312-314, 316-319, 322-331, 333-336, 369-372, 374-384, 386-394, 396-414, 416-421, 423, 433-439, 453-455, 457-463, 466-485, 487-488, 491-494, 558, 605, 644-646 | `unreachable-in-this-environment` (132 lines, count moved out of the claim cell: PLAN §5.2) | **cfg-split, not dead code; the failure-injection seam was ruled out by the supervisor (§6.2).** `spawn_start_retry` (`#[cfg(test)] return;` at `:246`, real body `#[cfg(not(test))] :248-298`), `spawn_runtime_reconnect` (`:307`/`:310-336`), `GenerationWatch` + its `run` (`#[cfg(not(test))]` types at `:361` and `:369-423`), `spawn_runtime_supervisor` (`:427`/`:432-494`) and `spawn_web_reconnect` (`:446`/`:451-494`) each put a test arm beside the real one, and `:558`, `:605` are the same shape inside their callers. **Corrected 2026-10-17 against lcov2: the non-test copy does not stay entirely `DA:0`.** The harness also runs three integration-test binaries: `tests/headless_cli.rs` and `tests/re_pair_entry_points.rs` *spawn the instrumented `futureos`/`futureos-headless` children* (`env!("CARGO_BIN_EXE_futureos")`), and `tests/gui_main_thread_drive.rs` (`harness = false`) re-executes its own binary (`std::env::current_exe()`); all three link the lib **without** `cfg(test)`, and the children call `remote::start()` (`start.rs:24`), so `spawn_revoke_cleanup` and `spawn_start_retry` really execute. Measured here: `:250-258` once (the singleton swap), `:260-282` twice or three times (one retry-loop iteration: sleep, retryable status, backoff), `:638-643`/`:647-648` (the revoke worker's `retry_pending_revokes` returning `Ok`). What stays `DA:0` is what that one child lifetime never reaches: the loop's two exits (`:274` no-longer-requested/Ready, `:283` a non-retryable status or `Err`), the whole post-loop tail (`:286-298`: clear the next-at stamp and the singleton, re-arm), the deeper reconnect/supervisor bodies (`:312-336`, `:369-421`, `:433-494`), and the revoke worker's `Err` report (`:644-646`). Reaching those needs a broker that goes retryable → Ready (or refuses non-retryably) while the app stays up, plus a persisted revoke that fails on retry — and the supervisor **ruled against building that failure-injection seam on 2026-10-17 (§6.2)**. Same category as the previous round's checked row; the count and the covered prefix are the correction. |
| `remote/supervisor/start.rs` 70, 81, 93, 124, 215 | `unreachable-in-this-environment` | `start_generation`'s early returns: the start-request latch cleared by a concurrent `stop()` between two awaits (`:70`, `:81`), the per-pairing shared runtime gone at `:93`, `build_transport` refusing at `:124` (a broker that answers CONNECT but refuses the secure handshake), and a racing `start` that already installed `SUPERVISOR.state` at `:215`. Each needs a state change landing inside another task's window; no seam exists and the fixture cannot schedule it. |
| `remote/supervisor/start.rs` 158 | `unreachable-by-construction` | the `else` arm of `if web_client_enabled()`. In the test build that function is the constant `true` (`remote/web_server.rs:34-40`), so no test input reaches the arm; the non-test copy is only linked into the bin. |
| `remote/verify_e2e.rs` 1429 | `unreachable-in-this-environment` (1 line, was 68 before §8) | the **`Some(url)` arm of the baseline read** — `Some(e2e.phone_accounting(url).await.expect("phone connection"))`. Driving it needs a monitoring endpoint alive *during* a real measurement that, at its second reading, reports exactly the bytes the run is about to produce: a number only the run itself knows. A scripted server therefore cannot satisfy the window's exact equality, and a monitor reading its own tap would be modelling the broker rather than measuring it. The rest of that window (the two subtractions, both assertions, the printed accounting) is covered — see §8.4 — so what is registered here is the *plumbing that supplies a live monitor*, not the comparison. |
| `remote/verify_e2e.rs` 1572 | `attribution-artifact` | the `if let (Some(before), Some(url)) = …` guard whose body is `assert_broker_accounting_against(...)`; the callee's lines are covered (they are exercised directly, §8.4) and llvm-cov attributes the *branch* line to the never-taken `Some` path, the same shape as the closing braces below. |
| `remote/verify_e2e.rs` 341-416 | `unreachable-in-this-environment` (64 lines) | the **cross-process `InstalledHost` fixture** (`install_host`, `install_host_with_creds`, its `Drop`). Its only caller is `remote/real_network.rs`, whose tests are `#[ignore]`d because they need `nats-server`, a `future-agent` and a model endpoint (§7); the ordinary suite never constructs it. Same category and the same reason as the harness file itself. |
| `remote/real_network.rs` 1-1065 | `unreachable-in-this-environment` (788 measured lines, count kept out of the claim cell: F.94) | **the real-network harness itself.** Every test in the file is `#[ignore]`d (they spawn two child processes, a real `nats-server` and a real `future-agent`); under the ordinary suite nothing in it runs. It is verification machinery, not product code: the behaviour it exercises is the shipping bridge/transfer/prompt path, measured in `docs/testing/real-network.md`. Registered rather than excluded because this campaign's metric is per-line `DA:0` in the merged LCOV, and an unregistered zero is an unwaived zero. |
| `remote/verify_e2e.rs` 1927, 2125 | `attribution-artifact` | `:1927` is the `Some(value) => set_var(…)` arm of the test's own env restore, which cannot be taken while the suite runs with `VERIFY_E2E_NATS_URL` starting unset — the `None` arm beside it **is** covered; `:2125` is the closing brace of the scripted `/connz` task's accept block, whose statements (`:2123-2124`) and continuation (`:2126-2127`) are covered. (Previously written as `:1769`/`:1967`; the drift is §8's new tests.) |
| `remote/transport.rs` 197, 231-244, 267-274, 293-300 | `unreachable-in-this-environment` (31 lines, count moved out of the claim cell: PLAN §5.2) | the credential-refresh failure paths: a refreshed credential the broker *rejects* (`:231-244`, needs a server that refuses the CONNECT), `build_transport` failing after a refresh (`:267-274`), and a generation/pairing that changed between two awaits of the refresh (`:197`, `:293-300`). Each needs a broker or a concurrent `stop()` to land inside the refresh window; the documented fix is a failure-injection transport. |
| `remote/supervisor/shutdown.rs` 12, 26, 95-96, 106, 225, 233-237, 243-250 | `unreachable-in-this-environment` (19 lines, count moved out of the claim cell: PLAN §5.2) | what remains `DA:0` after the integration-test children (spawned `futureos`, see the `start.rs` row) covered the notice *senders*: `:95-96` is the `crate::runtime::spawn` fallback of `stop_gracefully` (the child exits through the `is_stopping()` arm at `:92` instead), `:106` is `notify_mobile_disconnect`'s `Some` arm (the child calls it with no bridge installed, so `:103` takes the `None` return), and `:12`/`:26`/`:225`/`:233-250` need a live pair-scoped publish at the exact instant of `unpair`/`stop` and a `start_once` scripted to *fail* inside `handle_system_resume`'s recovery spawn so its retryable and `Err` arms run. Both are timing/state injections the fixture cannot schedule. `:92`, `:102-104` and `:107` were covered in this report and are no longer claims. |
| `remote/publisher.rs` 336, 532, 612-619 | `unreachable-in-this-environment` (10 lines, count moved out of the claim cell: PLAN §5.2) | the publisher's own tick task: `:336` needs `publish_batch` to fail mid-flush, `:532` a catalog read that fails between two statements of the tick, `:612` a secure publish that reports "not sent" (`Ok(false)`), and `:613-619` one that errors — i.e. the channel or the store losing its state between two ticks. A seam would have to inject a failing `secure::publish`/store read. The two lines this row used to add, `:624` (the block's closing brace) and `:628` (`continue` on a `None` workspaces read), read `DA:1` in lcov2 and `DA:0` at 20:54 and are dropped: that is the per-run scheduling flip §4.2 item 3 describes, not a different code path. |
| `remote/commands.rs` 318, 474 | `attribution-artifact` | closing braces of blocks whose statements are covered (`:313-317`, `:469-473`); llvm-cov attributes the region end to the brace. |
| `remote/commands.rs` 345, 800, 900, 903, 915 | `unreachable-in-this-environment` | `SUPERVISOR.access.commit` refusing a handshake epoch between two statements (`:345`, `:800`), a `REPLY` scope installed while the seal fails (`:900`), the `#[cfg(not(test))]` arm of that function (`:903`, cfg-split as in `start.rs`), and a `flush()` that fails on a healthy in-process broker (`:915`). |
| `remote/commands.rs` 879, 882 | `unreachable-by-construction` | `encode_reply_payload_with_gzip`: `write_all` into an in-memory `Vec` and `GzEncoder::finish` cannot fail (`:879`, `:882`). Kept by decision §6.1. |
| `remote/commands.rs` 885 | `unreachable-in-this-environment` | **Corrected 2026-10-17 (§6.1): not `unreachable-by-construction`.** `compressed.len() >= plain.len()` is the deflate-gives-up fallback, reachable for any payload ≥ `REMOTE_JSON_GZIP_THRESHOLD_BYTES` (32 KiB, `commands.rs:30`) that deflate fails to shrink — a property of the codec, not of this code, so no construction argument applies. Every byte-diverse JSON encoding measured shrinks, far from the ≥ 1.0 the arm needs: base64 0.77, hex 0.59, distinct-float array 0.49, supplementary-plane UTF-8 0.76, printable-ASCII 0.83 (at 40 KiB, previous round); and added by this pass at `Compression::fast` — random CJK 0.74, random supplementary-plane 0.67, random 2-byte UTF-8 0.81, 64 KiB base64 0.77, 3000 distinct keys 0.45. Why it cannot be reached: serialized JSON is text, so its zero-order entropy is far below 8 bits/byte (ASCII alone caps Huffman at 7), while this arm needs a ratio ≥ 1.0. Kept by decision §6.1. |
| `remote/transfer.rs` 44-48, 103 | `unreachable-in-this-environment` | a `queue_subscribe` the fake broker refuses (`:44-48`) and a seal failure on the transfer reply lane (`:103`); both need a broker/channel that fails on demand. |
| `remote/test_support.rs` 678, 683 | `platform-unmeasured` | the `else` arms of the mock's `probe_sandbox` default answer (`"unsupported_platform"` at `:678`, `"none"` at `:683`): the mock must report what its own host has, and this host is macOS, so `cfg!(target_os = "macos")` takes the `if` side and these are measured by a Windows/Linux run — the mirror image of the previous round's Windows measurement, which covered them and left these. The row previously said `:668`/`:673`; against lcov4 those were `DA:0` (the arms then sat there), and in lcov2 those numbers are the readiness mock's `protocolVersion` field and the `probe_sandbox` arm's opening line, covered 2508/767 times — the row was re-pointed, not re-argued. |
| `remote/secure.rs` 814 | `unreachable-by-construction` | `Ok(_) => panic!("an inert transport must not open")`: the line exists to fail if `open` ever returns `Ok` for a transport whose `enabled()` is false, the invariant asserted three lines above (`:811-813`). |
| `remote/web_server.rs` 43 | `unreachable-in-this-environment` | the `#[cfg(not(test))]` arm of `web_client_enabled()` (cfg-split, as in `start.rs`); the test build hits the `#[cfg(test)]` arm one line above. |
| `remote/lifecycle.rs` 40 | `unreachable-by-construction` | `changed.changed() == Err` needs every `watch::Sender` dropped while the receiver is borrowed from the same `AccessEpoch`; the sender is that struct's own field, held by the borrow, so the type makes it unproducible. |
| `remote_host/sync_measurement.rs` 822-861 | `unreachable-in-this-environment` (40 lines, count moved out of the claim cell: PLAN §5.2) | the `#[ignore]`d `serve_real_snapshot` harness: an isolated DB snapshot (`MEASUREMENT_ISOLATED_HOME`), a real Agent endpoint, a web root and port supplied by `scripts/measure/measure-sync-browser.py`, and a browser driving it. Previous round's checked row; the handler itself is covered by the 16 tests above. |
| `remote_host/lean.rs` 501 | `unreachable-by-construction` | `let Some(fields) = block.as_object_mut() else { continue }` inside the `"tool_call"` arm: taking that arm requires `block["kind"] == "tool_call"`, i.e. an object with a string `kind`, so `as_object_mut()` is always `Some`. |
| `remote_host/lean.rs` 1222 | `unreachable-by-construction` | `Some(false) => panic!("a history result must not send isError: false")` in the fixture validator: the repository's fixtures are generated from the shipping shapes and #849 stopped emitting `isError: false`, so a green suite cannot reach it — it exists to fail a fixture that regressed. |
| `remote_host/lean.rs` 1246-1249, 1414-1418, 1423-1426, 1428-1429 | `unreachable-in-this-environment` | the env gates and report printing of the two `#[ignore]`d drivers (`LEAN_RENDER_FIXTURES` from `scripts/screenshots/gen-lean-render-fixtures.mjs`, `LEAN_HISTORY_ENTRIES`/`LEAN_HISTORY_OUT` from `scripts/measure/measure-lean-history.py`). Their bodies are covered by §3.2. |
| `remote_host/lean.rs` 1354, 1453 | `attribution-artifact` | closing braces of conditional blocks whose bodies are covered (`:1352`, `:1451`); region ends attributed to the brace. |
| `remote_host/business/catalog.rs` 104-106 | `unreachable-by-construction` | "Fork produced no session.": `agent_bridge::fork_agent_session` returns `Err("Fork did not return a session.")` when the Agent's reply carries no session id (`agent_bridge/session.rs:273-275`) and only returns `Ok` with the id it created the thread for, so the check cannot fire. |
| `remote_host/business/catalog.rs` 122-128, 130 | `unreachable-in-this-environment` | "Forked thread could not be loaded" and the read error beside it: the row `get_thread(new_thread_id)` reads was written by the same call a statement earlier, so `Ok(None)` needs a concurrent deleter and `Err` a store failure between two statements of one function. |
| `remote_host/business/settings.rs` 148-150, 176-180 | `platform-unmeasured` | `product_sandbox_available()` is `Ok(true)` **unconditionally on macOS** (`settings.rs:300-304`), so neither the `Ok(false)` tier arms nor the probe's `Err` arm can be reached by a macOS measurement; the Windows/Linux flavor measures them. The previous (Windows) round covered them. |
| `remote_host/business/prompt.rs` 145-151 | `unreachable-in-this-environment` | the "Future Agent did not acknowledge the prompt" reply fires only when `accepted_rx` is dropped with no send, i.e. when the spawned `run_prepared_prompt_inner` future dies before its receipt send — a panic or an abort of a task whose `JoinHandle` nobody retains. No caller can abort it and the green suite produces no panic. |
| `remote_host/business/providers.rs` 58-64 | `unreachable-by-construction` | `reply_view`'s `Err(_)` arm for `serde_json::to_value(ProvidersView)`: the view is two `Vec`s of structs of `String`/`bool`/`usize`/`i32`, every one a derived `Serialize` with no map and no hand-written impl (`agent_providers/mod.rs:55-105`), and `to_value` errors only on non-string map keys or a `Serialize` impl that itself errors — so it cannot fail here. Kept by decision §6.1. |
| `remote_host/business/wire_limits.rs` 717-722 | `unreachable-in-this-environment` | the ignored `measure_phone_page` driver's env gate (`VERIFY_E2E_PHONE_PAGE`, dumped by `scripts/measure/measure-lean-history.py --phone-page` against a live agent). `measure_page`'s body is covered by §3.2; only the file read and the env var remain. |
| `remote_host/files.rs` 367 | `unreachable-by-construction` | `images > MAX_IMAGES` is shadowed by `references.len() > MAX_ATTACHMENTS` at `:335`: `MAX_ATTACHMENTS == MAX_IMAGES == 10`, so a list that reaches `:366` holds at most 10 images. Kept by decision §6.1 (the shadowing is a constant coincidence, and this guard is the only enforcement of `MAX_IMAGES`). |
| `remote_host/files.rs` 1145, 1183 | `unreachable-in-this-environment` | a `std::fs::copy` that fails after a successful size check, and a `remove_file` on a path a concurrent writer already removed — filesystem faults the fixture cannot order deterministically. Previous round's checked row. |
| `remote_host/session_files.rs` 74 | `unreachable-in-this-environment` | `target.metadata()` failing *after* `canonicalize` succeeded: the entry must be removed inside that window. `:68` (dangling symlink) and `:77` (FIFO) are now covered by the `#[cfg(unix)]` test added here; the TOCTOU window is not schedulable. |
| `remote_host/read_pages.rs` 70 | `unreachable-by-construction` | the `else { break }` of the eviction loop: `insert` refuses `bytes.len() > MAX_SNAPSHOT_BYTES` (16 MiB) at `:50` and `MAX_CACHE_BYTES` is 32 MiB, so with an empty cache the loop condition is false and `min_by_key` always yields `Some`. Kept by decision §6.1 (it is the loop's liveness guard, not a duplicate). |
| `remote_host/pairing.rs` 124 | `attribution-artifact` | the `)?)` of `create_pairing`'s `code_expires_at.ok_or_else(...)?`; `a_pairing_code_without_an_expiry_is_refused_by_name` asserts that arm's refusal, so the expression runs and the line is a span end. |
| `remote_host/business/history.rs` 273 | `attribution-artifact` | a closing brace inside the page-shape builder whose statements are covered above it. |
| `remote_host/business/transfers.rs` 59 | `unreachable-in-this-environment` | `spawn_blocking`'s `JoinError`: the blocking `session_files::list` must panic or the runtime be shut down. Needs a seam (or a bug) inside `list`. |
| `remote_peer/runtime.rs` 152-154 | `unreachable-in-this-environment` | `delay_unit()`'s `#[cfg(not(test))]` body (seconds) beside the `#[cfg(test)]` one (milliseconds): cfg-split, so the production body exists only in the bin-linked copy — and the one non-test path the harness runs (the retry loop, see the `start.rs` row) does not call `reconnect_delay` (`:165-171`), the only caller of `delay_unit()`. |
| `remote_peer/runtime.rs` 1659, 1723, 1736, 1761-1763 | `unreachable-in-this-environment` | the client's chunked-read guards, **re-pointed from lcov4's 1603/1667/1680/1705-1707** (see §4 for the two commits that moved them): `:1659` refuses a page whose `totalBytes` exceeds `MAX_PAGE_BYTES`, `:1723` is `store_chunk`'s id/total/offset check, `:1736` its empty/short-chunk check, and `:1761-1763` is `page_shape_error` itself, whose every call site is one of those arms. Reaching them needs the *host* to answer an oversized or mismatched chunked read — the in-process host computes `totalBytes` from the page it built and caps it, so a test cannot make it lie without changing product code; today's fixture answers one correct session-scoped page (`runtime_tests.rs::a_page_larger_than_one_reply_arrives_intact`). |
| `remote_peer/runtime.rs` 2721 | `attribution-artifact` | the closing brace of the `visibility_change_for_test` wrapper (was `:2544` before `ccb51ffa3`/`5569d8c22` inserted 150 lines above it, then `:2694` before TAU-D7's own test added 27 more); `:2717-2720` are covered and the region end is attributed to the brace. |
| `remote_peer/session.rs` 767 | `unreachable-in-this-environment` | `let options = options.require_tls(true)` under `#[cfg(not(test))]` — cfg-split: the test build connects to the in-process plaintext fake broker by design. The row previously said `:668`, which carried the same line in lcov4; in lcov2 `:668` has no `DA` record and the statement is at `:767`. |
| `remote/test_support.rs` 1790, 1804 | `unreachable-by-construction` | the failure report of `no_body_takes_the_home_guard_before_the_mock_lock`: `:1790` pushes a violation when a body in `src/` takes `HomeGuard::new`/`TEST_HOME_LOCK` before `mock_agent_lock(`, and `:1804` formats the list — the latter only when the `assert!` at `:1799` fails. The invariant holds: the census walked all 191 files, found 2 inversions, both in `remote_host/business/settings.rs`, and both were fixed (docs/testing/flaky-tauri.md §1), so a green suite cannot reach either line. Same class as `remote_host/lean.rs:1222` — the line exists to fail a regression. |
| `remote/test_support.rs` 2477 | `unreachable-by-construction` | `Err(error) => panic!("expected the deferred arm, got the failed arm: …")` in `install_fake_state_installs_a_live_bridge_and_drives_the_deferred_arm`: the test installs a live fake bridge (`SUPERVISOR.state` set, phase `Ready`, no terminal health error) and then asserts `remote::start()` returns the *deferred* `Ok` (AccountAuthorization). The `Err` arm is the regression guard for that contract, so a passing run cannot enter it. (`:2433` before TAU-D7's two scanner tests were inserted above it.) |

### 4.2 The nine OPEN lines, closed (and the honest limits of this ledger)

**CLOSED 2026-10-17 (TAU-D7): nine lines, seven tests, no product change.** The
previous revision of this record carried these as OPEN — reachable by a test with
the seams that already existed, therefore not waivers. Each is now `DA>0` in
`/tmp/tauri-tau-d7b-lcov.info`, and the table below is a **crosswalk, not a
ledger row set**: its first cell is a test name, so it claims no uncovered line
(F.95). The tests are listed in §3.5 with what each one drives.

| test that closes it | file | line(s) it closes | how it reaches them |
|---|---|---|---|
| `remote_peer::transfer::tests::a_transport_failure_costs_one_retry_and_the_transfer_completes` | `remote_peer/transfer.rs` | 459, and 458 with it | `retrying` takes its step as an `FnMut()`, so a `#[cfg(test)] mod tests` inside `transfer.rs` (the sibling `transfer_tests` cannot — `retrying` is private) drives one `AppError::RemoteTransport` and then a success: `:459` records the first error, `:458` is the brace that `return` used to skip. |
| `remote_peer::transfer::tests::a_chunk_that_never_lands_reports_its_first_failure` | `remote_peer/transfer.rs` | 466 | every attempt returns `RemoteTransport`, so the loop is spent and the closing `Err(first.expect(...))` runs; the assertion is that the **first** message is the one reported. |
| `remote_peer::runtime::tests::reset_for_test_aborts_a_resident_supervisor` | `remote_peer/runtime.rs` | 202-203 | `install_supervisor_for_test` puts a live loop in `live.supervisors`, then `reset_for_test` drains it — the branch the fixture's own setup never sees, because `teardown()` has already emptied the map. |
| `remote_peer::runtime_tests::an_activity_packet_arrives_tagged_with_its_catalogue_domain` | `remote_peer/runtime.rs` | 1159 | the peer subscribes, a thread is created (forcing a structural packet the peer sees), then `record_thread_message_activity` moves only its recency; the host's catalogue publisher answers on `state.activity` and the client's `StateEventKind::Activity` arm tags it `sessions_activity`. |
| `remote_peer::runtime_tests::a_run_scoped_page_repeats_its_run_on_every_chunk` | `remote_peer/runtime.rs` | 1699 | a ~640 KiB page (past the host's 512 KiB threshold) requested **with a `runId`**; `ChunkOwner::from` copies it from the client's own command, so `fetch_chunk` sets `command["runId"]` and the host's owner check passes instead of refusing the chunk. |
| `remote::test_support::tests::a_fn_without_a_body_is_not_a_call_site` | `remote/test_support.rs` | 1838 | `function_bodies("fn f")` — a `fn` token with no `{` after it — takes the scanner's `break`. |
| `remote::test_support::tests::a_nested_block_comment_is_blanked_whole` | `remote/test_support.rs` | 1896-1897 | `blank_comments_and_literals("a /* b /* c */ d */ e")` enters the depth branch, which no comment in this crate does. |

A **first version of the activity test was flaky and was fixed before this
measurement**, which is worth recording because the failure mode is a real
property of the publisher: it created the thread *before* connecting, so whether
the structural snapshot had already been published (NATS Core does not replay)
decided whether the later recency move produced an activity packet at all. It
passed alone and failed in both full suites (gui `565 s`, headless `434 s`). The
committed order — subscribe first, then create the row, then wait for the
structural packet, then move the recency — is deterministic, and the test ran
10/10 green across both flavors before the measurement. Neither failed run is
part of the pinned report.

With those closed, all 1195 counted lines are waived, and three kinds of
row should not be read as "the code is fine, only untestable":

1. **Dead guards** (`remote_host/files.rs:367`, `remote_host/read_pages.rs:70`)
   and **arms that cannot fail** (`remote/commands.rs:879-882`,
   `remote_host/business/providers.rs:58-64`). Each is unreachable code *in the
   shipping shape*, so the tempting fix is to delete it — which a coverage change
   may not do on its own authority (PLAN §4 forbids deleting a guard to make its
   line disappear). **DECIDED 2026-10-17 (§6.1): all four are KEPT, with the
   per-item reasoning and the alternative's exact cost recorded there.** Two
   corrections to this item as first written: `remote/commands.rs:885` is *not*
   one of the "arms that cannot fail" (it is a codec-dependent size heuristic,
   re-registered as `unreachable-in-this-environment` in §4.1), and "the honest
   fix is to delete it" turned out to be wrong for the other three —
   `read_pages.rs:70` is the eviction loop's only exit.
2. **The cfg-split bodies** (`remote/supervisor/start.rs`, 132 of this
   ledger's lines) are
   the single largest block and are *deliberately* not compiled into the test
   binary. Registering them is what the campaign's waiver rule allows; the
   failure-injection seam proposed in the first draft is **withdrawn — the
   supervisor ruled against it (§6.2)** — so these rows stay registered with the
   cfg mechanism named, which is the honest 口径 A answer for them.
3. **Timing-driven lines** (`remote/publisher.rs`) move in and out of the ledger
   between runs: a line that was covered at 19:38 was `DA:0` at 20:54 with the
   same tests. A "0 uncovered" claim for this file would be a claim about one
   run's scheduling.

## 5. Findings

### 5.1 Two gates caught real defects in this unit's own work

- **The instrumented run caught test pollution I had just introduced.**
  `bridge_tests::the_mock_answers_an_unknown_tool_call_identity_with_nothing`
  declares `lean_events_v1` to reach the lazy-fetch path, and that declaration is
  a *process-global* flag (`remote_host::lean::set_enabled` via
  `apply_declared_features`). The test passed alone and reddened two later tests
  in the same binary (`tool_call_args_is_reachable_only_for_a_client_that_declared_lean`
  at `commands.rs:2422`, `a_client_that_did_not_declare_keeps_the_full_lane` at
  `tests.rs:909`). Reproduction: run any filtered subset that includes only the
  new test (green), then the whole `remote::` family (red). Fixed by withdrawing
  the declaration at the end and asserting `!lean::enabled()`; `remote::*` is
  green (251 passed) with it. **Any test in this crate that declares a capability
  must withdraw it** — the sibling test does, for the same reason.
- **Clippy is a separate gate for a reason.** `cargo clippy --all-targets -- -D
  warnings` found 6 errors the test run could not see: five in this unit's new
  code (`&journal` where the parameter is already `&[JournalEvent]` at
  `verify_e2e.rs:1349/1366/1395/1406`, `sized(&entries)` on a `&mut Value` at
  `lean.rs:1437` — `needless_borrow`) and one in `src/terminal/session.rs:1212`
  (`unused_mut`, another unit's file, fixed by its owner while this ran). Both
  flavors now exit 0. Without the clippy run this unit would have shipped 5 CI
  errors from `-D warnings`.

### 5.2 Host flakes seen while measuring (not regressions)

- `remote_peer::*` / `commands::remote_peer::*` fail at
  `src/remote_peer/testing.rs:30` (`the test web port never freed`, port 8022)
  in a rotating set: 5 failures in this run's headless flavor, 0 in its gui
  flavor, 4/5/1/0 in earlier runs, with no code change between some of them.
- `skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin`
  timed out once under load in the uninstrumented run (`CLI discovery timed out`),
  passes alone in 0.44 s, and was green in the instrumented run.
  Both families are outside this unit's scope and neither touches the ledger.

### 5.3 Next useful checks

1. **Done (2026-10-17).** The cfg-split pairing in `remote/supervisor/start.rs`
   is confirmed by reading both arms and the seam that would have reached it is
   **withdrawn by decision** — see §6.2. Nothing here is outstanding; the rows
   stand as registered (132 of this ledger's lines; the earlier 167 counted
   lines the merged report now covers — §4).
2. **Done (TAU-A3, §8).** `remote/verify_e2e.rs`'s driver/window row went 68 → 1
   line without running `scripts/measure/verify-e2e-bytes.py` and without
   touching the operator's home: each `#[ignore]`d driver is now executed
   through its own body on the in-process broker, with the inputs the script
   would have supplied. What remains registered is the *live-monitor plumbing*
   (§8.4), not the drivers.
3. A `remote_peer` host fixture that can answer a mismatched/oversized/run-scoped
   chunked read would close the 8 lines in `remote_peer/runtime.rs`.
4. For the shell unit (TAU-C4): `remote::test_support::install_fake_state` now
   exists (§3.4) — re-derive the `lib.rs`/`desktop_shutdown.rs` row numbers first,
   since that region drifted ~28 lines after the shell report was written, and
   note that the two `remote::start` arm outcomes it needs are reachable without
   the fixture. What is still unsettled there is the shutdown-deadline budget
   (§3.4, last two bullets).

## 6. The waiver questions, decided (TAU-A2, 2026-10-17)

The todo left three questions open: the four deletion candidates (§4.2 item 1),
the 132 cfg-split lines of `remote/supervisor/start.rs` (§4.2 item 2), and the
`verify-e2e-bytes.py` route to the `verify_e2e.rs` drivers. All three are
answered below. **No production code changed in this pass** — that is the
outcome of 6.1 and 6.2, not an omission: for every candidate, deleting was
measured against what it would cost, and for every candidate the cost was higher
than the registered line.

### 6.1 The four deletion candidates: all four KEPT

Evidence for "dead" is re-derived from the source in this worktree and the
`DA:0` state re-read from the report this record pins
(`/tmp/tauri-tau-remote-lcov4.info`): `files.rs:366,13 / 367,0`;
`read_pages.rs:69,1 / 70,0`; `commands.rs:878,6 / 879,0 / 880,6 / 881,6 / 882,0 /
884,6 / 885,0 / 886,6`; `providers.rs` 57 covered, `58-64` all `DA:0`.

| # | lines | why it is unreachable | what deleting buys | what deleting costs | verdict |
|---|---|---|---|---|---|
| 1 | `remote_host/files.rs:367` (`images > MAX_IMAGES`) | `images ≤ references.len()` and the earlier guard at `:335` refuses `references.len() > MAX_ATTACHMENTS = 10 = MAX_IMAGES` (`files.rs:26/:30`) | 1 line | it is the **only** enforcement of `MAX_IMAGES`, a separately named `pub const`. If `MAX_ATTACHMENTS` is ever raised — the plausible product change "more files, still at most 10 images" — this guard starts working again and deleting it would silently widen the image cap. The overlap is *known and deliberate*: its own test says so and asserts the other guard's message — `files.rs:1819-1821`: `// Eleven images exceed the ten-image cap (and the ten-attachment one).` / `assert!(error.to_string().contains("at most 10 attachments"));`. Reordering (images first) is **not** behaviour-neutral: it changes the client-visible error text for an 11-image upload. | **KEEP** |
| 2 | `remote_host/read_pages.rs:70` (the `else { break }`) | `MAX_SNAPSHOT_BYTES` 16 MiB (`:16`) `< MAX_CACHE_BYTES` 32 MiB (`:17`), so the eviction condition can only be true with a non-empty map, and `min_by_key` always yields `Some` | 1 line | it is the loop's **liveness guard**: with the constants ordered the other way (or after an edit that lets `bytes.len()` approach `MAX_CACHE_BYTES`), an empty map makes `while sum + bytes > MAX_CACHE_BYTES` spin forever — on the phone read path, inside a command handler. A hang is strictly worse than one registered line. | **KEEP** |
| 3 | `remote/commands.rs:879`, `:882` (the `write_all`/`finish` fallbacks into a `Vec`) | `impl io::Write for Vec<u8>` never returns `Err`, so `GzEncoder::<Vec<u8>>::finish` cannot either | 4 lines | the honest deletions are not behaviour-identical: `let _ = encoder.write_all(&plain);` discards a `Result` a reader must re-derive as infallible, and removing the `finish` arm needs `.expect(...)` (a panic on the reply path where today there is a graceful fallback) or `.unwrap_or_else(\|_\| plain.clone())` (the impossible path changes from "reuse the plaintext" to "clone it"). The equivalence proof would rest on `flate2`/std internals rather than on this code. | **KEEP** (registry, not edit) |
| 4 | `remote_host/business/providers.rs:58-64` (`to_value(view)`'s `Err`) | `ProvidersView` is two `Vec`s of structs of `String`/`bool`/`usize`/`i32`, all derived `Serialize`, no map, no hand-written impl (`agent_providers/mod.rs:55-105`); `to_value` errors only on non-string map keys or an erroring `Serialize` impl | 7 lines (the arm's `reply(...)` spans 58-64) | the replacement is `.expect(...)` — a panic in a phone reply path where the code today answers the client with a message "Could not serialize the provider configuration.". A real (if unreachable) behaviour difference, for 7 lines of a 450-line ledger. | **KEEP** |

**Cumulative cost of "keep": 12 registered lines** (1 + 1 + 3 + 7).

**If the supervisor would rather spend them**: the edits I *could* make are
(i) delete `files.rs:366-368`, (iii) the two `commands.rs` arms, (iv)
`providers.rs:57-65` → `.expect("a provider view always serializes")`, i.e. 12
lines for 3 production edits whose equivalences rest on `MAX_ATTACHMENTS ==
MAX_IMAGES`, `flate2`'s sink behaviour, and serde_json's derived-`Serialize`
rules. Item 2's line is **not** offered at any price — it is a liveness guard.
My recommendation is to keep all four; the counter-argument is strongest for (4),
where the arm is 7 lines and its only job is to avoid a panic that cannot happen.

### 6.2 `remote/supervisor/start.rs`: no production seam (supervisor ruling, recorded)

The supervisor's decision of 2026-10-17, recorded here as the record's own:

> **do NOT add the production seam.** Injecting a failing connect result into
> `start_once` / a failure transport is a redesign of production control flow
> whose only purpose is to make lines measurable, and PLAN §4b authorizes a seam
> only when the injection POINT ALREADY EXISTS and is already exercised. Keep
> these 167 lines registered as `unreachable-in-this-environment` with the cfg
> mechanism named — "unreachable in this test harness because the failure
> transport is not injectable" is an honest 口径 A answer, and an honest residual
> beats a redesigned production path.

(The decision quoted 167 lines because that was the block's measured size on
2026-10-17 morning; later landings covered 35 of them, so the row this ledger
carries is 132, and the decision is unchanged.)

Consequences in this record: the seam proposed in §3.2 is withdrawn (first
draft), §4.2 item 2 no longer proposes it, and the §4.1 rows for `start.rs` stand
unchanged except for the mechanism's name. The cfg-split pairing itself was
re-verified against the working tree in this pass — `#[cfg(test)] return;` at
`:245` with the real body under `#[cfg(not(test))]` from `:248`; the same shape at
`:307`/`:310`, `:361` (`GenerationWatch` struct, `#[cfg(not(test))]`) /`:367`
(its impl), `:427`/`:432`, `:446`/`:451`, and inside their callers at `:604` and
`:633` (the first draft's `:558`/`:605`/`:638-641`/`:643-648` have drifted with the
file, 20-30 lines). Two of the five seams are `#[cfg(test)] { let _ = ... }`
stubs rather than early returns, so the *test* copy of those functions is
legitimately executed and covered; only the `#[cfg(not(test))]` bodies stay
`DA:0`, which is exactly what a registered cfg-split row claims.

**Related rows that name the same kind of seam** (not decided by this todo, but
the same PLAN §4b bar applies to them): `remote/transport.rs` 197, 231-300 (31
lines), `remote/publisher.rs` 336-628 (11), `remote/transfer.rs` 44-48/103 (6)
and `remote/supervisor/start.rs` 5 more lines at 70-215 — each says "needs a
failure-injection transport". If the ruling above is the general rule (no seam
whose injection point does not already exist), those rows stay registered for the
same reason and no work is owed on them; if a future round does build a
failure-injection transport for another purpose, these rows should be re-read,
because the *existing* seam would then make them coverable.

### 6.3 `verify-e2e-bytes.py --broker both`: not run, the 68 lines stay registered

Not attempted, and the ledger's `verify_e2e.rs` row is unchanged. Reasons, all
checkable:

1. **Outside this unit's declared write set.** The script is
   `scripts/measure/verify-e2e-bytes.py`, and the todo's own constraint is to ask
   before touching it.
2. **It reads the operator's live home.** `verify-e2e-bytes.py:49-50`:
   `SOCKET = Path.home() / ".future" / "run" / "agent.sock"`,
   `AGENT_DB = Path.home() / ".future" / "agent" / "agent.db"` — the running
   agent's socket and database — and it starts a real `nats-server` (needs the
   binary on `PATH`, `:95-97`, plus the `-m` monitoring port). The campaign rule
   is that a test must not disturb the running agent or the real `~/.future`, and
   the todo itself asks for a check before real user state is touched.
3. Even with the script run, the drivers are `#[ignore]`d, so covering them needs
   `--ignored` alongside it: the 68 lines are the env gates plus the `/connz`
   window.

**A safe, test-only alternative exists but was not taken** (it belongs to the
supervisor because it changes what the ordinary suite does): part of that 68-line
set sits in `measure_catalog_activity_lane`, whose inputs already have in-code
defaults (`CATALOG_ACTIVITY_ROWS` `unwrap_or(232)`, writes `unwrap_or(20)`,
`verify_e2e.rs:1163-1170`) and whose body runs on `FakeNats` + `HomeGuard` — no
operator home, no real broker. The same is true of the §3.2 seam
`run_real_traffic_with(broker, …)`; with `VERIFY_E2E_NATS_MONITOR` pointed at a
*scripted local HTTP server* the `/connz` accounting could be exercised without a
real `nats-server`. Cost: it moves a measurement into the suite (runtime, a
fake-monitor fixture, env-var handling that must be panic-safe per `FLAKES.md`
item 27) and weakens the "measured against a real broker" claim the driver
exists to make. Flagged for a decision; nothing was changed *in that pass*.

**Superseded by §8 (TAU-A3, 2026-10-17).** The alternative was adopted and the
68-line row is now 1 line. The one part of the suggestion that turned out to be
impossible as written is the scripted `/connz` **end-to-end**: the window's
second reading is taken inside the measurement, so a pre-scripted server would
have to know the byte count the run is about to produce. §8.4 records the split
that replaced it, and what stayed registered because of it.

## 7. Real-network runs (2026-10-17)

The real-broker, two-process and failure-path results — including what was **not**
exercised (TLS/WebSocket, the platform pair/claim endpoints, a real vendor model)
and why — are in [`real-network.md`](real-network.md).

## 8. TAU-A3: the ignored drivers, driven on the in-process broker (2026-10-17)

The 68 lines registered in §4 as `remote/verify_e2e.rs`'s driver/window row are
now **1**. Nothing was deleted, no `#[ignore]` was removed, no assertion was
weakened, and the operator's home was not read: every driver was *executed*
through its own body with the inputs the measurement script would have handed it.

### 8.1 How a driver is driven without a real broker

Two facts make this possible, and both were already true:

* **a driver's body is ordinary code.** `verify_e2e_real_broker_round_trip`,
  `measure_real_e2ee_bytes`, `measure_real_broker_bytes` and
  `measure_catalog_activity_lane` are `#[tokio::test]` functions, i.e. plain
  blocking fns that build their own runtime. Calling one from a `#[test]` (no
  ambient runtime) is legal; calling one from inside a runtime would panic
  ("Cannot start a runtime from within a runtime"), which is why the new tests
  are `#[test]` and not `#[tokio::test]`.
* **the only non-local input is an address.** `Broker::real_from_env()` reads
  `VERIFY_E2E_NATS_URL` and nothing else; the `Broker::Real` arm uses that URL to
  connect and to answer a publish/subscribe echo. The in-process `FakeNats`
  answers that echo, so the arm runs end to end against it — the *same* arm a
  real `nats-server` takes. `measure_real_e2ee_bytes` starts its own `FakeNats`
  and needs no env at all beyond the dump paths.

The four new tests:

| test | drives | env it supplies (all via `EnvGuard`) |
|---|---|---|
| `the_ignored_measurement_drivers_run_their_own_bodies` | `measure_real_e2ee_bytes()` **and** `measure_real_broker_bytes()` | `VERIFY_E2E_JOURNAL/SESSION/RUN/ENTRIES` + `VERIFY_E2E_NATS_URL` |
| `the_ignored_fixture_driver_runs_its_own_body` | `verify_e2e_real_broker_round_trip()` | `VERIFY_E2E_NATS_URL` |
| `the_ignored_catalog_driver_runs_its_own_body` | `measure_catalog_activity_lane()` | `CATALOG_ACTIVITY_ROWS=2`, `CATALOG_ACTIVITY_WRITES=2` |
| `the_accounting_window_reads_the_monitor_and_accepts_a_truthful_broker` | `assert_broker_accounting_against` (the window's reading + arithmetic) | — (a loopback `/connz` served by `serve_connz`) |

The journal and history dumps are written to a temp dir in the exact two shapes
`scripts/measure/verify-e2e-bytes.py` writes (JSON lines out of `run_events`;
the bare entries array), built from the module's own `fixture_journal()` and
`traffic_entries()`, so the drivers parse them the way they parse a real dump.
`VERIFY_E2E_NATS_MONITOR` is deliberately left **unset**: see §8.4.

**Panic-safety.** `EnvGuard` (`cli_tool.rs`) restores each variable on drop,
including while an assertion unwinds — the `FLAKES.md` item 27 requirement. A
lock (`DRIVER_ENV`) serializes the writers, and it is explicitly *not* the
restore: the two mechanisms do different jobs and both are present.

### 8.2 What the drivers reported on the fixture inputs

```
VERIFY_E2E_FIXTURE {"live":{"undeclared":{"messages":7,"wireBytes":2283,…},
  "declared":{"messages":5,"wireBytes":970,…}},"history":{"undeclared":{"full":14305,
  "paged":14311},"declared":{"full":550,"paged":556}}}
CATALOG_ACTIVITY {"rows":2,"writes":2,"declared":{"activityPackets":2,
  "payloadBytes":405},"undeclared":{"sessionsPackets":2,"payloadBytes":1946},
  "payloadSaved":0.7919}
VERIFY_E2E_LIVE_PHONE … / VERIFY_E2E_LIVE … / VERIFY_E2E_HISTORY … / VERIFY_E2E_LAZY_FETCH …
```

Every invariant the measurement asserts (lane accounting, the trim, the
coalesced lane, both history shapes, the lazy fetch) held on both `Broker`
arms. The `#[ignore]`d annotations and their reason strings are untouched and
still name what a *real* run needs.

### 8.3 The measured result

Report: **`/tmp/tauri-tau-a3-lcov2.info`**, written 2026-10-17 14:48 CST, merged
over both flavors with the §1 procedure (`gui` exit 101 with one failure, below;
`headless` exit 0; `--no-fail-fast` keeps the profraw usable either way).

| | before (this file, run of §1) | after (this run) |
|---|---|---|
| `remote/verify_e2e.rs` driver/window row | 68 lines | **1 line** (`:1429`) |
| `remote/verify_e2e.rs` total `DA:0` | 70 | 68 = 64 (new `InstalledHost`) + 4 |
| three subtrees total `DA:0` | 450 | 1205 (of which 852 is new harness code, §4) |
| three subtrees excluding that harness | 450 | **353** |

Those are §8's own run of 2026-10-17 14:48. The **current** pin (TAU-D7, §3.5)
is 1195 counted lines / **343** without the harness; §4 carries the ledger for it.

Of the 68 registered lines, **67 are covered**: 46 are the same source lines as
before (the drivers' own bodies, `run_real_traffic`, the catalog driver) and 21
moved into `assert_broker_accounting` / `assert_broker_accounting_against` when
the window was given its two sides as arguments — those functions have no
`DA:0` line at all. The one that remains is named in §4.

**Red instrumented run.** The gui flavor reported `2104 passed; 1 failed; 14
ignored`, the failure being
`gui::startup_paths_tests::an_exit_requested_from_the_mock_loop_takes_the_guarded_quit_arm`
(`src/lib.rs:2771`, "EXIT-REQUESTED-PROBE: the arm ran"). It is the *same single
test* in both instrumented runs of this ticket (two gui runs, two occurrences,
the rest of the flavor green each time). **Outside this unit**: it is the shell
unit's startup-path arm, it does not touch `src/remote/**`, and it passes alone
in 1.02 s (`cargo test --lib gui::startup_paths_tests::…`, exit 0) — the
load-flake direction, not a regression. The direction argument: my change adds
four tests that each own a process-global env window and a `HomeGuard`, both
restored on drop; it adds no global that the quit arm reads, and the arm failed
in the flavor whose suite ran 2104 tests in 544 s (the headless flavor, which
runs the same new tests, exited 0 both times).

### 8.4 Why the `/connz` window is asserted in two pieces (and what stays)

The ticket's suggestion was "point `VERIFY_E2E_NATS_MONITOR` at a scripted local
HTTP server". Taken literally that cannot work, and the reason is structural: the
window asserts `after.out_bytes − before.out_bytes − probe == what the phone
decrypted`, and the second reading is taken *inside* the measurement. A
scripted server must therefore produce, before the run starts, a number only the
run can compute. Any server that *could* produce it would have to read the
phone's own counters — which is the tautology the assertion exists to avoid.

So the window was split into the two things it actually does:

* **the reading** — `assert_broker_accounting_against(&e2e, before, url, session)`
  — one HTTP call, covered by driving it against a loopback `/connz`
  (`serve_connz`) whose counters are the ones a truthful server reports for the
  traffic the test just generated (a non-zero baseline, the probe it relayed,
  and exactly what the phone decrypted);
* **the arithmetic** — `assert_broker_accounting(before, after, receipt, …)` —
  pure, covered by two ordinary tests that supply **a non-zero baseline and a
  non-zero probe**, so an implementation that forgot either subtraction fails,
  and by two `#[should_panic]` tests where the broker is one byte / one message
  over the receipt.

That is not the real-broker claim: the independence of the two sides (a real
server's own accounting vs. what the client decrypted) rests on
`measure_real_broker_bytes` against `nats-server`, which was run for real on
2026-10-17 and reported matching `/connz` totals (`real-network.md` §3.1). What
this ticket adds is that the *code path* is exercised on every ordinary run.

**What stays registered (2 lines).** `:1429` (the baseline's `Some(url)` arm) and
`:1572` (the `if let` that consumes it): reaching either requires a live
monitor during a real measurement, i.e. exactly the scripted-endpoint problem
above. Registering two plumbing lines is cheaper and more honest than a fixture
that fabricates the numbers the assertion compares.
## 9. COALESCE-GROWTH: the finding was the assertion, not the coalescer (2026-10-17)

`real-network.md` §3.3a recorded the ignored driver
`remote::publisher::coalesce::tests::measure_real_journal` **failing** at the
production window with `coalescing must not grow the lane`, and read that as
"the envelope makes the published lane larger". Measured properly, **the lane
never grew: it was byte-identical**, and the coalescer was right. The predicate
was wrong.

### 9.1 What the instrument actually reported

The driver's marker was printed *after* its assertions, so the failing run
reported no numbers at all — which is why the first reading had to be a guess.
Printing before asserting (now permanent, §9.3) gives:

```
LIVE_LANE {"events":19,"todayBytes":9196,"todayDataBytes":2206,
           "coalescedBytes":9196,"coalescedDataBytes":2206,
           "published":19,"parties":19,"windowMs":100,"ratio":1.0}
```

`todayBytes == coalescedBytes`, `published == events`: **nothing was merged and
nothing was added.** `Coalescer::flush` already hands a lone fragment straight
back (`if pending.merged == 0 { out.push(pending.event) }`, `coalesce.rs:208`),
so the "merged envelope on a single event" the ticket asked about cannot happen.

Why nothing merged: that journal's `text_chunk` events are **100.9–110 ms
apart** and the window is 100 ms (`COALESCE_WINDOW`), so every fragment arrived
after its predecessor's window had closed. The old assertion was
`coalesced_bytes < today_bytes` — strict — which no journal that cannot merge
can satisfy.

### 9.2 The production window on burst traffic (the question that mattered)

The slow journal above comes from a model stub paced at ~100 ms per token, so it
is the *floor* case, not what a real provider does. A second journal was produced
on the same real stack (isolated `future-agent`, `nats-server`, local stub) with
the stub paced at 20 ms: 69 events, 63 `text_chunk`s, gaps **8–30 ms**, all
inside the window, plus `tool_start`/`tool_delta`/`tool_end` between them so the
flush-on-non-fragment rule is exercised. Production window, unchanged code:

```
LIVE_LANE {"events":69,"todayBytes":31375,"todayDataBytes":6147,
           "coalescedBytes":13666,"coalescedDataBytes":5619,
           "published":21,"parties":69,"windowMs":100,"ratio":2.30}
LEAN_LANE {"events":69,"delivered":67,"fullBytes":31375,"leanBytes":16102,
           "published":19,"coalescedBytes":8470,"windowMs":100,"reduction":0.4868}
```

The 100 ms window merges the burst 2.30× (31375 → 13666 bytes, 21 publishes for
69 events) with every invariant holding: 69 parties accounted for, no character
lost, newest index published. So:

| journal (real, same stack) | fragment gaps | window | publishes | bytes | verdict |
|---|---|---|---|---|---|
| slow (stub 100 ms/token) | 100.9–110 ms | 100 ms | 19/19 | 9196 → 9196 | nothing to merge; **unchanged is correct** |
| slow, same journal | 100.9–110 ms | 5000 ms | 7/19 | 9196 → 4748 (1.94×) | merge path correct |
| burst (stub 20 ms/token) | 8–30 ms | **100 ms (production)** | 21/69 | 31375 → 13666 (2.30×) | **window does its job** |

### 9.3 What changed, and what did not

**Production behaviour: nothing.** The only non-test hunk in `coalesce.rs` is a
`#[cfg(test)] fn assert_lane_claims` (`cargo clippy -D warnings` proves it: an
un-gated copy is dead code in a non-test build). `COALESCE_WINDOW` stays at
100 ms, `MAX_MERGED_FRAGMENTS`/`MAX_MERGED_BYTES` are untouched.

**The instrument's claim, corrected — three predicates instead of one:**

| case | was | now |
|---|---|---|
| any | `coalesced < today` | `coalesced <= today` ("never grow") |
| merged (`published < events`) | same assertion | `coalesced < today` (strict, unchanged in force) |
| no merge (`published == events`) | same assertion → **fails by construction** | every published payload is **byte-identical** to its source body |

The third row is the one that would catch the bug the ticket feared — a merged
envelope leaking onto a lone event — and it did not exist before. Dropping the
strict comparison where nothing merged while adding byte-identity there is a
strengthening, not a waiver.

**Supporting changes (all test-side):**

* the `LIVE_LANE` marker is printed before the assertions, so a future failure
  reports its numbers instead of nothing (the driver's own `markers()` helper
  reads markers and treats the exit code as the proof of a full run);
* `a_lane_claiming_shrink_equal_and_reject_the_two_failure_shapes` — drives the
  three verdicts, including two `catch_unwind` cases where the lane is one byte
  larger and where a lone event carries `coalescedCount`; both must be rejected;
* `a_fragment_that_merges_nothing_is_published_byte_identical` — one and two
  fragments spaced at 1×/2×/10× the window, plus the end-of-stream flush;
* `MeasurementEnv` (RAII) replaces the manual save/restore in
  `the_measurement_harnesses_hold_over_a_synthetic_journal`, plus
  `a_panicking_measurement_restores_its_environment`. The synthetic driver now
  runs the new claims, so it can fail in a new way; without the guard that
  failure would leak `SYNC_MEASURE_*` into the rest of the suite (`FLAKES.md`
  item 27 — the leak path the campaign has measured before).

**Coverage** (module-filtered gui `cargo llvm-cov`, `/tmp/cg-lcov2.info`,
private target dir): `remote/publisher/coalesce.rs` **949 measured lines, 0
`DA:0`** — the new helper, the new tests and the guard are all covered, because
the synthetic driver test calls both measurements on every run.

### 9.4 A number in `real-network.md` §3.3a that did not reproduce

§3.3a recorded `todayBytes 9234` / `coalescedBytes 4762`. The same md5-pinned
journal (`7812abc1567f1796e75e2018c4f17475`) now gives `9196` / `4748`. The
difference is fully explained, not ignored: it is exactly **one extra byte in the
session id per published body** (+1 in `sessionId`, +1 in `eventId`; 19 bodies ⇒
+38, 7 ⇒ +14), and re-running with a session id one character longer reproduces
the old pair digit for digit:

```
SYNC_MEASURE_SESSION=<stored id>X  SYNC_MEASURE_WINDOW_MS=5000
→ todayBytes 9234, coalescedBytes 4762, ratio 1.9391
```

So that run measured with a slightly wrong session id (a stray character from the
shell capture). Bytes only; the conclusion there — 1.94× when the window can
merge — is unchanged.

The same cause explains **every** figure that drifted, and that is now measured
rather than extrapolated: `real-network.md` §3.4's lean block (`fullBytes 9234`,
`leanBytes 4459`, `leanDataBytes 8021`, `reduction 0.5171`, and every per-type
total) reproduces digit for digit with the extra character and becomes
`9196 / 4442 / 7987 / 0.5170` with the stored id; §3.1's `plaintextBytes 9234`
and `4459` are the same +1-per-body offset (its wire bytes vary run to run
regardless, because the pair id is minted per run, so only `plaintextBytes` is
pinned). The blast radius is therefore three sections, not one, and the staged
patch below corrects §3.3 and §3.4 while leaving §3.1's markers as measured with
a provenance note.

### 9.4b The `real-network.md` correction is **staged, not applied**

The wrong diagnosis is still in `docs/testing/real-network.md` §3.3a, and so is
§8's finding 1 ("Coalescing can grow the lane when nothing merges"). That file is
**not** in this ticket's recorded write set — the kernel's own
`workspace_lock_acquired` event for this todo lists
`desktop/src-tauri/src/remote`, `…/remote_host`, `…/remote_peer` and
`docs/testing/module-tauri-remote.md`, and nothing else — so the edit is prepared
and verified rather than written. (Dispatch of a todo is not a scope grant: the
declared set is creation-time state, and the campaign has a recorded incident for
inferring a grant from it.)

What is ready, and checked:

| item | value |
|---|---|
| patch | `/tmp/realnet/doc-correction.patch` (108 lines, 57 insertions / 22 deletions) |
| builder | `/tmp/realnet/build_correction.py` (line-anchored splices, each asserting its anchor text) |
| `git apply --check` from the repo root | **exit 0** |
| apply command | `git apply /tmp/realnet/doc-correction.patch` |

The patch changes exactly three places and nothing in §5 or §7:

1. **§3.3 (a)** — replaces "FAILS … the coalesced lane was larger" with the
   measured equality (`todayBytes 9196 == coalescedBytes 9196`, `published
   19/19`, `ratio 1.0`), the reason nothing could merge (100.9–110 ms gaps
   against a 100 ms window), and the fact that `flush` already returns a lone
   fragment unchanged;
2. **§3.3 (b)+(c)** — keeps the 1.94× merge-path datapoint and **adds** the
   burst-journal result at the unchanged production window (`31375 → 13666`,
   `ratio 2.2958`, 21 publishes for 69 events, gaps 8–30 ms), plus a
   *Number provenance* paragraph covering all three sections;
3. **§3.4** — the stored-id lean figures (`9196 / 4442 / 7987 / 0.5170`);
4. **§8 finding 1** — withdrawn and marked as such, pointing at this section.

The one-line ask to the supervisor: **grant `docs/testing/real-network.md` for
this correction** (or apply the patch yourself). Until then this section is the
authoritative statement of what §3.3a/§3.4/§8 should say, so the wrong diagnosis
is at least contradicted inside the same record set rather than unopposed.

Because `/tmp` is not durable, the substantive correction is repeated here in the
words §3.3's first bullet should carry — so the record does not depend on the
patch file surviving:

> **(a) the production window: PASSES, and the lane is byte-identical.** This
> line originally recorded a *failure* — "the coalesced lane was larger than
> today's per-event lane". That reading was wrong and is corrected here; it
> survived because the driver printed its numbers *after* its assertions, so the
> failing run reported none. The lane is exactly equal, not larger:
> `todayBytes 9196 == coalescedBytes 9196`, `published 19/19`, `ratio 1.0` —
> nothing merged and nothing was added. This journal's `text_chunk`s are
> 100.9–110 ms apart against the 100 ms window, so nothing *may* merge, and
> `Coalescer::flush` hands a lone fragment back unchanged (`merged == 0`): a
> merged envelope cannot land on a single event. The defect was the
> measurement's strict `coalesced < today`, unsatisfiable for a journal that
> cannot merge; fixed in §9 of this file. **No production behaviour changed.**
>
> **(c) the production window on a burst-paced journal: PASSES, 2.30×** — the
> case the finding was really about. A second journal from the same stack,
> stub-paced at 20 ms, has 63 `text_chunk`s 8–30 ms apart with
> `tool_start`/`tool_delta`/`tool_end` between them; at the unchanged 100 ms
> window it goes `31375 → 13666` bytes in 21 publishes for 69 events, with 69
> parties accounted for, no character lost and the newest index published.

### 9.5 Rejected approaches

* **Relax to `<=` and stop.** It passes both journals, but leaves the no-merge
  regime unasserted: an envelope added to a lone event would shrink nothing and
  still pass, because equality is the expectation.
* **Widen `COALESCE_WINDOW` until the slow journal merges.** That is changing
  production to make a measurement pass, and the anti-pattern this campaign has
  rejected before (F-TAURI-3's withdrawn tolerance). The window's 100 ms is a
  product choice (display frame + client commit coalescing); it should be
  revisited on real-provider latency data, not on a stub's pacing.
* **Re-pace the stub and call the failure fixed.** That hides the no-merge case
  rather than asserting it — and the case is reachable whenever a provider
  streams slower than the window.
* **Pretend the lane grew and "fix" the coalescer.** Measurement says it did
  not; a change made on the ticket's premise would have been a regression risk
  with no defect behind it.

### 9.6 Uncertainty and next check

The burst journal is the ceiling this harness can produce (a stub at 20 ms), and
the slow one is the floor; a real provider's token cadence is somewhere between
and is **not measured here** (no vendor credentials in this environment). Next
useful check: run `measure_real_journal` against a real vendor's journal — the
script route and the inputs are in `real-network.md` §3 — and report the share
of inter-fragment gaps under 100 ms. That number, not the ratio on a stub,
decides whether the window deserves a different value.
