# Module: `tau-remote` — the `desktop/src-tauri` remote lane

> Remote extraction (2026-10-19): source paths and results below are historical.
> Current test ownership is tracked in the [Remote migration inventory](../architecture/remote-test-migration.json).
> Measurement scripts now target migrated Server cases where applicable. Final branch acceptance is pending.


> **FIGURE (2026-10-19, TAU-D29): 1500 uncovered, measured at `16ea6f388`.**
> `origin/main` was merged at `52d80801f`; #1179/#1184/#1188 add 994 lines inside these scopes and
> the figure moved 1083 -> 1500. Report: `.future/cov100/pinned/union-refresh4.info` (gui leg `rc=0`,
> headless leg `rc=0`, whole crate, `clean` between them).
>
> **Composition: 1500 = 788 `real_network.rs` envelope + 433 `verify_e2e.rs` envelope + 279 production
> lines, all 1500 registered, 0 unregistered.** The two envelopes are one row each, with their line
> ranges GENERATED from the union so the claim cannot drift from the report (§15.5). D29 registered the
> 279 production lines that #1179 brought in (§15.3), which is what took `reconcile.py` from MISSING 357
> to MISSING 0.
>
> **One main-move stale, measured not assumed:** three files changed in scope after `16ea6f388`
> (`remote_peer/runtime.rs` +141, `session.rs` +18, `tests.rs` 12) and the gate reports the pairing as
> `stale`. The new items are `#[cfg(test)]` helpers, but the gate does not get to assume that and
> neither do I: **this is a reading of `16ea6f388`, not of `main`.** A consolidation measurement is owed.
>
> Earlier readings are kept below as history: 1197 (post-merge) -> 1118 (D22) -> 1115 (D23) -> 1083 (D26)
> -> 1500 (D29). Each step is a line closed by a fixture or a line the merged tree added - never a row
> moved out of the ledger.

> **ARTIFACT STATUS (2026-10-18).** The authoritative report is
> `.future/cov100/pinned/union-d26.info`. The 277 ABSENT rows are non-code lines inside the
> `real_network.rs` envelope and the `verify_e2e.rs` fixture range, not disagreements (D23's reading
> of the previous report was 283; the difference is rows this segment re-pointed). Superseded
> pins: `union-d23.info` (1115 at `a9c5df505`), `union-d22.info` (1118 at `b382b40c5`),
> `union-postmerge2-green.info` (1197 at `32ba1e13e`), `union-HEAD-20261018.info` (1119). `/tmp`
> pins named in §8–§11 were removed in F.100 and must not be quoted. **Re-derive with §14.3's
> recipe** over the three scopes (`remote`, `remote_host`, `remote_peer`) and expect **1083** with
> `reconcile.py` reporting MISSING 0 / COVERED 0. **Re-pin at HEAD before quoting anything** — this
> module's figure has twice been stale by the time it was published.

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


> **TWO FIXTURES ARE `platform-unmeasured` ON LINUX (2026-10-18, measured).**
> `tests/remote_start_refusal.rs`'s `a_live_broker_drives_the_generation_watch_and_reconnect` and
> `a_healthy_generation_resets_the_budget_and_a_retryable_reconnect_arms_the_retry` are gated to macOS.
> Three CI runs failed a **different one of the two** each time (90 s and 35 s in `Reconnecting`, then
> the platform's 500), which is a race between the fixture's own broker error timer and the child's
> startup, not a property of the product: both pass on macOS in ~10 s and ~70 s wall clock. The
> Linux-only behaviour they used to lean on - the shared port 8022 - is now waited for explicitly by
> `ChildRun::spawn`, so what remains is host scheduling, not the arm under test. The other ten tests in
> the file keep running everywhere.
>
> **The readiness budget has its own lesson.** I first answered the CI failures by raising the wait from
> 30 s to 90 s. That only moved the failure later - twice. A budget is not a fix: the child was never
> going to reach the phase, and a longer wait converts a fast red into a slow red. The port wait was the
> fix; the budget stayed only as headroom.

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
further down — §4's current pin (TAU-D11) is **1117** counted lines (**265**
without the real-network harness), i.e. **999 / 106 / 12** per subtree. Read §2
as the shape of the drop, not as the residual.

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
purpose is to make lines measurable. Those lines are registered instead —
**OPEN with a named child-process fixture, not as an environment property
(TAU-D9, §10.4).**

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

## 4. Uncovered-line ledger (residual: 1083 lines at `0b7b2ca17`; 227 without the real-network harness)

**1083 counted lines: all 1083 WAIVED under a `waiver-ledger.md` category, 0 OPEN, 0 unexplained.**
Every number is read from the retained report `.future/cov100/pinned/union-d26.info` (two legs,
separate renders, `clean` between them, unioned; pin and run identity in the banner above). The 1083
is `DA:0` over the three scopes; §4.3 records what the 82 pre-merge rows became, §4.4 the per-file
composition and the delta, and §14 which lines D26 closed and which four it waived as races.

Method: the report is the authority. Every uncovered line's source was re-read at `b382b40c5` and
given exactly one disposition — a waiver class with its mechanism, `OPEN` with the seam that is
missing, or removal (a line that no longer exists). A construct that moved keeps its classification
and gets the new number plus a `moved ->` note; it is never silently renumbered. TAU-D22 then
*covered* 82 of the rows this table carried (§12), so those rows are gone from §4.1 rather than
re-argumented.

### 4.1 Waived, checked against the code

| file:lines | category | mechanism |
|---|---|---|
| `remote/real_network.rs` 1-1065 | `unreachable-in-this-environment` (788 measured lines; count kept out of the claim cell: F.94) | **the real-network harness itself.** Every test in the file is `#[ignore]`d (they spawn two child processes, a real `nats-server` and a real `future-agent`); under the ordinary suite nothing in it runs. It is verification machinery, not product code: the behaviour it exercises is the shipping bridge/transfer/prompt path, measured in `docs/testing/real-network.md`. Registered rather than excluded because the campaign's metric is per-line `DA:0` in the merged LCOV, and an unregistered zero is an unwaived zero. Unchanged by the merge (the file is 1065 lines at HEAD too). |
| `remote/verify_e2e.rs` 350-353, 357-360, 365-383, 385-416, 421-422, 425-431 | `unreachable-in-this-environment` (68 lines) | the **cross-process `InstalledHost` fixture** (`install_host`, `install_host_with_creds`, its `Drop`). Its only caller is `remote/real_network.rs`, whose tests are `#[ignore]`d because they need `nats-server`, a `future-agent` and a model endpoint (§7); the ordinary suite never constructs it. The merge added the per-device fields to the `RemoteState` literal it builds (`pairing_code`, `pairing_confirmed`), which is why the block runs 350-431 at HEAD; same class and the same reason as the harness file itself. |
| `remote/verify_e2e.rs` 1949 | `attribution-artifact` | the `Some(value) => set_var("VERIFY_E2E_NATS_URL", …)` arm of the test's own env restore, which cannot be taken while the suite runs with the variable starting unset — the `None` arm beside it **is** covered. `moved ->` 1927 (the +22-line drift is §11's per-device `RemoteState` fields). |
| `remote/verify_e2e.rs` 2147 | `unreachable-in-this-environment` | the `return` of the scripted `/connz` task's `let Ok(..) = listener.accept().await else` arm: a loopback listener does not fail `accept` while it is alive, and the task is dropped with the test. The statements inside the loop (`:2148-2151`) and its continuation are covered. `moved ->` 2125, whose row called it a closing brace; the +22 drift put the accept guard on that number, so the mechanism is stated from the source instead of carried over. |
| `remote/verify_e2e.rs` 2532 | `unreachable-by-construction` | `Ok(()) => panic!("a zeroed monitor must not satisfy the receipt arithmetic")` in the overflow test: a zeroed `/connz` reading is driven through `assert_broker_accounting` under `catch_unwind`, and the arithmetic must panic; a passing run takes `Err(payload)` (`:2533-2536`). The `Ok` arm exists to fail an implementation that stopped subtracting, so a green suite cannot enter it. `moved ->` 2510 (+22). |
| `remote/supervisor/start.rs` 76, 111, 282, 294, 401 | `unreachable-in-this-environment` (5 lines; the construct is named per line) | **concurrent writers the child cannot be.** `:76`/`:111` return `empty()` when `start_requested` is cleared between two awaits of `start_generation` (only `perform_stop`/`stop` writes it, and both then tear the runtime down); `:282` is the same check after `connect_nats`; `:294` is `access.commit(epoch, …)` yielding `None` because a stop invalidated the epoch between statements; `:401` is the second `commit` returning `None` for the same reason. All five are written by `#1174` (the per-slot loop) and none was reachable before it either. `moved ->` 74, 109, 280, 292, 399 (the +2 is D26's guard at `:52`, §14.1). The sixth line of D23's row, `:423` (the start-retry singleton flag already held), is **covered** since D26: the budget child asks for two retries at once and the second returns early. |
| `remote/supervisor/start.rs` 116 | `unreachable-by-construction` | `(0, None) => Ok(empty())` in `start_generation`'s tail: `started == 0 && failure.is_none()` requires `plans` to be empty, and `establish_slots()` either returns an error or pushes at least one plan (it mints an invitation when the book is empty, `:212-222`). `moved ->` 114. |
| `remote/supervisor/start.rs` 739 | `unreachable-by-construction` | the `_ => RemoteFailureReason::Local` arm of `start_failure`'s reason match. `pairing::error_code` returns exactly `network`, `revoked`, `account_authorization`, `service_authorization`, `server` or `None` (`pairing.rs:52-67`; the last is the `AppError::Remote { code: _ }` fallback), and each of those has its own arm above, so no `Some(code)` reaches `_`. `moved ->` 737. |
| `remote/supervisor/start.rs` 595, 603 | `unreachable-in-this-environment` (2 lines) | the two watcher exits that race teardown, not inputs: `:595` is `GenerationWatch`'s `start_requested` return one 1 s tick later, cancelled first; `:603` is the watch's "the entry vanished or its generation changed" return, which needs a replacement to land between the tick and the lookup. `moved ->` 593, 601. The third line of D23's row, `:448` (the retry loop's `!start_requested || phase == Ready` break), is **covered** since D26: the retry child's terminal attempt reaches the same `break` and the loop exits through it. |
| `remote/supervisor/start.rs` 469-470 | `unreachable-in-this-environment` | the retry worker's **re-arm**: `if start_requested && retryable_start_status(&latest) { spawn_start_retry(); }` after the loop's exit tail. Every attempt writes `last_error_code = "connecting"` (`:59`) before it asks anything, and an uncoded failure leaves it there, so the status the tail re-reads is `Connecting` — *not* retryable. The only way to satisfy the condition is a `stop()`→`start()` pair landing inside the few statements between the break and `:468`, which is the race the comment above this code describes; it has no injection seam (the child is the only writer and it cannot act inside a synchronous block). Measured `DA:0` in both D26 children that reach the tail. |
| `remote/supervisor/start.rs` 497-498 | `unreachable-in-this-environment` | `if attempt > MAX_RUNTIME_RECONNECT_ATTEMPTS { … return; }` in `spawn_runtime_reconnect`. The counter is incremented only by `record_runtime_failure` (`:490`), whose only caller is this function, and every caller first reads it at `:627` behind `>= MAX_RUNTIME_RECONNECT_ATTEMPTS` and returns instead. Reaching `attempt == 4` therefore needs two watchers' load (`:625-628`) and record (`:490`) to interleave inside a few hundred nanoseconds — the 1 s ticks of the budget child's four devices are ~10-50 ms apart, and `--test-threads=1` does not align them. The enumerated missing seam is a way for a child to pre-arm `runtime_reconnect_attempts` (it is `pub(in crate::remote)`, so an integration test cannot call `record_runtime_failure` itself); none exists. |
| `remote/supervisor/start.rs` 669-671, 673-679, 682-701, 703-704, 707-710 | `unreachable-in-this-environment` (36 lines) | `spawn_web_reconnect`'s body. In a non-test build `web_client_enabled()` is true only when the resolved platform URL is exactly `https://test.future-os.cn` (`web_server.rs:41`), so the fixture platform — a loopback HTTP server — selects the other arm and `web_bind_failed` is never set. Reaching this block needs the real staging platform (a test-platform account) plus port 8022 already bound so `bind_web_listener` fails. The fixture cannot supply either: loopback cannot be both the staging URL and a local server without intercepting its TLS, and the busy port alone would not enable the branch. Not a product seam; the listener is deliberately staging-only. `moved ->` 667-708 (D26's +2 at `:52`; the five lines D23 counted at `:672`, `:680-681`, `:702`, `:705-706` had no `DA:` record in this render — the block is the same source). |
| `remote/supervisor/shutdown.rs` 308 | `attribution-artifact` | the closing brace of `handle_system_resume`'s `if transition.rotate_epoch` block; the statements inside it (`:303-307`) are covered and llvm-cov attributes the region end to the brace. |
| `remote/supervisor/state.rs` 278, 280, 296 | `unreachable-in-this-environment` (3 lines) | `pairing::set_active`'s write failing inside `claim_control` (`:278`), the closing brace of that error arm (`:280`), and the same failure on the detached thread of `claim_control_soon` (`:296`). The write goes through `config_io::write_json_atomic` into the test `HOME`, which is a writable temp directory; reaching it needs a filesystem that refuses the write (read-only mount/ENOSPC). There is no `INJECT_SAVE_FAILURE`-style seam for `set_active` — the existing seam covers `save_creds` only. |
| `remote/supervisor/status.rs` 187 | `unreachable-by-construction` | `status_focused`'s `None => return stopped_status(prefer)` in the focus match: the function returns early when `guard` is empty (`:143-147`), so a non-empty `guard` puts at least one `device_row` into `devices` in the second loop (`:164-176`) and `devices.first()` is always `Some`. The arm is dead. |
| `remote/commands.rs` 332, 510 | `attribution-artifact` | closing braces of blocks whose statements are covered: `:332` ends the `if matches!(activated, Some(Ok(())))` reply block (`:329-330`), `:510` ends the `slots.retain(…)` closure (`:504-508`). llvm-cov attributes each region end to the brace. `moved ->` 318 (`:332`) and 474 (`:510`); the +125-line drift is `#1174`'s command surface. |
| `remote/commands.rs` 367, 907 | `unreachable-in-this-environment` (2 lines) | `SUPERVISOR.access.commit(epoch, …)` returning `None` for a handshake whose epoch was invalidated between two statements: `:367` on the secure-handshake path, `:907` on the pairing-confirmation path. Only a concurrent `stop()`/power transition writes the epoch, and both statements run inside one command handler. `moved ->` 345 (`:367`) and 800 (`:907`). |
| `remote/commands.rs` 1004, 1007, 1010 | `unreachable-by-construction` | `encode_reply_payload_with_gzip`: `write_all` into an in-memory `Vec`, `GzEncoder::finish`, and the "compression did not shrink" arm. The first two cannot fail (`impl io::Write for Vec<u8>`); the third is the codec invariant — `plain` is `serde_json` text of at least 32 KiB (`REMOTE_JSON_GZIP_THRESHOLD_BYTES`), whose per-byte entropy is bounded below 8 bits/byte, while `compressed.len() >= plain.len()` needs ≥ 8 bits/byte plus gzip's 18-byte header/trailer. Every byte-diverse JSON encoding measured still shrinks (base64 0.77, hex 0.59, distinct-float array 0.49, printable-ASCII 0.83, CJK 0.74, 3000 distinct keys 0.45). Kept by decision §6.1. `moved ->` 879, 882, 885. |
| `remote/commands.rs` 1025, 1028, 1040 | `unreachable-in-this-environment` (3 lines) | the reply lane's failure arms: `:1025` a `REPLY` scope whose seal returns `Err`; `:1028` the `#[cfg(not(test))]` return of the "no REPLY scope" arm (cfg-split: the test build falls through to `:1031`, and in the non-test child every reply is published inside a scope); `:1040` a `flush()` that fails on a healthy in-process broker. Each needs the channel/store to lose its state between two statements. `moved ->` 900, 903, 915. |
| `remote/presence.rs` 123 | `unreachable-by-construction` | `publish_control`'s `let Ok(bytes) = serde_json::to_vec(&payload) else { return };`: `payload` is a `json!` object (string keys, scalar values), and `to_vec` on a `Value` errors only on non-string map keys or a failing `Serialize` impl — neither exists here. New file in `#1174`. |
| `remote/publisher.rs` 347, 554, 634-641 | `unreachable-in-this-environment` (10 lines) | the publisher's own tick task: `:347` needs `publish_batch` to fail while flushing on shutdown; `:554` a catalog/workspace read that returns `None` between two statements of the tick; `:634` a secure activity publish that reports "not sent" (`Ok(false)`), and `:635-641` one that errors. A seam would have to inject a failing `secure::publish`/store read. **The oscillation, recorded rather than claimed:** `:646` (the workspace block's closing brace) and `:650` (the `continue` on a `None` workspaces read) read **`DA:0` in `union-d22` and `DA>0` in `union-d23`** with no code change between the two pins — the same pair this record has carried since TAU-D11 at `:624`/`:628`. They are excluded from this claim because only a `DA:0` reading may be claimed, and a reader must not read their absence as a fix. |
| `remote/transport.rs` 243, 334, 353-360 | `unreachable-in-this-environment` (9 lines) + 1 `attribution-artifact` | the credential-refresh generation guards: `:243` returns when the generation died between two awaits of the refresh, `:353-360` aborts the freshly built tasks when the pairing's generation changed under the state lock. Both need a concurrent `stop()`/replacement to land inside the refresh window; the child's refresh worker is the only writer. `:334` is the `};` that closes the `connected_nats` match — an attribution artifact: its own `Err` arm (`:320-333`) is covered by §12's rebuild-failure test, and this one run attributes the region end to the brace. `moved ->` 221, 331-338 (D22's `#[cfg(test)]` seam moved the file +21). |
| `remote/trust.rs` 105-110 | `unreachable-in-this-environment` | the platform trust store's **error** return in `load_platform_certs` (`load_native_certs` reporting any errors, e.g. the macOS Security-framework read failing). The non-test child fixture calls `connect_nats` -> `trust::root_bundle()` and covers `:41-43` (the `#[cfg(not(test))]` snapshot path), `:102-104` and the whole success path (`:111-117`); the error arms need the platform store read itself to fail, which is the intermittent host fault the snapshot exists to survive, not a fixture input. Unchanged by the merge. |
| `remote/secure.rs` 814 | `unreachable-by-construction` | `Ok(_) => panic!("an inert transport must not open")`: the line exists to fail if `open` ever returns `Ok` for a transport whose `enabled()` is false, the invariant asserted three lines above (`:811-813`). |
| `remote/lifecycle.rs` 40 | `unreachable-by-construction` | `changed.changed() == Err` needs every `watch::Sender` dropped while the receiver is borrowed from the same `AccessEpoch`; the sender is that struct's own field, held by the borrow, so the type makes it unproducible. |
| `remote/transfer.rs` 44-48, 103 | `unreachable-in-this-environment` | a `queue_subscribe` the fake broker refuses (`:44-48`) and a seal failure on the transfer reply lane (`:103`); both need a broker/channel that fails on demand. Unchanged by the merge. |
| `remote/test_support.rs` 689, 694 | `platform-unmeasured` | the `else` arms of the mock's `probe_sandbox` default answer (`"unsupported_platform"` at `:689`, `"none"` at `:694`): the mock must report what its own host has, and this host is macOS, so `cfg!(target_os = "macos")` takes the `if` side and these are measured by a Windows/Linux run. `moved ->` 678, 683 (+11: `#1174`'s per-connection capability fields). |
| `remote/test_support.rs` 1811, 1825, 2498 | `unreachable-by-construction` | `:1811` pushes a violation when a body in `src/` takes `HomeGuard::new`/`TEST_HOME_LOCK` before `mock_agent_lock(`, and `:1825` formats the list — the latter only when the `assert!` at `:1820` fails. The invariant holds: the census walked every file, found 2 inversions, both in `remote_host/business/settings.rs`, and both were fixed (`docs/testing/flaky-tauri.md` §1). `:2498` is `Err(error) => panic!("expected the deferred arm, got the failed arm: …")` in `install_fake_state_installs_a_live_bridge_and_drives_the_deferred_arm` — the regression guard for that contract, which a passing run cannot enter. Same class as `remote_host/lean.rs:1243`. `moved ->` 1790 (`:1811`), 1804 (`:1825`), 2477 (`:2498`) (+21). |
| `remote_host/lean.rs` 522 | `unreachable-by-construction` | `let Some(fields) = block.as_object_mut() else { continue }` inside the `"tool_call"` arm: taking that arm requires `block["kind"] == "tool_call"`, i.e. an object with a string `kind`, so `as_object_mut()` is always `Some`. `moved ->` 501 (+21). |
| `remote_host/lean.rs` 1243 | `unreachable-by-construction` | `Some(false) => panic!("a history result must not send isError: false")` in the fixture validator: the repository's fixtures are generated from the shipping shapes and #849 stopped emitting `isError: false`, so a green suite cannot reach it — it exists to fail a fixture that regressed. `moved ->` 1222 (+21). |
| `remote_host/lean.rs` 1267-1270, 1435-1439, 1444-1447, 1449-1450 | `unreachable-in-this-environment` (16 lines) | the env gates and report printing of the two `#[ignore]`d drivers (`LEAN_RENDER_FIXTURES` from `scripts/screenshots/gen-lean-render-fixtures.mjs`, `LEAN_HISTORY_ENTRIES`/`LEAN_HISTORY_OUT` from `scripts/measure/measure-lean-history.py`). Their bodies are covered by §3.2. `moved ->` 1246-1249, 1414-1418, 1423-1426, 1428-1429 (+21). |
| `remote_host/lean.rs` 1375, 1474 | `attribution-artifact` | closing braces of conditional blocks whose bodies are covered (the footer assertion at `:1371-1374`, the per-block tally at `:1465-1473`); llvm-cov attributes each region end to the brace. `moved ->` 1354, 1453 (+21). |
| `remote_host/business/catalog.rs` 104-106 | `unreachable-by-construction` | "Fork produced no session.": `agent_bridge::fork_agent_session` returns `Err("Fork did not return a session.")` when the Agent's reply carries no session id (`agent_bridge/session.rs:273-275`) and only returns `Ok` with the id it created the thread for, so the check cannot fire. |
| `remote_host/business/catalog.rs` 122-128, 130 | `unreachable-in-this-environment` | "Forked thread could not be loaded" and the read error beside it: the row `get_thread(new_thread_id)` reads was written by the same call a statement earlier, so `Ok(None)` needs a concurrent deleter and `Err` a store failure between two statements of one function. |
| `remote_host/business/history.rs` 273 | `attribution-artifact` | a closing brace inside the page-shape builder whose statements are covered above it. |
| `remote_host/business/prompt.rs` 145-151 | `unreachable-in-this-environment` | the "Future Agent did not acknowledge the prompt" reply fires only when `accepted_rx` is dropped with no send, i.e. when the spawned `run_prepared_prompt_inner` future dies before its receipt send — a panic or an abort of a task whose `JoinHandle` nobody retains. No caller can abort it and the green suite produces no panic. |
| `remote_host/business/providers.rs` 58-64 | `unreachable-by-construction` | `reply_view`'s `Err(_)` arm for `serde_json::to_value(ProvidersView)`: the view is two `Vec`s of structs of `String`/`bool`/`usize`/`i32`, every one a derived `Serialize` with no map and no hand-written impl (`agent_providers/mod.rs:55-105`), and `to_value` errors only on non-string map keys or a `Serialize` impl that itself errors — so it cannot fail here. Kept by decision §6.1. |
| `remote_host/business/settings.rs` 148-150, 176-180 | `platform-unmeasured` | `product_sandbox_available()` is `Ok(true)` **unconditionally on macOS** (`settings.rs:300-304`), so neither the `Ok(false)` tier arms nor the probe's `Err` arm can be reached by a macOS measurement; the Windows/Linux flavor measures them. |
| `remote_host/business/transfers.rs` 59 | `unreachable-in-this-environment` | `spawn_blocking`'s `JoinError`: the blocking `session_files::list` must panic or the runtime be shut down. Needs a seam (or a bug) inside `list`. |
| `remote_host/business/wire_limits.rs` 717-722 | `unreachable-in-this-environment` | the ignored `measure_phone_page` driver's env gate (`VERIFY_E2E_PHONE_PAGE`, dumped by `scripts/measure/measure-lean-history.py --phone-page` against a live agent). `measure_page`'s body is covered by §3.2; only the file read and the env var remain. |
| `remote_host/files.rs` 367 | `unreachable-by-construction` | `images > MAX_IMAGES` is shadowed by `references.len() > MAX_ATTACHMENTS` at `:335`: `MAX_ATTACHMENTS == MAX_IMAGES == 10`, so a list that reaches `:366` holds at most 10 images. Kept by decision §6.1 (the shadowing is a constant coincidence, and this guard is the only enforcement of `MAX_IMAGES`). |
| `remote_host/files.rs` 1145, 1183 | `unreachable-in-this-environment` | `:1145` is the **`std::fs::copy` fallback taken when `hard_link` fails** (`:1144`); the size check is *after* it (`:1147`). It needs a filesystem that refuses `link()`, i.e. a cross-device target (`transfer_root()` is `temp_dir()`, the cache lives under `HOME`) or a volume without POSIX links. `:1183` is the **`remove_file` inside the rename-failure arm** (`:1182`): it executes when `rename` fails, not when the remove races a concurrent writer. |
| `remote_host/read_pages.rs` 70 | `unreachable-by-construction` | the `else { break }` of the eviction loop: `insert` refuses `bytes.len() > MAX_SNAPSHOT_BYTES` (16 MiB) at `:50` and `MAX_CACHE_BYTES` is 32 MiB, so with an empty cache the loop condition is false and `min_by_key` always yields `Some`. Kept by decision §6.1 (it is the loop's liveness guard, not a duplicate). |
| `remote_host/session_files.rs` 74 | `unreachable-in-this-environment` | `target.metadata()` failing *after* `canonicalize` succeeded: the entry must be removed inside that window. `:68` (dangling symlink) and `:77` (FIFO) are covered by the `#[cfg(unix)]` test; the TOCTOU window is not schedulable. |
| `remote_host/pairing.rs` 292-293 | `unreachable-in-this-environment` | the `remove_file` error arm of `remove_slot`: both files are unlinked when the last device goes. It runs only when the unlink fails — a filesystem that refuses removal of an existing file (the test `HOME` is a writable temp dir). |
| `remote_host/pairing.rs` 345 | `unreachable-in-this-environment` | the `?` on `PairingIdentity::new(code_expires_at)` in `create_pairing`. **The failing source is LOCAL key generation, not the platform**: the constructor's only fallible step is `future_remote_crypto::generate_identity()` (`secure.rs:30-32` — one `snow` keypair, then an OS-RNG secret). Covering it needs local keygen to fail (fault injection on the crypto crate); it has nothing to do with what the platform supplies. `moved ->` 146 (the whole file grew with the pairing book, +199). |
| `remote_peer/runtime.rs` 152-154 | `unreachable-in-this-environment` | `delay_unit()`'s `#[cfg(not(test))]` body (seconds) beside the `#[cfg(test)]` one (milliseconds): cfg-split, so the production body exists only in the bin-linked copy — and the one non-test path the harness runs (the retry loop) does not call `reconnect_delay`, the only caller of `delay_unit()`. |
| `remote_peer/runtime.rs` 1659, 1723, 1736, 1761-1763 | `unreachable-in-this-environment` | the client's chunked-read guards: `:1659` refuses a page whose `totalBytes` exceeds `MAX_PAGE_BYTES`, `:1723` is `store_chunk`'s id/total/offset check, `:1736` its empty/short-chunk check, and `:1761-1763` is `page_shape_error` itself, whose every call site is one of those arms. Reaching them needs the *host* to answer an oversized or mismatched chunked read — the in-process host computes `totalBytes` from the page it built and caps it, so a test cannot make it lie without changing product code. |
| `remote_peer/runtime.rs` 2724 | `attribution-artifact` | the closing brace of the `visibility_change_for_test` wrapper; `:2721-2723` are covered and the region end is attributed to the brace. `moved ->` 2721 (#1174 added 3 lines above it). |
| `remote_peer/session.rs` 767, 773 | `unreachable-in-this-environment` (2 lines) | `:767` is the `Some(bundle)` arm of `trust::root_bundle()`: in the test build `remote/trust.rs`'s `bundle_path()` is the constant `None` (`trust.rs:49-52`), so the arm needs the non-test child fixture — the same one the bridge rows use, but this is the client role's own connection (`connect_nats` in `remote_peer`), which the bridge children never call. `:773` is `let options = options.require_tls(true)` under `#[cfg(not(test))]` — cfg-split: the test build connects to the in-process plaintext fake broker by design. |

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

**CLOSED 2026-10-18 (TAU-D9): three more lines, three tests, no product
change.** These were the review's measured fall (§10.2) and one new drift line
in `trust.rs` (§10.5). Same crosswalk shape (test name first, so no uncovered
line is claimed):

| test that closes it | file | line(s) it closes | how it reaches them |
|---|---|---|---|
| `remote::verify_e2e::tests::the_baseline_monitor_reading_runs_before_the_traffic` | `remote/verify_e2e.rs` | 1429, 1572 | a loopback `/connz` answers a zeroed baseline for every candidate pair id, then the shipping `run_real_traffic_with(.., Some(url))` takes the `Some(url)` arm of the baseline read; the zeroed second reading cannot match the receipt, so the arithmetic panics on purpose and the test asserts that panic message (reachability is the claim). |
| `remote::tests::runtime_tests::credential_refresh_returns_when_the_broker_rejects_the_refreshed_credential` | `remote/transport.rs` | 258-271 | the platform issue a credential for a loopback NATS endpoint that answers `CONNECT` with `-ERR 'Authorization Violation'`; `classify_nats_connect_error` maps it to `RemoteAuthorization`, so the refresh loop records `service_authorization` and returns. |
| `remote::trust::tests::a_failed_write_keeps_the_previous_bundle` | `remote/trust.rs` | 61-63 | the staging path is occupied by a directory, so `write_bundle` fails at its first statement; `bundle_at` must keep the bundle already on disk rather than answer `None`. |

**Re-checked against the retained post-merge union (2026-10-18):** all twelve closures still hold.
Every anchor the tables give is `DA>0` at that number except two, which moved again with `#1174` and
are `DA>0` at their new numbers — `verify_e2e.rs`'s baseline `Some(url)` arm is now `:1451` (was 1429)
and its consumer `if let` is `:1593` (was 1572), and `transport.rs`'s CONNECT-refuser arm is now
`:262-276` (was 258-271). The old numbers are covered statements of the surrounding code, not the
constructs. The `/tmp` pins these closures were first measured on were removed in F.100, which is why
the re-check is against the retained union.

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

With those closed, **1046 of the 1117 counted lines are waived and 71 are OPEN
with a named construct (§4.1 and §11.4; they are not waivers and must not be read
as environment properties)**, and three kinds of row should not be read as "the
code is fine, only untestable":

1. **Dead guards** (`remote_host/files.rs:367`, `remote_host/read_pages.rs:70`)
   and **arms that cannot fail** (`remote/commands.rs:879-882`,
   `remote_host/business/providers.rs:58-64`). Each is unreachable code *in the
   shipping shape*, so the tempting fix is to delete it — which a coverage change
   may not do on its own authority (PLAN §4 forbids deleting a guard to make its
   line disappear). **DECIDED 2026-10-17 (§6.1): all four are KEPT, with the
   per-item reasoning and the alternative's exact cost recorded there.** Two
   corrections to this item as first written: `remote/commands.rs:885` is *not*
   one of the "arms that cannot fail" (it is a codec-dependent size heuristic,
   re-registered as `unreachable-by-construction` with the codec invariant named
   in §4.1), and "the honest fix is to delete it" turned out to be wrong for the
   other three — `read_pages.rs:70` is the eviction loop's only exit.
2. **The cfg-split bodies** (`remote/supervisor/start.rs`) were the single
   largest block. **TAU-D11 built the child fixture (§11) and closed 75 of the
   139 lines** the old row carried: the retry worker's prefix and one iteration,
   `spawn_runtime_reconnect`'s entry and `Err` arm, `GenerationWatch`'s healthy
   reset and unhealthy branch, `spawn_runtime_supervisor`, `classify_nats_connect_error`'s
   authorization arm, `trust::root_bundle`'s non-test path, and
   `spawn_revoke_cleanup`'s error arm. **64 remain** (28 race/construct rows and
   36 web-listener lines), each with its own construct in §4.1 - they are no
   longer one undifferentiated "needs a fixture" block. The failure-injection
   seam proposed in the first draft stays **withdrawn - the supervisor ruled
   against it (§6.2)** - and that ruling did not cover the test-side fixture this
   segment built.
3. **Timing-driven lines** (`remote/publisher.rs`) move in and out of the ledger
   between runs: a line that was covered at 19:38 was `DA:0` at 20:54 with the
   same tests. A "0 uncovered" claim for this file would be a claim about one
   run's scheduling.

### 4.3 The 82 pre-merge rows: what they became (closed by TAU-D22)

`reconcile.py` reported **82 covered rows** against the post-merge pin: line numbers the pre-merge
table still cited as `DA:0` that the merged tree showed `DA>0`. §4.3 of that revision was a crosswalk
explaining where each construct had moved. **TAU-D22 then wrote the tests those rows were waiting
for, so the constructs themselves are now measured** (§12), and no row in §4.1 claims them.

What the crosswalk established, and why it is kept:

- Every one of the 82 was a **move**, not a silent "no longer `DA:0`, so drop it". `#1174` rewrote
  `start.rs`, `shutdown.rs`, `commands.rs`, `publisher.rs`, `transport.rs`, `test_support.rs`,
  `verify_e2e.rs` and `remote_host/pairing.rs`, and added `presence.rs`, `supervisor/state.rs`,
  `supervisor/status.rs` and `supervisor/mod.rs`. Each old number landed on *new* code the merged
  suite already executed, while the construct the row described sat elsewhere.
- The mapping below was the work list for §12: the construct in the "where it is now" column is the
  line D22's test drives, and its `DA:0 → DA>0` transition is the measurement.
- The constructs that did **not** survive the merge into a testable shape are OPEN with named seams:
  `start.rs` 455/458-470, 495-496/505, 530-553 and 604 (`#[cfg(not(test))]` bodies no `#[test]` in
  this crate can reach). `re_pair.rs` 25 was filed here too; **that was wrong and D23 closed it
  (§13.1)** — it was a fact about which CLI entry calls `re_pair()`, not about reachability.

| old row (file, pre-merge anchors) | where the construct is at `32ba1e13e` | why the old number is now `DA>0` |
|---|---|---|
| `commands.rs` — the two brace rows (318, 474) | 332, 510 | the numbers now land on the closing braces of the reply block and the `slots.retain` closure in the rewritten command loop; both statements they close are covered by the command-loop tests. |
| `commands.rs` — the epoch refusals (800, and 345 which was ABSENT) | 907, 367 | `access.commit` returning `None` moved with the per-device command gate; the old numbers sit inside `confirm_pairing` / the handshake body, which the handshake tests run. |
| `commands.rs` — the gzip codec arms (879, 882, 885) | 1004, 1007, 1010 | `encode_reply_payload_with_gzip` moved +125; the old numbers land in `reply()`/`publish_reply_payload`, covered by every command test. |
| `commands.rs` — the reply-lane failures (900, 903, 915 which was ABSENT) | 1025, 1028, 1040 | `publish_reply_payload` moved; the old numbers land in `reply()`, covered. |
| `publisher.rs` — the tick failures (336, 532, 612-619) | 347, 554, 634-641 | `#1174`'s standby gate and control packet moved the tick +11/+22; the old numbers land in the covered snapshot/workspace sections. |
| `remote_host/lean.rs` — every row (501, 1222, 1246-1249, 1354, 1414-1418, 1423-1426, 1428-1429, 1453) | +21 uniformly: 522, 1243, 1267-1270, 1375, 1435-1439, 1444-1447, 1449-1450, 1474 | `#1174` added the per-connection lean field and its comments, shifting the file by 21 lines; the old numbers land in the covered trim/fixture bodies. |
| `remote_host/pairing.rs` — the keygen row (146) | 345 | the pairing book (`#1173`) and multi-device (`#1174`) grew the file by 199 lines; `PairingIdentity::new` moved into `create_pairing`'s creds literal and the old number is covered book-reading code. |
| `remote_peer/runtime.rs` — the wrapper brace (2721) | 2724 | `#1174` added three lines above the test wrapper; the old number is a covered statement of the wrapper's call. |
| `remote/supervisor/shutdown.rs` — notice senders and resume arms (106, 225, 243-250) | notice helper 89-101, send loop 172, resume arms 316-320 / 326-333 | the file was rewritten (single `state` → map, per-device notices): the old numbers land inside `stop_gracefully` / `unpair_slot` / `offline_notice` code that the lifecycle tests execute. |
| `remote/supervisor/start.rs` — the concurrency guards (70, 81, 93, 215, 251, 316, 384) | 74, 109, 280, 292, 399 | the per-slot start loop (`#1174`) replaced `start_generation`; the old numbers land on the new `establish_slots`/`start_slot` statements, which the start tests and children run. |
| `remote/supervisor/start.rs` — the retry-loop exits (274, 283, 286-289, 293-298) | 446, plus the tail 458-470 | the retry worker was rewritten (per-slot `start_once`, `restart_slot`); the old numbers land in `start_slot`'s covered setup. |
| `remote/supervisor/start.rs` — the non-retryable reconnect (331) | 505 | same rewrite; the old number is a covered statement of `start_slot` (`} = transport;`). |
| `remote/supervisor/start.rs` — the watch exits (376, 387, 409) | 593, 601, 604 | `GenerationWatch` was rewritten; the old numbers land in the covered `RemoteState` literal / `Ok(())` of `start_slot`. |
| `remote/supervisor/start.rs` — the reconnect budget (416, 418) | 495-496, 630-632 | the budget moved to the per-device worker and the watch; the old numbers are `spawn_start_retry`'s signature and its `#[cfg(test)] return;`, both covered. |
| `remote/supervisor/start.rs` — the web block (453-455, 457-463, 466-485, 487-488, 491-494) | 667-708 | `spawn_web_reconnect` moved down 214 lines; the old numbers now sit in the retry worker's sleep/backoff and `spawn_runtime_reconnect`'s `#[cfg(test)]` head, which the children execute. |
| `remote/supervisor/start.rs` — the construction row (558) | 737 | `start_failure` moved +179; the old number is `record_runtime_failure`'s first statement, covered. |
| `remote/supervisor/start.rs` — `build_transport` refusal (124) | 322 | `start_slot` was split out of `start_generation`; the old number is the `web_bind_failed` expression's `.unwrap()`, covered. |
| `remote/test_support.rs` — the platform mock and census (678, 683, 1804, and 1790/2477 which were ABSENT) | 689, 694, 1825, and 1811/2498 | `#1174`'s per-connection capability fields moved the mock +11 and the census/test fixtures +21; the old numbers are covered statements of the same functions. |
| `remote/transport.rs` — the refresh guards (224, 297-300, 327-329) | 221, 301-303, 331-333 | the refresh loop was rewritten (`build_transport` now retries, the generation filter is `pairing_confirmed`); the old numbers land in the covered refresh prefix. |
| `remote/verify_e2e.rs` — the env restore and the accept guard (1927, 2125) | 1949, 2147 | the per-device `RemoteState` fields added 22 lines to the fixture; the old numbers are covered statements of the restore and the `/connz` task. |

**Rows that were already re-pointed before the merge** are carried above by their current numbers, not
by the pre-merge ones: `remote_peer/runtime.rs`'s chunk guards (1659, 1723, 1736, 1761-1763),
`remote_peer/session.rs` 767/773, and `remote/test_support.rs` 689/694. Those three rows moved with
`ccb51ffa3`/`5569d8c22`/`#1160` *before* this merge, which is why the pre-merge table and this one
agree on the constructs.

### 4.4 Composition of the 1083, and where it moved

Per file at `0b7b2ca17` (`DA:0` counts; the three subtrees sum to 1083):

| subtree | file | lines |
|---|---|---|
| `remote` | `supervisor/start.rs` | 49 |
| | `real_network.rs` | 788 |
| | `verify_e2e.rs` | 71 |
| | `commands.rs` | 10 |
| | `publisher.rs` | 10 |
| | `transport.rs` | 10 |
| | `transfer.rs` | 6 |
| | `trust.rs` | 6 |
| | `test_support.rs` | 5 |
| | `supervisor/state.rs` | 3 |
| | `supervisor/status.rs` | 1 |
| | `supervisor/shutdown.rs` | 1 |
| | `presence.rs` | 1 |
| | `secure.rs` | 1 |
| | `lifecycle.rs` | 1 |
| | **subtotal** | **963** |
| `remote_host` | `sync_measurement.rs` | 40 |
| | `lean.rs` | 19 |
| | `business/catalog.rs` | 11 |
| | `business/settings.rs` | 8 |
| | `business/prompt.rs` | 7 |
| | `business/providers.rs` | 7 |
| | `business/wire_limits.rs` | 6 |
| | `pairing.rs` | 3 |
| | `files.rs` | 3 |
| | `business/history.rs` | 1 |
| | `business/transfers.rs` | 1 |
| | `read_pages.rs` | 1 |
| | `session_files.rs` | 1 |
| | **subtotal** | **108** |
| `remote_peer` | `runtime.rs` | 10 |
| | `session.rs` | 2 |
| | **subtotal** | **12** |

**1197 → 1118 (D22) → 1115 (D23) → 1083 (D26).** D22 closed 82 lines with tests. D23 closed
**one** (`supervisor/re_pair.rs` 25) and that run measured **two more** as covered: `publisher.rs`
646 and 650, the pair §4.1 records as an oscillation rather than a fix. D26 moved the total by
**exactly the file it touched**: `supervisor/start.rs` 81 → 49, i.e. **1115 − 32 = 1083**, and no
other file's count changed in this render (the oscillating pair happens to be `DA>0` here, which is
why nothing else moved either way). Of those 32, 30 are the OPEN lines §13.3 costed and 2 (`:423`,
`:446` in D23's numbering) are lines D23 had already waived that D26's production fix made
reachable — §14.1 and §14.2. `supervisor/re_pair.rs` and `supervisor/mod.rs` no longer appear in the
table at all.

**Excluding the two harness files** (`real_network.rs` 788 and the `verify_e2e.rs` `InstalledHost`
block 68) the residual is **227**, all 227 waived, **0 OPEN**. Four of them are the race windows
§14.4 names; the rest are the rows below.

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
   **withdrawn by decision** — see §6.2. The rows stand as **OPEN with the named
   fixture** (133 at HEAD; TAU-D9 §10.4 reclassified them out of
   `unreachable-in-this-environment`; the earlier 167 counted lines the merged
   report now covers — §4).
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

Those are §8's own run of 2026-10-17 14:48. The **current** pin (TAU-D11) is
**1117** counted lines / **265** without the harness (§11.2); §4 carries the
ledger for it. The 1208 was TAU-D9's pin (§10.1), and the 1195 this section
names was the previous (d7b) figure — neither may be quoted at HEAD.

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

## 10. TAU-D9: the re-pin at HEAD, the two measured falls closed, and the 146-line registration defect (2026-10-18)

This segment did four things and changed no production behaviour: it re-measured
the module at HEAD, closed the review's two measurable falls, built the
review's loopback CONNECT refuser and covered what it covers, and replaced the
`unreachable-in-this-environment` label on the cfg-split block with the truth —
**an unbuilt test, not a host property**. It also corrected the three wrong
mechanisms in place.

### 10.1 The re-pin: 1195 → 1208, with the drift named

The review's §0.1 (`docs/testing/review-tauri-remote.md`, whose heading is "The report is pinned to a
revision that is not HEAD") is right and this segment confirms it by measurement: the
previous pin (`d7b`, 1195) describes `0e80e4204` plus the unit's uncommitted
test files, **not HEAD**. At HEAD `692c6e2bd` the same procedure measures
**1229** before this segment's tests, and **1208** after them. The bridge from
one number to the other is fully accounted for; nothing is carried forward.

| file | d7b (1195) | HEAD before D9 | final (1208) | what moved, and why |
|---|---|---|---|---|
| `remote/trust.rs` | — (absent) | 22 | **19** | **new file** in #1158 (`29e49d5c5`). `:61-63` closed here (§10.5); `41-43`/`102-117` are OPEN (§10.4) |
| `remote/transport.rs` | 31 | 35 | **21** | +4 from #1158's refresh rewrite (the network-failure arm grew a retry sleep); `:258-271` (14) closed here (§10.3) |
| `remote/supervisor/start.rs` | 138 | 139 | **139** | +9 source lines from #1158; one newly `DA:0` line (`:608`, the trust `Some` arm) |
| `remote/verify_e2e.rs` | 68 | 68 | **66** | `:1429` and `:1572` closed here (§10.2) |
| `remote_host/pairing.rs` | 1 | 4 | **4** | #1160 (`d8f907c0c`) added `:146` and the revoke-retire `eprintln!` (`:261-263`) |
| `remote/supervisor/re_pair.rs` | — (absent) | 1 | **1** | **new file** in #1160; `:25` is the failed-retire log |
| `remote_peer/session.rs` | 1 | 2 | **2** | #1160 inserted the trust `Some` arm at `:767`; the `require_tls` arms is now `:773` |
| `remote/publisher.rs` | 10 | 12 | **10** | the documented oscillation (`:624`, `:628`): covered in d7b, `DA:0` at the HEAD baseline, covered again in the final run — **no code change**, see §4.2 item 3 |
| all other files | unchanged | unchanged | unchanged | — |
| **total** | **1195** | **1229** | **1208** | +34 drift − 19 closed − 2 oscillation |

Report identity: `/tmp/tau-remote-final/final.info` (2026-10-18 03:04 CST).
`gui`: 2147 passed / 7 failed / 14 ignored in 574.48 s. `headless`: 1725 passed
/ 1 failed / 14 ignored in 428.62 s. Six gui failures are the port-8022 family
(the record's §5.2; the same tests pass in the headless leg, which is where
their lines are measured), and the seventh — plus the single headless failure —
is this segment's new baseline test under the full-suite id window; it was
re-run alone with coverage and that profraw is merged into this report
(§10.2.1). `--no-fail-fast` keeps both legs' profraw usable.

### 10.2 The two measured falls (`verify_e2e.rs:1429`, `:1572`)

The review's §3 probe is landed as `the_baseline_monitor_reading_runs_before_the_traffic`
in `verify_e2e.rs`'s test module (`#[tokio::test]`). It drives the shipping
`run_real_traffic_with(broker, …, Some(url))` against a loopback `/connz` that
offers zeroed counters, two answers deep (the window reads twice):

* **`:1429`** — the `Some(url)` arm of the baseline read — executes on the first
  answer. `DA:0 → 3` (both flavors plus the re-run).
* **`:1572`** — the `if let` that consumes `before` — executes after the
  measurement, because the baseline is `Some`. `DA:0 → 1`.

The arithmetic that follows is *supposed* to fail (a zeroed monitor cannot know
the probe bytes), so the test catches the unwind and asserts the panic text is
that arithmetic (`attempt to subtract with overflow` in a debug build, or `the
broker must relay exactly` in release) rather than the baseline's own
`expect("phone connection")`. Reachability is the row's claim, and the message
assertion is what keeps the test from passing on the failure mode it replaced.
`mock_agent_lock` is **not** taken by the test: `run_real_traffic_with` takes it
itself, and taking it twice deadlocks the non-reentrant mutex (the review
measured 3 m 41 s at 0 % CPU; this segment did not repeat that).

**The record's old mechanism is half true.** The *second* reading cannot be
scripted in advance — it reports bytes the run produces — but the baseline needs
no such number. §8.4's "a fixture-specific constant" route was a policy refusal,
not an impossibility; the probe shows an even cheaper route (any answer at all
for the connection name).

#### 10.2.1 The id window, and why the report carries a re-run

`E2e` mints its pair id at construction (`pair_{unique("e2e")}`), and
`unique()`'s counter is process-global. The review's probe offered ids 1–599;
that holds in isolation (counter ≈ 8) but **not** in a 2167-test suite, where
the counter is far past 600 — the first full run failed at `:1429`'s
`expect("phone connection")` for exactly that reason, in both flavors. The
committed test reserves the next counter value and offers the 4096 ids that can
follow it, which is deterministic because the measured suite runs one test at a
time. The fixed test passes alone (0.70 s) and its single-test coverage run
(`cargo llvm-cov --no-report --lib -- the_baseline_monitor…`) is merged into the
report; no other test's profraw was re-run or discarded.

### 10.3 The CONNECT refuser, and what it covers (`transport.rs:258-271`)

The review's second construct is landed: `NatsAuthRefuser` (in
`remote/tests.rs`, ~55 lines with its docs) is a loopback NATS endpoint that
answers the protocol minimum — `INFO`, read until `CONNECT`, then
`-ERR 'Authorization Violation'`. `async_nats` lowercases that text into
`ServerError::AuthorizationViolation` → `ConnectErrorKind::AuthorizationViolation`
→ `classify_nats_connect_error` → `AppError::RemoteAuthorization`. The test
`credential_refresh_returns_when_the_broker_rejects_the_refreshed_credential`
scripts `MockPlatform` to issue a credential for the refuser and drives the real
`spawn_credential_refresh`: the loop records `service_authorization` on the live
bridge, leaves `SUPERVISOR.state` in place (stopping is the supervisor's), and
returns. **`:258-271` (14 lines) are covered, `DA:2`.**

It covers no `start.rs` line: that block's bodies are `#[cfg(not(test))]` and
are absent from the lib test binary. The review's other construct (a child
stopped during the retry sleep, `shutdown.rs:49` → `start.rs:274`) is equally
valid but needs a non-test child, i.e. the fixture in §10.4; one was enough for
this segment, and the refuser is the one that also explains the transport row.

### 10.4 What remains OPEN: 157 lines, fixture F named, owner named

**HISTORICAL — superseded 2026-10-18 by §11.** The fixture named here was built
in TAU-D11: 75 of this section's 139 `start.rs`-family lines are now `DA>0`, the
`trust.rs` row is down to its 6 error arms, and `pairing.rs:261-263` is closed.
Read the rows in §4.1 for the current residual; the paragraph below is TAU-D9's
own record of the decision it handed to the supervisor.

**The 133 cfg-split lines of `start.rs` and the 24 lines of `trust.rs` /
`pairing.rs` / `re_pair.rs` below are OPEN, not waived.** The obstacle is a test
that has not been written, and the record must not call it an environment
property (the review's §4.1: `unreachable-in-this-environment` "is not
available for a line whose obstacle is an unbuilt test").

**Fixture F (start.rs, 133 lines; trust.rs, 19; `remote_peer/session.rs:767`):**
one integration target under `desktop/src-tauri/tests/` that spawns the
instrumented `futureos-headless` child with a stored pairing and a scripted
platform, then (a) points the broker at the §10.3 refuser — a non-retryable
start status breaks the retry loop at `:283` and runs the post-loop tail
(`:286-298`), and `connect_nats` reaches `trust::root_bundle()`'s non-test
`bundle_path()` and `load_platform_certs` — and (b) kills a broker the test
controls under a live child, which reaches
`spawn_runtime_reconnect`/`spawn_runtime_supervisor` (`:312-336`, `:369-421`,
`:433-494`) and the `GenerationWatch` exits. No product change; the supervisor's
2026-10-17 ruling (§6.2) refused a *production seam*, not this test-only child
fixture. **Owner of the decision to build it: the supervisor**, because the
target lives outside this todo's declared write set
(`desktop/src-tauri/src/{remote,remote_host,remote_peer}` only).

The remaining OPEN lines are three one-line fixtures, all inside the write set
but not built in this segment: `remote_host/pairing.rs:146` (a scripted malformed
`secure` identity), `:261-263` (a config write that fails while retiring a
revoke), and `remote/supervisor/re_pair.rs:25` (a `/pair/revoke` answered 500).
Each row in §4.1 names the exact fixture; none is registered as environmental.

### 10.5 The three wrong mechanisms, corrected in place

The review's §5.2 findings are applied **as text edits only; no verdict
changed**: `remote_host/files.rs:1145` is the `hard_link` fallback, not a copy
after a size check; `:1183` is the `remove_file` inside the rename-failure arm;
`remote/commands.rs:885` is `unreachable-by-construction` with the codec
invariant stated (text entropy < 8 bits/byte versus a ratio ≥ 1.0 plus gzip's
18-byte overhead) — the row survives, and the earlier "no construction argument
applies" sentence is withdrawn. The review's fourth §5.2 note (the
`real_network.rs` range) is left as the file envelope with its 788 measured
lines named in the mechanism cell; the range's 289 record-less lines are
reported by the gate as ABSENT, not COVERED.

### 10.6 The gate readback

**TAU-D9's readback, kept as history; the current gate line is §11.2.**

`reconcile.py` over the three scopes at this pin:

```
[unit] files with uncovered lines=29 uncovered lines=1208
  mentioned in module-tauri-remote.md: 1208
  MISSING (uncovered, not mentioned): 0
  STALE (doc cites it as uncovered; old doc sections also match unless --section is given): 290
    COVERED (DA>0 - a real disagreement, the actionable half): 0
    ABSENT (no `DA:` record -- NOT evidence: a blank line inside a claimed range, a comment, or a report from another revision): 290
```

`python3 .future/cov100/staletable.py --gate`, verbatim for this pairing
(header included so the columns are readable):

```
pairing                    role             doc                          claims  uncov  STALE ABSENT COVERED named         verdict
tauri-remote               authoritative    module-tauri-remote.md         1497   1208    290    290       0 section       PASS
```

The 290 ABSENT lines are the `real_network.rs` envelope's line-table-only
entries plus blank lines inside the `verify_e2e.rs` 341-416 range; ABSENT is
"no `DA:` record", not evidence of coverage. The registry's `expect` and
`section_heading` are re-pointed to this report and heading.
## 11. TAU-D11: fixture F built - the non-test child, 75 lines closed, and the 64 that remain (2026-10-18)

### 11.1 What was built, and why this shape is measured

`desktop/src-tauri/tests/remote_start_refusal.rs`, headless-gated (`#![cfg(feature =
"headless")]`; `futureos-headless` has `required-features = ["headless"]`, so the GUI
leg compiles the file to nothing). It spawns the **instrumented
`futureos-headless` bin** against a fixture the test owns:

* a **loopback platform** (a std-thread HTTP server answering
  `/client/v1/account/profile`, `/client/v1/remote/auth/token`,
  `/client/v1/remote/pair/code` and `/client/v1/remote/pair/revoke` from a script),
  selected by the child's `auth.json` `base_url`;
* a **controlled NATS broker** implemented as a real WebSocket server
  (`tokio-tungstenite::accept_async`, binary frames carrying `INFO`/`PONG`/`-ERR`).
  `ws://` is not a shortcut around the product's security: the non-test
  `connect_nats` calls `require_tls(true)`, and async-nats deliberately **skips the
  TLS upgrade for WebSocket addresses** (`connector.rs`: `!server_addr.is_websocket()`).
  A plaintext `nats://` listener can therefore only produce an IO/TLS error (a
  *retryable* transport status) and can never deliver the broker's own `-ERR`; a
  `ws://` listener speaks the protocol and can. No real account, relay, NATS server
  or TLS trust change is involved.
* a stored v2 pairing generated in the test (`nkeys::KeyPair::new_user` +
  `future_remote_crypto::generate_identity`), so the child is signed in and paired
  and does not need a terminal; one test uses `openpty` for `--re-pair`, which does.

Why a **spawned bin** and not a lib test: these lines exist only in the *non-test*
copy of `futureos_lib`. A `#[test]` body links the test copy, where
`spawn_start_retry`, `spawn_runtime_reconnect`, `GenerationWatch` and
`spawn_runtime_supervisor` are `#[cfg(test)] return;`/`()` stubs. An integration
test target links the normal lib; a spawned bin links the normal lib too. That is the
same seam `tests/headless_cli.rs`, `tests/re_pair_entry_points.rs` and
`tests/gui_main_thread_drive.rs` already use.

### 11.2 The measurement (per flavour, clean between legs, unioned)

The two legs are rendered **separately** because a single merged render accumulates
the first leg's profraw into the second (the defect `leg-flavour-check.py` exists to
catch). Recipe in `/tmp/tau11-measure.sh`:

```bash
cd desktop/src-tauri
export CARGO_TARGET_DIR=$W/target/cov-tauri-remote-final
cargo llvm-cov clean && cargo llvm-cov --no-report -j 4 --no-fail-fast -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/tau11-measure/gui.info
cargo llvm-cov clean && cargo llvm-cov --no-report -j 4 --no-default-features \
    --features headless --no-fail-fast -- --test-threads=1
cargo llvm-cov report --lcov --output-path /tmp/tau11-measure/headless.info
python3 .future/cov100/lcov-union.py --out /tmp/tau11-measure/union.info \
    /tmp/tau11-measure/gui.info /tmp/tau11-measure/headless.info
python3 .future/cov100/leg-flavour-check.py /tmp/tau11-measure/gui.info /tmp/tau11-measure/headless.info
```

Both legs **exit 0**: gui **2154 passed / 0 failed / 14 ignored**; headless
**1726 passed / 0 failed / 14 ignored** plus this target's **4 tests, 81 s**.
`leg-flavour-check`: **LEG IDENTITY: ok - each leg matches the flavour it claims**
(gui 170 files, headless 138; gui-only `commands/agent.rs`/`menu.rs` absent from the
headless leg, headless-only `no_gui.rs`/`headless/mod.rs`/`bin/futureos-headless.rs`
present). This run is the first all-green pair for this module in the campaign
(the six port-8022 failures of TAU-D9 did not recur).

| subtree | before (TAU-D9) | after (union) |
|---|---|---|
| `remote` | 1087 | **999** |
| `remote_host` | 109 | **106** |
| `remote_peer` | 12 | **12** |
| **total** | **1208** | **1117** |

The −91 is `start.rs` 139 → 64 (−75), `trust.rs` 19 → 6 (−13),
`remote_host/pairing.rs` 4 → 1 (−3), `web_server.rs` 1 → 0 (−1), and
`verify_e2e.rs` 66 → 67 (+1: `:2510` is not attributed to a covered region by the
union recipe; registered in §4.1).

`reconcile.py` over the three scopes against the union:

```
[unit] files with uncovered lines=28 uncovered lines=1117
  mentioned in module-tauri-remote.md: 1117
  MISSING (uncovered, not mentioned): 0
  STALE (doc cites it as uncovered; old doc sections also match unless --section is given): 290
    COVERED (DA>0 - a real disagreement, the actionable half): 0
    ABSENT (no `DA:` record -- NOT evidence: a blank line inside a claimed range, a comment, or a report from another revision): 290
```

`python3 .future/cov100/staletable.py --gate`, verbatim for this pairing:

```
pairing                    role             doc                          claims  uncov  STALE ABSENT COVERED named         verdict
tauri-remote               authoritative    module-tauri-remote.md         1406   1117    290    290       0 section       PASS
```

(The whole gate still reports two *other* pairings failing - `channels-wide` and a
CLI wide pairing, both pre-existing and untouched by this change. The `tauri-remote`
row above is the one this record owns.)

### 11.3 The four tests, and the lines each closes

| test | what it drives | lines it closed |
|---|---|---|
| `a_broker_that_refuses_the_credential_is_reported` | a stored pairing + platform credential; the controlled broker answers `CONNECT` with `-ERR 'Authorization Violation'`. `classify_nats_connect_error` → `RemoteAuthorization`, `start_failure` → `Failed/service_authorization`, the headless entry exits non-zero naming it. A corrupt `remote_pending_revokes.json` makes the compensation worker's arm run. | `start.rs` 607-608, 613-618, 628-629, 653-655; `trust.rs` 41-43, 102-104, 111-117; `web_server.rs` 43 |
| `a_re_pair_with_an_unreadable_revoke_queue_reports_and_still_refuses` | the same corrupt queue through the `--re-pair` entry (pty): `unpair` starts the compensation worker, `create_pairing` reports the retire failure, and the start after it is refused. | `remote_host/pairing.rs` 261-263; `start.rs` 648 (second `spawn_revoke_cleanup` returns early) |
| `a_live_broker_drives_the_generation_watch_and_reconnect` | live broker → `Ready`; a healthy watch tick; the broker rejects the **live** link. With an already-expired bridge JWT the health records `reconnect_required` (not terminal), so `GenerationWatch` calls `spawn_runtime_reconnect`; the worker reports `RT001` and `start_once(true)` returns the platform's undecodable response (`Err` arm). | `start.rs` 312-313, 317-323, 326-330, 333-336, 369-372, 374-375, 377-383, 386, 388-394, 396-398, 405-406, 407-408, 410-414, 419-421, 423, 433-439 |
| `a_healthy_generation_resets_the_budget_and_a_retryable_reconnect_arms_the_retry` | a 65 s healthy dwell crosses `RUNTIME_HEALTHY_RESET_SECS` (60), then a refusal whose reconnect start is a `server` failure takes the worker's `Ok(status) if retryable` arm. | `start.rs` 398-404, 324-325 |

`:158` (`if web_client_enabled()`'s false arm in the non-test build), `:608` and
`:614` are covered by the same children. `remote_peer/session.rs:767` is **not**:
that is the client role's own `root_bundle()` arm, and the children drive the
bridge, not the peer - it stays in §4.1.

### 11.4 The 64 that remain, and what exactly is missing

All of them are rows in §4.1; grouped here so the shape of the remainder is visible:

* **36 `spawn_web_reconnect` lines** (`453-455, 457-463, 466-485, 487-488,
  491-494`). In a non-test build `web_client_enabled()` is true only for
  `https://test.future-os.cn`; the loopback platform selects the other arm. Needs a
  real staging account and port 8022 bound - not a test-side construct.
* **12 retry-loop exits** (`274`, `283`, `286-289`, `293-298`). `:274` needs
  `start_requested == false` at a wake inside the worker's 0.8-1.2 s jittered sleep;
  `perform_stop` sets the flag and drops the runtime ~200 ms later, so the worker is
  cancelled unless the stop lands in that last 200 ms. A probe measured `:283` once
  this way (a stop-vs-loop race); it is not reproducible as a test.
* **7 concurrency guards** (`70`, `81`, `93`, `215`, `251`, `316`, `384`). Each
  needs a second writer (a user `start`/`stop`) to land between two statements of
  the child's own task; the fixture's child is the only writer.
* **1 secure-handshake refusal** (`124`). `build_transport` fails after a broker
  that answered CONNECT refuses the v2 secure handshake. The fixture broker accepts
  subscriptions; scripting the refusal at exactly the handshake request is the one
  broker variant not built.
* **3 watch exits** (`376`, `387`, `409`). `:376` is cancelled by the same teardown
  race as `:274`; `:387` races the headless loop's 1 s `status()` poll (both exit
  paths are 1 s polls); `:409` would need a watch tick while the worker's flag is
  set, but the watch returns immediately after spawning it.
* **3 reconnect-budget lines** (`416-418`). Need three consecutive generation
  replacements (`runtime_reconnect_attempts >= 3`). A 3-cycle variant was probed;
  the credential-refresh worker's 3 s `start_lock` hold made the interleaving
  unstable, so it is registered rather than committed as a flake.
* **1 non-retryable reconnect result** (`331`). Needs `start_once(true)` to return
  `Ok` with a non-retryable status: a `LiveThenRefuse` broker (accept the first
  generation, refuse every reconnection) with an unconfirmed pairing so the refresh
  worker cannot race the refusal. The same target can host it.
* **1 construction row** (`558`, `error_code`'s closed return set) and **1
  construction row** (`verify_e2e.rs:2510`, the overflow guard).

`re_pair.rs:25` is the only OPEN line outside `start.rs`/`trust.rs` (a GUI-entry
fixture; §4.1 says why it was not built here). `remote_host/pairing.rs:146` moved to
`unreachable-by-construction` with a **corrected mechanism**: the platform never
supplies the identity, so its old "malformed platform `secure` block" fixture was
not buildable as described.

### 11.5 What this segment changed, and what it did not

**No production line changed.** The test file is the only code added; `trust.rs`'s
snapshot, `start.rs`'s workers and the pairing/re-pair code are byte-identical. The
existing `#[cfg(not(test))]` bodies were measured through the seam the tree already
uses, not through a new one.

## 12. TAU-D22: the 35 remaining seams, 82 closed, and the fixture that is still not ours (2026-10-18)

### 12.1 What was built, and where each cluster lives

The post-merge ledger left **117 OPEN** lines, each with a seam named in §4.1. This segment wrote
tests for **82 of them** and closed them; **35 remain OPEN** and §4.1 still names each one's missing
fixture. No production behaviour changed, and no assertion was weakened.

| cluster | test(s) | file | lines closed |
|---|---|---|---|
| add / takeover / control | `add_device_connects_and_names_the_device_it_paired`, `takeover_from_the_desktop_switches_control`, `controlling_pair_falls_back_to_the_only_running_device`, `a_single_saved_device_claims_the_machine_at_start`, `promotion_leaves_a_claimed_pointer_alone`, `a_start_without_replace_keeps_an_installed_slot` | `remote/tests.rs` (`mod slot_tests`) | `start.rs` 82, 162-178, 245, 271; `supervisor/mod.rs` 53-60; `supervisor/state.rs` 258 |
| two devices, control notifications | `a_first_secure_handshake_stands_the_previous_controller_down`, `a_first_confirmation_stands_the_previous_controller_down`, `a_standby_device_is_answered_with_its_own_liveness_only` | `remote/commands.rs` (`mod bridge_tests`) | `commands.rs` 400, 656, 949 |
| notices and power | `unpair_notifies_every_connected_device`, `unpairing_one_device_notifies_that_device_by_name`, `a_graceful_stop_and_a_notice_call_reach_a_connected_device`, `a_slot_without_an_active_secure_channel_announces_nothing`, `a_resume_refused_with_a_retryable_status_arms_the_retry`, `a_resume_with_a_local_fault_records_reconnect_required` | `remote/tests.rs` | `shutdown.rs` 12-13, 51, 89-101, 163-164, 172, 316-320, 326-333, 387 |
| transport rebuild failure | `a_start_whose_transport_cannot_be_built_reports_the_failure`, `a_refresh_whose_transport_cannot_be_rebuilt_keeps_the_running_bridge` | `remote/tests.rs` | `start.rs` 322; `transport.rs` 301-312 (now 320-331) |
| a disconnected client | `a_disconnected_client_is_failed_when_no_start_was_requested` | `remote/tests.rs` | `status.rs` 57 |
| the pairing book's default | `a_book_without_a_version_decodes_as_the_current_one` | `remote_host/pairing.rs` (`mod tests`) | `remote_host/pairing.rs` 83-85 |

**The one test-only seam.** `remote/transport.rs` gained one `#[cfg(test)]` one-shot,
`INJECT_BUILD_FAILURE`, consumed with `swap(0)` at the top of `build_transport`. It is the same shape
and the same justification as `remote_host/pairing.rs`'s `INJECT_SAVE_FAILURE`: the arms it reaches
are "the transport could not be built after a credential refresh", whose real trigger is a broker
that completes `CONNECT` and then refuses the secure handshake — a behaviour `FakeNats` does not
model. There is **no production copy**: the statement is behind `#[cfg(test)]`, `cargo clippy --lib
--tests -D warnings` is clean, and the shipping control flow is unchanged. This is the distinction
§6.2 drew: that ruling refused a *production* seam; this is a test-side injection at a point the
function already fails through on its own.

### 12.2 The measurement

Report **`.future/cov100/pinned/union-d22.info`**, written by `/tmp/d22-measure.sh` (recipe §11:
`cargo llvm-cov clean --workspace`, one leg per flavour, `report --lcov` per leg, `lcov-union.py`,
`leg-flavour-check.py`). Both legs **exit 0**:

| leg | result | time |
|---|---|---|
| gui | **2210 passed / 0 failed / 14 ignored** | 577.65 s |
| headless | **1768 passed / 0 failed / 14 ignored** (+13 integration, +4 `remote_start_refusal`) | 450.15 s + 81.39 s |

`leg-flavour-check`: **LEG IDENTITY: ok** (gui 171 files, headless 139; the gui-only
`commands/agent.rs`/`menu.rs` absent from the headless leg, the headless-only
`no_gui.rs`/`headless/mod.rs`/`bin/futureos-headless.rs` present). Totals:

| | post-merge pin (`32ba1e13e`) | this revision (`b382b40c5`) |
|---|---|---|
| uncovered | 1197 | **1118** |
| OPEN | 117 | **35** |
| waived | 1080 | **1083** |

A first run of this measurement read **1119** and that number is worth recording: it showed
`start.rs:271` *not* flipped, because the test as first written asserted the slot was untouched while
`start_once(false)` was still returning early with the running status — so the per-slot guard was
never consulted. `slot_health` only reports a slot as not-running when a critical task has finished
and the crash-loop budget is spent; aborting the task is what makes the caller go on to try the slot.
The test was corrected, a module probe confirmed `271` at `DA:1`, and the run above is the re-measure.
**A passing assertion is not evidence that a line ran** — this is the failure mode to look for in the
35 rows that stay OPEN.

### 12.3 What stays OPEN, and the one thing this unit cannot reach

**Corrected by D23 (§13.1): this list said 35 and named `re_pair.rs:25`; that line was reachable and is now closed. What remains is 34, every one a `#[cfg(not(test))]` body:**

- `start.rs` 455, 458-461, 465-470 — the retry worker's terminal result and its exit tail;
- `start.rs` 495-496, 505, 630-632 — the runtime-reconnect budget's exhaustion and its non-retryable
  `Ok(_)` arm;
- `start.rs` 530-553 — `restart_slot`'s four other inputs;
- `start.rs` 604 — `GenerationWatch` on a terminal health;

Every one of these lives in the **non-test copy** of `futureos_lib` (`#[cfg(not(test))]` replaces them
with stubs in the lib test binary). Reaching them needs a spawned child, i.e. a new test in
**`desktop/src-tauri/tests/`** — which is **outside this todo's declared write set**
(`src/{remote,remote_host,remote_peer}` and this document). TAU-D11's `tests/remote_start_refusal.rs`
is exactly that target, so the work is an *extension of an existing fixture*, not a new one; the
supervisor has to grant the path. This is recorded as the successor todo rather than done here.

### 12.4 Rejected approaches

- **Widening `INJECT_BUILD_FAILURE` into a general production knob.** It stays `#[cfg(test)]`; a
  runtime-injectable transport failure is the seam §6.2 refused.
- **Making `start_once(false)` reach `271` by asking it to `replace_existing`.** That tests a different
  branch: `271` exists precisely so a non-replacing start leaves a live slot alone.
- **Asserting `status()` right after `remote::takeover`.** The stopped projection reads the book, and
  the pointer is written on a detached thread by design, so the test waits for the persisted value
  instead of racing it. Asserting the returned status would have pinned a millisecond window as a
  contract.
- **Calling `handle_system_resume` with an empty state and expecting a recovery attempt.** With nothing
  running, `stopped_status` reports `Reconnecting` (from `system_sleep`), `runtime_active` is true, and
  `start_once(false)` returns before the platform is ever asked. The tests install a slot whose health
  is *terminal* so the recovery actually runs; the first version of the retryable test asserted a code
  the early return could never produce.
- **Registering the three `DA:0` lines this run attributes differently as "new uncovered".** They are
  the documented oscillation (`publisher` 646/650) and a region-end brace (`transport` 334); they are
  waived with that mechanism, not counted as drift.

### 12.5 The gate readback

```
[tauri-remote] files with uncovered lines=31 uncovered lines=1118
  mentioned in module-tauri-remote.md: 1118
  MISSING (uncovered, not mentioned): 0
  STALE (doc cites it as uncovered; old doc sections also match unless --section is given): 283
    COVERED (DA>0 - a real disagreement, the actionable half): 0
    ABSENT (no `DA:` record -- NOT evidence): 283
```

`staletable.py --gate`, the `tauri-remote` row:

```
tauri-remote             authoritative    module-tauri-remote.md    1361   1118    283    283    0 section PASS
```

The 283 ABSENT are the `real_network.rs` envelope's line-table-only entries and blank lines inside the
`verify_e2e.rs` fixture range; ABSENT is "no `DA:` record", not evidence of coverage.

## 13. TAU-D23: a wrong row retracted, one line closed, and the four gates that remain (2026-10-18)

### 13.1 The row that was filed wrong

§4.1 carried `remote/supervisor/re_pair.rs` 25 as **`OPEN - needs a gui-flavour child`**, on the
argument that `re_pair()` is the GUI entry's path (`lib.rs::spawn_remote_re_pair`) while the headless
`--re-pair` entry calls `unpair()` + `start()` directly. Every clause of that argument is true, and
the conclusion did not follow from it. **It is a statement about which CLI entry reaches the
function, not about whether the function can be called.**

`re_pair` is a plain `pub async fn` with no `cfg` gate anywhere in its file (§13.3 shows the whole
body). A `#[tokio::test]` in the crate's own lib test binary calls it directly, and the input it
needs is a file the test already owns: a corrupt `remote_pending_revokes.json`.

The arm's own effect, as asserted:

| assertion | what it proves |
|---|---|
| `re_pair()` returns `Ok` | the settlement failure is not fatal — the contract in its own doc comment |
| `status.reason == Some(AccountAuthorization)` | execution reached `start()`, so the log arm completed rather than returning |
| the queue file still holds the exact corrupt bytes | the failure path did not "fix" the queue, so the `Err` branch is the one that ran |

Measured: `DA:0 → DA:1`. This is the sixth row in this record whose mechanism described the API
surface rather than the executed path (§9.4, §10.4, §11.3, §13.3 of the pre-D22 revision, §12.1's
`start.rs:271`); the pattern is worth stating once more because it keeps paying: **a caller list is
not a reachability proof.**

### 13.2 The measurement

Report **`.future/cov100/pinned/union-d23.info`**, written by `/tmp/d23-measure.sh` (recipe §11:
`cargo llvm-cov clean --workspace`, one leg per flavour, `report --lcov` per leg, `lcov-union.py`,
`leg-flavour-check.py`), on the tree at **`a9c5df505`**. Both legs **exit 0**:

| leg | result |
|---|---|
| gui | **2211 passed / 0 failed / 14 ignored** in 574.44 s (`rc=0`, report `rc=0`) |
| headless | **1769 passed / 0 failed / 14 ignored** in 453.02 s (`rc=0`, report `rc=0`) |
| union | `rc=0`; **LEG IDENTITY: ok — each leg matches the flavour it claims** |

Each leg is one more test than D22's (2210 / 1768): D23 added exactly one `#[test]`. Neither leg
logged a failure, so this is the second consecutive all-green pair for this module.

`1197 → 1118 → 1115`, and the last step is **three** lines, not one: `re_pair.rs` 25 (this segment's
test) plus `publisher.rs` 646 and 650, which read `DA:0` in `union-d22` and `DA>0` here. §4.1 keeps
that pair out of the claim and says why; a reader who wants a single number should read
**1115 with two known oscillators**, not "three lines closed".

### 13.3 The 34 that remain: what the gate is, and the cheapest seam for each

All 34 sit behind four `#[cfg(not(test))]` gates, verified by reading the attribute that precedes
each one:

| gate (line) | lines | what it is |
|---|---|---|
| 420 | 455, 458-461, 465-470 | `spawn_start_retry`'s worker body |
| 488 | 495-496, 505 | `spawn_runtime_reconnect`'s worker body |
| 523 | 530-531, 534-535, 538, 540-541, 545-553 | `restart_slot` (the whole fn) |
| 584 | 604, 630-632 | `GenerationWatch`'s impl |

In the lib test binary these four are replaced by `return;`/`()` stubs, so **no `#[test]` in this
crate can reach them at all** — this is a compilation fact, not a difficulty. They exist only in the
non-`test` copy that the bins link, which is why the route is a spawned child under
`desktop/src-tauri/tests/`.

**`tests/remote_start_refusal.rs` already owns everything the route needs** (read, not assumed): a
loopback platform with a *sequenced* `Vec<TokenReply>` whose last entry repeats, `TokenReply::
{Credential{exp_offset}, Malformed, ServerError}`, a real `ws://` broker with
`BrokerMode::{Refuse, Live{error_after}}` and a connect counter, `write_auth`/`write_pairing` for an
isolated HOME, a pty `--re-pair` entry, and `ChildRun` with `wait_for`/`interrupt`.

The two cheapest rows — both reachable by changing one value, no new fixture code:

- **`:604`** (`GenerationWatch`'s terminal-health return). The existing live test passes
  `TokenReply::Credential { exp_offset: -60 }` *on purpose*, with the comment "the credential expires
  immediately, so a live refusal is a transient generation failure rather than terminal". A fresh
  `exp_offset: 3600` makes `mark_authorization_rejected` set `service_config_error`, and the watch
  then takes this arm. One integer.
- **`:455`, `:458-461`, `:465-470`** (the retry worker's terminal result and its exit tail). The
  existing `ServerError` is retryable, so the worker always ends in the retryable arm. Sequences
  `[ServerError, Malformed]` make the *second* attempt unclassified, taking
  `Ok(_) | Err(_) => break` and then the tail. Ordering only.

The rest, with the piece each still needs:

- **`:495-496`, `:630-632`, `:505`** need a broker that accepts one generation and then **refuses
  every reconnection** — `BrokerMode::Live` accepts again and only errors on the live link. A new
  mode (the `LiveThenRefuse` this record has named since TAU-D11) is the one piece of new fixture
  code the whole set needs; four live-failure cycles then walk the budget to `MAX` and the
  non-retryable `Ok(_)` arm.
- **`:530-531`, `:534-535`, `:538`, `:540-541`, `:545-553`** need `restart_slot`'s inputs varied: the
  book edited between generations (`write_pairing` already writes it), a v1 slot, and a refresh
  answered with `invalid_remote_credential` — the last needs one new `TokenReply` variant, because
  the enum has no revoked shape today.

So the successor is not "write a fixture": it is **six small variations on a fixture that exists**,
and the only structural additions are one broker mode and one `TokenReply` variant.

### 13.4 Why it is not done in this segment

`desktop/src-tauri/tests/` is outside this todo's declared write set (`src/{remote,remote_host,
remote_peer}` and this document), and the kernel's own `workspace_lock_acquired` event for this turn
lists exactly those four paths — so there is no grant to read as one. The campaign has resolved this
class of block the same way before: TAU-D20 was released by a **user gate** granting
`tests/headless_cli.rs`, after five worker turns asked for it. Filing the gate once, with §13.3 as
the costed plan, is what this segment can do; asking a sixth time in a worker todo is what the gate
exists to avoid.

### 13.5 The gate readback

```
[tauri-remote] files with uncovered lines=30 uncovered lines=1115
  mentioned in module-tauri-remote.md: 1115
  MISSING (uncovered, not mentioned): 0
  STALE (doc cites it as uncovered; old doc sections also match unless --section is given): 283
    COVERED (DA>0 - a real disagreement, the actionable half): 0
    ABSENT (no `DA:` record -- NOT evidence): 283
```

`staletable.py --gate`, the `tauri-remote` row, pasted verbatim (the tool gained a `MISSING` column
between D22 and this run, so the row has seven numbers where §12.5's had six):

```
pairing                    role             doc                          claims  uncov MISSING  STALE ABSENT COVERED named         verdict
tauri-remote               authoritative    module-tauri-remote.md         1358   1115       0    283    283       0 section       PASS
```

The whole gate is green this time — `authoritative pairings: 15 -> clean 12, justified covered rows 3,
unverifiable 0, failing 0`, exit 0 — which is *other units'* work landing between D22 and now; this
record does not claim it.

### 13.6 Rejected approaches

- **Re-labelling the 34 as `unreachable-in-this-environment`.** A test file in the same repository is
  not a host limitation; the waiver policy's discriminator is "reachable here with a better fixture",
  and for these the fixture is better and merely out of scope. Re-labelling would be the exact
  failure the campaign keeps catching.
- **Adding a production seam so a lib test can drive them.** §6.2's ruling refused that, and the
  `#[cfg(test)]` one-shot D22 added is not a substitute here: these bodies do not exist in the test
  build, so no test-side injection can call one.
- **A plain `#[cfg(not(test))]` duplicate of the worker bodies compiled for tests.** That is a second
  implementation whose coverage says nothing about the shipped one.
- **Claiming `publisher.rs` 646/650 as closed.** They are `DA>0` in this run and `DA:0` in the last
  one, with no code change. §4.1 records the pair and both readings instead of picking the flattering
  one.
- **Editing `tests/remote_start_refusal.rs` anyway.** Out of scope by declaration; the previous six
  segments' precedent is that the gate, not the edit, is the move.

## 14. TAU-D26: the four `cfg(not(test))` gates closed to 30 of 34 lines, and the production defect the probe exposed (2026-10-18)

**Revision: `0b7b2ca17`.** Report **`.future/cov100/pinned/union-d26.info`** — 733709 bytes, md5
`42329a2b17c6bd438247df0f3d908861`, union of the two per-flavour legs over the three scopes
(`remote`, `remote_host`, `remote_peer`). **1083 uncovered, 0 OPEN, MISSING 0, COVERED 0.** The four
lines that remain are two race windows, §14.4.

### 14.1 What landed: eight children, four gates, one production fix

D23 filed 34 OPEN lines behind four `#[cfg(not(test))]` gates and was blocked on the write set
(`desktop/src-tauri/tests/`), not on the code. D26 was granted that path and closed 30 of the 34.

The gates at HEAD, re-read for this section (D23 wrote 420/488/523/584; every line below 55 moved by
**+2** when §14.1's guard was added — net two lines: two comments and one longer condition):

| gate | line | what it guards | test twin |
|---|---|---|---|
| `spawn_start_retry` | **422** | the worker body, `423-473` | `#[cfg(test)] return;` at `419-420` |
| `spawn_runtime_reconnect` | **490** | the worker body, `491-515` | `#[cfg(test)] { let _ = pair_id; }` at `485-488` |
| `restart_slot` | **525** | the whole function, `526-557` | none — the function does not exist in a test build |
| `GenerationWatch` | **580**, impl **586** | `run`, `588-639` | `spawn_runtime_supervisor`'s stub at `643-646` |

**Why a child and not a test.** In the lib test binary these four bodies are replaced (or absent), so
no `#[test]` in the crate can reach them — a compilation fact, not a difficulty. The only build that
contains them is the **non-test** copy of `futureos_lib` that the binaries link. Every child spawns
`env!("CARGO_BIN_EXE_futureos-headless")` (`tests/remote_start_refusal.rs:406`, `:522`) — the shipped
entry point — and asserts the **arm's effect**: another request on the mock platform, a forgotten book
entry, a socket on the rebuilt endpoint, a classified `network` episode. None of them asserts that a
process exited.

**Eight children, not six.** The file's own header comment says "Six children" and then numbers
**seven**, and the commit message repeats six; the diff adds **eight** `#[test]` functions (the target
has 12, four of which predate D26). The eighth, `a_reconnect_whose_rebuild_cannot_connect_is_retryable`
(a rebuilt slot whose endpoint refuses IO → `start_failure` classifies `network` → the retryable arm at
`:506` instead of the `Err` arm), is the one the comment omits. Recorded because a count in a comment
is exactly the class of claim this campaign has had to correct, and because the measured line set — not
the comment — is what §14.2 certifies.

| child | drives | lines it accounts for |
|---|---|---|
| `a_retry_that_cannot_be_classified_ends_the_worker_and_rearms_it` | retry worker: terminal `Ok(_) | Err(_)` result, then the exit tail | `457`, `460-463`, `467-468` |
| `a_freshly_refreshed_credential_rejected_on_a_live_link_is_terminal` | `GenerationWatch`'s terminal-health return (`service_config_error` from a *valid* credential) | `606` |
| `a_reconnect_for_a_device_the_book_no_longer_holds_stops_that_runtime` | `restart_slot`: the device is gone from the book | `532-533`, and the wrapper's `Ok(_)` arm `507` |
| `a_reconnect_with_a_pre_v2_slot_forgets_it` | `restart_slot`: a pre-v2 slot (no signed identity) | `536-537` |
| `a_revoked_answer_is_forgotten_whichever_ask_receives_it` | a revoked refresh is forgotten: `restart_slot` when its rebuild asks, the device's own renewal worker otherwise | `542-543` when the rebuild asks (the worker absorbs the other ordering) |
| `a_reconnect_refreshes_the_credential_and_rebuilds_the_slot` | `restart_slot`: refresh succeeds, plan built, slot restarted | `540`, `547-555` |
| `the_reconnect_budget_exhausts_into_a_terminal_watch` | the crash-loop budget: three devices spend it, the fourth reads `>= MAX` and exits terminally | `632-634` |
| `a_reconnect_whose_rebuild_cannot_connect_is_retryable` | `restart_slot`'s `start_slot` `Err` → classified and handed to the retry worker | `506` — the retryable arm, 9 runs across the reconnect children; this child is the only one whose *design* is that arm |

**The production fix, and why it was in scope.** `start_generation`'s no-replace guard read
`runtime_active(&status())`. A **start retry** runs with nothing installed, and `status()` reports the
retry itself as `Reconnecting` (because `start_retry_running` is set and `last_error_code` is `server`),
which `runtime_active` counts as a live runtime (`types.rs:140-148`) — so the guard returned early and
**the documented startup retry never asked the platform**. The guard now also requires a non-empty
runtime map (`start.rs:52-56`):

```rust
if runtime_active(&current) && !SUPERVISOR.state.lock().unwrap().is_empty() {
    return Ok(current);
}
```

This is a behaviour change in shipped code, made because the probe child *measured* it: before the fix
`a_retry_that_cannot_be_classified_ends_the_worker_and_rearms_it` saw exactly one platform request and
then silence. After it, the retry's own attempt is request two. The fix is also why two lines D23 had
**waived** now read `DA>0` — `425` (the singleton guard's early return, 2 runs) and `448` (the loop's
`!start_requested || phase == Ready` break, 1 run). Both lie on the retry worker's path: without the
fix the worker never got past its first attempt, and the paper trail cannot separate the fix from the
children's scheduling — what is measured is `0 → 2` and `0 → 1`, and §4.4 counts them as the two
extra lines.

**One comment the measurement falsified.** Child 1's doc comment says the tail "re-arms the worker,
whose own attempt is the third platform request". Its **body** asserts the opposite — exactly two
requests, then four seconds of silence, because the terminal result ended the worker — and the report
agrees with the body: `469-470` read `0`. The reason the re-arm cannot fire on this path is
deterministic, not a race: every attempt writes `last_error_code = "connecting"` before it asks
anything (`start.rs:59`), a *coded* failure then overwrites it (`:726`), and the last attempt here fails
**uncoded** (`Malformed`), so the tail re-reads `connecting` → phase `Connecting`, reason `None` →
`retryable_start_status` is false (`types.rs:131-138`). The comment is corrected here rather than left
to describe an effect that did not happen; see §14.4 for what *would* satisfy the condition.

### 14.2 The 30 closed, and the four that remain

D23's 34 OPEN lines, mapped to their post-fix numbers (`+2`) and to what the report says now:

| gate | D23 lines | now | closed | remains |
|---|---|---|---|---|
| `spawn_start_retry` | 455; 458-461; 465-470 | 457; 460-463; 467-472 | 9 | **469-470** |
| `spawn_runtime_reconnect` | 495-496; 505 | 497-498; 507 | 1 | **497-498** |
| `restart_slot` | 530-531; 534-535; 538; 540-541; 545-553 | 532-533; 536-537; 540; 542-543; 547-555 | 16 | — |
| `GenerationWatch` | 604; 630-632 | 606; 632-634 | 4 | — |
| | **34** | | **30** | **4** |

Per-line `DA` in `union-d26.info`, the whole set (read from the pin, not from a log):

```
457 1   460 2   461 2   462 2   463 2   467 2   468 2   469 0   470 0   471 2   472 2
507 4   497 0   498 0
532 1   533 1   536 1   537 1   540 2   542 1   543 1   547..555 all >0 (9 lines)
506 9   606 1   632 1   633 1   634 1
```

**The arithmetic, and it is exact.** `supervisor/start.rs` 81 → 49 uncovered: **−32** = the 30 OPEN
lines above plus the two previously-waived lines the fix reached (`425`, `448`). No other file's count
changed in this render, so the scope total is `1115 − 32 = 1083`. The ABSENT rows (277 in this reading,
against D23's 283 — they are the same two harness files' non-code lines, and the delta is this
segment's re-pointed row set) are **not** evidence of anything and stay out of the composition, and
`COVERED` is 0: **no row in this ledger disagrees with the pin in either direction.**

**The two reclassifications this section implies, both already applied to §4.1's rows.** The
five-line concurrency row lost `:423` (now `425`, covered) and the two-line watch row lost `:446` (now
`448`, covered); the row that carried `:604` became `:606` and the budget row became `:632-634`. The
four remaining lines moved from OPEN to a named class — `unreachable-in-this-environment`, with the
race stated in §14.4 — **after** the children proved that every other input reaches its arm. That
order matters: they were not waived first.

### 14.3 The measurement

| leg | result | files | exit |
|---|---|---|---|
| gui (`-j 4`, `-- --test-threads=1`) | **2214 passed / 0 failed / 14 ignored**, plus the 12 integration tests | 171 | **rc=0** (report rc=0) |
| headless (`--no-default-features --features headless`) | **1769 passed / 0 failed / 14 ignored**, plus the same 12 | 139 | **rc=0** (report rc=0) |
| union | `lcov-union.py`, then `leg-flavour-check.py` → **LEG IDENTITY: ok** | 173 | rc=0 |

The lib counts are unchanged from D25 (`2214` / `1769`) because D26 added **no** lib test — its eight
children live in `tests/remote_start_refusal.rs`, which is why each leg carries the same twelve
integration tests on top. `leg-flavour-check` is **fully green for this pair** — 171/139 files, the two
gui-only sources excluded from the headless leg, `no_gui.rs` and `headless/mod.rs` present, and
`src/bin/futureos-headless.rs` present — because these legs run every target. (D25's appdata pair, by
contrast, is two `--lib` runs and FAILs that one marker; the difference is the leg shape, not the
crate.)

**The headless leg is a second run of the same tree.** Its first attempt reddened
`remote_peer::tests::a_host_event_reaches_a_subscriber_decrypted` ("the host's event never arrived") —
a timing flake in a subtree this change does not touch. The leg was run again on `0b7b2ca17` and that
green run is the one quoted, rather than publishing either a red run or a single re-render of a red
run's profraw. Two notes a reader needs: a re-run cannot hide a covered line (a line reads `0` only if
**no** retained attempt executed it), and the gui half of the union is a **single** render —
`gui-first.info` is a byte-identical copy of `gui.info` (both md5 `2373b903a3e8cc16cb2628e7482efb3d`),
so the headless re-run kept the gui report instead of re-rendering it.

**What is retained, and what re-derives from it.** An earlier revision of this section stated that the
per-leg reports and the run's script were not retained. Both statements were wrong, and the correction
matters because it decides what a reader may re-check instead of take on trust:

| artifact | path | size | what it re-derives |
|---|---|---|---|
| gui leg report | `.future/cov100/d26/gui.info` | 3496779 B | **171** `SF:` / 74723 `DA:` |
| headless leg report | `.future/cov100/d26/headless.info` | 2754606 B | **139** `SF:` / 61124 `DA:` |
| gui copy kept for the re-run | `.future/cov100/d26/gui-first.info` | 3496779 B | identical to `gui.info` (same md5) |
| the full run | `.future/cov100/measure-d26.sh` | — | gui leg, `clean`, headless leg, union, flavour check |
| the headless re-run | `.future/cov100/measure-d26-headless.sh` | — | headless leg only, gui report kept |

Re-checked after this section was first written: `lcov-union.py` over those two leg reports writes a
file **byte-identical to the pin** (md5 `42329a2b17c6bd438247df0f3d908861` for both), and
`leg-flavour-check.py` prints **LEG IDENTITY: ok** over 171/139 (`FLAVOUR_EXIT=0`). So the file counts,
the 173-file union, the `DA` totals (74801 records, 1805 zeros, 1083 in the three scopes) and every
per-line value in §14.2 are re-derivable from retained artifacts rather than quoted. What no retained
artifact shows is the legs' **pass/fail counts and `*_EXIT=` lines**: that transcript did not survive,
so those cells are the run's recorded output, not re-verified here — the pin can be recomputed but not
re-interpreted into a test result.

**The commands, as the retained script runs them** (`measure-d26.sh`; `$W` is this worktree):

```bash
cd $W/desktop/src-tauri
export HOME=$W/target/test-home CARGO_TARGET_DIR=$W/target/cov-tauri-d26
make -C $W desktop-sidecar-placeholder
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 -- --test-threads=1
cargo llvm-cov report --lcov --output-path $W/.future/cov100/d26/gui.info
cargo llvm-cov clean
cargo llvm-cov --no-report --no-fail-fast -j 4 --no-default-features --features headless \
    -- --test-threads=1
cargo llvm-cov report --lcov --output-path $W/.future/cov100/d26/headless.info
python3 $W/.future/cov100/lcov-union.py --out $W/.future/cov100/pinned/union-d26.info \
    $W/.future/cov100/d26/gui.info $W/.future/cov100/d26/headless.info
python3 $W/.future/cov100/leg-flavour-check.py \
    $W/.future/cov100/d26/gui.info $W/.future/cov100/d26/headless.info
```

Two things differ from §11.2's form and are deliberate: the target directory is `cov-tauri-d26` (not
`cov-tauri-remote`), and the reports land **in-repo** under `.future/cov100/d26/` rather than `/tmp`
(F.100) — which is why the tables above can name them. The previous revision's block wrote
`$W/target/d26/`, a path that matched neither the script nor the disk; it is corrected here rather than
left as a command a reader cannot run.

Three properties of that sequence are load-bearing:

1. **No `--lib`.** The legs run every target, which is what makes the children execute *inside* the
   instrumented run; a `--lib` leg would leave all 30 gated lines uncovered and could not have produced
   this figure. That the gated bodies read `DA>0` is itself the proof the children ran: those bodies
   exist in no test build, so only the spawned `futureos-headless` (linking the non-test lib) can
   execute them.
2. **`clean` between the legs**, so neither leg inherits the other's profraw (§11.2, and what
   `leg-flavour-check.py` exists to catch).
3. **Per-leg `report --lcov`**, then the union: `DA` max per line, which is the correct metric for
   "uncovered iff no flavour executed it".

What the pin does and does not contain, measured: **all 173 `SF:` records are under
`desktop/src-tauri/src/`** — the integration target's own file carries no record, so it is outside the
metric by construction, exactly as it is in every pin this record has quoted. The union carries 74801
`DA:` records, 1805 of them zero; 1083 of those zeros fall in the three scopes.

### 14.4 The four race windows, and why no child can schedule them

Both windows are `unreachable-in-this-environment`, not `unreachable-by-construction`: the arm *can*
execute on a host where a second writer interleaves, and what is missing is a seam for a test to be
that writer. Each names the writer, the window, and why the child that comes closest cannot close it.

**`start.rs:469-470` — the retry worker's re-arm** (`if start_requested && retryable_start_status(&latest) { spawn_start_retry(); }`, `:468-471`).

- *The race*: the worker has already cleared `start_retry_running` (`:461-463`) and is about to die.
  For the body to run, the status read at `:467` must be retryable — i.e. phase `Reconnecting` **and**
  reason `network`/`server` — while `start_requested` is still true. Both are only reachable if another
  actor records a *coded* retryable failure (or stops and starts again, which re-writes
  `start_requested` and `last_error_code`) **between the singleton clear and the `status()` call**.
- *Why it cannot be scheduled here*: on the single-writer path the condition is deterministically
  **false**, and §14.1 gives the reason — the last attempt writes `"connecting"` at `:59` and its
  uncoded failure leaves it there, so the tail re-reads `Connecting` with `reason: None`, which
  `retryable_start_status` rejects. The window between `:463` and `:467` is a few synchronous
  statements: no `.await`, no yield, and the only writers (`start()` via the CLI/IPC entry, `stop()`)
  run on other tasks. A child can drive them, but not land inside that block — and the child that
  comes closest (child 1) asserts the **opposite** outcome, that no third request arrives, which is
  precisely why `469-470` read `0`.
- *The enumerated missing seam*: a test-only way to insert a writer between `:463` and `:467` — e.g. a
  `#[cfg(test)]` hook the tail calls before re-reading the status. §6.2 refused production seams for
  reachability, and this one would be in shipped code (`spawn_start_retry`'s non-test body), so it is
  named and not built.

**`start.rs:497-498` — the budget guard** (`if attempt > MAX_RUNTIME_RECONNECT_ATTEMPTS { last_error_code = "reconnect_required"; return; }`, `:496-499`).

- *The race*: `attempt` is produced by `record_runtime_failure` (`:559-575`), whose **only** caller is
  `spawn_runtime_reconnect` itself (`:492`), and every caller first reads the counter at `:627-630`
  behind `>= MAX_RUNTIME_RECONNECT_ATTEMPTS` (`= 3`, `state.rs:447`) and **returns** instead of
  calling. So a call can only see `attempt == 4` if two watchers both pass the `>= 3` check and both
  then record — the load at `:627` and the increment at `:572` are not atomic together.
- *Why it cannot be scheduled here*: the window is the few hundred nanoseconds between one watcher's
  `load` and the other's `fetch_add`. Child 7 (`the_reconnect_budget_exhausts_into_a_terminal_watch`)
  puts three devices on a broker that refuses them ~600 ms into each generation and the controlling one
  on a second broker that fails 5.5 s later, so the budget is spent *sequentially* and the last watcher
  takes the terminal exit at `:632-634` — which that child does cover and assert
  (`GenerationUnhealthy` in the entry's status). A child cannot pre-arm the counter either:
  `runtime_reconnect_attempts` and `record_runtime_failure` are `pub(in crate::remote)`, so an
  integration test cannot call the recorder directly, and the counter is process-global, so no
  per-device reset exists to exploit.
- *What the arm is for*: a defensive duplicate of the watch's own `>= MAX` check (same effect, same
  error code). Its **behaviour** is covered through `:632-634`; only this second copy of the check is
  not. Recording that distinction is why the row is honest rather than convenient: nothing about the
  user-visible contract is untested, and the alternative — a production seam to make the duplicate
  fire — would buy one line with a shipped hook.

### 14.5 The gate readback, and what is left

`reconcile.py` over the three scopes against `union-d26.info` (verbatim head):

```
[tauri-remote] files with uncovered lines=30 uncovered lines=1083
  mentioned in module-tauri-remote.md: 1083
  MISSING (uncovered, not mentioned): 0
    COVERED (DA>0 - a real disagreement, the actionable half): 0
    ABSENT (no `DA:` record -- NOT evidence): 277
```

```
pairing                    role             doc                          claims  uncov MISSING  STALE ABSENT COVERED named         verdict
tauri-remote               authoritative    module-tauri-remote.md         1320   1083       0    277    277       0 section       PASS
```

**1083 = 1083 waived, 0 OPEN**, over 30 files with uncovered lines. Excluding the two harness files
(`real_network.rs` 788 and the `verify_e2e.rs` `InstalledHost` block 68) the residual is **227**, of
which four are §14.4's race windows, 36 are `spawn_web_reconnect`'s staging-only body
(`start.rs:669-710`), 5 are the concurrent-writer row (`76, 111, 282, 294, 401`), 2 are the watch's
teardown races (`595, 603`), and the rest are the per-file rows in §4.1.

**What D26 did not do.** It did not touch the two harness files, the staging-only web reconnect body, or
any row outside `supervisor/start.rs` — the file-level arithmetic in §14.2 is the evidence: this render
moves exactly one file. It did not add a production seam (§6.2). It did not re-label the four remaining
lines as `unreachable-by-construction`: §14.4 shows a writer that *could* satisfy each, which is a
missing seam, not an impossible state.


> **THE CONSOLIDATION MEASUREMENT IS THIS SECTION (TAU-D31).** The blockquote that stood here said the
> figure was a reading of `16ea6f388`, not of `main`, and that a consolidation measurement was owed. It
> has been run: `.future/cov100/pinned/union-refresh5.info`, **measured at `5ad40491f`** — `origin/main`'s
> tip, which contains #1192's peer-session lock fix and #1193's dwell fixture. Both flavours were
> measured in a **detached worktree at that revision** (`/tmp/cov-d31`, `git rev-parse HEAD` =
> `5ad40491f0878a589529871f86d5637838c60c9a`), one `cargo llvm-cov clean` before each leg.
>
> The measured tree is `origin/main`'s, not the branch tip's, **and that is deliberate**: F.108 judges
> freshness with a tree diff `measured_at..origin/main`, and `main` lacks TAU-F6's `a9c383e45`
> (`remote_peer/runtime.rs` +58, `remote_peer/session.rs` +71 — the only two scope files that differ
> between the branch and `main`). Measuring the branch would keep the module `stale` forever, because its
> extra code is *ahead of* main rather than behind it. TAU-F6's addition is a test plus the
> `exchange_within` split that lets it drive the leg with a custom budget; it is covered when the branch
> runs and absent here, so it contributes no uncovered line to either figure. When F6 lands, the rows
> that move are the ones below `session.rs` line 1032 and `runtime.rs` line 2272 (i.e. only the `mod
> tests` brace at `:2811`, which already carries a `was` note in 15.3).

## 15. The post-merge re-measurement: what the 994 new lines are, and the two harness envelopes

**Measured at `5ad40491f`** (report `.future/cov100/pinned/union-refresh5.info`; legs reported
separately — gui `2246 passed / 3 failed / 14 ignored` in 590.80 s, headless `1800 passed / 0 failed /
14 ignored` in 457.96 s and `rc=0`, `clean` between them).

**The three gui failures are the host's port-8022 family, and this is measured, not asserted.** All three
are `remote_peer::runtime_tests` panicking in `testing.rs:30` with `the test web port never freed`
after `web server bind on port 8022 failed: Address already in use` — a concurrent measurement in the
other worktree held the port. Re-run alone once the port was free, the same three **pass** (`3 passed;
0 failed`, 6.35 s) and that run is folded into the gui report the union is built from. So the leg's own
`rc` is **101**, and it is environmental: every in-scope test passes in this measurement, and the union
is the union of all the profraw the two legs produced, including the folded run.

**Why there are two gui runs.** The first leg's target directory sat under `/tmp`, which on macOS is a
symlink, and tauri's updater refuses `current_exe()` when it traverses one (`StartingBinary found
current_exe() that contains a symlink on a non-allowed platform: /tmp`). That cost five
`commands::update` tests — outside these scopes — and nothing else. Re-running the leg with its target
dir on `/private/tmp` fixed exactly those five and produced the leg the union uses; the port-8022
trio above is what the second run hit instead, from a concurrent measurement, and its own re-run is
folded in.

`origin/main` was merged at `52d80801f`; #1179/#1184/#1188 add **994 lines** inside these scopes, and
#1192/#1193 moved the `remote_peer` ones.

    uncovered in scope: 1504 across 31 files
      remote/real_network.rs      788    <- harness envelope (see 15.1)
      remote/verify_e2e.rs        433    <- harness envelope + InstalledHost fixture (see 15.2)
      everything else             283    <- production lines, registered per line in 15.3

### 15.1 `remote/real_network.rs`, lines 50 to 1065 — the real-network harness, 788 lines

One envelope row, not 788 rows. Every line in the file is the harness that drives a **real
`nats-server` and a real agent process** (`REAL_NET`, `real-network.md`): the fixtures are `#[ignore]`d
because nothing in a normal `cargo test` run can start a broker, and the file's own module
documentation says so. A per-line ledger for it would be 788 rows all carrying the same sentence,
which is exactly the shape F.94 ruled out.

* class: `unreachable-in-this-environment`
* mechanism: `#[ignore]`d real-broker harness. The ignored drivers DO run - `real-network.md` records
  six of them executing against a live broker - but only under `cargo test -- --ignored` with a
  broker on `NATS_URL`, which no measurement run does.
* the honest consequence: this file contributes 788 uncovered lines to every figure in this record
  and always will, until the metric changes to "covered or registered" with the envelope as one
  registered object.

### 15.2 `remote/verify_e2e.rs` 350-450, 2642-2688, … — the cross-process `InstalledHost` fixture, 433 lines

The second envelope, same class and a different mechanism: these are the **child-process** fixtures
(`InstalledHost`) that spawn the real binary to drive the e2e path end to end. They are `#[ignore]`d
for the same reason as 15.1 - a spawn needs a broker - but they exist because the arm they reach has
no in-process route.

* class: `unreachable-in-this-environment`
* mechanism: `#[ignore]`d cross-process fixture; needs a spawned binary plus a broker.
* the biggest contiguous run is **419-450 (32 lines)**, the child's own harness body.

### 15.3 The 283 production lines, registered per line

Every range below is generated from `union-refresh5.info` (**measured at `5ad40491f`**) with this
pairing's own three scopes: a line is in a range iff its `DA:0`, so the claim and the report cannot
drift apart without the gate saying so. The rows claim all 283 production lines; 52 of them restate a
§4.1 row that already sat at the same number (`start.rs` 49, `commands.rs` 332/367, `lean.rs` 522), so
this section stands alone instead of depending on a row 1,600 lines above it.

**The delta from the previous pin, re-derived line by line.** `16ea6f388` → `5ad40491f` is #1192 (the
peer-session lock fix) plus #1193 (a dwell in an integration fixture, outside these scopes). The
union moves by **four lines and twelve relocations**, and none of the twelve relocated rows changes class:

| file | refresh4 (`16ea6f388`) | refresh5 (`5ad40491f`) | what moved |
|---|---|---|---|
| `remote_peer/runtime.rs` | 152-154, 1659, 1723, 1736, 1761-1763, 2782 (10) | 163-165, 1707, 1771, 1784, 1809-1811, 2194, 2811 (11) | the same six constructs at +11 (the `delay_unit` body), +48 (the chunk guards and `page_shape_error`) and +29 (the `mod tests` brace); **one new line**, `:2194`, in #1192's own test; the old `:1723` (`));` of the chunk-fetch loop) carries no `DA:` record now and is not claimed |
| `remote_peer/session.rs` | 767, 773 | 775, 781 | the same two constructs at +8: #1192's `Arc`/`request(&self)` prose sits above them |
| `remote/supervisor/start.rs` | 49 rows | 50 rows | `:554` reads `DA:0` again (see the oscillation table) |
| `remote/publisher.rs` | 347, 554, 634-641 (10) | + 646, 650 (12) | the two oscillators this record has carried since TAU-D11, now `DA:0` (see below) |

**The oscillation, recorded with both readings rather than explained away.** Three lines read `DA>0` in
`union-refresh4` and `DA:0` here although **their source is byte-identical between the two pins**
(`git diff 16ea6f388 5ad40491f -- start.rs publisher.rs remote_host/` prints nothing):

| line | construct | refresh4 | refresh5 | what the counts show |
|---|---|---|---|---|
| `remote/supervisor/start.rs` 554 | `Ok(()) => Ok(status_focused(Some(pair_id)))`, the success arm of `restart_slot` | 2 | 0 | the neighbouring arms moved in both runs (`:552` 6→2, `:555` 3→1), so the restart path ran and returned a failure this time |
| `remote/publisher.rs` 646 | the closing brace of the activity-publish block | 1 | 0 | the block itself ran (`:649` 19086→4764) |
| `remote/publisher.rs` 650 | `continue` on a `None` workspaces read | 1 | 0 | the `None` read happened once in refresh4 and not at all here |

The hit-count collapse on `publisher.rs:649` (19086 → 4764) is the tell: the two runs drove this tick a
very different number of times (leg composition, `--test-threads=1` ordering), and these three lines sit
at the margin. They are registered as `OPEN`, each with the sibling reading named, because a mechanism
sentence for a line that executes in the other run would be false.

**The false clean the D29 pass found (history; still true of the tool).** `reconcile.py` reported `MISSING 133` against this
section, and the 133 were *not* "279 minus the rows". Of the 114 lines whose record was outside the
section (19 moved + 95 never registered), **94 counted as "mentioned" by accident**: `cited_numbers()`
reads the LCOV-style range in §15.1's heading (`50-1065`, meant as that one file's span) as a claim
about **every** number 50–1065 in **every** file, and §15.3's old per-file table named
`test_support.rs`, `sync_measurement.rs` and `commands.rs`. File named + line number mentioned ⇒
"mentioned". So the gate was blind to the 40-line measurement server, the 42-line reconnect fixture
and the ten reply/standby lines even though no ledger row described them. The heading now reads
"lines 50 to 1065" and the rows exist here, so the MISSING count now means what it says.

**Registration is not a waiver.** `OPEN` below means the line *can* run and the fixture that would
reach it is named; the class vocabulary is the one §4.1 uses (`unreachable-by-construction`,
`unreachable-in-this-environment` with the obstacle named, `platform-unmeasured` with the platform
named, `attribution-artifact` with the region named). Where a construct is unchanged from §4.1 the
mechanism is restated here rather than only cross-referenced, so a reader of this section does not
have to hold two revisions at once.

#### `remote_host/`

| file and lines | class | mechanism |
|---|---|---|
| `src/remote_host/business/catalog.rs` 104-106 | `unreachable-by-construction` (3 lines) | "Fork produced no session." `agent_bridge::fork_agent_session` refuses an empty id (`agent_bridge/session.rs:285-287`, "Fork did not return a session.") and creates the child row with `agent_session_id: Some(new_session_id)` (`:341-352`), so the row `get_thread` returns one statement later carries a non-empty id. |
| `src/remote_host/business/catalog.rs` 122-128, 130 | `unreachable-in-this-environment` (8 lines) | "Forked thread could not be loaded." and the read error beside it. The row read at `:100` was written by the call at `:97`; `Ok(None)` needs a concurrent deleter between two statements of one handler and `Err` a store read failure inside the writable test `HOME`. No seam fails `store::get_thread`. |
| `src/remote_host/files.rs` 367 | `unreachable-by-construction` (1 line) | `images > MAX_IMAGES`. `MAX_ATTACHMENTS == MAX_IMAGES == 10` (`files.rs:26`, `:30`) and `:335` already refuses `references.len() > 10`, so a list that reaches `:366` holds at most ten images. Kept by decision §6.1: it is the only enforcement of `MAX_IMAGES`. |
| `src/remote_host/files.rs` 1145, 1183 | `unreachable-in-this-environment` (2 lines) | `:1145` the `std::fs::copy` fallback when `hard_link` fails; `:1183` the `remove_file` inside the rename-failure arm. Both need the filesystem to refuse `link()`/`rename()` for two paths on the same writable volume, which this host never does. |
| `src/remote_host/business/history.rs` 267 | `attribution-artifact` (1 line) | the closing brace of `if let Some(events) = data["events"].as_array_mut()` (`:263-267`). The body runs 44× in the union (`DA:44` at `:264-266`); the zero region the brace carries is the if-let's implicit else — a page reply with no `events` array, which the agent never sends and no fixture builds. `was 273`. |
| `src/remote_host/lean.rs` 522 | `unreachable-by-construction` (1 line) | `let Some(fields) = block.as_object_mut() else { continue }` inside the `"tool_call"` arm: taking that arm requires `block["kind"] == "tool_call"`, i.e. an object with a string `kind`, so `as_object_mut()` is always `Some`. |
| `src/remote_host/lean.rs` 1243 | `unreachable-by-construction` (1 line) | the fixture validator's `Some(false) => panic!`: the render fixtures are generated by the shipping trim from full-feed captures, and #849 stopped emitting `isError: false` for a result, so no fixture the repository can produce takes the arm. |
| `src/remote_host/lean.rs` 1267-1270 | `unreachable-in-this-environment` (4 lines) | `generate_render_fixtures`, `#[ignore = "fixture generation: needs LEAN_RENDER_FIXTURES"]`: the gate reads the env var and derives the files from real full-feed captures (`scripts/screenshots/gen-lean-render-fixtures.mjs`). `derive_render_fixtures` itself runs in the ordinary suite (§3.2). |
| `src/remote_host/lean.rs` 1375 | `attribution-artifact` (1 line) | the closing brace of `if !kept.is_empty()` (`:1361-1375`) in the render-fixture assertions: the body runs 4× and the brace carries the else — a `tool_end`/`tool_result` payload that keeps neither `text` nor `result`. No fixture has one. |
| `src/remote_host/lean.rs` 1435-1439, 1444-1447, 1449-1450 | `unreachable-in-this-environment` (11 lines) | `measure_real_entries`, `#[ignore = "measurement: needs LEAN_HISTORY_ENTRIES"]`: the env gates (`LEAN_HISTORY_ENTRIES`, `LEAN_HISTORY_OUT`, from `scripts/measure/measure-lean-history.py`) and the report print. `measure_entries` is covered by §3.2's fixture; the gate is what a measurement run does not execute. |
| `src/remote_host/lean.rs` 1474 | `attribution-artifact` (1 line) | `measure_entries`' closing brace of `if let Some(list) = entries.as_array()` (`:1466-1474`): the tally runs 4×, and the brace carries the else — entries that are not an array, which no fixture feeds to the measurement. |
| `src/remote_host/pairing.rs` 292-293 | `unreachable-in-this-environment` (2 lines) | `remove_slot`'s `remove_file` error arm: runs only when unlinking an existing credential file fails, and the test `HOME` is a writable temp directory. |
| `src/remote_host/pairing.rs` 345 | `unreachable-in-this-environment` (1 line) | the `?` on `PairingIdentity::new(code_expires_at)` in `create_pairing`: its only fallible step is local key generation (`future_remote_crypto::generate_identity()`), i.e. an OS-RNG fault — nothing the platform supplies. A fixture would need fault injection on the crypto crate. `was 146` before the pairing book grew the file. |
| `src/remote_host/business/prompt.rs` 145-151 | `unreachable-in-this-environment` (7 lines) | "Future Agent did not acknowledge the prompt.": `accepted_rx` is dropped without a send only if the spawned `run_prepared_prompt_with_acceptance` future dies before its receipt — a panic, or an abort of a task whose `JoinHandle` nobody retains. No caller can abort it and the green suite produces no panic. |
| `src/remote_host/business/providers.rs` 58-64 | `unreachable-by-construction` (7 lines) | `reply_view`'s `Err(_)` for `serde_json::to_value(ProvidersView)`: two `Vec`s of structs of `String`/`bool`/`usize`/`i32`, every one a derived `Serialize`, no map keys, no hand-written impl; `to_value` cannot fail. Kept by §6.1. |
| `src/remote_host/business/settings.rs` 148-150, 176-180 | `platform-unmeasured` (8 lines) | `product_sandbox_available()` is `Ok(true)` unconditionally on **macOS** (`settings.rs:300-304`), so the probe's `Err` arm and the two `Ok(false)` tier arms cannot be reached by this measurement; the Windows/Linux flavour measures them. |
| `src/remote_host/business/transfers.rs` 59 | `unreachable-in-this-environment` (1 line) | the `JoinError` arm of the `spawn_blocking(session_files::list)` join: `list` must panic or its runtime be shut down. No seam inside `list`. |
| `src/remote_host/business/wire_limits.rs` 717-722 | `unreachable-in-this-environment` (6 lines) | `measure_phone_page`, `#[ignore = "measurement: needs VERIFY_E2E_PHONE_PAGE"]`: the env gate and the file read (`scripts/measure/measure-lean-history.py --phone-page`); `measure_page` is covered by §3.2's page fixture. |
| `src/remote_host/read_pages.rs` 70 | `unreachable-by-construction` (1 line) | the eviction loop's `else { break }`: `insert` refuses `bytes.len() > MAX_SNAPSHOT_BYTES` (16 MiB, `:50`) and `MAX_CACHE_BYTES` is 32 MiB, so the loop is only entered with a non-empty map and `min_by_key` always yields `Some`. Kept by §6.1. |
| `src/remote_host/session_files.rs` 74 | `unreachable-in-this-environment` (1 line) | `target.metadata()` failing after `canonicalize` succeeded: the entry must be removed inside that window. The dangling symlink (`:68`) and the FIFO (`:77`) are covered by the `#[cfg(unix)]` test; the TOCTOU window is not schedulable. |
| `src/remote_host/sync_measurement.rs` 822-861 | `unreachable-in-this-environment` (40 lines) | `serve_real_snapshot`, `#[ignore = "requires isolated DB snapshot; run scripts/measure/measure-sync-browser.py"]`: the server binds a port and serves real pages, gated on `FUTURE_AGENT_GRPC_ADDR`, an isolated `HOME` marked `MEASUREMENT_ISOLATED_HOME`, `SYNC_MEASURE_SAMPLES`/`SYNC_MEASURE_WEB_ROOT`/`SYNC_MEASURE_WEB_PORT`, and a live agent. Same class and obstacle as §15.1's real-broker harness; the file's in-process gzip and env-override paths are covered by the ordinary suite. **Never registered before this pass.** |

#### `remote/`

| file and lines | class | mechanism |
|---|---|---|
| `src/remote/lifecycle.rs` 40 | `unreachable-by-construction` (1 line) | `changed.changed() == Err` needs the `watch::Sender` dropped while the receiver is borrowed; the sender is a field of the same `AccessEpoch` the borrow keeps alive, so the type makes it unproducible. |
| `src/remote/presence.rs` 78-81 | **`OPEN`** (4 lines) | `holder_payload`'s `Some(identity)` arm — the controller's declared name and kind, recorded by `record_client_identity` from its `secure_ready` (`commands.rs:106-111`). Fixture: a standby device's presence packet after the controller has declared an identity. The suite's controller never declares one, so the match takes the neutral `None` arm. |
| `src/remote/presence.rs` 134 | `unreachable-by-construction` (1 line) | `publish_control`'s `let Ok(bytes) = serde_json::to_vec(&payload) else { return }`: `payload` is a `json!` object with string keys and scalar values; `to_vec` cannot fail. `was 123`. |
| `src/remote/publisher.rs` 347 | `unreachable-in-this-environment` (1 line) | `publish_batch` failing while flushing on shutdown (`Some(None)`, the subscriber closed the channel): the fake broker's publish does not fail, and there is no injectable failing publish. |
| `src/remote/publisher.rs` 554 | `unreachable-in-this-environment` (1 line) | the `JoinError` of the tick's `spawn_blocking` snapshot: the closure must panic or the runtime be shut down. |
| `src/remote/publisher.rs` 634-641 | `unreachable-in-this-environment` (8 lines) | the activity packet's two failure arms: `Ok(false)` (the secure publish reports "not sent") and `Err` (it fails), with `HEARTBEAT_PUBLISH_EPISODE.record`. A seam would have to inject a failing `secure::publish`; the test transport seals and delivers every packet. |
| `src/remote/publisher.rs` 646, 650 | **`OPEN`** (2 lines) | the two oscillators this record has carried since TAU-D11, now `DA:0`: `:646` is the closing brace of the activity-publish block (the block ran — `:649` 4764 hits), `:650` is the `continue` on a `None` workspaces read. Both read `DA:1` in `union-refresh4` with **byte-identical source** (empty `git diff 16ea6f388 5ad40491f -- publisher.rs`), so the covering run exists; none is deterministic. Both readings are in §15.3's oscillation table. |
| `src/remote/supervisor/mod.rs` 61-67 | **`OPEN`** (7 lines) | `takeover`'s success path after the `device_offline` guard: `claim_control_soon`, the standby notice to the previous holder, `publish_control` and `status()`. The suite's only call — `an_offline_device_cannot_take_control` (`tests.rs:3583`) — takes the refusal at `:59-61`. Fixture: a live runtime installed for the standby device (`install_supervisor_for_test`) plus a secure channel to publish on. **Never registered before this pass.** |
| `src/remote/supervisor/shutdown.rs` 308 | `attribution-artifact` (1 line) | the closing brace of `if transition.rotate_epoch` (`:303-308`): the rotation body runs 12× and the brace carries the else — a resume that needs no epoch rotation. No statement on the line. |
| `src/remote/supervisor/state.rs` 304, 306, 322 | `unreachable-in-this-environment` (3 lines) | `pairing::set_active`'s write failing in `claim_control` (`:304`), the brace of that error arm (`:306`), and the same failure on `claim_control_soon`'s detached thread (`:322`). The write goes through `config_io` into the writable test `HOME`; `INJECT_SAVE_FAILURE` covers `save_creds` only (`pairing.rs:168-176`). `was 278, 280, 296` (+26). |
| `src/remote/supervisor/status.rs` 187 | `unreachable-by-construction` (1 line) | `status_focused`'s `None => return stopped_status(prefer)`: the function returns early when `guard` is empty (`:143-147`), and a non-empty `guard` puts at least one device into `devices` (`:166-176`), so `devices.first()` is always `Some`. |
| `src/remote/supervisor/start.rs` 76, 111 | `unreachable-in-this-environment` (2 lines) | `start_generation` returning `empty()` when `start_requested` is cleared between two awaits (after `establish_slots`, and between two slots of the per-slot loop). Only `perform_stop`/`stop` writes that flag, and both then tear the runtime down, so no fixture can clear it inside the same synchronous step. |
| `src/remote/supervisor/start.rs` 116 | `unreachable-by-construction` (1 line) | `(0, None) => Ok(empty())` in `start_generation`'s tail: `started == 0 && failure.is_none()` needs `plans` to be empty, and `establish_slots()` either errors or pushes at least one plan (it mints an invitation when the book is empty, `:212-222`). |
| `src/remote/supervisor/start.rs` 282, 294, 401 | `unreachable-in-this-environment` (3 lines) | the three epoch/flag checks after a network await: `:282` `start_requested` cleared after `connect_nats`, `:294` and `:401` `access.commit(epoch, …)` returning `None` because a `stop()` invalidated the epoch between statements (the second is the `let Some(previous) = previous else` form). The child is the only writer and cannot interleave inside a synchronous block. |
| `src/remote/supervisor/start.rs` 469-470 | `unreachable-in-this-environment` (2 lines) | the retry worker's re-arm (`if start_requested && retryable_start_status(&latest) { spawn_start_retry() }`): every attempt writes `last_error_code = "connecting"` before it asks anything, so the status the tail re-reads is `Connecting`, which is not retryable. Satisfying it needs a `stop()`→`start()` pair inside the few statements between the break and `:468`. |
| `src/remote/supervisor/start.rs` 497-498 | `unreachable-in-this-environment` (2 lines) | `spawn_runtime_reconnect`'s attempt budget (`if attempt > MAX_RUNTIME_RECONNECT_ATTEMPTS`): the counter is incremented only by `record_runtime_failure`, whose only caller is this function, and every caller first reads it behind `>= MAX` at `:627` and returns. Reaching `attempt == MAX + 1` needs two watchers' load/record to interleave inside a tick. No test can pre-arm it (`record_runtime_failure` is `pub(in crate::remote)`). |
| `src/remote/supervisor/start.rs` 595, 603 | `unreachable-in-this-environment` (2 lines) | `GenerationWatch`'s two teardown-racing exits: `:595` the `start_requested` check one tick later (the watch is aborted first), `:603` the `let Some(state)` else (the entry vanished or its generation changed between the tick and the lookup). |
| `src/remote/supervisor/start.rs` 669-671, 673-679, 682-701, 703-704, 707-710 | `unreachable-in-this-environment` (36 lines) | `spawn_web_reconnect`'s `#[cfg(not(test))]` body — cfg-split, so the test build compiles only `let _ = pair_id;` (`:661-665`, covered) — and the bin copy is enabled only by the **staging** platform URL (`web_client_enabled()` is true only for `https://test.future-os.cn`, `web_server.rs:41`) plus a failed `bind_web_listener()`. A loopback fixture platform selects the other arm. |
| `src/remote/supervisor/start.rs` 739 | `unreachable-by-construction` (1 line) | the `_ => RemoteFailureReason::Local` arm of `start_failure`'s reason match: `pairing::error_code` returns exactly `network`, `revoked`, `account_authorization`, `service_authorization`, `server` or `None`, and each has its own arm above, so no `Some(code)` reaches `_`. |
| `src/remote/supervisor/start.rs` 554 | **`OPEN`** (1 line) | the success arm of `restart_slot`: `Ok(()) => Ok(status_focused(Some(pair_id)))`. It is **not** unreachable — the sibling measurement of the identical source read `DA:2` in `union-refresh4` and the neighbouring arms ran here too (`:552` 2, `:555` 1) — so this is an oscillation, not an obstacle. Seam that would make it deterministic: a child that restarts a slot it knows is *healthy* (the existing children all restart one whose rebuild fails). Registered as OPEN with both readings (§15.3's oscillation table) rather than given a mechanism that the other run would falsify. |
| `src/remote/secure.rs` 814 | `unreachable-by-construction` (1 line) | `Ok(_) => panic!("an inert transport must not open")`: the guard for the invariant asserted at `:811-813`; a green run cannot enter it. |
| `src/remote/transfer.rs` 44-48 | `unreachable-in-this-environment` (5 lines) | the transfer subscription's `queue_subscribe` error arm: the fake broker accepts every queue. Needs a broker that refuses on demand. |
| `src/remote/transfer.rs` 103 | `unreachable-in-this-environment` (1 line) | the if-let's implicit else on `reply_security.seal(&bytes)`: the transfer reply is dropped when the channel cannot seal, which the test transport never does. |
| `src/remote/transport.rs` 243 | `unreachable-in-this-environment` (1 line) | the refresh worker returning when its generation died between two awaits: only a concurrent `stop()`/replacement writes that state, and it cannot land between them from a test. |
| `src/remote/transport.rs` 334 | `attribution-artifact` (1 line) | the `};` closing the `connected_nats` match: its `Err` arm (`:320-333`) is covered by §12's rebuild-failure test, and this run attributes the region end to the brace. |
| `src/remote/transport.rs` 353-360 | `unreachable-in-this-environment` (8 lines) | aborting the freshly built transport tasks when the pairing's generation changed under the state lock: needs the concurrent replacement above inside the refresh window. |
| `src/remote/trust.rs` 105-110 | `unreachable-in-this-environment` (6 lines) | `load_platform_certs`' error return: `rustls_native_certs::load_native_certs()` reporting errors, i.e. a failing platform trust-store read — the host fault the snapshot exists to survive, not a fixture input. The success path (`:111-117`) and the non-test `bundle_path` are covered by the child fixture (§4.1). |
| `src/remote/commands.rs` 332 | `attribution-artifact` (1 line) | the closing brace of `if matches!(activated, Some(Ok(())))` (`:328-332`) on the secure-handshake path: the reply block runs 510× and the brace carries the else — an `activated` that is `None` (stale epoch) or `Some(Err(_))`, which no fixture produces for this call. |
| `src/remote/commands.rs` 367 | `unreachable-in-this-environment` (1 line) | the same handshake's `None => return` when `access.commit(epoch, process)` finds the epoch invalidated between two statements: only a concurrent `stop()`/power transition writes it. |
| `src/remote/commands.rs` 409 | **`OPEN`** (1 line) | the standby notice in the secure-handshake takeover (`claim_control_soon` at `:407`): `Some(previous)` is a *different* device that held control. Fixture: device B's `secure_open` while device A is the controller and holds a secure channel to be told. **Never registered** (new in the multi-device work). |
| `src/remote/commands.rs` 519 | `attribution-artifact` (1 line) | the closing brace of `if inserted` (`:505-519`) in the single-flight path: the TTL task inside runs (37× at `:511-518`) and the brace carries the else — a duplicate command id already in flight. `was 510` (+9). |
| `src/remote/commands.rs` 916 | `unreachable-in-this-environment` (1 line) | the pairing-confirmation handshake's `None => return` when `access.commit(epoch, …)` finds the epoch invalidated between two statements: only a concurrent `stop()`/power transition writes it. `was 907` (+9). |
| `src/remote/commands.rs` 962 | **`OPEN`** (1 line) | the standby notice in the *reconnect* path (`claim_control_soon` at `:960`): the same fixture as `:409` with a reconnect handshake (`pairing: false`). **Never registered.** |
| `src/remote/commands.rs` 1017, 1020 | `unreachable-by-construction` (2 lines) | `write_all` and `GzEncoder::finish` into an in-memory `Vec<u8>`: `impl io::Write for Vec<u8>` cannot fail. `was 1004, 1007` (+13). |
| `src/remote/commands.rs` 1023 | `unreachable-by-construction` (1 line) | "compression did not shrink": `plain` is `serde_json` text of at least `REMOTE_JSON_GZIP_THRESHOLD_BYTES` (32 KiB), whose per-byte entropy is bounded below 8 bits/byte, while `compressed.len() >= plain.len()` needs ≥ 8 bits/byte plus gzip's 18-byte header/trailer. §4.1 records the measured ratios (base64 0.77, hex 0.59, distinct floats 0.49, printable ASCII 0.83, CJK 0.74). `was 1010` (+13). |
| `src/remote/commands.rs` 1038 | `unreachable-in-this-environment` (1 line) | a `REPLY` scope whose `seal` returns `Err`: the effective transport seals every reply, and the oversized-reply case is refused earlier at `:1004`. `was 1025` (+13). |
| `src/remote/commands.rs` 1041 | `unreachable-in-this-environment` (1 line) | the `#[cfg(not(test))]` return of the "no REPLY scope" arm: cfg-split — the test build falls through to the plain payload, and in the non-test child every reply is published inside a scope. `was 1028` (+13). |
| `src/remote/commands.rs` 1053 | `unreachable-in-this-environment` (1 line) | `flush()` failing on a healthy in-process broker. `was 1040` (+13). |
| `src/remote/test_support.rs` 278-288, 290-293, 295-300, 302-312, 314-323 | **`OPEN`** (42 lines) | `reconnect_secure_channel`, the IK reconnect fixture: no test calls it. Every secure fixture opens a *pairing* handshake (`secure_open` with `pairing: true`), and the reconnect pattern is the one a client uses once a pairing exists. Fixture: one test that reconnects with the pinned desktop key (the fake broker plus a stored pairing identity). **Never registered** — and invisible to `reconcile.py` (see the false-clean note above). |
| `src/remote/test_support.rs` 763, 768 | `platform-unmeasured` (2 lines) | the mock's non-macOS `probe_sandbox` answers (`"unsupported_platform"`, `"none"`): the mock reports what its own host has, and this host is **macOS**. `was 689, 694` (+74). |
| `src/remote/test_support.rs` 1885, 1899, 2572 | `unreachable-by-construction` (3 lines) | `:1885` pushes a violation when a body takes `HomeGuard::new` before `mock_agent_lock(`, `:1899` formats the list only when the `assert!` fails, and `:2572` is the `Err(error) => panic!("expected the deferred arm…")` regression guard. A green run cannot enter any of them. `was 1811, 1825, 2498` (+74). |

#### `remote_peer/`

| file and lines | class | mechanism |
|---|---|---|
| `src/remote_peer/runtime.rs` 163-165 | `unreachable-in-this-environment` (3 lines) | `delay_unit()`'s `#[cfg(not(test))]` body (seconds) beside the `#[cfg(test)]` one (milliseconds): cfg-split, and the non-test retry loop does not call `reconnect_delay`, the only caller. `was 152-154` (+11: #1192's `Arc`/`session_handle` prose above it). |
| `src/remote_peer/runtime.rs` 1707 | `unreachable-in-this-environment` (1 line) | `reassemble`'s `remote_read_too_large` guard on `totalBytes > MAX_PAGE_BYTES`: the in-process host computes `totalBytes` from the page it built and caps it, so a test cannot make it lie without changing product code. `was 1659` (+48). |
| `src/remote_peer/runtime.rs` 1771 | `unreachable-in-this-environment` (1 line) | `store_chunk`'s id/`totalBytes`/offset check: every chunk the fake host sends matches the snapshot the first reply pinned. `was 1723` (+48). |
| `src/remote_peer/runtime.rs` 1784 | `unreachable-in-this-environment` (1 line) | `store_chunk`'s empty/short-chunk rule: the host's chunks are full-sized except the last. `was 1736` (+48). |
| `src/remote_peer/runtime.rs` 1809-1811 | `unreachable-in-this-environment` (3 lines) | `page_shape_error`'s body: every call site is one of the unfixtured guards above (`:1707`, `:1767-1771`, `:1777-1784`, `:1787-1788`) or the `chunk_*` helpers' `ok_or_else`, which a well-formed page never reaches. `was 1761-1763` (+48). |
| `src/remote_peer/runtime.rs` 2194 | `attribution-artifact` (1 line) | the `});` that closes the `tokio::spawn` in **#1192's own test**, `a_command_in_flight_does_not_block_the_peer_list` (`:2188-2194`): the spawned request runs (`:2192`, `:2193` read 1) while the test asserts `list()` answers, then the future is dropped when the test's runtime shuts down, so the block's closing region is never reached. New code in #1192; first registration here. |
| `src/remote_peer/runtime.rs` 2811 | `attribution-artifact` (1 line) | the closing brace of the `visibility_change_for_test` wrapper: `:2807-2810` are covered (1×) and the coroutine's exit region is attributed to the brace. `was 2782` (+29). |
| `src/remote_peer/session.rs` 775, 781 | `unreachable-in-this-environment` (2 lines) | `:775` the `Some(bundle)` arm of `trust::root_bundle()`: the test build's `bundle_path()` is the constant `None` (`trust.rs:49-52`), so the `None` arm runs (`:774` 145, `:776` 145) and this one never does. `:781` `options.require_tls(true)` under `#[cfg(not(test))]`: cfg-split — the test build connects to the in-process plaintext fake broker by design (`:782` 145). `was 767, 773` (+8). |

### 15.4 The arithmetic, so a reader can check it without the tool

    1504 uncovered
    - 788  real_network.rs envelope          (registered, 15.1 + 15.5)
    - 433  verify_e2e.rs envelope            (registered, 15.2 + 15.5)
    ------
     283  production lines, registered per line in 15.3
            = 279  registered at the previous pin (16ea6f388), of which 12 moved
                   inside `remote_peer` and 3 oscillated back to `DA:0`
            +   4  new at this pin: start.rs 554, publisher.rs 646/650 (the three
                   oscillators) and runtime.rs 2194 (#1192's own new test)
    ------
       0  MISSING

**How the 16ea6f388 registrations were arrived at** (kept because it is the tool's behaviour, not
this pin's arithmetic): the earlier version of this section said "279 − 146 carried by rows written
before the merge = 133 MISSING". The 146 was not rows: 94 of the lines that `reconcile.py` counted as
mentioned were matched by *numbers*, not by ledger rows — §15.1's heading range `50-1065` expands to
every number in that span for every file, and the old per-file table named the three files
(`test_support.rs`, `sync_measurement.rs`, `commands.rs`) whose uncovered lines sit inside it. Those 94
lines were therefore neither registered nor reported. The heading now says "lines 50 to 1065" and every
line has a row above, so the MISSING count and the ledger agree.

The 133 that `reconcile.py` did report at that pin were the lines outside the accidental net: `113`
rows-at-their-numbers that were not named in §15, plus the `20` of the 114 moved/new lines whose numbers
fall outside `50-1065` (`state.rs` 3, `test_support.rs` 1885/1899/2572, `history.rs` 267,
`remote_peer/runtime.rs` 2782 at that pin, `presence.rs` 78-81/134, `supervisor/mod.rs` 61-67).

**The identity a reader can check with the tool.** 1504 distinct `(file, line)` claims for 1504
uncovered lines — §15.5's two envelope rows claim 1221 of them, §15.3's rows the other 283 — and
`reconcile.py` reports MISSING 0, COVERED 0, ABSENT 0 against the invocation in the pairing entry. The
claims are the report's own `DA:0` set: there is no line in this section that the union shows covered or
record-less. The tripwire parses **1507** claim *pairs* rather than 1504 because the three oscillating
lines are cited twice — once in §15.3's per-file row and once in the oscillation table — and the two
spellings (`remote/publisher.rs` vs `src/remote/publisher.rs`) collapse to the same `(file, line)` when
resolved. That is why `expect.claims` in the pairing entry records 1507 while `expect.uncovered`
records 1504.

### 15.5 The two envelope claims, derived from the union

Every range below is generated from `union-refresh5.info` (a line is in the range iff its `DA:0`),
so the claim and the report cannot drift apart without the gate saying so. **Regenerated at this pin and
byte-identical to the previous pin's rows**: neither file is touched by #1192, and #1193's dwell is in
`tests/remote_start_refusal.rs`, outside these scopes. The count and ranges are the generator's output,
not a carried-over cell (`.future/cov100/pinned/d31/gen-envelope.py`).

| file | lines | class | mechanism |
|---|---|---|---|
| `src/remote/real_network.rs` 50-63, 66-68, 70-72, 74-76, 78-86, 88-92, 95-118, 120-126, 128-130, 132-133, 135-136, 138-146, 148, 152-161, 163-169, 171, 174-184, 188-190, 193, 196-199, 203-206, 210-213, 215-228, 230, 233-236, 240-253, 255-258, 260-263, 265, 267, 269, 272-278, 280, 282-283, 287-295, 298-307, 309-332, 334-343, 346-347, 349-353, 360-368, 371, 373-377, 381-388, 391, 393-397, 402-409, 411-414, 423-438, 440, 443-465, 467, 472-498, 500-507, 509-514, 518-528, 530-532, 548-561, 563-589, 591-616, 618-624, 626-632, 634-637, 639-648, 658-664, 667-679, 681-686, 688, 691-711, 715-727, 729-732, 734, 736, 739-755, 757-761, 768-774, 776-782, 784-788, 791, 794-803, 805-833, 836-842, 844, 846, 852-862, 864, 869-889, 892-902, 904, 906-910, 912-914, 916-918, 921-922, 925, 930-934, 936-939, 941, 944-949, 953-954, 956, 958, 960, 963-968, 971-979, 981-990, 992, 994, 996-997, 1000-1001, 1009-1016, 1018-1019, 1021-1022, 1025-1026, 1030-1031, 1034-1041, 1043-1045, 1047-1053, 1055-1056, 1058-1065 | 788 | `unreachable-in-this-environment` | the real-network harness: `#[ignore]`d, drives a real `nats-server` and agent process (15.1) |
| `src/remote/verify_e2e.rs` 350-353, 357-360, 365-376, 379-385, 404-417, 419-450, 453, 456-458, 1976, 2174, 2559, 2605, 2642-2653, 2657-2668, 2670-2674, 2676-2677, 2679-2688, 2693-2698, 2700-2713, 2716-2718, 2723-2729, 2731-2744, 2747-2752, 2754-2756, 2759, 2761, 2769-2770, 2772-2776, 2778-2786, 2788, 2793-2798, 2802-2803, 2805-2812, 2817-2822, 2824-2828, 2830-2837, 2842-2858, 2862-2867, 2870-2879, 2890, 2893-2894, 2897-2900, 2904-2905, 2908-2910, 2913-2919, 2921-2922, 2926-2931, 2934-2935, 2939, 2953-2954, 2957-2965, 2970-2979, 2984-2985, 2987-2988, 2991-2992, 2996-3000, 3003, 3005, 3007-3008, 3011-3020, 3023-3025, 3028-3029, 3034-3041, 3044-3047, 3049-3051, 3053-3058, 3061-3062, 3065, 3072, 3075-3077, 3080-3081, 3084-3085, 3089-3092, 3096-3097, 3100-3106, 3108-3110, 3131-3132, 3134-3139, 3142, 3144, 3148-3150, 3152-3155, 3160-3163, 3165, 3168-3169, 3172-3173, 3176-3177, 3181-3191, 3193-3195 | 433 | `unreachable-in-this-environment` | the cross-process `InstalledHost` fixtures: `#[ignore]`d, spawn the real binary (15.2) |

### 15.6 What this section does NOT claim

* It does not claim the module is at any particular percentage: **1504 uncovered, of which 1221 are
  two documented envelopes**.
* It does not re-derive the per-line table in §4. §4.1 is the D26 ledger at `0b7b2ca17` and stays
  that; §15.3 carries the current numbering, and its rows restate the mechanism rather than
  pointing at a number that has since moved.
* It does not fix anything. It records, so that the next reader starts from the measurement instead
  of from a banner.
* It does not claim the figure describes the branch tip. It describes **`5ad40491f`, `origin/main`'s
  tip** — the tree F.108 compares against, measured in a detached worktree at that commit. The branch
  additionally carries TAU-F6's `a9c383e45` (a test plus the `exchange_within` split), which `main`
  does not have; when F6 lands the rows move by the offsets §15.3's `was` notes predict, and the only
  number that changes is the `mod tests` brace.
* It does not claim the union is a quiet-host run. The gui leg's three port-8022 failures are recorded
  in the header, their re-run is folded in, and the three oscillating lines are named with both
  readings — a reader who wants a single clean rc can reproduce the leg from
  `.future/cov100/pinned/d31/measure2.sh` and `measure3.sh`.
